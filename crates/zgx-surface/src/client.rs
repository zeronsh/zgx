//! The client's side: connect to the host that started this program, take
//! its buffers, draw into them and present them, and hear its input.
//!
//! ```no_run
//! use zgx_surface::{Client, HostMessage, Offer};
//! let mut client = Client::connect(vec![Offer::Shm]).expect("started by a zgx host");
//! loop {
//!     while let Some(event) = client.poll() {
//!         match event {
//!             HostMessage::Buffers(generation) => { /* map or import each buffer */ }
//!             HostMessage::Close => return,
//!             _ => {}
//!         }
//!     }
//!     if let Some(index) = client.acquire() {
//!         // Draw into buffer `index`, wait for the GPU, then:
//!         client.present(index).unwrap();
//!     } else if !client.wait(None) {
//!         return;
//!     }
//! }
//! ```
use crate::{
    iosurface::ClientChannel,
    protocol::{ClientMessage, Cursor, Hello, HostMessage, Offer, PROTOCOL_VERSION, Welcome},
    ring::{ClientRing, Slot},
    transport::{self, Receiver, Sender},
};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

/// How long a generation's IOSurfaces may take to follow its `Buffers`
/// (they are sent first: this is a stuck host).
const MACH_WAIT: Duration = Duration::from_secs(2);

type Waker = Arc<Mutex<Option<Box<dyn Fn() + Send>>>>;

/// A connection to the host showing this program's frames.
pub struct Client {
    sender: Sender,
    incoming: mpsc::Receiver<Option<HostMessage>>,
    waker: Waker,
    welcome: Welcome,
    ring: ClientRing,
    queued: VecDeque<HostMessage>,
    closed: bool,
}

