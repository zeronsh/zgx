//! The zgx side: start a program with a surface's connection (`App`), show
//! its frames as an element (`surface`), and forward the element's input.
//!
//! ```no_run
//! # fn demo(cx: &mut zgx::compose::Context, runtime: &zgx::reactive::Runtime) {
//! use zgx::compose::prelude::*;
//! use zgx_surface::host::{App, AppOptions, surface};
//! let app = App::spawn(runtime, AppOptions::new("spinner_client")).unwrap();
//! cx.render(surface(&app).w(640.).h(400.).rounded(16.));
//! # }
//! ```
//!
//! A session belongs to whoever holds an `App` (zgx-script's `ui.app`
//! keeps them across reloads): its program is told to close and, a moment
//! later, killed as the session closes (`App::close`) or its last clone
//! drops; on Linux it dies with the host however the host ends
//! (`PR_SET_PDEATHSIG`).
mod painter;

use crate::{
    protocol::{
        BufferKind, Button, ClientMessage, Cursor, Hello, HostMessage, KeyEvent, Modifiers,
        WheelUnit,
    },
    ring::{HostRing, Slot},
    transport::{Receiver, Sender},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use zgx::{
    compose::{NodeHandle, TaskRunner, TaskToken, View, component, gpu_view},
    gpu_view::GpuView,
    input::{EventContext, InputEvent, PointerButton},
    reactive::{Effect, Runtime, Signal},
};

/// Buffers in a generation's ring: one showing, one waiting to show, one
/// being drawn.
pub const RING: u32 = 3;
/// How long a new size holds before a new generation is made for it (a
/// window being dragged shows the last frame stretched until then).
pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(120);
/// Input events queued for a client that isn't reading, beyond which more
/// are dropped (buffers and releases never are).
const MAX_QUEUED_INPUT: usize = 1024;
/// How long a closing program has to exit before it is killed.
const CLOSE_GRACE: Duration = Duration::from_millis(1500);

/// How to start a program (no shell: `command` is run with `args`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppOptions {
    /// The program: a path, or a name looked up on `PATH`.
    pub command: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Where it starts; the host's own directory if `None`.
    pub cwd: Option<PathBuf>,
    /// Variables added to the host's environment for it.
    pub env: Vec<(String, String)>,
}
impl AppOptions {
    /// `command` with no arguments.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            ..Self::default()
        }
    }
    /// With `args`.
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }
}

/// Where a session's program is.
#[derive(Clone, Debug, PartialEq)]
pub enum AppStatus {
    /// Started; it hasn't said hello yet.
    Starting,
    /// Connected.
    Running,
    /// It exited, with its code (None if a signal ended it).
    Exited(Option<i32>),
    /// It couldn't start.
    Failed(String),
}
impl AppStatus {
    /// `"starting"`, `"running"`, `"exited"` or `"failed"`.
    pub fn name(&self) -> &'static str {
        match self {
            AppStatus::Starting => "starting",
            AppStatus::Running => "running",
            AppStatus::Exited(_) => "exited",
            AppStatus::Failed(_) => "failed",
        }
    }
}

/// What the connection's thread hands the UI thread.
enum Note {
    Hello(Hello),
    Presented,
    Title(String),
    Cursor(Cursor),
    Lock(bool),
    Exited(Option<i32>),
}

/// What both threads share.
#[derive(Default)]
pub(crate) struct Shared {
    pub ring: HostRing,
    notes: Vec<Note>,
    /// When the newest frame waiting to show was presented
    /// (`ZGX_SURFACE_LOG`'s delay from present to drawn).
    pub presented_at: Option<std::time::Instant>,
}

