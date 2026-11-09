//! The UI thread's side of a session: its state as signals, its element
//! (`surface`), and the element's input forwarded to the guest's thread.
use crate::{
    painter::Painter,
    runtime::{Log, ToGuest, bindings::zgx::app::types as wit},
    shared::{Ended, Link, Note, Shared},
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
    time::{Duration, Instant},
};
use zgx::{
    compose::{NodeHandle, TaskRunner, TaskToken, View, component, gpu_view},
    gpu_view::GpuView,
    input::{EventContext, InputEvent, PointerButton},
    reactive::{Effect, Runtime, Signal},
};

/// Input events queued for a guest that isn't reading, beyond which more
/// are dropped (sizes, devices and frames never are).
const MAX_QUEUED_INPUT: usize = 1024;

/// What to run, and the sandbox's limits.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct WasmOptions {
    /// The component (`.wasm`, built for `wasm32-wasip2` against
    /// `zgx:app/app`).
    pub module: PathBuf,
    /// Its linear memories' most, in bytes (1 GiB).
    pub memory_limit: u64,
    /// The longest any call into it may run (`init`, a `frame`, an
    /// `event`) before it is interrupted and the session fails (5 s).
    pub timeout: Duration,
    /// The most its GPU buffers and textures may hold, in bytes (1 GiB).
    pub gpu_bytes: u64,
    /// The most GPU objects it may hold at once (16384).
    pub gpu_objects: u32,
    /// A folder of its own, seen as `/data` (read and write): none by
    /// default, when it sees no file at all.
    pub data_dir: Option<PathBuf>,
    /// Whether it may read the time of day; it reads the Unix epoch if not.
    pub wall_clock: bool,
}
impl WasmOptions {
    /// `module`, with the default limits and nothing granted.
    pub fn new(module: impl Into<PathBuf>) -> Self {
        Self {
            module: module.into(),
            memory_limit: 1 << 30,
            timeout: Duration::from_secs(5),
            gpu_bytes: 1 << 30,
            gpu_objects: 16384,
            data_dir: None,
            wall_clock: false,
        }
    }
    pub fn memory_limit(mut self, bytes: u64) -> Self {
        self.memory_limit = bytes;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
    pub fn gpu_bytes(mut self, bytes: u64) -> Self {
        self.gpu_bytes = bytes;
        self
    }
    pub fn gpu_objects(mut self, objects: u32) -> Self {
        self.gpu_objects = objects;
        self
    }
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }
    pub fn wall_clock(mut self, granted: bool) -> Self {
        self.wall_clock = granted;
        self
    }
}

/// Where a session is.
#[derive(Clone, Debug, PartialEq)]
pub enum WasmStatus {
    /// Compiling and instantiating.
    Starting,
    Running,
    /// It exited (`wasi:cli/exit`), or was closed (`None`).
    Exited(Option<i32>),
    /// It couldn't start, trapped or ran too long: why.
    Failed(String),
}
impl WasmStatus {
    /// `"starting"`, `"running"`, `"exited"` or `"failed"`, as an external
    /// surface's `AppStatus` names them.
    pub fn name(&self) -> &'static str {
        match self {
            WasmStatus::Starting => "starting",
            WasmStatus::Running => "running",
            WasmStatus::Exited(_) => "exited",
            WasmStatus::Failed(_) => "failed",
        }
    }
}

type ExitListener = Box<dyn FnMut(Option<i32>)>;

/// A WebAssembly app running in this process: show it with `surface`.
/// Clones share it; the last dropped closes it.
#[derive(Clone)]
pub struct WasmApp(pub(crate) Rc<Inner>);

