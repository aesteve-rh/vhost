// Copyright (C) 2019-2021 Alibaba Cloud. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use libc;
use std::mem;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use super::connection::Endpoint;
use super::message::*;
use super::{Error, HandlerResult, Result};

/// Define services provided by masters for the slave communication channel.
///
/// The vhost-user specification defines a slave communication channel, by which slaves could
/// request services from masters. The [VhostUserMasterReqHandler] trait defines services provided
/// by masters, and it's used both on the master side and slave side.
/// - on the slave side, a stub forwarder implementing [VhostUserMasterReqHandler] will proxy
///   service requests to masters. The [SlaveFsCacheReq] is an example stub forwarder.
/// - on the master side, the [MasterReqHandler] will forward service requests to a handler
///   implementing [VhostUserMasterReqHandler].
///
/// The [VhostUserMasterReqHandler] trait is design with interior mutability to improve performance
/// for multi-threading.
///
/// [VhostUserMasterReqHandler]: trait.VhostUserMasterReqHandler.html
/// [MasterReqHandler]: struct.MasterReqHandler.html
/// [SlaveFsCacheReq]: struct.SlaveFsCacheReq.html
pub trait VhostUserMasterReqHandler {
    /// Handle device configuration change notifications.
    fn handle_config_change(&self) -> HandlerResult<u64> {
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// Handle virtio-fs map file requests.
    fn fs_slave_map(&self, _fs: &VhostUserFSSlaveMsg, fd: RawFd) -> HandlerResult<u64> {
        // Safe because we have just received the rawfd from kernel.
        unsafe { libc::close(fd) };
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// Handle virtio-fs unmap file requests.
    fn fs_slave_unmap(&self, _fs: &VhostUserFSSlaveMsg) -> HandlerResult<u64> {
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// Handle virtio-fs sync file requests.
    fn fs_slave_sync(&self, _fs: &VhostUserFSSlaveMsg) -> HandlerResult<u64> {
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// Handle virtio-fs file IO requests.
    fn fs_slave_io(&self, _fs: &VhostUserFSSlaveMsg, fd: RawFd) -> HandlerResult<u64> {
        // Safe because we have just received the rawfd from kernel.
        unsafe { libc::close(fd) };
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    // fn handle_iotlb_msg(&mut self, iotlb: VhostUserIotlb);
    // fn handle_vring_host_notifier(&mut self, area: VhostUserVringArea, fd: RawFd);
}

/// A helper trait mirroring [VhostUserMasterReqHandler] but without interior mutability.
///
/// [VhostUserMasterReqHandler]: trait.VhostUserMasterReqHandler.html
pub trait VhostUserMasterReqHandlerMut {
    /// Handle device configuration change notifications.
    fn handle_config_change(&mut self) -> HandlerResult<u64> {
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// Handle virtio-fs map file requests.
    fn fs_slave_map(&mut self, _fs: &VhostUserFSSlaveMsg, fd: RawFd) -> HandlerResult<u64> {
        // Safe because we have just received the rawfd from kernel.
        unsafe { libc::close(fd) };
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// Handle virtio-fs unmap file requests.
    fn fs_slave_unmap(&mut self, _fs: &VhostUserFSSlaveMsg) -> HandlerResult<u64> {
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// Handle virtio-fs sync file requests.
    fn fs_slave_sync(&mut self, _fs: &VhostUserFSSlaveMsg) -> HandlerResult<u64> {
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// Handle virtio-fs file IO requests.
    fn fs_slave_io(&mut self, _fs: &VhostUserFSSlaveMsg, fd: RawFd) -> HandlerResult<u64> {
        // Safe because we have just received the rawfd from kernel.
        unsafe { libc::close(fd) };
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    // fn handle_iotlb_msg(&mut self, iotlb: VhostUserIotlb);
    // fn handle_vring_host_notifier(&mut self, area: VhostUserVringArea, fd: RawFd);
}

impl<S: VhostUserMasterReqHandlerMut> VhostUserMasterReqHandler for Mutex<S> {
    fn handle_config_change(&self) -> HandlerResult<u64> {
        self.lock().unwrap().handle_config_change()
    }

    fn fs_slave_map(&self, fs: &VhostUserFSSlaveMsg, fd: RawFd) -> HandlerResult<u64> {
        self.lock().unwrap().fs_slave_map(fs, fd)
    }

    fn fs_slave_unmap(&self, fs: &VhostUserFSSlaveMsg) -> HandlerResult<u64> {
        self.lock().unwrap().fs_slave_unmap(fs)
    }

    fn fs_slave_sync(&self, fs: &VhostUserFSSlaveMsg) -> HandlerResult<u64> {
        self.lock().unwrap().fs_slave_sync(fs)
    }

    fn fs_slave_io(&self, fs: &VhostUserFSSlaveMsg, fd: RawFd) -> HandlerResult<u64> {
        self.lock().unwrap().fs_slave_io(fs, fd)
    }
}

/// Server to handle service requests from slaves from the slave communication channel.
///
/// The [MasterReqHandler] acts as a server on the master side, to handle service requests from
/// slaves on the slave communication channel. It's actually a proxy invoking the registered
/// handler implementing [VhostUserMasterReqHandler] to do the real work.
///
/// [MasterReqHandler]: struct.MasterReqHandler.html
/// [VhostUserMasterReqHandler]: trait.VhostUserMasterReqHandler.html
pub struct MasterReqHandler<S: VhostUserMasterReqHandler> {
    // underlying Unix domain socket for communication
    sub_sock: Endpoint<SlaveReq>,
    tx_sock: UnixStream,
    // Protocol feature VHOST_USER_PROTOCOL_F_REPLY_ACK has been negotiated.
    reply_ack_negotiated: bool,
    // the VirtIO backend device object
    backend: Arc<S>,
    // whether the endpoint has encountered any failure
    error: Option<i32>,
}

impl<S: VhostUserMasterReqHandler> MasterReqHandler<S> {
    /// Create a server to handle service requests from slaves on the slave communication channel.
    ///
    /// This opens a pair of connected anonymous sockets to form the slave communication channel.
    /// The socket fd returned by [Self::get_tx_raw_fd()] should be sent to the slave by
    /// [VhostUserMaster::set_slave_request_fd()].
    ///
    /// [Self::get_tx_raw_fd()]: struct.MasterReqHandler.html#method.get_tx_raw_fd
    /// [VhostUserMaster::set_slave_request_fd()]: trait.VhostUserMaster.html#tymethod.set_slave_request_fd
    pub fn new(backend: Arc<S>) -> Result<Self> {
        let (tx, rx) = UnixStream::pair().map_err(Error::SocketError)?;

        Ok(MasterReqHandler {
            sub_sock: Endpoint::<SlaveReq>::from_stream(rx),
            tx_sock: tx,
            reply_ack_negotiated: false,
            backend,
            error: None,
        })
    }

    /// Get the socket fd for the slave to communication with the master.
    ///
    /// The returned fd should be sent to the slave by [VhostUserMaster::set_slave_request_fd()].
    ///
    /// [VhostUserMaster::set_slave_request_fd()]: trait.VhostUserMaster.html#tymethod.set_slave_request_fd
    pub fn get_tx_raw_fd(&self) -> RawFd {
        self.tx_sock.as_raw_fd()
    }

    /// Set the negotiation state of the `VHOST_USER_PROTOCOL_F_REPLY_ACK` protocol feature.
    ///
    /// When the `VHOST_USER_PROTOCOL_F_REPLY_ACK` protocol feature has been negotiated,
    /// the "REPLY_ACK" flag will be set in the message header for every slave to master request
    /// message.
    pub fn set_reply_ack_flag(&mut self, enable: bool) {
        self.reply_ack_negotiated = enable;
    }

    /// Mark endpoint as failed or in normal state.
    pub fn set_failed(&mut self, error: i32) {
        if error == 0 {
            self.error = None;
        } else {
            self.error = Some(error);
        }
    }

    /// Main entrance to server slave request from the slave communication channel.
    ///
    /// The caller needs to:
    /// - serialize calls to this function
    /// - decide what to do when errer happens
    /// - optional recover from failure
    pub fn handle_request(&mut self) -> Result<u64> {
        // Return error if the endpoint is already in failed state.
        self.check_state()?;

        // The underlying communication channel is a Unix domain socket in
        // stream mode, and recvmsg() is a little tricky here. To successfully
        // receive attached file descriptors, we need to receive messages and
        // corresponding attached file descriptors in this way:
        // . recv messsage header and optional attached file
        // . validate message header
        // . recv optional message body and payload according size field in
        //   message header
        // . validate message body and optional payload
        let (hdr, rfds) = self.sub_sock.recv_header()?;
        let rfds = self.check_attached_rfds(&hdr, rfds)?;
        let (size, buf) = match hdr.get_size() {
            0 => (0, vec![0u8; 0]),
            len => {
                if len as usize > MAX_MSG_SIZE {
                    return Err(Error::InvalidMessage);
                }
                let (size2, rbuf) = self.sub_sock.recv_data(len as usize)?;
                if size2 != len as usize {
                    return Err(Error::InvalidMessage);
                }
                (size2, rbuf)
            }
        };

        let res = match hdr.get_code() {
            SlaveReq::CONFIG_CHANGE_MSG => {
                self.check_msg_size(&hdr, size, 0)?;
                self.backend
                    .handle_config_change()
                    .map_err(Error::ReqHandlerError)
            }
            SlaveReq::FS_MAP => {
                let msg = self.extract_msg_body::<VhostUserFSSlaveMsg>(&hdr, size, &buf)?;
                // check_attached_rfds() has validated rfds
                self.backend
                    .fs_slave_map(&msg, rfds.unwrap()[0])
                    .map_err(Error::ReqHandlerError)
            }
            SlaveReq::FS_UNMAP => {
                let msg = self.extract_msg_body::<VhostUserFSSlaveMsg>(&hdr, size, &buf)?;
                self.backend
                    .fs_slave_unmap(&msg)
                    .map_err(Error::ReqHandlerError)
            }
            SlaveReq::FS_SYNC => {
                let msg = self.extract_msg_body::<VhostUserFSSlaveMsg>(&hdr, size, &buf)?;
                self.backend
                    .fs_slave_sync(&msg)
                    .map_err(Error::ReqHandlerError)
            }
            SlaveReq::FS_IO => {
                let msg = self.extract_msg_body::<VhostUserFSSlaveMsg>(&hdr, size, &buf)?;
                // check_attached_rfds() has validated rfds
                self.backend
                    .fs_slave_io(&msg, rfds.unwrap()[0])
                    .map_err(Error::ReqHandlerError)
            }
            _ => Err(Error::InvalidMessage),
        };

        self.send_ack_message(&hdr, &res)?;

        res
    }

    fn check_state(&self) -> Result<()> {
        match self.error {
            Some(e) => Err(Error::SocketBroken(std::io::Error::from_raw_os_error(e))),
            None => Ok(()),
        }
    }

    fn check_msg_size(
        &self,
        hdr: &VhostUserMsgHeader<SlaveReq>,
        size: usize,
        expected: usize,
    ) -> Result<()> {
        if hdr.get_size() as usize != expected
            || hdr.is_reply()
            || hdr.get_version() != 0x1
            || size != expected
        {
            return Err(Error::InvalidMessage);
        }
        Ok(())
    }

    fn check_attached_rfds(
        &self,
        hdr: &VhostUserMsgHeader<SlaveReq>,
        rfds: Option<Vec<RawFd>>,
    ) -> Result<Option<Vec<RawFd>>> {
        match hdr.get_code() {
            SlaveReq::FS_MAP | SlaveReq::FS_IO => {
                // Expect an fd set with a single fd.
                match rfds {
                    None => Err(Error::InvalidMessage),
                    Some(fds) => {
                        if fds.len() != 1 {
                            Endpoint::<SlaveReq>::close_rfds(Some(fds));
                            Err(Error::InvalidMessage)
                        } else {
                            Ok(Some(fds))
                        }
                    }
                }
            }
            _ => {
                if rfds.is_some() {
                    Endpoint::<SlaveReq>::close_rfds(rfds);
                    return Err(Error::InvalidMessage);
                } else {
                    Ok(rfds)
                }
            }
        }
    }

    fn extract_msg_body<T: Sized + VhostUserMsgValidator>(
        &self,
        hdr: &VhostUserMsgHeader<SlaveReq>,
        size: usize,
        buf: &[u8],
    ) -> Result<T> {
        self.check_msg_size(hdr, size, mem::size_of::<T>())?;
        let msg = unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const T) };
        if !msg.is_valid() {
            return Err(Error::InvalidMessage);
        }
        Ok(msg)
    }

    fn new_reply_header<T: Sized>(
        &self,
        req: &VhostUserMsgHeader<SlaveReq>,
    ) -> Result<VhostUserMsgHeader<SlaveReq>> {
        if mem::size_of::<T>() > MAX_MSG_SIZE {
            return Err(Error::InvalidParam);
        }
        self.check_state()?;
        Ok(VhostUserMsgHeader::new(
            req.get_code(),
            VhostUserHeaderFlag::REPLY.bits(),
            mem::size_of::<T>() as u32,
        ))
    }

    fn send_ack_message(
        &mut self,
        req: &VhostUserMsgHeader<SlaveReq>,
        res: &Result<u64>,
    ) -> Result<()> {
        if self.reply_ack_negotiated && req.is_need_reply() {
            let hdr = self.new_reply_header::<VhostUserU64>(req)?;
            let def_err = libc::EINVAL;
            let val = match res {
                Ok(n) => *n,
                Err(e) => match &*e {
                    Error::ReqHandlerError(ioerr) => match ioerr.raw_os_error() {
                        Some(rawerr) => -rawerr as u64,
                        None => -def_err as u64,
                    },
                    _ => -def_err as u64,
                },
            };
            let msg = VhostUserU64::new(val);
            self.sub_sock.send_message(&hdr, &msg, None)?;
        }
        Ok(())
    }
}

impl<S: VhostUserMasterReqHandler> AsRawFd for MasterReqHandler<S> {
    fn as_raw_fd(&self) -> RawFd {
        self.sub_sock.as_raw_fd()
    }
}