/// Messages to the client, written by a thread of their own so a client
/// that stops reading never holds up the UI.
#[derive(Clone)]
pub(crate) struct Outbox {
    /// Each message, and whether it counts as input queued.
    tx: mpsc::Sender<(HostMessage, bool)>,
    input: Arc<AtomicUsize>,
}
impl Outbox {
    /// Send what must arrive (a welcome, buffers, a release).
    pub fn send(&self, message: HostMessage) {
        let _ = self.tx.send((message, false));
    }
    /// Send input, unless the client has this much unread already.
    fn input(&self, message: HostMessage) {
        if self.input.load(Ordering::Relaxed) < MAX_QUEUED_INPUT {
            self.input.fetch_add(1, Ordering::Relaxed);
            let _ = self.tx.send((message, true));
        }
    }
}

/// What `App::on_exit` calls.
type ExitListener = Box<dyn FnMut(Option<i32>)>;

/// A program started with a surface's connection: show it with `surface`.
/// Clones share it; the last dropped closes it.
#[derive(Clone)]
pub struct App(Rc<AppInner>);

pub(crate) struct AppInner {
    runtime: Runtime,
    pub(crate) shared: Arc<Mutex<Shared>>,
    pub(crate) out: Outbox,
    wake: zgx::task::LatestSender<()>,
    woken: RefCell<Option<zgx::task::LatestReceiver<()>>>,
    pump: RefCell<Option<TaskToken>>,
    runner: RefCell<Option<TaskRunner>>,
    pub(crate) view: GpuView,
    status: Signal<AppStatus>,
    title: Signal<String>,
    pid: Option<u32>,
    on_exit: RefCell<Vec<ExitListener>>,
    /// The element showing it, while one is mounted.
    node: RefCell<Option<NodeHandle>>,
    /// Its bounds in the window, to put pointer positions in its pixels.
    bounds: RefCell<Option<Signal<zgx::scene::Rect>>>,
    /// Device pixels a point, as last drawn.
    pub(crate) scale: Cell<f32>,
    /// The client's hello, until the painter welcomes it.
    pub(crate) hello: RefCell<Option<Hello>>,
    /// How buffers are shared, once welcomed.
    pub(crate) kind: Cell<Option<BufferKind>>,
    /// The device last drawn with: polled while releases wait on its work.
    pub(crate) device: RefCell<Option<zgx_gpu::wgpu::Device>>,
    /// Releases waiting for the GPU's work (`on_submitted_work_done`).
    pub(crate) awaiting: Arc<AtomicUsize>,
    polling: RefCell<Option<TaskToken>>,
    resize: RefCell<Option<TaskToken>>,
    visible: Cell<Option<bool>>,
    exited: Arc<AtomicBool>,
    closed: Cell<bool>,
    /// Revoked (`revoke`): its frames show no more, its input goes nowhere.
    pub(crate) revoked: Cell<bool>,
}