pub(crate) struct Inner {
    runtime: Runtime,
    pub(crate) link: Link,
    tx: mpsc::Sender<ToGuest>,
    log: Log,
    closing: Arc<AtomicBool>,
    /// A frame was asked of the guest and it hasn't drawn it yet: no other
    /// is asked meanwhile (a slow guest drops frames, never queues them).
    frame_asked: Arc<AtomicBool>,
    woken: RefCell<Option<zgx::task::LatestReceiver<()>>>,
    pump: RefCell<Option<TaskToken>>,
    runner: RefCell<Option<TaskRunner>>,
    pub(crate) view: GpuView,
    status: Signal<WasmStatus>,
    title: Signal<String>,
    on_exit: RefCell<Vec<ExitListener>>,
    node: RefCell<Option<NodeHandle>>,
    bounds: RefCell<Option<Signal<zgx::scene::Rect>>>,
    /// Device pixels a point, as last drawn (1 until drawn).
    pub(crate) scale: Cell<f32>,
    /// The painter has drawn: its size is the element's from then on (until
    /// then, the layout's times the scale).
    pub(crate) painted: Cell<bool>,
    /// What the guest was last told: width, height, scale.
    told: Cell<Option<(u32, u32, f32)>>,
    started: Instant,
    /// Frames are asked every refresh (`set-animating`), or once asked.
    animating: Cell<bool>,
    wanted: Cell<bool>,
    visible: Cell<Option<bool>>,
    closed: Cell<bool>,
    pub(crate) revoked: Cell<bool>,
    locked: Cell<bool>,
    focused: Cell<bool>,
    frames: RefCell<Option<zgx::frame::FrameClock>>,
    refresh: Cell<f32>,
    refreshing: RefCell<Option<TaskToken>>,
    refresh_checked: Cell<Option<Instant>>,
}

impl WasmApp {
    /// Start `options.module` on a thread of its own (compiled there: this
    /// returns at once), its state signals on `runtime`. Its notes are read
    /// once it is attached to a task runner (`attach`; `surface` does as it
    /// mounts). A module that can't start is a failed session.
    pub fn spawn(runtime: &Runtime, options: WasmOptions) -> WasmApp {
        let (tx, rx) = mpsc::channel();
        let app = Self::new(runtime, tx);
        let link = app.0.link.clone();
        crate::runtime::start(
            options,
            rx,
            link,
            app.0.log.clone(),
            app.0.closing.clone(),
            app.0.frame_asked.clone(),
        );
        app
    }
    /// A session that couldn't start: its status says why.
    pub fn failed(runtime: &Runtime, why: impl Into<String>) -> WasmApp {
        let app = Self::new(runtime, mpsc::channel().0);
        app.0.status.set(WasmStatus::Failed(why.into()));
        app.0.closed.set(true);
        app
    }
    fn new(runtime: &Runtime, tx: mpsc::Sender<ToGuest>) -> WasmApp {
        let (wake, woken) = zgx::task::latest::<()>();
        let link = Link {
            shared: Arc::new(Mutex::new(Shared::default())),
            wake,
            input: Arc::new(AtomicUsize::new(0)),
        };
        let inner = Rc::new_cyclic(|weak: &Weak<Inner>| Inner {
            runtime: runtime.clone(),
            link,
            tx,
            log: Log::default(),
            closing: Arc::default(),
            frame_asked: Arc::default(),
            woken: RefCell::new(Some(woken)),
            pump: RefCell::default(),
            runner: RefCell::default(),
            view: zgx_gpu::viewport::view(Painter::new(weak.clone())),
            status: runtime.signal(WasmStatus::Starting),
            title: runtime.signal(String::new()),
            on_exit: RefCell::default(),
            node: RefCell::default(),
            bounds: RefCell::default(),
            scale: Cell::new(1.),
            painted: Cell::new(false),
            told: Cell::new(None),
            started: Instant::now(),
            animating: Cell::new(true),
            wanted: Cell::new(true),
            visible: Cell::new(Some(true)),
            closed: Cell::new(false),
            revoked: Cell::new(false),
            locked: Cell::new(false),
            focused: Cell::new(false),
            frames: RefCell::new(None),
            refresh: Cell::new(0.),
            refreshing: RefCell::new(None),
            refresh_checked: Cell::new(None),
        });
        WasmApp(inner)
    }

