//! The fetcher `run` gives a script (`ui.fetch`, remote images): HTTP and
//! HTTPS through ureq, TLS by rustls with Mozilla's roots, each request on
//! a thread of its own so the window never waits for the network. The
//! body reaches the script as it arrives (`on_chunk`, `on_event`); a
//! cancelled fetch closes its connection at once, whatever the peer is
//! doing. A peer silent for five minutes ends the request. A host that
//! brings its own transport replaces it (`ScriptHost::fetcher`).
//!
//! Redirects are followed here, as ureq's own would be.
use std::{
    cell::RefCell,
    future::Future,
    io::Read,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock, Weak},
    task::{Context, Poll, Waker},
};
use ureq::{
    http::Uri,
    unversioned::{
        resolver::DefaultResolver,
        transport::{
            Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, ShutdownHandle,
            Transport,
        },
    },
};
use zgx_script::{Body, Request, Response, ScriptHost};

/// Give `host` the default fetcher, as `run` does before a host's own
/// setup (which may replace it).
pub fn install(host: &ScriptHost) {
    host.streaming_fetcher(Fetching::start);
}

/// An answer's status and headers.
type Head = (u16, Vec<(String, String)>);
/// What a request's thread has heard, for the task awaiting it.
#[derive(Default)]
struct Heard {
    /// The status and headers, or why there are none.
    head: Option<Result<Head, String>>,
    /// Body read since the task last took it.
    pieces: Vec<u8>,
    /// How much of the request's body has gone out, since the task last
    /// told the script: `(sent, total)`.
    sent: Option<(u64, u64)>,
    /// The body ended, or broke off (`failed`).
    ended: bool,
    failed: Option<String>,
    /// The task stopped listening: the thread stops reading.
    cancelled: bool,
    waker: Option<Waker>,
    /// The connection the request is on now (`Tracked`), shut down by a
    /// cancel: it alone closes, the pool's others stay open.
    socket: Option<Weak<dyn ShutdownHandle>>,
}
type Shared = Arc<Mutex<Heard>>;

/// One request: its thread reads, this future hands what it read to the
/// script's body and answers with the head once the body ends.
struct Fetching {
    shared: Shared,
    body: Body,
}
impl Fetching {
    fn start(request: Request, body: Body) -> Self {
        let shared = Shared::default();
        let heard = shared.clone();
        std::thread::Builder::new()
            .name("zgx-run fetch".into())
            .spawn(move || read(request, heard))
            .expect("a thread for the request");
        Self { shared, body }
    }
}
impl Future for Fetching {
    type Output = Result<Response, String>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            let (pieces, done) = {
                let mut heard = self.shared.lock().unwrap();
                if let Some((sent, total)) = heard.sent.take() {
                    // Outside the lock: the script's `on_progress` runs.
                    drop(heard);
                    self.body.sent(sent, total);
                    continue;
                }
                let pieces = std::mem::take(&mut heard.pieces);
                let done = pieces.is_empty() && (heard.ended || matches!(heard.head, Some(Err(_))));
                if pieces.is_empty() && !done {
                    heard.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
                (pieces, done)
            };
            if !pieces.is_empty() {
                // Outside the lock: the script's handlers run in it.
                self.body.write(&pieces);
                if !self.body.is_open() {
                    self.shared.lock().unwrap().cancelled = true;
                    return Poll::Ready(Err("cancelled".into()));
                }
                continue;
            }
            debug_assert!(done);
            let mut heard = self.shared.lock().unwrap();
            if let Some(failed) = heard.failed.take() {
                return Poll::Ready(Err(failed));
            }
            return Poll::Ready(match heard.head.take() {
                Some(Ok((status, headers))) => Ok(Response {
                    status,
                    headers,
                    body: Vec::new(),
                }),
                Some(Err(error)) => Err(error),
                None => Err("the request ended without an answer".into()),
            });
        }
    }
}
impl Drop for Fetching {
    /// A task cancelled (or the script reloaded) drops this: its
    /// connection closes, waking a read or write blocked on a silent peer,
    /// and the thread stops.
    fn drop(&mut self) {
        // Under the lock: the thread clears `socket` under it before a
        // finished connection goes back to the pool, so no other request's
        // connection is shut down.
        let mut heard = self.shared.lock().unwrap();
        heard.cancelled = true;
        if let Some(socket) = heard.socket.take().and_then(|socket| socket.upgrade()) {
            socket.shutdown();
        }
    }
}

thread_local! {
    /// The request this thread makes (a thread a request): where `Tracked`
    /// records the connection it is on.
    static ACTIVE: RefCell<Option<Shared>> = const { RefCell::new(None) };
    /// The shutdown handle of the connection this thread just made.
    static MADE: RefCell<Option<Weak<dyn ShutdownHandle>>> = const { RefCell::new(None) };
}