impl App {
    /// Start `options.command` with the client's end of a connection
    /// (`ZGX_SURFACE_FD`), its state signals on `runtime`. Its messages are
    /// read once it is attached to a task runner (`attach`; `surface` does
    /// as it mounts); until then they wait.
    pub fn spawn(runtime: &Runtime, options: AppOptions) -> std::io::Result<App> {
        let mut command = std::process::Command::new(&options.command);
        command
            .args(&options.args)
            .stdin(std::process::Stdio::null());
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }
        command.envs(options.env.iter().map(|(k, v)| (k, v)));
        let spawning = crate::transport::Spawning::new()?;
        spawning.prepare(&mut command);
        let child = command.spawn()?;
        let pid = child.id();
        #[cfg(unix)]
        let (sender, receiver) = spawning.started()?;
        #[cfg(windows)]
        let (sender, receiver) = {
            use std::os::windows::io::AsRawHandle;
            spawning.started(child.as_raw_handle())?
        };
        Ok(Self::start(
            runtime,
            sender,
            receiver,
            Some(child),
            Some(pid),
        ))
    }
    /// A session over a connection already made, with no program of its
    /// own (tests; a client in this process).
    pub fn over(runtime: &Runtime, sender: Sender, receiver: Receiver) -> App {
        Self::start(runtime, sender, receiver, None, None)
    }

    fn start(
        runtime: &Runtime,
        sender: Sender,
        receiver: Receiver,
        child: Option<std::process::Child>,
        pid: Option<u32>,
    ) -> App {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let (wake, woken) = zgx::task::latest::<()>();
        let exited = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<(HostMessage, bool)>();
        let out = Outbox {
            tx,
            input: Arc::default(),
        };
        writer(sender, rx, out.input.clone());
        reader(receiver, shared.clone(), out.clone(), wake.clone());
        if let Some(child) = child {
            waiter(child, shared.clone(), wake.clone(), exited.clone());
        }
        let inner = Rc::new_cyclic(|weak: &Weak<AppInner>| AppInner {
            runtime: runtime.clone(),
            shared,
            out,
            wake,
            woken: RefCell::new(Some(woken)),
            pump: RefCell::default(),
            runner: RefCell::default(),
            view: zgx_gpu::viewport::view(painter::Painter::new(weak.clone())),
            status: runtime.signal(AppStatus::Starting),
            title: runtime.signal(String::new()),
            pid,
            on_exit: RefCell::default(),
            node: RefCell::default(),
            bounds: RefCell::default(),
            scale: Cell::new(1.),
            hello: RefCell::default(),
            kind: Cell::new(None),
            device: RefCell::default(),
            awaiting: Arc::default(),
            polling: RefCell::default(),
            resize: RefCell::default(),
            // A client takes its surface as shown until told otherwise.
            visible: Cell::new(Some(true)),
            exited,
            closed: Cell::new(false),
            revoked: Cell::new(false),
        });
        App(inner)
    }

    /// A session whose program couldn't start: its status says why.
    pub fn failed(runtime: &Runtime, why: impl Into<String>) -> App {
        let ((a_tx, a_rx), _) = crate::transport::pair().expect("a socket pair");
        let app = Self::start(runtime, a_tx, a_rx, None, None);
        app.0.status.set(AppStatus::Failed(why.into()));
        app.0.closed.set(true);
        app
    }

    /// Read its messages with `runner`'s tasks, from now. Once is enough;
    /// later calls do nothing.
    pub fn attach(&self, runner: &TaskRunner) {
        let Some(mut woken) = self.0.woken.borrow_mut().take() else {
            return;
        };
        *self.0.runner.borrow_mut() = Some(runner.clone());
        let weak = Rc::downgrade(&self.0);
        let task = runner.spawn(Box::pin(async move {
            while woken.recv().await.is_some() {
                let Some(app) = weak.upgrade() else { return };
                App(app).drain();
            }
        }));
        *self.0.pump.borrow_mut() = Some(task);
        // What arrived before it was attached.
        self.0.wake.send(());
    }
    /// Whether `other` is this session (a clone of it).
    pub fn ptr_eq(&self, other: &App) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    /// Its state: starting, running, exited or failed (tracked).
    pub fn status(&self) -> Signal<AppStatus> {
        self.0.status.clone()
    }
    /// The title its program set; empty until it does (tracked).
    pub fn title(&self) -> Signal<String> {
        self.0.title.clone()
    }
    /// Its program's process id, if it started.
    pub fn pid(&self) -> Option<u32> {
        self.0.pid
    }
    /// The GPU view its frames show in (`surface` shows it).
    pub fn view(&self) -> &GpuView {
        &self.0.view
    }
    /// Call `f` with its exit code (None for a signal) as its program
    /// exits.
    pub fn on_exit(&self, f: impl FnMut(Option<i32>) + 'static) {
        self.0.on_exit.borrow_mut().push(Box::new(f));
    }
    /// Forget the exit listeners (`on_exit`).
    pub fn clear_on_exit(&self) {
        self.0.on_exit.borrow_mut().clear();
    }
    /// Whether it is closed (`close`, or never started).
    pub fn is_closed(&self) -> bool {
        self.0.closed.get()
    }
    /// End the session for good, as a permission taken back does: closed
    /// as `close` does, and every element showing it shows nothing from
    /// now on and forwards nothing.
    pub fn revoke(&self) {
        self.0.revoked.set(true);
        self.0.close();
        self.0.view.invalidate();
    }
    /// Whether it was revoked (`revoke`).
    pub fn is_revoked(&self) -> bool {
        self.0.revoked.get()
    }
    /// End the session: the program is told to close and killed if it
    /// hasn't exited a moment later.
    pub fn close(&self) {
        self.0.close();
    }

    /// What the connection's thread handed over.
    fn drain(&self) {
        let notes = std::mem::take(&mut self.0.shared.lock().unwrap().notes);
        for note in notes {
            match note {
                Note::Hello(hello) => {
                    *self.0.hello.borrow_mut() = Some(hello);
                    if matches!(
                        self.0.status.with_untracked(Clone::clone),
                        AppStatus::Starting
                    ) {
                        self.0.status.set(AppStatus::Running);
                    }
                    // The painter answers with what its device can share.
                    self.0.view.invalidate();
                }
                Note::Presented => self.0.view.invalidate(),
                Note::Title(title) => {
                    self.0.title.set(title);
                }
                Note::Cursor(cursor) => {
                    let node = self.0.node.borrow().clone();
                    if let Some(node) = node
                        && let Some(scene) = node.scene()
                    {
                        scene
                            .borrow_mut()
                            .set_cursor(node.node(), Some(zgx_cursor(cursor)));
                    }
                }
                Note::Lock(lock) => {
                    let node = self.0.node.borrow().clone();
                    match (lock, node) {
                        (true, Some(node)) => {
                            if !node.lock_pointer() {
                                self.0.out.input(HostMessage::PointerLock(false));
                            }
                        }
                        (true, None) => self.0.out.input(HostMessage::PointerLock(false)),
                        (false, Some(node)) => node.unlock_pointer(),
                        (false, None) => {}
                    }
                }
                Note::Exited(code) => {
                    self.0.status.set(AppStatus::Exited(code));
                    let mut listeners = std::mem::take(&mut *self.0.on_exit.borrow_mut());
                    for listener in &mut listeners {
                        listener(code);
                    }
                    let mut kept = self.0.on_exit.borrow_mut();
                    listeners.append(&mut kept);
                    *kept = listeners;
                }
            }
        }
    }

    /// The element showing it mounted.
    fn mounted(&self, handle: NodeHandle) {
        *self.0.bounds.borrow_mut() = handle.observe_bounds();
        *self.0.node.borrow_mut() = Some(handle.clone());
        let (app, held) = (Rc::downgrade(&self.0), handle.clone());
        // Whether it shows, once laid out (before, it shows nowhere yet).
        handle.after_layout(move || {
            let visible = held.observe_visibility();
            let shown = held.observe_shown();
            let Some(inner) = app.upgrade() else { return };
            let weak = app.clone();
            let effect: Effect = inner.runtime.effect(move || {
                let on = visible.as_ref().is_none_or(Signal::get)
                    && shown.as_ref().is_none_or(Signal::get);
                if let Some(app) = weak.upgrade() {
                    app.set_visible(on);
                }
            });
            held.retain(Mounted {
                app,
                node: held.node(),
                _effect: effect,
            });
        });
    }

    /// The element's input, forwarded.
    fn input(&self, event: &mut EventContext) {
        if self.0.revoked.get() {
            return;
        }
        let scale = self.0.scale.get();
        let origin = self
            .0
            .bounds
            .borrow()
            .as_ref()
            .map_or((0., 0.), |b| b.with_untracked(|r| (r.x, r.y)));
        let at = |x: f32, y: f32| ((x - origin.0) * scale, (y - origin.1) * scale);
        let input = event.event.clone();
        let message = match &input {
            InputEvent::PointerMove { x, y } => {
                let (x, y) = at(*x, *y);
                HostMessage::PointerMove { x, y }
            }
            InputEvent::PointerDown { x, y, button } | InputEvent::PointerUp { x, y, button } => {
                let down = matches!(input, InputEvent::PointerDown { .. });
                let (x, y) = at(*x, *y);
                HostMessage::Button {
                    button: button_of(*button),
                    down,
                    x,
                    y,
                }
            }
            InputEvent::PointerMotion { dx, dy } => HostMessage::PointerMotion { dx: *dx, dy: *dy },
            InputEvent::Scroll {
                delta_x, delta_y, ..
            } => {
                event.prevent_default();
                HostMessage::Wheel {
                    dx: delta_x * scale,
                    dy: delta_y * scale,
                    unit: WheelUnit::Pixels,
                }
            }
            InputEvent::KeyDown {
                modifiers, repeat, ..
            } => {
                let message = HostMessage::Key(KeyEvent {
                    code: event.key_code().unwrap_or_default().to_owned(),
                    down: true,
                    repeat: *repeat,
                    modifiers: modifiers_of(*modifiers),
                });
                // Its keys are the program's (Tab doesn't move focus out);
                // the app's shortcuts (Command, Control) stay the app's.
                if !modifiers.primary_shortcut() {
                    event.prevent_default();
                }
                message
            }
            InputEvent::KeyUp { modifiers, .. } => HostMessage::Key(KeyEvent {
                code: event.key_code().unwrap_or_default().to_owned(),
                down: false,
                repeat: false,
                modifiers: modifiers_of(*modifiers),
            }),
            InputEvent::Text(text) | InputEvent::ImeCommit(text) => HostMessage::Text(text.clone()),
            InputEvent::Focus => HostMessage::Focus(true),
            InputEvent::Blur => HostMessage::Focus(false),
            InputEvent::PointerLockChange(locked) => HostMessage::PointerLock(*locked),
            InputEvent::PointerLeave => HostMessage::PointerLeave,
            _ => return,
        };
        self.0.out.input(message);
    }
}