    /// Read its notes with `runner`'s tasks, from now. Once is enough.
    pub fn attach(&self, runner: &TaskRunner) {
        let Some(mut woken) = self.0.woken.borrow_mut().take() else {
            return;
        };
        *self.0.runner.borrow_mut() = Some(runner.clone());
        let weak = Rc::downgrade(&self.0);
        let task = runner.spawn(Box::pin(async move {
            while woken.recv().await.is_some() {
                let Some(app) = weak.upgrade() else { return };
                WasmApp(app).drain();
            }
        }));
        *self.0.pump.borrow_mut() = Some(task);
        self.0.link.wake.send(());
    }
    /// Whether `other` is this session (a clone of it).
    pub fn ptr_eq(&self, other: &WasmApp) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
    /// Starting, running, exited or failed (tracked).
    pub fn status(&self) -> Signal<WasmStatus> {
        self.0.status.clone()
    }
    /// The title the app set; empty until it does (tracked).
    pub fn title(&self) -> Signal<String> {
        self.0.title.clone()
    }
    /// The last lines of its log: its `log` calls, standard output and
    /// error, and GPU errors it left uncaught.
    pub fn log(&self) -> Vec<String> {
        self.0.log.lines()
    }
    /// The GPU view its frames show in (`surface` shows it).
    pub fn view(&self) -> &GpuView {
        &self.0.view
    }
    /// Call `f` as it ends: its exit code, or `None` (closed, trapped).
    pub fn on_exit(&self, f: impl FnMut(Option<i32>) + 'static) {
        self.0.on_exit.borrow_mut().push(Box::new(f));
    }
    pub fn clear_on_exit(&self) {
        self.0.on_exit.borrow_mut().clear();
    }
    /// Whether it is closed (`close`, or never started).
    pub fn is_closed(&self) -> bool {
        self.0.closed.get()
    }
    /// End the session for good, as a permission taken back does: closed,
    /// and every element showing it shows nothing and forwards nothing.
    pub fn revoke(&self) {
        self.0.revoked.set(true);
        self.0.close();
        self.0.view.invalidate();
    }
    pub fn is_revoked(&self) -> bool {
        self.0.revoked.get()
    }
    /// End the session: a call running in the guest is interrupted.
    pub fn close(&self) {
        self.0.close();
    }

