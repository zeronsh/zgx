//! A frame profiler for zgx apps: pacing, each frame's phase split, damage
//! rects, reactive writes and effects, resident memory, an in-app HUD and a
//! Chrome trace export.
//!
//! Attach it to a window's context:
//!
//! ```no_run
//! use zgx_desktop::{Application, WindowOptions};
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     Application::new()
//!         .window(WindowOptions { title: "Profiled".into(), ..Default::default() })
//!         .run(|cx| {
//!             let profiler = zgx_profiler::attach(cx, zgx_profiler::Config::default());
//!             let toggling = profiler.clone();
//!             cx.render(
//!                 zgx::compose::column()
//!                     .keymap(zgx_profiler::keymap())
//!                     .on_action(move |_: &zgx_profiler::ToggleHud, cx| {
//!                         cx.prevent_default();
//!                         toggling.toggle();
//!                     }),
//!             );
//!         })
//! }
//! ```
//!
//! The HUD is a tree of ordinary scene nodes above the interface, inert, so
//! clicks and the keyboard pass through it. It rewrites only the numbers that
//! changed: an idle window stays idle while it is shown, and a small watchdog
//! wakes the window once to write "idle" before stopping.
//!
//! Everything it reports comes from measurements the engine already makes:
//! [`zgx::profile`]'s phases, [`zgx::trace`]'s per-frame writes and effect runs
//! (built with zgx's `trace` feature, which this crate turns on), and
//! [`zgx::diagnostics`]'s per-frame damage, which the desktop host publishes.
mod chrome;
mod hud;
mod record;
mod resident;

pub use record::{FrameRecord, Summary, THRASH_WRITES};

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    io,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use zgx::{
    actions::{Action, KeyBinding, Keymap, Keystroke},
    diagnostics::FrameSample,
    input::{Key, Modifiers},
    scene::Rect,
    widgets::WeakUi,
};
use zgx_desktop::{TaskSpawner, WindowContext, WindowHandle};

use hud::Model;
use record::History;

/// How long a window may draw nothing before the profiler calls it idle.
pub const IDLE_AFTER: Duration = Duration::from_millis(600);
/// How often the HUD's watchdog checks that it still has something to say.
const WATCHDOG: Duration = Duration::from_millis(250);
/// How often resident memory is read.
const RSS_EVERY: Duration = Duration::from_millis(500);

/// What the profiler records and shows when it is attached.
#[derive(Clone, Debug)]
pub struct Config {
    /// Show the HUD at once (F12 toggles it either way).
    pub enable_hud: bool,
    /// Highlight the last frame's damage rects (F11 toggles it).
    pub damage_overlay: bool,
    /// Frames kept for the trace export and for the graph's history: the
    /// window's whole recording, oldest dropped first. zgx's own frame log
    /// keeps at most [`zgx::trace::MOST_KEPT`].
    pub trace_buffer_capacity: usize,
    /// How often the HUD rewrites its numbers, in hertz: each rewrite costs
    /// the nodes it changes, which per frame would outweigh what a small app
    /// changes. 0 draws it on every frame.
    pub hud_rate: f64,
    /// Keep a watchdog awake while the HUD is shown: it wakes the window once
    /// when the app falls idle, so the HUD can say so, and does nothing until
    /// the app draws again.
    pub watchdog: bool,
    /// Write `zgx-profile.json`, this session's frames, when the window closes.
    pub export_chrome_trace: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enable_hud: true,
            damage_overlay: false,
            trace_buffer_capacity: 8_192,
            hud_rate: 30.,
            watchdog: true,
            export_chrome_trace: false,
        }
    }
}

/// F12: show or hide the HUD.
pub struct ToggleHud;
/// F11: highlight the last frame's damage rects.
pub struct ToggleDamage;

/// The profiler's keys: F12 for the HUD, F11 for the damage overlay. Bind it
/// on a view and handle [`ToggleHud`] and [`ToggleDamage`] with the profiler's
/// [`Profiler::toggle`] and [`Profiler::toggle_damage`].
pub fn keymap() -> Keymap {
    fn binding<T: 'static>(key: Key, action: T) -> KeyBinding {
        KeyBinding::new(
            [Keystroke::new(key, Modifiers::default())],
            Action::new(action),
        )
        .expect("one stroke has a binding")
    }
    Keymap::new()
        .bind(binding(Key::Function(12), ToggleHud))
        .bind(binding(Key::Function(11), ToggleDamage))
}

/// An attached profiler. Clones share it.
#[derive(Clone)]
pub struct Profiler(Rc<Inner>);

struct Inner {
    ui: WeakUi,
    window: WindowHandle,
    tasks: TaskSpawner,
    began: Instant,
    history: RefCell<History>,
    hud: RefCell<hud::Hud>,
    visible: Cell<bool>,
    damage_overlay: Cell<bool>,
    trace_index: Cell<u64>,
    last_frame_at: Cell<Option<Instant>>,
    last_app_damage: RefCell<Vec<Rect>>,
    markers: RefCell<Vec<(f64, String)>>,
    rss_at: Cell<Option<Instant>>,
    hud_rect: Cell<Option<Rect>>,
    last_draw: Cell<Option<Instant>>,
    hud_rate: f64,
    live_shown: Cell<bool>,
    watchdog: Cell<bool>,
    watchdog_enabled: bool,
}