/// Held by the element's node: as it goes, the program hears it hidden.
struct Mounted {
    app: Weak<AppInner>,
    node: zgx::scene::NodeId,
    _effect: Effect,
}
impl Drop for Mounted {
    fn drop(&mut self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let ours = app
            .node
            .borrow()
            .as_ref()
            .is_some_and(|n| n.node() == self.node);
        if ours {
            *app.node.borrow_mut() = None;
            *app.bounds.borrow_mut() = None;
            app.set_visible(false);
        }
    }
}

impl AppInner {
    fn set_visible(&self, visible: bool) {
        if self.visible.replace(Some(visible)) != Some(visible) {
            self.out.input(HostMessage::Visible(visible));
        }
    }
    /// The runner its tasks run on, once attached.
    fn runner(&self) -> Option<TaskRunner> {
        self.runner.borrow().clone()
    }
    /// Draw again after `delay` (a size held long enough for a new
    /// generation).
    pub(crate) fn invalidate_after(self: &Rc<Self>, delay: Duration) {
        let Some(runner) = self.runner() else { return };
        let view = self.view.clone();
        let task = runner.spawn(Box::pin(async move {
            zgx::timer::sleep(delay).await;
            view.invalidate();
        }));
        *self.resize.borrow_mut() = Some(task);
    }
    /// Poll the device until the releases waiting on its work have gone:
    /// wgpu calls them back only when polled or submitted to, and a client
    /// out of buffers submits nothing to the host's device.
    pub(crate) fn poll_until_released(self: &Rc<Self>) {
        let running = self.polling.borrow().is_some();
        if running {
            return;
        }
        let (Some(runner), Some(device)) = (self.runner(), self.device.borrow().clone()) else {
            return;
        };
        let (awaiting, weak) = (self.awaiting.clone(), Rc::downgrade(self));
        let task = runner.spawn(Box::pin(async move {
            let mut wait = Duration::from_millis(2);
            while awaiting.load(Ordering::Acquire) > 0 {
                zgx::timer::sleep(wait).await;
                let _ = device.poll(zgx_gpu::wgpu::PollType::Poll);
                wait = (wait * 2).min(Duration::from_millis(50));
            }
            if let Some(app) = weak.upgrade() {
                // This task is ending: the next wait starts another.
                drop(app.polling.borrow_mut().take());
            }
        }));
        *self.polling.borrow_mut() = Some(task);
    }
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.out.send(HostMessage::Close);
        if let Some(node) = self.node.borrow().as_ref() {
            node.unlock_pointer();
        }
        let (pid, exited) = (self.pid, self.exited.clone());
        if let Some(pid) = pid {
            let _ = std::thread::Builder::new()
                .name("zgx-surface close".into())
                .spawn(move || {
                    let until = std::time::Instant::now() + CLOSE_GRACE;
                    while std::time::Instant::now() < until {
                        if exited.load(Ordering::Acquire) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    if !exited.load(Ordering::Acquire) {
                        kill(pid);
                    }
                });
        }
    }
}
impl Drop for AppInner {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(unix)]
fn kill(pid: u32) {
    // Safety: a signal to the process this session started, not yet reaped
    // (the waiter sets `exited` before it is).
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
}
#[cfg(not(unix))]
fn kill(_pid: u32) {
    // Windows: TerminateProcess on the child's handle (follow-up work).
}

