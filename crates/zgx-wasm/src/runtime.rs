//! The guest's side: one engine for the process, and for each session a
//! thread of its own running the component in a store of its own, with
//! nothing granted but what the host says.
//!
//! What a session may do is a store's: no preopened directory (unless the
//! host grants a data folder), no network, environment, arguments or stdin,
//! a wall clock stopped at the epoch (unless granted), a monotonic clock in
//! whole milliseconds, a cap on its linear memory, and a time limit on every
//! call into it (epoch interruption: a guest looping forever is stopped).
//! A trap, an exit, a time limit or a panic of the host's while serving it
//! ends that session alone.
use crate::{
    WasmOptions,
    device::{Gpu, HostGpu, WorkCaps},
    shared::{Ended, Link, Note, Presented},
};
use bindings::zgx::app::types as wit;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use wasmtime::{
    Engine, Store, StoreLimits, StoreLimitsBuilder, UpdateDeadline,
    component::{HasSelf, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "app",
        imports: { default: trappable },
        with: {
            "zgx:app/gpu.buffer": crate::gpu::Buffer,
            "zgx:app/gpu.texture": crate::gpu::Texture,
            "zgx:app/gpu.texture-view": crate::gpu::TextureView,
            "zgx:app/gpu.sampler": crate::gpu::Sampler,
            "zgx:app/gpu.shader-module": crate::gpu::ShaderModule,
            "zgx:app/gpu.bind-group-layout": crate::gpu::BindGroupLayout,
            "zgx:app/gpu.pipeline-layout": crate::gpu::PipelineLayout,
            "zgx:app/gpu.bind-group": crate::gpu::BindGroup,
            "zgx:app/gpu.render-pipeline": crate::gpu::RenderPipeline,
            "zgx:app/gpu.compute-pipeline": crate::gpu::ComputePipeline,
            "zgx:app/gpu.command-buffer": crate::gpu::CommandBuffer,
            "zgx:app/gpu.command-encoder": crate::gpu::CommandEncoder,
            "zgx:app/gpu.render-pass": crate::gpu::RenderPass,
            "zgx:app/gpu.compute-pass": crate::gpu::ComputePass,
            "zgx:app/gpu.query-set": crate::bundles::QuerySet,
            "zgx:app/gpu.render-bundle": crate::bundles::RenderBundle,
            "zgx:app/gpu.render-bundle-encoder": crate::bundles::RenderBundleEncoder,
            "zgx:app/gpu.render-pipeline-future": crate::pipelines::RenderPipelineFuture,
            "zgx:app/gpu.compute-pipeline-future": crate::pipelines::ComputePipelineFuture,
        },
    });
}

/// How often the epoch ticks while a guest runs (the precision of the time
/// limit, and of closing a session mid-call).
const TICK: Duration = Duration::from_millis(10);
/// Lines of a session's log kept (`WasmApp::log`), and the longest line.
const LOG_LINES: usize = 256;
const LOG_LINE: usize = 1024;

/// The process's engine, and the thread ticking its epoch while any guest
/// runs (it sleeps while none does: idle sessions cost nothing).
struct Runtime {
    engine: Engine,
    ticker: Arc<Ticker>,
}
#[derive(Default)]
struct Ticker {
    busy: Mutex<usize>,
    wake: Condvar,
}
impl Ticker {
    fn enter(&self) {
        *self.busy.lock().unwrap() += 1;
        self.wake.notify_one();
    }
    fn leave(&self) {
        *self.busy.lock().unwrap() -= 1;
    }
}

