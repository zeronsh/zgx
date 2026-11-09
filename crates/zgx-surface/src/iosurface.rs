//! IOSurfaces between a host and its client (macOS), and the Mach channel
//! their ports travel over.
//!
//! A Unix socket carries no Mach ports, so a session has a Mach channel
//! beside it, set up as Chromium's `MachPortRendezvous` is: the host checks
//! in a service of an unguessable name with the bootstrap server
//! (`bootstrap_check_in`) and starts the program with the name in
//! `ZGX_SURFACE_MACH`; the client looks it up (`bootstrap_look_up`) and
//! sends the host a send right to a port of its own (`Client::connect` does
//! this before its `Hello`). One service serves every session of the
//! process (launchd keeps a name for the life of the process that checked
//! it in); a check-in reaches only the session that started its sender (the
//! message's audit token names the sender's pid, compared with the child's).
//! From then on each generation of IOSurfaces goes to the client as one
//! Mach message of their ports (`IOSurfaceCreateMachPort`), sent before the
//! socket's `Buffers` naming them, so a client reading the `Buffers` finds
//! the message waiting; it makes each port a surface again
//! (`IOSurfaceLookupFromMachPort`).
//!
//! Why not XPC: an anonymous XPC listener's endpoint travels only over XPC,
//! so it would need the same bootstrap name to start from, and libxpc's
//! dispatch queues for what is one message a resize. Why not global
//! IOSurface ids (`kIOSurfaceIsGlobal`): deprecated, and any process could
//! read or draw into the surfaces by guessing an id.
//!
//! The core has no Metal: `Surface` is the IOSurface (`as_ptr`), for a
//! client on any wgpu (or none) to make its own texture from (`metal` and
//! `wgpu29` do it for objc2-metal and wgpu 29).

/// The environment variable a client's Mach channel is named by (macOS).
pub const ENV_MACH: &str = "ZGX_SURFACE_MACH";

pub use imp::{ClientChannel, HostChannel, Surface};

#[cfg(target_os = "macos")]
mod imp {
    use super::ENV_MACH;
    use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
    use objc2_io_surface::{
        IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceHeight, kIOSurfacePixelFormat,
        kIOSurfaceWidth,
    };
    use std::{
        collections::HashMap,
        ffi::{CString, c_char, c_void},
        fmt, io,
        sync::{Condvar, Mutex, OnceLock},
        time::{Duration, Instant},
    };