/// The thread writing messages to the client.
fn writer(mut sender: Sender, rx: mpsc::Receiver<(HostMessage, bool)>, input: Arc<AtomicUsize>) {
    let _ = std::thread::Builder::new()
        .name("zgx-surface write".into())
        .spawn(move || {
            // Only what `Outbox::input` counted is uncounted: a resize sent
            // with `send` once took the count below zero, and then every
            // input event after was dropped as too many unread.
            while let Ok((message, counted)) = rx.recv() {
                if counted {
                    input.fetch_sub(1, Ordering::Relaxed);
                }
                let close = matches!(message, HostMessage::Close);
                if sender.send_host(&message).is_err() {
                    return;
                }
                if close {
                    sender.shutdown();
                    return;
                }
            }
        });
}

/// The thread reading the client's messages.
fn reader(
    mut receiver: Receiver,
    shared: Arc<Mutex<Shared>>,
    out: Outbox,
    wake: zgx::task::LatestSender<()>,
) {
    let _ = std::thread::Builder::new()
        .name("zgx-surface read".into())
        .spawn(move || {
            loop {
                let message = match receiver.recv_client() {
                    Ok(Some(message)) => message,
                    Ok(None) => return,
                    Err(error) => {
                        eprintln!("zgx-surface: the client's connection failed: {error}");
                        return;
                    }
                };
                let note = match message {
                    ClientMessage::Hello(hello) => Note::Hello(hello),
                    ClientMessage::Present { generation, index } => {
                        let presented = {
                            let mut shared = shared.lock().unwrap();
                            shared.presented_at = Some(std::time::Instant::now());
                            shared.ring.present(Slot { generation, index })
                        };
                        match presented {
                            // A frame never shown, replaced: the client's again.
                            Ok(Some(skipped)) => out.send(HostMessage::Release {
                                generation: skipped.generation,
                                index: skipped.index,
                            }),
                            Ok(None) => {}
                            Err(slot) => {
                                eprintln!(
                                    "zgx-surface: the client presented buffer {} of generation {}, which the host holds",
                                    slot.index, slot.generation
                                );
                                continue;
                            }
                        }
                        Note::Presented
                    }
                    ClientMessage::SetCursor(cursor) => Note::Cursor(cursor),
                    ClientMessage::LockPointer(lock) => Note::Lock(lock),
                    ClientMessage::Title(title) => Note::Title(title),
                };
                shared.lock().unwrap().notes.push(note);
                if !wake.send(()) {
                    return;
                }
            }
        });
}