impl Client {
    /// Connect to the host that started this program (`ZGX_SURFACE_FD`, or
    /// `ZGX_SURFACE_PIPE` on Windows), offering `offers` (best first), and
    /// wait for its `Welcome`. The host answers once it can say how buffers
    /// will be shared, which is when it first shows the surface.
    ///
    /// Offering `IoSurface` (macOS), it first checks in on the session's
    /// Mach channel (`ZGX_SURFACE_MACH`); without one the offer is dropped.
    pub fn connect(mut offers: Vec<Offer>) -> io::Result<Client> {
        let (sender, receiver) = transport::connect_from_env()?;
        let mut mach = None;
        if offers.iter().any(|o| matches!(o, Offer::IoSurface { .. })) {
            match ClientChannel::from_env() {
                Ok(Some(channel)) => mach = Some(channel),
                Ok(None) => {}
                Err(error) => {
                    eprintln!("zgx-surface: the Mach channel couldn't be checked in: {error}")
                }
            }
            if mach.is_none() {
                offers.retain(|o| !matches!(o, Offer::IoSurface { .. }));
            }
        }
        Self::connect_with(sender, receiver, offers, mach)
    }
    /// As `connect`, over a connection already made (tests, a host's own
    /// client in the same process).
    pub fn connect_over(
        sender: Sender,
        receiver: Receiver,
        offers: Vec<Offer>,
    ) -> io::Result<Client> {
        Self::connect_with(sender, receiver, offers, None)
    }
    /// As `connect_over`, with a Mach channel already checked in for
    /// IOSurface buffers.
    pub fn connect_with(
        mut sender: Sender,
        mut receiver: Receiver,
        offers: Vec<Offer>,
        mach: Option<ClientChannel>,
    ) -> io::Result<Client> {
        sender.send_client(&ClientMessage::Hello(Hello {
            version: PROTOCOL_VERSION,
            offers,
        }))?;
        let welcome = loop {
            match receiver.recv_host()? {
                Some(HostMessage::Welcome(welcome)) => break welcome,
                Some(HostMessage::Close) | None => {
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "the host closed before welcoming",
                    ));
                }
                // Nothing else comes before the welcome.
                Some(_) => {}
            }
        };
        let (tx, incoming) = mpsc::channel();
        let waker: Waker = Arc::default();
        let wake = waker.clone();
        std::thread::Builder::new()
            .name("zgx-surface client".into())
            .spawn(move || {
                loop {
                    let mut message = receiver.recv_host().unwrap_or(None);
                    // IOSurfaces come over the Mach channel, sent before
                    // the `Buffers` naming them.
                    if let (Some(HostMessage::Buffers(generation)), Some(mach)) =
                        (&mut message, &mach)
                        && let Err(error) = mach.attach(generation, MACH_WAIT)
                    {
                        eprintln!("zgx-surface: {error}");
                    }
                    let end = message.is_none();
                    if tx.send(message).is_err() {
                        return;
                    }
                    if let Some(wake) = &*wake.lock().unwrap() {
                        wake();
                    }
                    if end {
                        return;
                    }
                }
            })?;
        Ok(Client {
            sender,
            incoming,
            waker,
            welcome,
            ring: ClientRing::new(),
            queued: VecDeque::new(),
            closed: false,
        })
    }
    /// How the host shares buffers, and on which device.
    pub fn welcome(&self) -> &Welcome {
        &self.welcome
    }
    /// Call `wake` (on the connection's thread) as each message arrives:
    /// to wake an event loop (winit's `EventLoopProxy`) that then `poll`s.
    pub fn set_waker(&mut self, wake: impl Fn() + Send + 'static) {
        *self.waker.lock().unwrap() = Some(Box::new(wake));
    }
    /// The next event, without waiting. Buffers' releases are taken here
    /// (`acquire` hands them out); `Buffers` is handed on after the ring
    /// moved to its generation; `Close` and the connection's end come as
    /// `Close`.
    pub fn poll(&mut self) -> Option<HostMessage> {
        loop {
            if let Some(message) = self.queued.pop_front() {
                return Some(message);
            }
            match self.incoming.try_recv() {
                Ok(message) => {
                    if let Some(event) = self.take(message) {
                        return Some(event);
                    }
                }
                Err(_) => return None,
            }
        }
    }
    /// Wait up to `timeout` (forever with `None`) for something to arrive:
    /// an event (for `poll`) or a released buffer (for `acquire`). False if
    /// nothing did, or the connection has ended.
    pub fn wait(&mut self, timeout: Option<Duration>) -> bool {
        if !self.queued.is_empty() {
            return true;
        }
        if self.closed {
            return false;
        }
        let message = match timeout {
            None => self.incoming.recv().ok(),
            Some(timeout) => self.incoming.recv_timeout(timeout).ok(),
        };
        let Some(message) = message else {
            return false;
        };
        if let Some(event) = self.take(message) {
            self.queued.push_back(event);
        }
        true
    }
    /// A message from the connection: kept in the ring, or the event to
    /// hand out.
    fn take(&mut self, message: Option<HostMessage>) -> Option<HostMessage> {
        match message {
            Some(HostMessage::Release { generation, index }) => {
                self.ring.release(Slot { generation, index });
                None
            }
            Some(HostMessage::Buffers(generation)) => {
                self.ring
                    .reset(generation.generation, generation.buffers.len() as u32);
                Some(HostMessage::Buffers(generation))
            }
            Some(HostMessage::Welcome(_)) => None,
            Some(HostMessage::Close) | None => {
                if self.closed {
                    return None;
                }
                self.closed = true;
                Some(HostMessage::Close)
            }
            Some(event) => Some(event),
        }
    }
    /// A buffer of the current generation to draw into, if one is free
    /// (taking releases that arrived meanwhile; other events wait for
    /// `poll`).
    pub fn acquire(&mut self) -> Option<u32> {
        while let Ok(message) = self.incoming.try_recv() {
            if let Some(event) = self.take(message) {
                self.queued.push_back(event);
            }
        }
        self.ring.acquire()
    }
    /// As `acquire`, waiting up to `timeout` for the host to release one.
    pub fn acquire_wait(&mut self, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(index) = self.acquire() {
                return Some(index);
            }
            if self.closed {
                return None;
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            let message = self.incoming.recv_timeout(left).ok()?;
            if let Some(event) = self.take(message) {
                self.queued.push_back(event);
            }
        }
    }
    /// How many buffers of the current generation are free now (taking
    /// releases that arrived): a client that draws only with two free never
    /// draws a frame replaced before it shows.
    pub fn free_buffers(&mut self) -> usize {
        while let Ok(message) = self.incoming.try_recv() {
            if let Some(event) = self.take(message) {
                self.queued.push_back(event);
            }
        }
        self.ring.free()
    }
    /// Give back a buffer acquired and not drawn.
    pub fn cancel(&mut self, index: u32) {
        self.ring.cancel(index);
    }
    /// Show buffer `index`, whose drawing is finished (the GPU's work done:
    /// v1 shares no semaphore). It is the host's until it releases it.
    pub fn present(&mut self, index: u32) -> io::Result<()> {
        self.send(&ClientMessage::Present {
            generation: self.ring.generation(),
            index,
        })
    }
    /// The current generation (0 before the first `Buffers`).
    pub fn generation(&self) -> u32 {
        self.ring.generation()
    }
    /// How the pointer looks over the surface.
    pub fn set_cursor(&mut self, cursor: Cursor) -> io::Result<()> {
        self.send(&ClientMessage::SetCursor(cursor))
    }
    /// Ask for the pointer's lock (granted just after a press or key; the
    /// answer comes as `PointerLock`), or let it go.
    pub fn lock_pointer(&mut self, locked: bool) -> io::Result<()> {
        self.send(&ClientMessage::LockPointer(locked))
    }
    /// The surface's title.
    pub fn set_title(&mut self, title: &str) -> io::Result<()> {
        self.send(&ClientMessage::Title(title.to_owned()))
    }
    /// Whether the host closed (or asked it to: `Close`).
    pub fn is_closed(&self) -> bool {
        self.closed
    }
    fn send(&mut self, message: &ClientMessage) -> io::Result<()> {
        self.sender.send_client(message)
    }
}