fn runtime() -> Result<&'static Runtime, String> {
    static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            let mut config = wasmtime::Config::new();
            config
                .wasm_component_model(true)
                .epoch_interruption(true)
                // A deep recursion traps (stack overflow) rather than taking
                // the thread's stack.
                .max_wasm_stack(2 << 20)
                .cranelift_opt_level(wasmtime::OptLevel::Speed);
            let engine =
                Engine::new(&config).map_err(|e| format!("the WebAssembly engine: {e}"))?;
            let ticker = Arc::new(Ticker::default());
            let (ticking, ticks) = (ticker.clone(), engine.clone());
            std::thread::Builder::new()
                .name("zgx-wasm epoch".into())
                .spawn(move || {
                    loop {
                        {
                            let mut busy = ticking.busy.lock().unwrap();
                            while *busy == 0 {
                                busy = ticking.wake.wait(busy).unwrap();
                            }
                        }
                        std::thread::sleep(TICK);
                        ticks.increment_epoch();
                    }
                })
                .map_err(|e| format!("the WebAssembly engine's clock: {e}"))?;
            Ok(Runtime { engine, ticker })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// What the UI thread tells the guest's.
pub(crate) enum ToGuest {
    /// The adapter zgx draws with (again as it changes): the guest's own
    /// device is made on it.
    Device(HostGpu),
    /// zgx's textures for its frames, a ring's generation (none: nothing
    /// is shared; it reads its frames back).
    Buffers {
        generation: u32,
        buffers: Vec<zgx_surface::protocol::Buffer>,
        /// zgx's clearing of them, to wait for before drawing.
        cleared: Option<crate::device::Cleared>,
    },
    /// The element's size, scale and refresh rate: the first is `init`.
    Resize(wit::SurfaceInfo),
    Event(wit::InputEvent),
    /// A pipeline made on another thread is ready (`pipeline-ready`).
    PipelineReady(u64),
    /// Draw a frame for this time (seconds since the session began).
    Frame(f64),
    Close,
}

/// A session's log: the guest's `log`, standard output and error, by line.
#[derive(Clone, Default)]
pub(crate) struct Log(Arc<Mutex<LogLines>>);
#[derive(Default)]
pub(crate) struct LogLines {
    lines: VecDeque<String>,
    partial: Vec<u8>,
}
impl Log {
    pub fn line(&self, line: String) {
        if std::env::var_os("ZGX_WASM_LOG").is_some_and(|v| v != "0") {
            eprintln!("zgx-wasm: {line}");
        }
        let mut lines = self.0.lock().unwrap();
        if lines.lines.len() == LOG_LINES {
            lines.lines.pop_front();
        }
        lines.lines.push_back(line);
    }
    /// Bytes the guest wrote: each whole line a line of the log.
    fn bytes(&self, prefix: &str, bytes: &[u8]) {
        let mut done = Vec::new();
        {
            let mut lines = self.0.lock().unwrap();
            for &byte in bytes {
                if byte == b'\n' {
                    done.push(std::mem::take(&mut lines.partial));
                } else if lines.partial.len() < LOG_LINE {
                    lines.partial.push(byte);
                }
            }
        }
        for line in done {
            self.line(format!("{prefix}{}", String::from_utf8_lossy(&line)));
        }
    }
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().lines.iter().cloned().collect()
    }
    /// What a Rust guest's panic said, from its standard error: the line
    /// after "panicked at …".
    fn panic_message(&self) -> Option<String> {
        let lines = self.0.lock().unwrap();
        let at = lines
            .lines
            .iter()
            .rposition(|l| l.contains("panicked at"))?;
        let message = lines.lines.get(at + 1)?;
        Some(message.trim_start_matches("stderr: ").to_owned())
    }
}