/// Attach the profiler to `cx`'s window: turn on zgx's frame tracing, watch
/// every finished frame, show the HUD if asked, and keep a watchdog awake
/// while the HUD is up. Handle [`keymap`]'s actions to toggle the HUD and the
/// damage overlay from the keyboard.
pub fn attach(cx: &mut WindowContext, config: Config) -> Profiler {
    zgx::trace::set_kept(config.trace_buffer_capacity.min(zgx::trace::MOST_KEPT));
    zgx::trace::set_enabled(true);
    let inner = Rc::new(Inner {
        ui: cx.ui.downgrade(),
        window: cx.window.clone(),
        tasks: cx.tasks.clone(),
        began: Instant::now(),
        history: RefCell::new(History::new(config.trace_buffer_capacity, Instant::now())),
        hud: RefCell::new(hud::Hud::default()),
        visible: Cell::new(config.enable_hud),
        damage_overlay: Cell::new(config.damage_overlay),
        trace_index: Cell::new(zgx::trace::frame_index()),
        last_frame_at: Cell::new(None),
        last_app_damage: RefCell::new(Vec::new()),
        markers: RefCell::new(Vec::new()),
        rss_at: Cell::new(None),
        hud_rect: Cell::new(None),
        last_draw: Cell::new(None),
        hud_rate: config.hud_rate,
        live_shown: Cell::new(false),
        watchdog: Cell::new(false),
        watchdog_enabled: config.watchdog,
    });
    let profiler = Profiler(inner.clone());
    let watching = profiler.clone();
    cx.diagnostics
        .on_frame(move |sample| watching.0.observe(sample));
    if config.enable_hud && config.watchdog {
        profiler.start_watchdog();
        cx.window.request_redraw();
    }
    if config.export_chrome_trace {
        let closing = profiler.clone();
        cx.on_closed(move || match closing.export("zgx-profile.json") {
            Ok(frames) => eprintln!("zgx-profiler: wrote zgx-profile.json ({frames} frames)"),
            Err(error) => eprintln!("zgx-profiler: could not write the trace: {error}"),
        });
    }
    profiler
}

impl Profiler {
    /// Show the HUD.
    pub fn show(&self) {
        if !self.0.visible.replace(true) {
            self.mark("hud on");
            self.start_watchdog();
            self.0.window.request_redraw();
        }
    }
    /// Hide it. The window goes back to drawing only what the app changes.
    pub fn hide(&self) {
        if self.0.visible.replace(false) {
            self.mark("hud off");
            self.0.window.request_redraw();
        }
    }
    pub fn toggle(&self) {
        match self.0.visible.get() {
            true => self.hide(),
            false => self.show(),
        }
    }
    pub fn visible(&self) -> bool {
        self.0.visible.get()
    }
    /// Highlight the last frame's damage rects in the HUD, or stop.
    pub fn toggle_damage(&self) {
        let on = !self.0.damage_overlay.get();
        self.0.damage_overlay.set(on);
        self.mark(if on {
            "damage overlay on"
        } else {
            "damage overlay off"
        });
        self.0.window.request_redraw();
    }
    pub fn damage_overlay(&self) -> bool {
        self.0.damage_overlay.get()
    }
    /// Whether zgx's frame tracing is on: what gives the HUD its phase split
    /// and its reactive numbers.
    pub fn capturing(&self) -> bool {
        zgx::trace::enabled()
    }
    /// Turn zgx's frame tracing on or off. With it off the HUD keeps pacing
    /// and damage, and loses phases, writes and effects.
    pub fn set_capturing(&self, on: bool) {
        zgx::trace::set_enabled(on);
    }
    /// A name at this moment in the trace (`scenario: spectrum off`).
    pub fn mark(&self, name: impl Into<String>) {
        let at = (Instant::now() - self.0.began).as_secs_f64() * 1e3;
        self.0.markers.borrow_mut().push((at, name.into()));
    }
    /// What the profiler has measured.
    pub fn summary(&self) -> Summary {
        self.0.history.borrow().summary(IDLE_AFTER)
    }
    /// The frames kept, oldest first.
    pub fn frames(&self) -> Vec<FrameRecord> {
        self.0.history.borrow().frames().iter().cloned().collect()
    }
    /// Forget every frame kept and every marker.
    pub fn clear(&self) {
        self.0.history.borrow_mut().clear();
        self.0.markers.borrow_mut().clear();
        self.0.trace_index.set(zgx::trace::frame_index());
    }
    /// Write the frames kept as a Chrome trace (`chrome://tracing`,
    /// `ui.perfetto.dev`): each frame's phases, its damage, its reactive work
    /// and the markers taken. Returns how many frames were written.
    pub fn export(&self, path: impl AsRef<Path>) -> io::Result<usize> {
        let history = self.0.history.borrow();
        chrome::write(path, history.frames(), &self.0.markers.borrow())
    }

