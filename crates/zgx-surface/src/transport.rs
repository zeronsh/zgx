//! Frames between the two processes, with their handles.
//!
//! - **Unix**: one stream socket of a `socketpair`, the client's end
//!   inherited by the program the host starts, which finds it by its number
//!   in `ZGX_SURFACE_FD`. Handles are file descriptors, passed with
//!   `SCM_RIGHTS` beside their frame's bytes; the reader keeps those that
//!   arrive in order and hands each frame as many as its header counts.
//! - **Windows**: two named pipes, `<name>.down` (host to client) and
//!   `<name>.up`, the client given `<name>` in `ZGX_SURFACE_PIPE` (two, as
//!   one handle's synchronous reads would hold up its writes). Handles are
//!   duplicated into the client (`DuplicateHandle`) by the host, which
//!   started it; their values in the client follow their frame as `u64`s.
use crate::{
    Handle,
    protocol::{self, ClientMessage, HEADER, HostMessage},
};
use std::io;

/// The environment variable a client's end of the socket is named by
/// (Unix: its file descriptor's number).
pub const ENV_FD: &str = "ZGX_SURFACE_FD";
/// The environment variable a client's pipes are named by (Windows).
pub const ENV_PIPE: &str = "ZGX_SURFACE_PIPE";

/// Whether this process was started as a surface's client.
pub fn in_surface() -> bool {
    std::env::var_os(if cfg!(windows) { ENV_PIPE } else { ENV_FD }).is_some()
}

/// Reads whole frames off the connection.
pub struct Receiver {
    inner: imp::Receiver,
    buf: Vec<u8>,
    start: usize,
}
/// Writes frames onto the connection; one writer at a time (a frame's
/// handles must arrive with it).
pub struct Sender {
    inner: imp::Sender,
}