    fn drain(&self) {
        let notes = std::mem::take(&mut self.0.link.shared.lock().unwrap().notes);
        for note in notes {
            match note {
                Note::Running => {
                    if matches!(
                        self.0.status.with_untracked(Clone::clone),
                        WasmStatus::Starting
                    ) {
                        self.0.status.set(WasmStatus::Running);
                    }
                    self.0
                        .view
                        .animate(self.0.animating.get() && !self.0.revoked.get());
                    self.0.view.invalidate();
                }
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
                                self.0.input(wit::InputEvent::PointerLock(false));
                            }
                        }
                        (true, None) => self.0.input(wit::InputEvent::PointerLock(false)),
                        (false, Some(node)) => node.unlock_pointer(),
                        (false, None) => {}
                    }
                }
                Note::Animating(on) => {
                    self.0.animating.set(on);
                    let running = matches!(
                        self.0.status.with_untracked(Clone::clone),
                        WasmStatus::Running
                    );
                    self.0.view.animate(on && running && !self.0.revoked.get());
                }
                Note::RequestFrame => {
                    self.0.wanted.set(true);
                    self.0.view.invalidate();
                }
                Note::Presented => self.0.view.invalidate(),
                Note::Ended(ended) => self.0.ended(ended),
            }
        }
    }

    fn mounted(&self, handle: NodeHandle) {
        let bounds = handle.observe_bounds();
        *self.0.bounds.borrow_mut() = bounds.clone();
        *self.0.node.borrow_mut() = Some(handle.clone());
        let (app, held) = (Rc::downgrade(&self.0), handle.clone());
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
            // Until the painter draws (and without a GPU, always), the
            // element's size is its layout's, at the scale last drawn.
            let weak = app.clone();
            let sized: Effect = inner.runtime.effect(move || {
                let Some(rect) = bounds.as_ref().map(Signal::get) else {
                    return;
                };
                let Some(app) = weak.upgrade() else { return };
                if app.painted.get() {
                    return;
                }
                let scale = app.scale.get();
                let resized = app.resize(
                    (rect.width * scale).round().max(0.) as u32,
                    (rect.height * scale).round().max(0.) as u32,
                    scale,
                );
                if resized {
                    app.view.invalidate();
                }
            });
            // A modifier alone is no key event of zgx's, yet a game's Shift
            // is crouch: while focused, each change is the guest's as that
            // key's press or release.
            let held_now = zgx::input::held_modifiers(&inner.runtime);
            let weak = app.clone();
            let last = Cell::new(held_now.with_untracked(|m| *m));
            let modifiers: Effect = inner.runtime.effect(move || {
                let now = held_now.get();
                let was = last.replace(now);
                let Some(app) = weak.upgrade() else { return };
                if !app.focused.get() || app.revoked.get() {
                    return;
                }
                for (code, before, after) in [
                    ("ShiftLeft", was.shift, now.shift),
                    ("ControlLeft", was.control, now.control),
                    ("AltLeft", was.alt, now.alt),
                    ("MetaLeft", was.meta, now.meta),
                ] {
                    if before != after {
                        app.input(wit::InputEvent::Key(wit::KeyEvent {
                            code: code.to_owned(),
                            down: after,
                            repeat: false,
                            modifiers: modifiers_of(now),
                        }));
                    }
                }
            });
            held.retain(Mounted {
                app,
                node: held.node(),
                _effects: [effect, sized, modifiers],
            });
        });
    }

    fn input(&self, event: &mut EventContext) {
        if self.0.revoked.get() || self.0.closed.get() {
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
                wit::InputEvent::PointerMove(wit::Point { x, y })
            }
            InputEvent::PointerDown { x, y, button } | InputEvent::PointerUp { x, y, button } => {
                let down = matches!(input, InputEvent::PointerDown { .. });
                let (x, y) = at(*x, *y);
                wit::InputEvent::Button(wit::ButtonEvent {
                    button: button_of(*button),
                    down,
                    x,
                    y,
                })
            }
            InputEvent::PointerMotion { dx, dy } => {
                wit::InputEvent::PointerMotion(wit::Motion { dx: *dx, dy: *dy })
            }
            InputEvent::Scroll {
                delta_x, delta_y, ..
            } => {
                event.prevent_default();
                wit::InputEvent::Wheel(wit::WheelEvent {
                    dx: delta_x * scale,
                    dy: delta_y * scale,
                    unit: wit::WheelUnit::Pixels,
                })
            }
            InputEvent::KeyDown {
                key,
                modifiers,
                repeat,
            } => {
                let message = wit::InputEvent::Key(wit::KeyEvent {
                    code: event.key_code().unwrap_or_default().to_owned(),
                    down: true,
                    repeat: *repeat,
                    modifiers: modifiers_of(*modifiers),
                });
                // Its keys are the app's (Tab doesn't move focus out); the
                // window's shortcuts stay the window's, unless the app holds
                // the pointer: then they're its own.
                if self.0.locked.get() || !modifiers.primary_shortcut() {
                    event.prevent_default();
                    // A key prevented types nothing (the window makes text
                    // only of keys no one took): what it types is sent here.
                    if let Some(text) = typed(key, *modifiers) {
                        self.0.input(message);
                        self.0.input(wit::InputEvent::Text(text));
                        return;
                    }
                }
                message
            }
            InputEvent::KeyUp { modifiers, .. } => wit::InputEvent::Key(wit::KeyEvent {
                code: event.key_code().unwrap_or_default().to_owned(),
                down: false,
                repeat: false,
                modifiers: modifiers_of(*modifiers),
            }),
            InputEvent::Text(text) | InputEvent::ImeCommit(text) => {
                wit::InputEvent::Text(text.clone())
            }
            InputEvent::Focus => {
                self.0.focused.set(true);
                wit::InputEvent::Focus(true)
            }
            InputEvent::Blur => {
                self.0.focused.set(false);
                self.0.set_locked(false);
                wit::InputEvent::Focus(false)
            }
            InputEvent::PointerLockChange(locked) => {
                self.0.set_locked(*locked);
                wit::InputEvent::PointerLock(*locked)
            }
            InputEvent::PointerLeave => wit::InputEvent::PointerLeave,
            _ => return,
        };
        self.0.input(message);
    }
}

/// Held by the element's node: as it goes, the guest hears it hidden.
struct Mounted {
    app: Weak<Inner>,
    node: zgx::scene::NodeId,
    _effects: [Effect; 3],
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
            app.focused.set(false);
            app.set_locked(false);
            *app.node.borrow_mut() = None;
            *app.bounds.borrow_mut() = None;
            app.set_visible(false);
        }
    }
}