    fn start_watchdog(&self) {
        if self.0.watchdog.replace(true) || !self.0.watchdog_enabled {
            return;
        }
        let inner = self.0.clone();
        self.0.tasks.spawn(async move {
            loop {
                zgx::timer::sleep(WATCHDOG).await;
                if !inner.visible.get() {
                    inner.watchdog.set(false);
                    break;
                }
                inner.tick();
            }
        });
    }
}

impl Inner {
    fn tick(&self) {
        if self.live_shown.get()
            && self
                .last_frame_at
                .get()
                .is_none_or(|at| at.elapsed() > IDLE_AFTER)
        {
            self.window.request_redraw();
        }
    }

    fn observe(&self, sample: &FrameSample) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let at = sample.at.unwrap_or_else(Instant::now);
        let interval = self.history.borrow_mut().interval(sample.at).unwrap_or(0.);
        let traced = zgx::trace::frames_since(self.trace_index.get());
        if let Some(last) = traced.last() {
            self.trace_index.set(last.index);
        }
        let mut phases: Vec<(&'static str, f32)> = Vec::new();
        let mut writes = 0u32;
        let mut effects = 0u32;
        let mut per_cause: HashMap<usize, u32> = HashMap::new();
        let mut thrashing: Vec<(usize, u32)> = Vec::new();
        let mut gpu_micros = None;
        for frame in &traced {
            for (name, _, micros) in &frame.phases {
                match phases.iter_mut().find(|(known, _)| known == name) {
                    Some((_, total)) => *total += micros,
                    None => phases.push((name, *micros)),
                }
            }
            for (signal, count) in &frame.writes {
                writes += count;
                if *count >= THRASH_WRITES {
                    thrashing.push((*signal, *count));
                }
            }
            effects += frame.runs.len() as u32;
            for run in &frame.runs {
                if let Some(cause) = run.cause {
                    *per_cause.entry(cause).or_default() += 1;
                }
            }
            if let Some(micros) = frame.gpu_micros {
                gpu_micros = Some(micros);
            }
        }
        let hud = self.hud_rect.get();
        let quiet = writes == 0 && effects == 0;
        let inside_hud = |rect: &Rect| {
            hud.is_some_and(|hud| rect.intersection(hud).is_some_and(|inside| inside == *rect))
        };
        let hud_only = quiet && sample.damage.iter().all(inside_hud);
        let app_damage: Vec<Rect> = sample
            .damage
            .iter()
            .filter(|rect| !inside_hud(rect))
            .copied()
            .collect();
        let app_area: f32 = app_damage
            .iter()
            .map(|rect| rect.width.max(0.) * rect.height.max(0.))
            .sum();
        if !hud_only {
            self.last_app_damage.borrow_mut().clone_from(&app_damage);
        }
        self.last_frame_at.set(Some(at));
        let record = FrameRecord {
            at_ms: (at - self.began).as_secs_f64() * 1e3,
            interval_ms: interval,
            damage: app_damage,
            damage_area: app_area,
            cascade: per_cause.values().copied().max().unwrap_or(0) as f32,
            gpu_micros,
            hud_only,
            layout_nodes: sample.layout_nodes,
            paint_nodes: sample.paint_nodes,
            phases,
            writes,
            effects,
            thrashing,
        };
        self.history.borrow_mut().push(record, at);
        if self
            .rss_at
            .get()
            .is_none_or(|read| read.elapsed() > RSS_EVERY)
        {
            self.rss_at.set(Some(at));
            self.history.borrow_mut().set_rss(resident::mib());
        }
        if self.visible.get() {
            if self.due() {
                self.last_draw.set(Some(Instant::now()));
                self.draw(&ui);
            }
        } else if self.hud.borrow().bounds.is_some() {
            self.draw(&ui);
        }
    }

    /// Whether the HUD is due a rewrite, at its own rate.
    fn due(&self) -> bool {
        match self.last_draw.get() {
            _ if self.hud_rate <= 0. => true,
            Some(drawn) => drawn.elapsed().as_secs_f64() >= 1. / self.hud_rate,
            None => true,
        }
    }

    fn draw(&self, ui: &zgx::widgets::Ui) {
        let history = self.history.borrow();
        let summary = history.summary(IDLE_AFTER);
        let damage = self.last_app_damage.borrow();
        let thrashing = history
            .last_app()
            .map_or(&[][..], |frame| frame.thrashing.as_slice());
        let model = Model {
            summary: &summary,
            graph: history.intervals(),
            thrashing,
            damage: damage.as_slice(),
            damage_overlay: self.damage_overlay.get(),
            visible: self.visible.get(),
        };
        let bounds = {
            let mut hud = self.hud.borrow_mut();
            hud.update(ui, &model);
            hud.bounds
        };
        self.hud_rect.set(bounds);
        self.live_shown.set(!summary.idle);
    }
}