/// The thread waiting for the program to exit.
fn waiter(
    mut child: std::process::Child,
    shared: Arc<Mutex<Shared>>,
    wake: zgx::task::LatestSender<()>,
    exited: Arc<AtomicBool>,
) {
    let _ = std::thread::Builder::new()
        .name("zgx-surface wait".into())
        .spawn(move || {
            // Exited, before it's reaped: no kill reaches a pid used again.
            #[cfg(unix)]
            loop {
                let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let id = child.id() as libc::id_t;
                let waited = unsafe {
                    libc::waitid(libc::P_PID, id, &mut info, libc::WEXITED | libc::WNOWAIT)
                };
                if waited == 0
                    || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                {
                    break;
                }
            }
            exited.store(true, Ordering::Release);
            let code = child.wait().ok().and_then(|s| s.code());
            shared.lock().unwrap().notes.push(Note::Exited(code));
            wake.send(());
        });
}

/// Shows `app`'s frames, laid out as any box (size it, round it, clip it,
/// put UI over it), focusable, forwarding its input to the program: keys
/// while focused, the pointer over it (in its device pixels), and a lock
/// of the pointer the program asks for just after a press.
pub fn surface(app: &App) -> View {
    let app = app.clone();
    component(move |cx| {
        if let Some(runner) = cx.try_service::<TaskRunner>() {
            app.attach(&runner);
        }
        let (events, mounted) = (app.clone(), app.clone());
        gpu_view("App", &app.0.view)
            .focusable(true)
            .on_event(move |event| events.input(event))
            .on_mount(move |handle| mounted.mounted(handle))
    })
}