impl Inner {
    /// Tell the guest what must arrive (a size, a device, a frame).
    pub(crate) fn send(&self, message: ToGuest) {
        let _ = self.tx.send(message);
    }
    /// Input, unless the guest has this much unread already.
    pub(crate) fn input(&self, event: wit::InputEvent) {
        if self.link.input.load(Ordering::Acquire) < MAX_QUEUED_INPUT {
            self.link.input.fetch_add(1, Ordering::AcqRel);
            let _ = self.tx.send(ToGuest::Event(event));
        }
    }
    /// The element's size in device pixels and its scale: the guest hears
    /// a change (its first is `init`) and is asked a frame for it; whether
    /// it changed. Never invalidates the view: the painter calls it while
    /// the renderer draws.
    pub(crate) fn resize(&self, width: u32, height: u32, scale: f32) -> bool {
        let now = (width, height, scale);
        if self.told.replace(Some(now)) == Some(now) {
            return false;
        }
        self.send(ToGuest::Resize(wit::SurfaceInfo {
            width,
            height,
            scale,
            refresh: self.refresh.get(),
        }));
        self.wanted.set(true);
        true
    }
    /// After drawing at `time`: ask the guest for the next frame, if it
    /// animates or asked for one, and isn't still drawing the last.
    pub(crate) fn ask_frame(&self, time: Instant) {
        if self.revoked.get() || self.closed.get() {
            return;
        }
        if !(self.animating.get() || self.wanted.get()) {
            return;
        }
        if self.frame_asked.swap(true, Ordering::AcqRel) {
            // Still drawing the last: asked again after it presents.
            return;
        }
        self.wanted.set(false);
        let seconds = time.saturating_duration_since(self.started).as_secs_f64();
        self.send(ToGuest::Frame(seconds));
    }
    /// The renderer's device, for the guest's GPU.
    pub(crate) fn device(
        &self,
        device: &zgx_gpu::wgpu::Device,
        queue: &zgx_gpu::wgpu::Queue,
        srgb_views: bool,
        generation: u64,
    ) {
        self.send(ToGuest::Device {
            device: device.clone(),
            queue: queue.clone(),
            srgb_views,
            generation,
        });
    }
    /// Tell the guest its display's refresh rate once running, and again as
    /// it changes: awaited from the frame clock at most every two seconds.
    pub(crate) fn tell_refresh(self: &Rc<Self>) {
        if self.refreshing.borrow().is_some() {
            return;
        }
        let now = Instant::now();
        if self
            .refresh_checked
            .get()
            .is_some_and(|at| now.duration_since(at) < Duration::from_secs(2))
        {
            return;
        }
        let (Some(runner), Some(clock)) =
            (self.runner.borrow().clone(), self.frames.borrow().clone())
        else {
            return;
        };
        self.refresh_checked.set(Some(now));
        let weak = Rc::downgrade(self);
        let task = runner.spawn(Box::pin(async move {
            let frame = clock.next().await;
            let Some(app) = weak.upgrade() else { return };
            app.refreshing.borrow_mut().take();
            let hz = frame.refresh_rate() as f32;
            if hz.is_finite() && hz > 1. && (hz - app.refresh.get()).abs() > 0.5 {
                app.refresh.set(hz);
                app.input(wit::InputEvent::Refresh(hz));
            }
        }));
        *self.refreshing.borrow_mut() = Some(task);
    }
    fn set_locked(&self, locked: bool) {
        self.locked.set(locked);
        if let Some(node) = self.node.borrow().as_ref() {
            node.capture_keys(locked);
        }
    }
    fn set_visible(&self, visible: bool) {
        if self.visible.replace(Some(visible)) != Some(visible) {
            self.input(wit::InputEvent::Visible(visible));
        }
    }
    fn ended(&self, ended: Ended) {
        self.closed.set(true);
        self.view.animate(false);
        self.view.invalidate();
        if let Some(node) = self.node.borrow().as_ref() {
            node.unlock_pointer();
        }
        let was_running = matches!(
            self.status.with_untracked(Clone::clone),
            WasmStatus::Running
        );
        let (status, code, heard) = match ended {
            Ended::Exited(code) => (WasmStatus::Exited(code), code, true),
            Ended::Closed => (WasmStatus::Exited(None), None, true),
            Ended::Failed(why) => (WasmStatus::Failed(why), None, was_running),
        };
        self.status.set(status);
        if heard {
            let mut listeners = std::mem::take(&mut *self.on_exit.borrow_mut());
            for listener in &mut listeners {
                listener(code);
            }
            let mut kept = self.on_exit.borrow_mut();
            listeners.append(&mut kept);
            *kept = listeners;
        }
    }
    fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.closing.store(true, Ordering::Release);
        self.send(ToGuest::Close);
        self.view.animate(false);
        if let Some(node) = self.node.borrow().as_ref() {
            node.unlock_pointer();
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.close();
    }
}