/// Record `socket` as the connection this thread's request is on, unless
/// the request is over.
fn claim(socket: &Weak<dyn ShutdownHandle>) {
    ACTIVE.with_borrow(|active| {
        if let Some(shared) = active {
            let mut heard = shared.lock().unwrap();
            if !heard.cancelled && !heard.ended {
                heard.socket = Some(socket.clone());
            }
        }
    });
}

/// ureq's connectors, each new connection's shutdown handle kept with it
/// (`Tracked`) as it is registered with the agent.
#[derive(Debug, Default)]
struct Tracking(DefaultConnector);
type Register = dyn Fn(Weak<dyn ShutdownHandle>) -> Result<(), ureq::Error> + Send + Sync;
impl Connector for Tracking {
    type Out = Box<dyn Transport>;
    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let register = details.register_shutdown.clone();
        let hook: Arc<Register> = Arc::new(move |socket: Weak<dyn ShutdownHandle>| {
            claim(&socket);
            MADE.set(Some(socket.clone()));
            register(socket)
        });
        let tracked = ConnectionDetails {
            uri: details.uri,
            addrs: details.addrs.clone(),
            config: details.config,
            request_level: details.request_level,
            resolver: details.resolver,
            now: details.now,
            timeout: details.timeout,
            current_time: details.current_time.clone(),
            run_connector: details.run_connector.clone(),
            register_shutdown: hook,
        };
        let made = self.0.connect(&tracked, chained)?;
        let socket = MADE.take();
        Ok(made.map(|inner| Box::new(Tracked { inner, socket }) as Box<dyn Transport>))
    }
}

/// A connection that tells the request using it which it is (`claim`), as
/// it reads or writes: a request reusing a pooled connection is on it.
struct Tracked {
    inner: Box<dyn Transport>,
    socket: Option<Weak<dyn ShutdownHandle>>,
}
impl std::fmt::Debug for Tracked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Tracked").field(&self.inner).finish()
    }
}
impl Tracked {
    fn claim(&self) {
        if let Some(socket) = &self.socket {
            claim(socket);
        }
    }
}
impl Transport for Tracked {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.claim();
        self.inner.transmit_output(amount, timeout)
    }
    fn maybe_await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.claim();
        self.inner.maybe_await_input(timeout)
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.claim();
        self.inner.await_input(timeout)
    }
    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }
    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

/// An agent: connections are kept for reuse, each closed alone by its
/// request's cancel (`Tracking`). A status other than 2xx is an answer the
/// script reads (`ok` false), not an error, and a redirect one `send`
/// follows. A read or write waits at most five minutes: a stream may be
/// quiet a while (a model thinking), a peer gone silent for good is not
/// waited on forever.
fn new_agent() -> ureq::Agent {
    let minutes = |n: u64| Some(std::time::Duration::from_secs(60 * n));
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(concat!("zgx-run/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(std::time::Duration::from_secs(30)))
        .timeout_per_read(minutes(5))
        .timeout_per_write(minutes(5));
    ureq::Agent::with_parts(
        config.build(),
        Tracking::default(),
        DefaultResolver::default(),
    )
}
/// The agent every request goes on.
fn agent() -> ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(new_agent).clone()
}

/// Make `request` and read its answer into `heard`, waking its task as
/// each part arrives, until it ends or the task stops listening.
fn read(request: Request, shared: Shared) {
    ACTIVE.set(Some(shared.clone()));
    let heard: &Mutex<Heard> = &shared;
    let tell = |change: &mut dyn FnMut(&mut Heard)| -> bool {
        let waker = {
            let mut heard = heard.lock().unwrap();
            change(&mut heard);
            if heard.cancelled {
                return false;
            }
            heard.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        true
    };
    let response = match send(request, &shared) {
        Ok(response) => response,
        Err(error) => {
            tell(&mut |heard| heard.head = Some(Err(error.clone())));
            return;
        }
    };
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(name, value)| {
            let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
            (name.as_str().to_owned(), value)
        })
        .collect();
    let status = response.status().as_u16();
    if !tell(&mut |heard| heard.head = Some(Ok((status, headers.clone())))) {
        return;
    }
    let mut reader = response.into_body().into_reader();
    let mut buffer = vec![0; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                // Done with the connection, which goes back to the pool: a
                // cancel from now on closes nothing.
                tell(&mut |heard| {
                    heard.ended = true;
                    heard.socket = None;
                });
                return;
            }
            Ok(n) => {
                if !tell(&mut |heard| heard.pieces.extend_from_slice(&buffer[..n])) {
                    return;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                tell(&mut |heard| {
                    heard.failed = Some(error.to_string());
                    heard.ended = true;
                });
                return;
            }
        }
    }
}