/// The guest's standard output or error, into its log.
struct LogStream {
    log: Log,
    prefix: &'static str,
}
impl wasmtime_wasi::cli::IsTerminal for LogStream {
    fn is_terminal(&self) -> bool {
        false
    }
}
impl wasmtime_wasi::cli::StdoutStream for LogStream {
    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        // Only WASI 0.2's streams are linked (`p2_stream`).
        Box::new(tokio::io::sink())
    }
    fn p2_stream(&self) -> Box<dyn wasmtime_wasi::p2::OutputStream> {
        Box::new(LogWriter {
            log: self.log.clone(),
            prefix: self.prefix,
        })
    }
}
struct LogWriter {
    log: Log,
    prefix: &'static str,
}
#[wasmtime_wasi::async_trait]
impl wasmtime_wasi::p2::Pollable for LogWriter {
    async fn ready(&mut self) {}
}
impl wasmtime_wasi::p2::OutputStream for LogWriter {
    fn write(&mut self, bytes: bytes::Bytes) -> wasmtime_wasi::p2::StreamResult<()> {
        self.log.bytes(self.prefix, &bytes);
        Ok(())
    }
    fn flush(&mut self) -> wasmtime_wasi::p2::StreamResult<()> {
        Ok(())
    }
    fn check_write(&mut self) -> wasmtime_wasi::p2::StreamResult<usize> {
        Ok(64 * 1024)
    }
}

/// A wall clock stopped at the Unix epoch: the time of day is a grant.
struct StoppedClock;
impl wasmtime_wasi::HostWallClock for StoppedClock {
    fn resolution(&self) -> Duration {
        Duration::from_secs(1)
    }
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}
/// The monotonic clock, in whole milliseconds: enough to pace and measure,
/// too coarse to time the host's caches by.
struct CoarseClock(Instant);
impl wasmtime_wasi::HostMonotonicClock for CoarseClock {
    fn resolution(&self) -> u64 {
        1_000_000
    }
    fn now(&self) -> u64 {
        let ms = self.0.elapsed().as_millis() as u64;
        ms * 1_000_000
    }
}

/// A store's data: the sandbox, the GPU, and the way to the UI thread.
pub(crate) struct Guest {
    wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    limits: StoreLimits,
    pub(crate) gpu: Gpu,
    pub(crate) link: Link,
    pub(crate) log: Log,
    /// What is being called (`init`, `frame`, `event`) and until when.
    calling: &'static str,
    deadline: Instant,
    timeout: Duration,
    timed_out: bool,
    closing: Arc<AtomicBool>,
    /// Its own thread's channel, for pipelines made elsewhere to say so.
    pub(crate) to_self: mpsc::Sender<ToGuest>,
    pub(crate) pending: Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) next_future: u64,
}
impl WasiView for Guest {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl bindings::zgx::app::types::Host for Guest {}
impl bindings::zgx::app::host::Host for Guest {
    fn lock_pointer(&mut self, lock: bool) -> wasmtime::Result<()> {
        self.link.note(Note::Lock(lock));
        Ok(())
    }
    fn set_cursor(&mut self, cursor: wit::Cursor) -> wasmtime::Result<()> {
        self.link.note(Note::Cursor(cursor));
        Ok(())
    }
    fn set_title(&mut self, title: String) -> wasmtime::Result<()> {
        let title: String = title.chars().take(512).collect();
        self.link.note(Note::Title(title));
        Ok(())
    }
    fn log(&mut self, level: wit::LogLevel, message: String) -> wasmtime::Result<()> {
        let level = match level {
            wit::LogLevel::Error => "error",
            wit::LogLevel::Warn => "warn",
            wit::LogLevel::Info => "info",
            wit::LogLevel::Debug => "debug",
        };
        let message: String = message.chars().take(LOG_LINE).collect();
        self.log.line(format!("{level}: {message}"));
        Ok(())
    }
    fn set_animating(&mut self, on: bool) -> wasmtime::Result<()> {
        self.link.note(Note::Animating(on));
        Ok(())
    }
    fn request_frame(&mut self) -> wasmtime::Result<()> {
        self.link.note(Note::RequestFrame);
        Ok(())
    }
    fn present_pixels(
        &mut self,
        pixels: Vec<u8>,
        width: u32,
        height: u32,
    ) -> wasmtime::Result<Result<(), String>> {
        let max = crate::MAX_PIXELS_SIDE;
        if width == 0 || height == 0 || width > max || height > max {
            return Ok(Err(format!(
                "present-pixels: {width} by {height}: each side 1 to {max}"
            )));
        }
        if pixels.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Ok(Err(format!(
                "present-pixels: {} bytes for {width} by {height} (RGBA8 is {})",
                pixels.len(),
                u64::from(width) * u64::from(height) * 4
            )));
        }
        let replaced = self.link.present(Presented::Pixels {
            pixels,
            width,
            height,
            bgra: false,
        });
        self.gpu.take_back(replaced);
        Ok(Ok(()))
    }
}