    /// An IOSurface: four bytes a pixel, BGRA (`'BGRA'`), premultiplied.
    #[derive(Clone)]
    pub struct Surface(CFRetained<IOSurfaceRef>);
    // Safety: IOSurface's reference counting and its accessors are
    // thread-safe; what draws into it is the GPU's, ordered by the ring.
    unsafe impl Send for Surface {}
    unsafe impl Sync for Surface {}
    impl PartialEq for Surface {
        fn eq(&self, other: &Self) -> bool {
            std::ptr::eq::<IOSurfaceRef>(&*self.0, &*other.0)
        }
    }
    impl Eq for Surface {}
    impl fmt::Debug for Surface {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Surface")
                .field("id", &self.id())
                .field("width", &self.width())
                .field("height", &self.height())
                .finish()
        }
    }
    impl Surface {
        /// A new surface of `width` by `height` BGRA pixels, not global (only
        /// a Mach port to it opens it elsewhere).
        pub fn new_bgra(width: u32, height: u32) -> io::Result<Self> {
            let number = |value: i32| CFNumber::new_i32(value);
            let (w, h, bytes, format) = (
                number(width as i32),
                number(height as i32),
                number(4),
                number(i32::from_be_bytes(*b"BGRA")),
            );
            // Safety: IOSurface's own constant CFStrings.
            let keys: [&CFString; 4] = unsafe {
                [
                    kIOSurfaceWidth,
                    kIOSurfaceHeight,
                    kIOSurfaceBytesPerElement,
                    kIOSurfacePixelFormat,
                ]
            };
            let values: [&CFType; 4] = [&w, &h, &bytes, &format];
            let properties = CFDictionary::from_slices(&keys, &values);
            // Safety: creation properties, each a CFNumber as IOSurface
            // documents for these keys.
            unsafe { IOSurfaceRef::new(properties.as_opaque()) }
                .map(Self)
                .ok_or_else(|| io::Error::other("IOSurfaceCreate failed"))
        }
        /// Its id (`IOSurfaceGetID`).
        pub fn id(&self) -> u32 {
            self.0.id()
        }
        /// Its width in pixels.
        pub fn width(&self) -> u32 {
            self.0.width() as u32
        }
        /// Its height in pixels.
        pub fn height(&self) -> u32 {
            self.0.height() as u32
        }
        /// The surface, for objc2's IOSurface (and Metal) calls.
        pub fn surface(&self) -> &IOSurfaceRef {
            &self.0
        }
        /// The `IOSurfaceRef`, for a client making its own Metal texture
        /// (`newTextureWithDescriptor:iosurface:plane:`). Valid while this
        /// lives.
        pub fn as_ptr(&self) -> *mut c_void {
            (&*self.0 as *const IOSurfaceRef).cast_mut().cast()
        }
    }

    // Mach, as <mach/message.h>, <mach/port.h> and <bootstrap.h> declare it
    // (libSystem exports each).
    type Port = libc::mach_port_t;
    type Kern = i32;
    unsafe extern "C" {
        static bootstrap_port: Port;
        static mach_task_self_: Port;
        fn bootstrap_check_in(bp: Port, name: *const c_char, port: *mut Port) -> Kern;
        fn bootstrap_look_up(bp: Port, name: *const c_char, port: *mut Port) -> Kern;
        fn mach_port_allocate(task: Port, right: u32, name: *mut Port) -> Kern;
        fn mach_port_deallocate(task: Port, name: Port) -> Kern;
        fn mach_port_mod_refs(task: Port, name: Port, right: u32, delta: i32) -> Kern;
        fn mach_msg(
            msg: *mut u32,
            option: i32,
            send_size: u32,
            rcv_size: u32,
            rcv_name: Port,
            timeout: u32,
            notify: Port,
        ) -> Kern;
    }
    const MACH_PORT_RIGHT_RECEIVE: u32 = 1;
    const MACH_SEND_MSG: i32 = 0x1;
    const MACH_RCV_MSG: i32 = 0x2;
    const MACH_SEND_TIMEOUT: i32 = 0x10;
    const MACH_RCV_TIMEOUT: i32 = 0x100;
    /// MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT): the sender's audit
    /// token (its pid) after each message.
    const MACH_RCV_TRAILER_AUDIT: i32 = 3 << 24;
    const MACH_RCV_TIMED_OUT: Kern = 0x1000_4003;
    const MACH_MSG_TYPE_MOVE_SEND: u32 = 17;
    const MACH_MSG_TYPE_COPY_SEND: u32 = 19;
    const MACH_MSG_TYPE_MAKE_SEND: u32 = 20;
    /// What a received send right's descriptor says (MACH_MSG_TYPE_PORT_SEND).
    const MACH_MSG_TYPE_PORT_SEND: u32 = 17;
    const MACH_MSGH_BITS_COMPLEX: u32 = 0x8000_0000;
    const MACH_MSG_PORT_DESCRIPTOR: u32 = 0;
    /// Header words (`mach_msg_header_t`), then the body's descriptor count.
    const HEADER_WORDS: usize = 6;
    /// Words of a port descriptor (`mach_msg_port_descriptor_t`).
    const DESCRIPTOR_WORDS: usize = 3;
    /// Message ids: a client's check-in, and a generation's surfaces.
    const CHECK_IN: i32 = 0x7a67_7301;
    const SURFACES: i32 = 0x7a67_7302;
    /// The most surfaces one message carries (a ring is three).
    const MAX_SURFACES: usize = 16;
    /// How long the host waits for a client's queue to take a message.
    const SEND_TIMEOUT: Duration = Duration::from_secs(1);

    fn task() -> Port {
        // Safety: the task's own port name, set as it started and never
        // written again.
        unsafe { mach_task_self_ }
    }
    fn check(kern: Kern, what: &str) -> io::Result<()> {
        if kern == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!("{what} failed: {kern:#x}")))
        }
    }
    fn millis(timeout: Duration) -> u32 {
        timeout.as_millis().min(u128::from(u32::MAX)) as u32
    }
    /// A receive right of this task's.
    fn receive_right() -> io::Result<Port> {
        let mut port = 0;
        // Safety: a new port, its name written into `port`.
        check(
            unsafe { mach_port_allocate(task(), MACH_PORT_RIGHT_RECEIVE, &mut port) },
            "mach_port_allocate",
        )?;
        Ok(port)
    }
    fn drop_receive(port: Port) {
        // Safety: a receive right this owns, given up once.
        unsafe { mach_port_mod_refs(task(), port, MACH_PORT_RIGHT_RECEIVE, -1) };
    }
    fn drop_send(port: Port) {
        // Safety: a send right this owns, given up once.
        unsafe { mach_port_deallocate(task(), port) };
    }

    /// A message to send: a complex message's header and body, its port
    /// descriptors, then `data` words.
    fn message(remote: Port, id: i32, ports: &[(Port, u32)], data: &[u32]) -> Vec<u32> {
        let mut words = Vec::with_capacity(HEADER_WORDS + 1 + ports.len() * 3 + data.len());
        let size = ((HEADER_WORDS + 1 + ports.len() * DESCRIPTOR_WORDS + data.len()) * 4) as u32;
        // MACH_MSGH_BITS(COPY_SEND, 0) | COMPLEX: no reply port.
        words.extend([
            MACH_MSG_TYPE_COPY_SEND | MACH_MSGH_BITS_COMPLEX,
            size,
            remote,
            0,
            0,
            id as u32,
        ]);
        words.push(ports.len() as u32);
        for &(port, disposition) in ports {
            // name, pad1, then pad2 (u16), disposition (u8), type (u8).
            words.extend([
                port,
                0,
                (disposition << 16) | (MACH_MSG_PORT_DESCRIPTOR << 24),
            ]);
        }
        words.extend_from_slice(data);
        words
    }
    fn send(mut words: Vec<u32>, timeout: Duration) -> io::Result<()> {
        let size = words[1];
        // Safety: a well-formed message `size` bytes long; on success its
        // rights are the receiver's.
        let kern = unsafe {
            mach_msg(
                words.as_mut_ptr(),
                MACH_SEND_MSG | MACH_SEND_TIMEOUT,
                size,
                0,
                0,
                millis(timeout),
                0,
            )
        };
        check(kern, "mach_msg (send)")
    }

    /// A message received: its id, the send rights it carried (this task's
    /// now: whoever takes them deallocates them), its data words and its
    /// sender's pid.
    struct Received {
        id: i32,
        ports: Vec<Port>,
        data: Vec<u32>,
        pid: u32,
    }
    impl Drop for Received {
        fn drop(&mut self) {
            for port in self.ports.drain(..) {
                drop_send(port);
            }
        }
    }
    /// The next message on `port`, waiting up to `timeout`; `None` if none
    /// came.
    fn receive(port: Port, timeout: Duration) -> io::Result<Option<Received>> {
        let mut buf = vec![0u32; 512];
        // Safety: `buf` is `rcv_size` bytes, 4-aligned; a larger message is
        // refused (and destroyed) by the kernel.
        let kern = unsafe {
            mach_msg(
                buf.as_mut_ptr(),
                MACH_RCV_MSG | MACH_RCV_TIMEOUT | MACH_RCV_TRAILER_AUDIT,
                0,
                (buf.len() * 4) as u32,
                port,
                millis(timeout),
                0,
            )
        };
        if kern == MACH_RCV_TIMED_OUT {
            return Ok(None);
        }
        check(kern, "mach_msg (receive)")?;
        let (bits, size, reply) = (buf[0], buf[1] as usize, buf[2]);
        if reply != 0 {
            // None is asked for; whoever sent one gets no answer.
            drop_send(reply);
        }
        let mut received = Received {
            id: buf[5] as i32,
            ports: Vec::new(),
            data: Vec::new(),
            pid: 0,
        };
        let words = (size / 4).min(buf.len());
        let mut at = HEADER_WORDS;
        let mut unexpected = false;
        if bits & MACH_MSGH_BITS_COMPLEX != 0 && words > at {
            let count = buf[at] as usize;
            at += 1;
            for _ in 0..count {
                if at + DESCRIPTOR_WORDS > words {
                    unexpected = true;
                    break;
                }
                let (name, kind) = (buf[at], buf[at + 2]);
                at += DESCRIPTOR_WORDS;
                let (disposition, descriptor) = ((kind >> 16) & 0xff, kind >> 24);
                if descriptor != MACH_MSG_PORT_DESCRIPTOR {
                    // Out-of-line memory or port arrays: never sent here
                    // (and longer than a port descriptor: the rest can't be
                    // read).
                    unexpected = true;
                    break;
                }
                match (name, disposition) {
                    (0, _) => {}
                    (_, MACH_MSG_TYPE_PORT_SEND) => received.ports.push(name),
                    // A receive right (MACH_MSG_TYPE_PORT_RECEIVE): not ours
                    // to keep.
                    (_, 16) => {
                        drop_receive(name);
                        unexpected = true;
                    }
                    _ => {
                        drop_send(name);
                        unexpected = true;
                    }
                }
            }
        }
        if unexpected {
            // `received` drops the send rights taken so far.
            return Err(io::Error::other("an unexpected Mach message"));
        }
        received.data = buf[at.min(words)..words].to_vec();
        // The audit trailer follows the message: type, size, seqno, the
        // sender's security token (2 words), its audit token (8; the pid
        // 6th).
        let trailer = size.div_ceil(4);
        if trailer + 13 <= buf.len() && buf[trailer + 1] >= 52 {
            received.pid = buf[trailer + 3 + 2 + 5];
        }
        Ok(Some(received))
    }

    /// The process's rendezvous: one bootstrap service for every session
    /// (launchd keeps a name checked in for the life of the process), its
    /// check-ins kept by the sender's pid until the session that started
    /// that process takes them.
    struct Rendezvous {
        name: String,
        checked_in: Mutex<HashMap<u32, Port>>,
        arrived: Condvar,
    }
    /// Check-ins kept unclaimed at most (a check-in names a process the host
    /// started and is taken at once; more is a process that found the name).
    const MAX_UNCLAIMED: usize = 64;
    fn rendezvous() -> io::Result<&'static Rendezvous> {
        static RENDEZVOUS: OnceLock<Result<Rendezvous, String>> = OnceLock::new();
        RENDEZVOUS
            .get_or_init(|| {
                let mut random = [0u8; 16];
                // Safety: fills the buffer it is given.
                unsafe { libc::arc4random_buf(random.as_mut_ptr().cast(), random.len()) };
                let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
                let name = format!("dev.zgx.surface.{}.{hex}", std::process::id());
                let c_name = CString::new(name.clone()).expect("no NUL in the name");
                let mut service = 0;
                // Safety: a name, and where its receive right goes.
                check(
                    unsafe { bootstrap_check_in(bootstrap_port, c_name.as_ptr(), &mut service) },
                    "bootstrap_check_in",
                )
                .map_err(|e| e.to_string())?;
                std::thread::Builder::new()
                    .name("zgx-surface mach".into())
                    .spawn(move || listen(service))
                    .map_err(|e| e.to_string())?;
                Ok(Rendezvous {
                    name,
                    checked_in: Mutex::default(),
                    arrived: Condvar::new(),
                })
            })
            .as_ref()
            .map_err(|e| io::Error::other(e.clone()))
    }
    /// The rendezvous's thread: each check-in kept by its sender's pid.
    fn listen(service: Port) {
        loop {
            let mut message = match receive(service, Duration::from_secs(3600)) {
                Ok(Some(message)) => message,
                Ok(None) => continue,
                Err(error) => {
                    eprintln!("zgx-surface: a Mach check-in was dropped: {error}");
                    continue;
                }
            };
            if message.id != CHECK_IN || message.ports.len() != 1 || message.pid == 0 {
                continue;
            }
            let Ok(rendezvous) = rendezvous() else { return };
            let mut checked_in = rendezvous.checked_in.lock().unwrap();
            if checked_in.len() >= MAX_UNCLAIMED && !checked_in.contains_key(&message.pid) {
                continue;
            }
            let port = message.ports.pop().unwrap();
            // A pid used again: the newer process's.
            if let Some(old) = checked_in.insert(message.pid, port) {
                drop_send(old);
            }
            rendezvous.arrived.notify_all();
        }
    }

    /// The host's side of a session's Mach channel: the process's
    /// rendezvous until its client has checked in, then a send right to the
    /// client's port.
    pub struct HostChannel {
        name: String,
        client: Mutex<Option<Port>>,
    }
    impl HostChannel {
        /// A channel through the process's rendezvous (checked in with the
        /// bootstrap server, under an unguessable name, the first time).
        pub fn new() -> io::Result<Self> {
            Ok(Self {
                name: rendezvous()?.name.clone(),
                client: Mutex::new(None),
            })
        }
        /// The rendezvous's name.
        pub fn name(&self) -> &str {
            &self.name
        }
        /// Make `command` find the channel, named in `ENV_MACH`.
        pub fn prepare(&self, command: &mut std::process::Command) {
            command.env(ENV_MACH, &self.name);
        }
        /// Take the check-in of the process `pid` (the program the host
        /// started, as the message's audit token says), waiting up to
        /// `timeout`. Another process's check-in never reaches this session.
        pub fn accept(&self, pid: u32, timeout: Duration) -> io::Result<()> {
            let rendezvous = rendezvous()?;
            let deadline = Instant::now() + timeout;
            let mut checked_in = rendezvous.checked_in.lock().unwrap();
            loop {
                if let Some(port) = checked_in.remove(&pid) {
                    if let Some(old) = self.client.lock().unwrap().replace(port) {
                        drop_send(old);
                    }
                    return Ok(());
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the client didn't check in on the Mach channel",
                    ));
                }
                checked_in = rendezvous.arrived.wait_timeout(checked_in, left).unwrap().0;
            }
        }
        /// Whether a client has checked in.
        pub fn is_connected(&self) -> bool {
            self.client.lock().unwrap().is_some()
        }
        /// Send `generation`'s surfaces (in buffer order) to the client.
        pub fn send(&self, generation: u32, surfaces: &[&Surface]) -> io::Result<()> {
            if surfaces.len() > MAX_SURFACES {
                return Err(io::Error::other("too many surfaces for one message"));
            }
            let Some(client) = *self.client.lock().unwrap() else {
                return Err(io::Error::other("no client has checked in"));
            };
            let ports: Vec<(Port, u32)> = surfaces
                .iter()
                .map(|s| (s.surface().create_mach_port(), MACH_MSG_TYPE_MOVE_SEND))
                .collect();
            if ports.iter().any(|(port, _)| *port == 0) {
                for (port, _) in ports {
                    if port != 0 {
                        drop_send(port);
                    }
                }
                return Err(io::Error::other("IOSurfaceCreateMachPort failed"));
            }
            let mut data = vec![generation, surfaces.len() as u32];
            data.extend(surfaces.iter().map(|s| s.id()));
            let sent = send(message(client, SURFACES, &ports, &data), SEND_TIMEOUT);
            if sent.is_err() {
                // Not sent: the rights are still this task's.
                for (port, _) in ports {
                    drop_send(port);
                }
            }
            sent
        }
    }
    impl Drop for HostChannel {
        fn drop(&mut self) {
            if let Some(client) = self.client.get_mut().unwrap().take() {
                drop_send(client);
            }
        }
    }

    /// The client's side of a session's Mach channel: its own port, which
    /// the host sends generations of surfaces to.
    pub struct ClientChannel {
        port: Port,
    }
    impl ClientChannel {
        /// The channel `ENV_MACH` names, checked in; `None` if this process
        /// wasn't started with one. The variable is removed, so programs it
        /// starts don't take it.
        pub fn from_env() -> io::Result<Option<Self>> {
            let Some(name) = std::env::var_os(ENV_MACH) else {
                return Ok(None);
            };
            // Safety: as `transport::connect_from_env`: a client connects as
            // it starts.
            unsafe { std::env::remove_var(ENV_MACH) };
            let name = name
                .into_string()
                .map_err(|_| io::Error::other("ZGX_SURFACE_MACH is not UTF-8"))?;
            Self::connect(&name).map(Some)
        }
        /// Look up the host's service `name` and check in: send it a send
        /// right to a port of this process's own.
        pub fn connect(name: &str) -> io::Result<Self> {
            let c_name = CString::new(name).map_err(io::Error::other)?;
            let mut service = 0;
            // Safety: a name, and where the send right goes.
            check(
                unsafe { bootstrap_look_up(bootstrap_port, c_name.as_ptr(), &mut service) },
                "bootstrap_look_up",
            )?;
            let port = match receive_right() {
                Ok(port) => port,
                Err(error) => {
                    drop_send(service);
                    return Err(error);
                }
            };
            let sent = send(
                message(service, CHECK_IN, &[(port, MACH_MSG_TYPE_MAKE_SEND)], &[]),
                SEND_TIMEOUT,
            );
            drop_send(service);
            if let Err(error) = sent {
                drop_receive(port);
                return Err(error);
            }
            Ok(Self { port })
        }
        /// The surfaces of `generation`, in buffer order, waiting up to
        /// `timeout` for them; an older generation's message (one never
        /// asked for) is dropped on the way.
        pub fn receive(&self, generation: u32, timeout: Duration) -> io::Result<Vec<Surface>> {
            let deadline = Instant::now() + timeout;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let Some(mut message) = receive(self.port, left)? else {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("generation {generation}'s surfaces didn't arrive"),
                    ));
                };
                let (Some(&got), Some(&count)) = (message.data.first(), message.data.get(1)) else {
                    continue;
                };
                if message.id != SURFACES || got < generation {
                    continue;
                }
                if got > generation {
                    return Err(io::Error::other(format!(
                        "generation {got}'s surfaces came for generation {generation}"
                    )));
                }
                let count = count as usize;
                if message.ports.len() != count || message.data.len() != 2 + count {
                    return Err(io::Error::other("a generation's message is malformed"));
                }
                let ids = message.data[2..].to_vec();
                let mut surfaces = Vec::with_capacity(count);
                for (port, id) in message.ports.drain(..).zip(ids) {
                    let surface = IOSurfaceRef::lookup_from_mach_port(port);
                    drop_send(port);
                    let surface = surface.map(Surface).ok_or_else(|| {
                        io::Error::other("IOSurfaceLookupFromMachPort found no surface")
                    })?;
                    if surface.id() != id {
                        return Err(io::Error::other("a surface isn't the one named"));
                    }
                    surfaces.push(surface);
                }
                return Ok(surfaces);
            }
        }
        /// Fill in the surfaces of `generation`'s IOSurface buffers, by id
        /// (`Client` does, as each `Buffers` arrives).
        pub fn attach(
            &self,
            generation: &mut crate::Generation,
            timeout: Duration,
        ) -> io::Result<()> {
            use crate::BufferDesc;
            let wanted = generation
                .buffers
                .iter()
                .any(|b| matches!(b.desc, BufferDesc::IoSurface { .. }));
            if !wanted {
                return Ok(());
            }
            let surfaces = self.receive(generation.generation, timeout)?;
            for buffer in &mut generation.buffers {
                if let BufferDesc::IoSurface { id, surface } = &mut buffer.desc {
                    *surface = surfaces.iter().find(|s| s.id() == *id).cloned();
                }
            }
            Ok(())
        }
    }
    impl Drop for ClientChannel {
        fn drop(&mut self) {
            drop_receive(self.port);
        }
    }
}