/// `location` read against `base`, as a browser would (RFC 3986's reference
/// resolution, as ureq's own redirects do).
fn join_location(base: &Uri, location: &str) -> Result<Uri, String> {
    let bad = || format!("a redirect to a bad location: {location}");
    if let Ok(uri) = location.parse::<Uri>()
        && uri.scheme().is_some()
    {
        return Ok(uri);
    }
    let scheme = base.scheme_str().ok_or_else(bad)?;
    if location.starts_with("//") {
        return format!("{scheme}:{location}").parse().map_err(|_| bad());
    }
    let authority = base.authority().ok_or_else(bad)?;
    let path = if location.starts_with('/') {
        location.to_owned()
    } else {
        let mut parts: Vec<&str> = base.path().split('/').collect();
        if parts.len() > 1 {
            parts.pop();
        }
        for segment in location.split('/') {
            match segment {
                "." => {}
                ".." if parts.len() == 1 => return Err(bad()),
                ".." => {
                    parts.pop();
                }
                segment => parts.push(segment),
            }
        }
        parts.join("/")
    };
    format!("{scheme}://{authority}{path}")
        .parse()
        .map_err(|_| bad())
}

/// `request` sent, redirects followed here, the last answer's head read; how much of its body has gone out told to
/// `shared` as it goes. A redirect goes as ureq's own would: at most ten,
/// without `Authorization`, `Cookie` or `Content-Length`; a 307 or 308 of a
/// request with a body, or of a `DELETE`, is not followed, and any other turns
/// what isn't a `GET` or `HEAD` into a `GET`.
fn send(request: Request, shared: &Shared) -> Result<ureq::http::Response<ureq::Body>, String> {
    let agent = agent();
    let mut uri: Uri = request
        .url
        .parse()
        .map_err(|e: ureq::http::uri::InvalidUri| e.to_string())?;
    let mut method = request.method.to_ascii_uppercase();
    let mut headers = request.headers;
    let mut body = request.body;
    let mut followed = 0;
    loop {
        if shared.lock().unwrap().cancelled {
            return Err("cancelled".into());
        }
        let response = send_once(&agent, &method, &uri, &headers, body.take(), shared)?;
        let status = response.status().as_u16();
        let location = response.headers().get("location");
        let (300..=399, Some(location)) = (status, location) else {
            return Ok(response);
        };
        if followed == 10 {
            return Err("too many redirects".into());
        }
        followed += 1;
        let location = location
            .to_str()
            .map_err(|_| "a redirect to a bad location".to_owned())?;
        let next = join_location(&uri, location)?;
        let carries_body = matches!(method.as_str(), "POST" | "PUT" | "PATCH");
        if matches!(status, 307 | 308) {
            if carries_body || method == "DELETE" {
                return Err(format!("a {status} redirect of a {method} isn't followed"));
            }
        } else if method != "GET" && method != "HEAD" {
            method = "GET".into();
        }
        headers.retain(|(name, _)| {
            !["authorization", "cookie", "content-length"]
                .iter()
                .any(|gone| name.eq_ignore_ascii_case(gone))
        });
        uri = next;
    }
}

/// One hop of a request.
fn send_once(
    agent: &ureq::Agent,
    method: &str,
    uri: &Uri,
    headers: &[(String, String)],
    body: Option<Vec<u8>>,
    shared: &Shared,
) -> Result<ureq::http::Response<ureq::Body>, String> {
    let mut builder = ureq::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let sent = match body {
        // No body at all (a GET's), not an empty one.
        None => builder.body(()).map(|request| agent.run(request)),
        Some(body) => {
            // Its length said, so it goes length-delimited, not chunked,
            // though read through a counter.
            let total = body.len() as u64;
            if !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            {
                builder = builder.header("content-length", total);
            }
            let counted = Counted {
                body: std::io::Cursor::new(body),
                total,
                told: 0,
                shared: shared.clone(),
            };
            builder
                .body(ureq::SendBody::from_owned_reader(counted))
                .map(|request| agent.run(request))
        }
    };
    sent.map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

/// A request's body read out to the connection, telling the script's task
/// how much has gone, at most a hundred times over the body.
struct Counted {
    body: std::io::Cursor<Vec<u8>>,
    total: u64,
    told: u64,
    shared: Shared,
}
impl Read for Counted {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = self.body.read(buffer)?;
        let sent = self.body.position();
        if sent == self.total || sent - self.told >= (self.total / 100).max(16 * 1024) {
            self.told = sent;
            let waker = {
                let mut heard = self.shared.lock().unwrap();
                heard.sent = Some((sent, self.total));
                heard.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }
        Ok(n)
    }
}