impl Receiver {
    /// The next frame and its handles, blocking until it arrives; `None`
    /// once the other side has closed.
    pub fn recv_frame(&mut self) -> io::Result<Option<(Vec<u8>, Vec<Handle>)>> {
        loop {
            let held = &self.buf[self.start..];
            if held.len() >= HEADER {
                let (len, _, count) = protocol::header(held[..HEADER].try_into().unwrap())?;
                let trailer = imp::TRAILER * count as usize;
                if held.len() >= len + trailer {
                    let frame = held[..len].to_vec();
                    let extra = held[len..len + trailer].to_vec();
                    self.start += len + trailer;
                    let handles = self.inner.handles(count as usize, &extra)?;
                    return Ok(Some((frame, handles)));
                }
            }
            if self.start > 0 {
                self.buf.drain(..self.start);
                self.start = 0;
            }
            let at = self.buf.len();
            self.buf.resize(at + 64 * 1024, 0);
            let read = self.inner.read(&mut self.buf[at..]);
            match read {
                Ok(0) => {
                    self.buf.truncate(at);
                    return Ok(None);
                }
                Ok(n) => self.buf.truncate(at + n),
                Err(error) => {
                    self.buf.truncate(at);
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }
    /// The next message from the host; `None` once it closed. Kinds this
    /// version doesn't know are skipped.
    pub fn recv_host(&mut self) -> io::Result<Option<HostMessage>> {
        loop {
            let Some((frame, handles)) = self.recv_frame()? else {
                return Ok(None);
            };
            if let Some(message) = HostMessage::decode(&frame, handles)? {
                return Ok(Some(message));
            }
        }
    }
    /// The next message from the client; `None` once it closed.
    pub fn recv_client(&mut self) -> io::Result<Option<ClientMessage>> {
        loop {
            let Some((frame, handles)) = self.recv_frame()? else {
                return Ok(None);
            };
            if let Some(message) = ClientMessage::decode(&frame, handles)? {
                return Ok(Some(message));
            }
        }
    }
}
impl Sender {
    /// Write `message`.
    pub fn send_host(&mut self, message: &HostMessage) -> io::Result<()> {
        let encoded = message.encode();
        self.inner.send(&encoded.bytes, &encoded.handles)
    }
    /// Write `message`.
    pub fn send_client(&mut self, message: &ClientMessage) -> io::Result<()> {
        let encoded = message.encode();
        self.inner.send(&encoded.bytes, &encoded.handles)
    }
    /// Stop: the other side reads the end of the connection (Unix: its
    /// write half is shut down).
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

/// The client's side, from its environment (`ENV_FD`, `ENV_PIPE`); the
/// variable is removed, so programs it starts don't take it.
pub fn connect_from_env() -> io::Result<(Sender, Receiver)> {
    let (sender, receiver) = imp::from_env()?;
    Ok(wrap(sender, receiver))
}
fn wrap(sender: imp::Sender, receiver: imp::Receiver) -> (Sender, Receiver) {
    (
        Sender { inner: sender },
        Receiver {
            inner: receiver,
            buf: Vec::new(),
            start: 0,
        },
    )
}

/// Two connected ends, both in this process (tests; a host's own client).
#[cfg(unix)]
pub fn pair() -> io::Result<((Sender, Receiver), (Sender, Receiver))> {
    let (a, b) = imp::socketpair()?;
    let a = imp::split(a)?;
    let b = imp::split(b)?;
    Ok((wrap(a.0, a.1), wrap(b.0, b.1)))
}

/// The host's end of a connection for a program it is about to start, and
/// what the program inherits.
#[cfg(unix)]
pub struct Spawning {
    host: std::os::unix::net::UnixStream,
    child: std::os::fd::OwnedFd,
}
#[cfg(unix)]
impl Spawning {
    /// A new socket pair, both ends closed on exec until `prepare` opens
    /// the child's in the child.
    pub fn new() -> io::Result<Self> {
        let (host, child) = imp::socketpair()?;
        Ok(Self {
            host,
            child: child.into(),
        })
    }
    /// Make `command` inherit the client's end, named in `ENV_FD`.
    pub fn prepare(&self, command: &mut std::process::Command) {
        use std::os::{fd::AsRawFd, unix::process::CommandExt};
        let fd = self.child.as_raw_fd();
        command.env(ENV_FD, fd.to_string());
        // Safety: between fork and exec only async-signal-safe calls run.
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                // The program ends with the host, however the host ends.
                #[cfg(target_os = "linux")]
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    }
    /// The program started: the host's end, the child's closed here.
    pub fn started(self) -> io::Result<(Sender, Receiver)> {
        drop(self.child);
        let (sender, receiver) = imp::split(self.host)?;
        Ok(wrap(sender, receiver))
    }
}

/// The host's end of a connection for a program it is about to start, and
/// what the program inherits (Windows: a pair of named pipes; handles are
/// duplicated into the program once it is started).
#[cfg(windows)]
pub struct Spawning {
    name: String,
    down: Handle,
    up: Handle,
}
#[cfg(windows)]
impl Spawning {
    /// New pipes, named for this process and a count.
    pub fn new() -> io::Result<Self> {
        let (name, down, up) = imp::pipes()?;
        Ok(Self { name, down, up })
    }
    /// Make `command` find the pipes, named in `ENV_PIPE`.
    pub fn prepare(&self, command: &mut std::process::Command) {
        command.env(ENV_PIPE, &self.name);
    }
    /// The program started (`process` its handle, to duplicate handles
    /// into): the host's ends, which connect as the program opens them.
    pub fn started(
        self,
        process: std::os::windows::io::RawHandle,
    ) -> io::Result<(Sender, Receiver)> {
        let (sender, receiver) = imp::host_ends(self.down, self.up, process)?;
        Ok(wrap(sender, receiver))
    }
}

#[cfg(unix)]
mod imp {
    use crate::Handle;
    use std::{
        collections::VecDeque,
        io, mem,
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
            unix::net::UnixStream,
        },
        ptr,
    };

    /// Bytes after a frame for each handle: none, they ride beside it.
    pub const TRAILER: usize = 0;
    /// The most descriptors one read takes (a generation's ring is three).
    const MAX_FDS: usize = 32;

    pub struct Sender(UnixStream);
    pub struct Receiver {
        stream: UnixStream,
        fds: VecDeque<OwnedFd>,
    }

    pub fn socketpair() -> io::Result<(UnixStream, UnixStream)> {
        let mut fds = [0 as RawFd; 2];
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
        let kind = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
        let kind = libc::SOCK_STREAM;
        if unsafe { libc::socketpair(libc::AF_UNIX, kind, 0, fds.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // Safety: both are new descriptors this owns.
        let (a, b) = unsafe {
            (
                UnixStream::from_raw_fd(fds[0]),
                UnixStream::from_raw_fd(fds[1]),
            )
        };
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
        for fd in fds {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        Ok((a, b))
    }
    pub fn split(stream: UnixStream) -> io::Result<(Sender, Receiver)> {
        let reader = stream.try_clone()?;
        Ok((
            Sender(stream),
            Receiver {
                stream: reader,
                fds: VecDeque::new(),
            },
        ))
    }
    pub fn from_env() -> io::Result<(Sender, Receiver)> {
        let value = std::env::var(super::ENV_FD)
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "ZGX_SURFACE_FD is not set"))?;
        let fd: RawFd = value.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "ZGX_SURFACE_FD is not a number",
            )
        })?;
        // Safety: `set_var` and `remove_var` race only with other threads
        // reading the environment; a client connects as it starts.
        unsafe { std::env::remove_var(super::ENV_FD) };
        if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
        // Safety: the host handed this process the descriptor to own.
        split(unsafe { UnixStream::from_raw_fd(fd) })
    }

    impl Sender {
        pub fn send(&mut self, bytes: &[u8], handles: &[&Handle]) -> io::Result<()> {
            let mut sent = 0;
            let mut first = true;
            while sent < bytes.len() {
                let fds: Vec<RawFd> = if first {
                    handles.iter().map(|h| h.as_raw_fd()).collect()
                } else {
                    Vec::new()
                };
                match sendmsg(self.0.as_raw_fd(), &bytes[sent..], &fds) {
                    Ok(n) => {
                        sent += n;
                        first = false;
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }
        pub fn shutdown(&self) {
            let _ = self.0.shutdown(std::net::Shutdown::Write);
        }
    }
    impl Receiver {
        pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let (n, fds) = recvmsg(self.stream.as_raw_fd(), buf)?;
            self.fds.extend(fds);
            Ok(n)
        }
        pub fn handles(&mut self, count: usize, _trailer: &[u8]) -> io::Result<Vec<Handle>> {
            if self.fds.len() < count {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a frame's file descriptors didn't arrive with it",
                ));
            }
            Ok(self.fds.drain(..count).collect())
        }
    }
    fn sendmsg(fd: RawFd, bytes: &[u8], fds: &[RawFd]) -> io::Result<usize> {
        let mut iov = libc::iovec {
            iov_base: bytes.as_ptr() as *mut _,
            iov_len: bytes.len(),
        };
        let space = if fds.is_empty() {
            0
        } else {
            unsafe { libc::CMSG_SPACE(mem::size_of_val(fds) as u32) as usize }
        };
        let mut control = vec![0u8; space];
        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        if !fds.is_empty() {
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = space as _;
            // Safety: the control buffer holds one header and `fds`.
            unsafe {
                let cmsg = libc::CMSG_FIRSTHDR(&msg);
                (*cmsg).cmsg_level = libc::SOL_SOCKET;
                (*cmsg).cmsg_type = libc::SCM_RIGHTS;
                (*cmsg).cmsg_len = libc::CMSG_LEN(mem::size_of_val(fds) as u32) as _;
                ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(cmsg).cast(), fds.len());
            }
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let flags = libc::MSG_NOSIGNAL;
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let flags = 0;
        let n = unsafe { libc::sendmsg(fd, &msg, flags) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    fn recvmsg(fd: RawFd, buf: &mut [u8]) -> io::Result<(usize, Vec<OwnedFd>)> {
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        let space =
            unsafe { libc::CMSG_SPACE((MAX_FDS * mem::size_of::<RawFd>()) as u32) as usize };
        let mut control = vec![0u8; space];
        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
        let flags = libc::MSG_CMSG_CLOEXEC;
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
        let flags = 0;
        let n = unsafe { libc::recvmsg(fd, &mut msg, flags) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut fds = Vec::new();
        // Safety: walking the control messages the kernel wrote.
        unsafe {
            let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
            while !cmsg.is_null() {
                if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                    let data = libc::CMSG_DATA(cmsg);
                    let len = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                    for i in 0..len / mem::size_of::<RawFd>() {
                        let raw = ptr::read_unaligned(data.cast::<RawFd>().add(i));
                        #[cfg(not(any(
                            target_os = "linux",
                            target_os = "android",
                            target_os = "freebsd"
                        )))]
                        libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC);
                        fds.push(OwnedFd::from_raw_fd(raw));
                    }
                }
                cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
            }
        }
        if msg.msg_flags & libc::MSG_CTRUNC != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "more file descriptors arrived than one read takes",
            ));
        }
        Ok((n as usize, fds))
    }
}