/// Start `options`'s module on a thread of its own, reading `rx`; what it
/// says goes through `link`. Compiling it is the thread's work too: the UI
/// thread never waits for a guest.
pub(crate) fn start(
    options: WasmOptions,
    rx: mpsc::Receiver<ToGuest>,
    to_self: mpsc::Sender<ToGuest>,
    link: Link,
    log: Log,
    closing: Arc<AtomicBool>,
    frame_asked: Arc<AtomicBool>,
) {
    let failed = link.clone();
    let spawned = std::thread::Builder::new()
        .name("zgx-wasm guest".into())
        .spawn(move || {
            let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(
                    options,
                    rx,
                    to_self,
                    link.clone(),
                    log,
                    closing,
                    frame_asked,
                )
            }))
            .unwrap_or_else(|panic| {
                let why = panic
                    .downcast_ref::<&str>()
                    .map(|s| (*s).to_owned())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                Ended::Failed(format!("the host failed serving the app: {why}"))
            });
            link.note(Note::Ended(ended));
        });
    if let Err(error) = spawned {
        failed.note(Note::Ended(Ended::Failed(format!(
            "no thread for the app: {error}"
        ))));
    }
}

/// The session's life on its thread.
fn run(
    options: WasmOptions,
    rx: mpsc::Receiver<ToGuest>,
    to_self: mpsc::Sender<ToGuest>,
    link: Link,
    log: Log,
    closing: Arc<AtomicBool>,
    frame_asked: Arc<AtomicBool>,
) -> Ended {
    let runtime = match runtime() {
        Ok(runtime) => runtime,
        Err(why) => return Ended::Failed(why),
    };
    let engine = &runtime.engine;
    let path = options.module.display().to_string();
    let bytes = match std::fs::read(&options.module) {
        Ok(bytes) => bytes,
        Err(error) => return Ended::Failed(format!("{path}: {error}")),
    };
    // Validated and compiled here, on this thread, or loaded compiled.
    let component = match crate::cache::component(engine, &bytes) {
        Ok((component, origin, took)) => {
            log.line(format!(
                "{} in {:.1} ms",
                match origin {
                    crate::cache::Origin::Compiled => "compiled",
                    crate::cache::Origin::Cached => "loaded from the cache",
                },
                took.as_secs_f64() * 1000.
            ));
            component
        }
        Err(error) => {
            return Ended::Failed(format!("{path}: not a component this host runs: {error}"));
        }
    };
    drop(bytes);
    let mut linker: Linker<Guest> = Linker::new(engine);
    if let Err(error) = wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .and_then(|()| bindings::App::add_to_linker::<Guest, HasSelf<Guest>>(&mut linker, |g| g))
    {
        return Ended::Failed(format!("the host's interfaces: {error}"));
    }
    let wasi = {
        let mut wasi = WasiCtx::builder();
        wasi.stdout(LogStream {
            log: log.clone(),
            prefix: "stdout: ",
        })
        .stderr(LogStream {
            log: log.clone(),
            prefix: "stderr: ",
        })
        .monotonic_clock(CoarseClock(Instant::now()))
        .allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false);
        if !options.wall_clock {
            wasi.wall_clock(StoppedClock);
        }
        if let Some(dir) = &options.data_dir
            && let Err(error) = wasi.preopened_dir(dir, "/data", wasmtime_wasi::FsPerms::ReadWrite)
        {
            return Ended::Failed(format!("its data folder {}: {error}", dir.display()));
        }
        wasi.build()
    };
    let limits = StoreLimitsBuilder::new()
        .memory_size(options.memory_limit.min(usize::MAX as u64) as usize)
        .memories(4)
        .tables(64)
        .table_elements(1 << 20)
        .instances(64)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(
        engine,
        Guest {
            wasi,
            table: ResourceTable::new(),
            limits,
            gpu: Gpu::new(
                options.gpu_bytes,
                options.gpu_objects,
                options.gpu_watchdog,
                WorkCaps {
                    draws: options.max_draws,
                    workgroups: options.max_workgroups,
                },
                options.shader_loop_budget,
            ),
            link: link.clone(),
            log: log.clone(),
            calling: "instantiate",
            deadline: Instant::now(),
            timeout: options.timeout,
            timed_out: false,
            closing: closing.clone(),
            to_self,
            pending: Arc::default(),
            next_future: 0,
        },
    );
    store.limiter(|guest| &mut guest.limits);
    store.epoch_deadline_callback(|mut cx| {
        let guest = cx.data_mut();
        if guest.closing.load(Ordering::Acquire) {
            return Err(wasmtime::Error::msg("the session closed"));
        }
        if Instant::now() > guest.deadline {
            guest.timed_out = true;
            return Ok(UpdateDeadline::Interrupt);
        }
        Ok(UpdateDeadline::Continue(1))
    });
    let mut session = Session {
        store: Some(store),
        ticker: runtime.ticker.clone(),
    };
    let app = match session.call("instantiate", |store| {
        bindings::App::instantiate(store, &component, &linker)
    }) {
        Ok(app) => app,
        Err(ended) => return ended,
    };
    link.note(Note::Running);

    let mut inited = false;
    // Events before `init` wait for it.
    let mut early: Vec<wit::InputEvent> = Vec::new();
    loop {
        let Ok(first) = rx.recv() else {
            return Ended::Closed;
        };
        let mut frame: Option<f64> = None;
        for message in std::iter::once(first).chain(rx.try_iter()) {
            match message {
                ToGuest::Close => return Ended::Closed,
                ToGuest::Device(host) => {
                    let guest = session.data();
                    match guest.gpu.set_host(&host, &guest.log) {
                        Ok(reset) => {
                            if reset
                                && inited
                                && let Err(ended) = session.event(&app, &wit::InputEvent::GpuReset)
                            {
                                return ended;
                            }
                        }
                        Err(why) => guest.log.line(format!("no GPU: {why}")),
                    }
                }
                ToGuest::Buffers {
                    generation,
                    buffers,
                    cleared,
                } => {
                    let guest = session.data();
                    if let Err(why) = guest.gpu.set_buffers(generation, buffers, cleared) {
                        guest.log.line(why);
                        link.note(Note::Unshared);
                    }
                }
                ToGuest::Resize(info) => {
                    session.data().gpu.set_size(info.width, info.height);
                    if !inited {
                        match session.call("init", |store| app.call_init(&mut *store, info)) {
                            Ok(Ok(())) => {}
                            Ok(Err(why)) => return Ended::Failed(format!("init: {why}")),
                            Err(ended) => return ended,
                        }
                        inited = true;
                        for event in early.drain(..) {
                            if let Err(ended) = session.event(&app, &event) {
                                return ended;
                            }
                        }
                    } else if let Err(ended) = session.event(&app, &wit::InputEvent::Resize(info)) {
                        return ended;
                    }
                }
                ToGuest::Event(event) => {
                    link.input_read();
                    if !inited {
                        if early.len() < 256 {
                            early.push(event);
                        }
                    } else if let Err(ended) = session.event(&app, &event) {
                        return ended;
                    }
                }
                ToGuest::PipelineReady(id) => {
                    if inited
                        && let Err(ended) = session.event(&app, &wit::InputEvent::PipelineReady(id))
                    {
                        return ended;
                    }
                }
                ToGuest::Frame(time) => frame = Some(time),
            }
        }
        if let Some(time) = frame {
            if inited {
                session.data().gpu.begin_frame();
                let drawn = session.call("frame", |store| app.call_frame(&mut *store, time));
                let guest = session.data();
                // Its GPU work done (the watchdog), then shown.
                match guest.gpu.end_frame() {
                    Ok(Some(frame)) => {
                        let replaced = guest.link.present(frame);
                        guest.gpu.take_back(replaced);
                    }
                    Ok(None) => {}
                    Err(why) => return Ended::Failed(why),
                }
                if let Err(ended) = drawn {
                    return ended;
                }
            }
            frame_asked.store(false, Ordering::Release);
        }
        if let Some(lost) = session.data().gpu.lost() {
            return Ended::Failed(lost);
        }
    }
}