/// Shows `app`'s frames, laid out as any box (size it, round it, clip it,
/// put UI over it), focusable, forwarding its input to the app: keys while
/// focused, the pointer over it (in its device pixels), and a lock of the
/// pointer the app asks for just after a press.
pub fn surface(app: &WasmApp) -> View {
    let app = app.clone();
    component(move |cx| {
        if let Some(runner) = cx.try_service::<TaskRunner>() {
            app.attach(&runner);
        }
        if let Some(frames) = cx.try_service::<zgx::frame::FrameClock>() {
            *app.0.frames.borrow_mut() = Some((*frames).clone());
        }
        let (events, mounted) = (app.clone(), app.clone());
        gpu_view("App", &app.0.view)
            .focusable(true)
            .on_event(move |event| events.input(event))
            .on_mount(move |handle| mounted.mounted(handle))
    })
}

fn button_of(button: PointerButton) -> wit::Button {
    match button {
        PointerButton::Primary => wit::Button::Left,
        PointerButton::Secondary => wit::Button::Right,
        PointerButton::Middle => wit::Button::Middle,
        PointerButton::Other(4) => wit::Button::Back,
        PointerButton::Other(5) => wit::Button::Forward,
        PointerButton::Other(n) => wit::Button::Other(n),
    }
}
/// What a key types, held with `modifiers`: nothing with Control, Alt or
/// Command held (those are chords).
fn typed(key: &zgx::input::Key, modifiers: zgx::input::Modifiers) -> Option<String> {
    if modifiers.control || modifiers.alt || modifiers.meta {
        return None;
    }
    match key {
        zgx::input::Key::Character(text) if !text.chars().any(char::is_control) => {
            Some(text.clone())
        }
        zgx::input::Key::Space => Some(" ".to_owned()),
        _ => None,
    }
}
fn modifiers_of(modifiers: zgx::input::Modifiers) -> wit::Modifiers {
    let mut out = wit::Modifiers::empty();
    for (held, flag) in [
        (modifiers.shift, wit::Modifiers::SHIFT),
        (modifiers.control, wit::Modifiers::CONTROL),
        (modifiers.alt, wit::Modifiers::ALT),
        (modifiers.meta, wit::Modifiers::META),
    ] {
        if held {
            out |= flag;
        }
    }
    out
}
fn zgx_cursor(cursor: wit::Cursor) -> zgx::cursor::Cursor {
    use zgx::cursor::Cursor as C;
    match cursor {
        wit::Cursor::Default | wit::Cursor::Hidden => C::Default,
        wit::Cursor::Pointer => C::Pointer,
        wit::Cursor::Text => C::Text,
        wit::Cursor::Crosshair => C::Crosshair,
        wit::Cursor::Move => C::Move,
        wit::Cursor::Grab => C::Grab,
        wit::Cursor::Grabbing => C::Grabbing,
        wit::Cursor::NotAllowed => C::NotAllowed,
        wit::Cursor::Wait => C::Wait,
        wit::Cursor::Progress => C::Progress,
        wit::Cursor::Help => C::Help,
        wit::Cursor::EwResize => C::EwResize,
        wit::Cursor::NsResize => C::NsResize,
        wit::Cursor::NeswResize => C::NeswResize,
        wit::Cursor::NwseResize => C::NwseResize,
    }
}