#[cfg(windows)]
mod imp {
    use crate::Handle;
    use std::{
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
        sync::atomic::{AtomicU32, Ordering},
    };
    use windows_sys::Win32::{
        Foundation::{
            DUPLICATE_SAME_ACCESS, DuplicateHandle, FALSE, GENERIC_READ, GENERIC_WRITE,
            INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
            PIPE_ACCESS_OUTBOUND, ReadFile, WriteFile,
        },
        System::{
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_TYPE_BYTE, PIPE_WAIT,
            },
            Threading::{GetCurrentProcess, GetCurrentProcessId},
        },
    };

    /// Each handle's value in the client follows its frame.
    pub const TRAILER: usize = 8;

    pub struct Sender {
        pipe: OwnedHandle,
        /// The client's process (the host's sender): handles go into it.
        /// None on the client's side, which sends none.
        process: Option<RawHandle>,
        connected: bool,
    }
    // Safety: a process handle is usable from any thread.
    unsafe impl Send for Sender {}
    pub struct Receiver {
        pipe: OwnedHandle,
        connected: bool,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }
    fn check(handle: windows_sys::Win32::Foundation::HANDLE) -> io::Result<OwnedHandle> {
        if handle == INVALID_HANDLE_VALUE || handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // Safety: a new handle this owns.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) })
    }

    pub fn pipes() -> io::Result<(String, Handle, Handle)> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            r"\\.\pipe\zgx-surface-{}-{}",
            unsafe { GetCurrentProcessId() },
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let make = |suffix: &str, access| {
            let path = wide(&format!("{name}.{suffix}"));
            check(unsafe {
                CreateNamedPipeW(
                    path.as_ptr(),
                    access | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    64 * 1024,
                    64 * 1024,
                    0,
                    std::ptr::null(),
                )
            })
        };
        let down = make("down", PIPE_ACCESS_OUTBOUND)?;
        let up = make("up", PIPE_ACCESS_INBOUND)?;
        Ok((name, down, up))
    }
    pub fn host_ends(
        down: Handle,
        up: Handle,
        process: RawHandle,
    ) -> io::Result<(Sender, Receiver)> {
        Ok((
            Sender {
                pipe: down,
                process: Some(process),
                connected: false,
            },
            Receiver {
                pipe: up,
                connected: false,
            },
        ))
    }
    pub fn from_env() -> io::Result<(Sender, Receiver)> {
        let name = std::env::var(super::ENV_PIPE)
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "ZGX_SURFACE_PIPE is not set"))?;
        unsafe { std::env::remove_var(super::ENV_PIPE) };
        let open = |suffix: &str, access| {
            let path = wide(&format!("{name}.{suffix}"));
            check(unsafe {
                CreateFileW(
                    path.as_ptr(),
                    access,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            })
        };
        let down = open("down", GENERIC_READ)?;
        let up = open("up", GENERIC_WRITE)?;
        Ok((
            Sender {
                pipe: up,
                process: None,
                connected: true,
            },
            Receiver {
                pipe: down,
                connected: true,
            },
        ))
    }
    /// Wait for the client to open the host's end of `pipe`.
    fn connect(pipe: &OwnedHandle, connected: &mut bool) -> io::Result<()> {
        if !*connected {
            let ok = unsafe { ConnectNamedPipe(pipe.as_raw_handle() as _, std::ptr::null_mut()) };
            // ERROR_PIPE_CONNECTED: it opened before we waited.
            if ok == 0 && io::Error::last_os_error().raw_os_error() != Some(535) {
                return Err(io::Error::last_os_error());
            }
            *connected = true;
        }
        Ok(())
    }

    impl Sender {
        pub fn send(&mut self, bytes: &[u8], handles: &[&Handle]) -> io::Result<()> {
            connect(&self.pipe, &mut self.connected)?;
            let mut frame = bytes.to_vec();
            for handle in handles {
                let Some(process) = self.process else {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "a client sends no handles",
                    ));
                };
                let mut theirs = std::ptr::null_mut();
                let ok = unsafe {
                    DuplicateHandle(
                        GetCurrentProcess(),
                        handle.as_raw_handle() as _,
                        process as _,
                        &mut theirs,
                        0,
                        FALSE,
                        DUPLICATE_SAME_ACCESS,
                    )
                };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                frame.extend_from_slice(&(theirs as u64).to_le_bytes());
            }
            let mut sent = 0;
            while sent < frame.len() {
                let mut written = 0;
                let ok = unsafe {
                    WriteFile(
                        self.pipe.as_raw_handle() as _,
                        frame[sent..].as_ptr(),
                        (frame.len() - sent) as u32,
                        &mut written,
                        std::ptr::null_mut(),
                    )
                };
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                sent += written as usize;
            }
            Ok(())
        }
        pub fn shutdown(&self) {}
    }
    impl Receiver {
        pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            connect(&self.pipe, &mut self.connected)?;
            let mut read = 0;
            let ok = unsafe {
                ReadFile(
                    self.pipe.as_raw_handle() as _,
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let error = io::Error::last_os_error();
                // ERROR_BROKEN_PIPE: the other side closed.
                if error.raw_os_error() == Some(109) {
                    return Ok(0);
                }
                return Err(error);
            }
            Ok(read as usize)
        }
        pub fn handles(&mut self, count: usize, trailer: &[u8]) -> io::Result<Vec<Handle>> {
            Ok((0..count)
                .map(|i| {
                    let value = u64::from_le_bytes(trailer[i * 8..i * 8 + 8].try_into().unwrap());
                    // Safety: the host duplicated it into this process for it to own.
                    unsafe { OwnedHandle::from_raw_handle(value as RawHandle) }
                })
                .collect())
        }
    }
}