struct Session {
    /// Taken as it ends, to be dropped once its GPU work is done (`reap`).
    store: Option<Store<Guest>>,
    ticker: Arc<Ticker>,
}
impl Drop for Session {
    fn drop(&mut self) {
        let Some(store) = self.store.take() else {
            return;
        };
        match store.data().gpu.device.as_ref().map(|d| d.device.clone()) {
            Some(device) => crate::device::reap(device, store),
            None => drop(store),
        }
    }
}
impl Session {
    fn store(&mut self) -> &mut Store<Guest> {
        self.store
            .as_mut()
            .expect("the store, until the session drops")
    }
    fn data(&mut self) -> &mut Guest {
        self.store().data_mut()
    }
    fn event(&mut self, app: &bindings::App, event: &wit::InputEvent) -> Result<(), Ended> {
        self.call("event", |store| app.call_event(&mut *store, event))
    }
    /// Call into the guest with the time limit running, and what goes wrong
    /// as how the session ends: an exit, a trap, the limit, a GPU error the
    /// guest left uncaught (logged, not fatal).
    fn call<R>(
        &mut self,
        what: &'static str,
        f: impl FnOnce(&mut Store<Guest>) -> wasmtime::Result<R>,
    ) -> Result<R, Ended> {
        {
            let guest = self.data();
            guest.calling = what;
            guest.deadline = Instant::now() + guest.timeout;
            guest.gpu.since = Instant::now();
            guest.timed_out = false;
        }
        let scope = self.data().gpu.scope();
        self.store().set_epoch_deadline(1);
        self.ticker.enter();
        let result = f(self.store());
        self.ticker.leave();
        if let Some(scope) = scope {
            for error in scope.pop() {
                self.data()
                    .log
                    .line(format!("gpu error in `{what}`: {error}"));
            }
        }
        result.map_err(|error| self.ended(what, error))
    }
    fn ended(&mut self, what: &str, error: wasmtime::Error) -> Ended {
        let guest = self.data();
        if let Some(exit) = error.downcast_ref::<wasmtime_wasi::I32Exit>() {
            return Ended::Exited(Some(exit.0));
        }
        if guest.closing.load(Ordering::Acquire) {
            return Ended::Closed;
        }
        if guest.timed_out {
            return Ended::Failed(format!(
                "`{what}` ran longer than {:.1} s: interrupted",
                guest.timeout.as_secs_f64()
            ));
        }
        // The cause, not the backtrace wasmtime puts around it.
        let mut why = match error.downcast_ref::<wasmtime::Trap>() {
            Some(trap) => format!("`{what}` trapped: {trap}"),
            None => format!("`{what}` trapped: {}", error.root_cause()),
        };
        if let Some(panic) = guest.log.panic_message() {
            why = format!("{why} (it panicked: {panic})");
        }
        Ended::Failed(why)
    }
}