/// Elsewhere: nothing of these exists (`Surface` is never made, the
/// channels never open).
#[cfg(not(target_os = "macos"))]
mod imp {
    use std::{io, time::Duration};

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "IOSurfaces are macOS's")
    }

    /// An IOSurface (macOS only: none exists here).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Surface {}

    /// The host's Mach channel (macOS only).
    pub enum HostChannel {}
    impl HostChannel {
        /// Unsupported here.
        pub fn new() -> io::Result<Self> {
            Err(unsupported())
        }
        /// Its name.
        pub fn name(&self) -> &str {
            match *self {}
        }
        /// Make `command` find the channel.
        pub fn prepare(&self, _command: &mut std::process::Command) {
            match *self {}
        }
        /// Take the client's check-in.
        pub fn accept(&self, _pid: u32, _timeout: Duration) -> io::Result<()> {
            match *self {}
        }
        /// Whether a client has checked in.
        pub fn is_connected(&self) -> bool {
            match *self {}
        }
        /// Send a generation's surfaces.
        pub fn send(&self, _generation: u32, _surfaces: &[&Surface]) -> io::Result<()> {
            match *self {}
        }
    }

    /// The client's Mach channel (macOS only).
    pub enum ClientChannel {}
    impl ClientChannel {
        /// Never one here.
        pub fn from_env() -> io::Result<Option<Self>> {
            Ok(None)
        }
        /// Unsupported here.
        pub fn connect(_name: &str) -> io::Result<Self> {
            Err(unsupported())
        }
        /// A generation's surfaces.
        pub fn receive(&self, _generation: u32, _timeout: Duration) -> io::Result<Vec<Surface>> {
            match *self {}
        }
        /// Fill in a generation's surfaces.
        pub fn attach(
            &self,
            _generation: &mut crate::Generation,
            _timeout: Duration,
        ) -> io::Result<()> {
            match *self {}
        }
    }
}