fn button_of(button: PointerButton) -> Button {
    match button {
        PointerButton::Primary => Button::Left,
        PointerButton::Secondary => Button::Right,
        PointerButton::Middle => Button::Middle,
        // zgx-desktop's numbers for winit's Back and Forward.
        PointerButton::Other(4) => Button::Back,
        PointerButton::Other(5) => Button::Forward,
        PointerButton::Other(n) => Button::Other(n),
    }
}
fn modifiers_of(modifiers: zgx::input::Modifiers) -> Modifiers {
    Modifiers {
        shift: modifiers.shift,
        control: modifiers.control,
        alt: modifiers.alt,
        meta: modifiers.meta,
    }
}
fn zgx_cursor(cursor: Cursor) -> zgx::cursor::Cursor {
    use zgx::cursor::Cursor as C;
    match cursor {
        Cursor::Default | Cursor::Hidden => C::Default,
        Cursor::Pointer => C::Pointer,
        Cursor::Text => C::Text,
        Cursor::Crosshair => C::Crosshair,
        Cursor::Move => C::Move,
        Cursor::Grab => C::Grab,
        Cursor::Grabbing => C::Grabbing,
        Cursor::NotAllowed => C::NotAllowed,
        Cursor::Wait => C::Wait,
        Cursor::Progress => C::Progress,
        Cursor::Help => C::Help,
        Cursor::EwResize => C::EwResize,
        Cursor::NsResize => C::NsResize,
        Cursor::NeswResize => C::NeswResize,
        Cursor::NwseResize => C::NwseResize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What must arrive (a resize, buffers) is not input: sending it never
    /// takes the input count below zero, which once dropped every input
    /// event after a session's first resize as too many unread.
    #[test]
    fn input_still_flows_after_what_must_arrive() {
        let ((host_tx, _host_rx), (_client_tx, mut client_rx)) = crate::transport::pair().unwrap();
        let (tx, rx) = mpsc::channel();
        let out = Outbox {
            tx,
            input: Arc::default(),
        };
        writer(host_tx, rx, out.input.clone());
        out.send(HostMessage::Resize {
            width: 640,
            height: 480,
            scale: 1.5,
        });
        out.send(HostMessage::Release {
            generation: 1,
            index: 0,
        });
        for i in 0..3 {
            out.input(HostMessage::PointerMove { x: i as f32, y: 0. });
        }
        let mut heard = Vec::new();
        for _ in 0..5 {
            heard.push(format!("{:?}", client_rx.recv_host().unwrap().unwrap()));
        }
        assert_eq!(
            heard[2..],
            [
                "PointerMove { x: 0.0, y: 0.0 }",
                "PointerMove { x: 1.0, y: 0.0 }",
                "PointerMove { x: 2.0, y: 0.0 }"
            ]
        );
        assert_eq!(
            out.input.load(Ordering::Relaxed),
            0,
            "every counted input uncounted, nothing more"
        );
    }
}
