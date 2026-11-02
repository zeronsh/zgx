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
//!             zgx_profiler::attach(cx, zgx_profiler::Config::default());
//!             cx.render(zgx::compose::column());
//!         })
//! }
//! ```
//!
//! F12 shows or hides the HUD, F11 outlines damage and F10 starts the
//! recording over. `zgx_profiler::attach_if_asked(cx)` attaches only when
//! `ZGX_PROFILER=1`, and `zgx-run --profile app/` profiles a Luau app.
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
//! The costliest named effects of a frame (`app.luau:30`, `@12 binding`), the
//! engine's own notes on why a frame happened, the worst frame kept and the
//! marker-delimited segments of a capture are all built from those.
mod chrome;
mod dump;
mod hud;
mod menu;
mod record;
mod resident;
mod serial;

pub use record::{FrameRecord, Segment, Summary, THRASH_WRITES};

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs::File,
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use zgx::{
    actions::{Action, KeyBinding, Keymap, Keystroke},
    diagnostics::FrameSample,
    input::{EventContext, EventPhase, InputEvent, Key, Modifiers},
    scene::Rect,
    widgets::WeakUi,
};
use zgx_desktop::{TaskSpawner, WindowContext, WindowHandle};

use hud::Model;
use record::{History, PhaseCost, SlowRun};

/// How long a window may draw nothing before the profiler calls it idle.
pub const IDLE_AFTER: Duration = Duration::from_millis(600);
/// How often the HUD's watchdog checks that it still has something to say.
const WATCHDOG: Duration = Duration::from_millis(250);
/// How often resident memory is read.
const RSS_EVERY: Duration = Duration::from_millis(500);
/// The costliest named effects a frame keeps.
const SLOW_RUNS: usize = 3;
/// Why-lines a frame keeps.
const NOTES: usize = 3;
/// Flashes kept (damage rects and their moments), the newest last.
const FLASH_MEMORY: usize = 64;
/// Where the frame log, the stress report and the crash dump are written.
const LOG_PATH: &str = "zgx-profile-log.txt";
const STRESS_PATH: &str = "zgx-stress-report.txt";
const DUMP_PATH: &str = "zgx-crash-dump.txt";
const TRACE_PATH: &str = "zgx-profile.json";

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
    pub keys: bool,
    pub out_dir: PathBuf,
}
impl Config {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Some(dir) = std::env::var_os("ZGX_PROFILER_OUT") {
            config.out_dir = dir.into();
        }
        config.export_chrome_trace = std::env::var_os("ZGX_PROFILER_TRACE").is_some();
        config
    }
}

pub fn asked() -> bool {
    std::env::var_os("ZGX_PROFILER").is_some_and(|value| value != "0")
}

pub fn attach_if_asked(cx: &mut WindowContext) -> Option<Profiler> {
    asked().then(|| attach(cx, Config::from_env()))
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
            keys: true,
            out_dir: PathBuf::from("."),
        }
    }
}

/// F12: show or hide the HUD.
pub struct ToggleHud;
/// F11: highlight the last frame's damage rects.
pub struct ToggleDamage;
/// F10: forget every frame kept and start the recording over.
pub struct ResetStats;

/// The profiler's keys: F12 for the HUD, F11 for the damage overlay, F10 to
/// start the recording over. Bind it on a view and handle [`ToggleHud`],
/// [`ToggleDamage`] and [`ResetStats`] with the profiler's [`Profiler::toggle`],
/// [`Profiler::toggle_damage`] and [`Profiler::clear`].
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
        .bind(binding(Key::Function(10), ResetStats))
}

/// An attached profiler. Clones share it.
#[derive(Clone)]
pub struct Profiler(Rc<Inner>);

struct Inner {
    ui: WeakUi,
    window: WindowHandle,
    tasks: TaskSpawner,
    weak: RefCell<Weak<Inner>>,
    began: Cell<Instant>,
    history: RefCell<History>,
    hud: RefCell<hud::Hud>,
    menu: RefCell<menu::Menu>,
    visible: Cell<bool>,
    damage_overlay: Cell<bool>,
    menu_open: Cell<bool>,
    menu_hover: Cell<Option<usize>>,
    flash: Cell<bool>,
    flashes: RefCell<Vec<(Rect, Instant)>>,
    overlay_rects: RefCell<Vec<Rect>>,
    flash_ticker: Cell<bool>,
    log_on: Cell<bool>,
    log: RefCell<Option<FrameLog>>,
    stressing: Cell<bool>,
    pending_break: Cell<bool>,
    serial: Cell<bool>,
    serial_mask: RefCell<Option<serial::SerialMask>>,
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
    out_dir: PathBuf,
    keys: RefCell<Option<zgx::input::InputBinding>>,
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
        weak: RefCell::new(Weak::new()),
        began: Cell::new(Instant::now()),
        history: RefCell::new(History::new(config.trace_buffer_capacity, Instant::now())),
        hud: RefCell::new(hud::Hud::default()),
        menu: RefCell::new(menu::Menu::default()),
        visible: Cell::new(config.enable_hud),
        damage_overlay: Cell::new(config.damage_overlay),
        menu_open: Cell::new(false),
        menu_hover: Cell::new(None),
        flash: Cell::new(false),
        flashes: RefCell::new(Vec::new()),
        overlay_rects: RefCell::new(Vec::new()),
        flash_ticker: Cell::new(false),
        log_on: Cell::new(false),
        log: RefCell::new(None),
        stressing: Cell::new(false),
        pending_break: Cell::new(false),
        serial: Cell::new(false),
        serial_mask: RefCell::new(None),
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
        out_dir: config.out_dir.clone(),
        keys: RefCell::new(None),
    });
    *inner.weak.borrow_mut() = Rc::downgrade(&inner);
    let profiler = Profiler(inner.clone());
    let watching = profiler.clone();
    cx.diagnostics
        .on_frame(move |sample| watching.0.observe(sample));
    if config.keys {
        let root = cx.ui.scene.borrow().root();
        let keyed = Rc::downgrade(&inner);
        *inner.keys.borrow_mut() = Some(cx.ui.input.listen(root, move |cx| {
            if let Some(inner) = keyed.upgrade() {
                Profiler(inner).key(cx);
            }
        }));
    }
    if config.enable_hud && config.watchdog {
        profiler.start_watchdog();
        cx.window.request_redraw();
    }
    if config.export_chrome_trace {
        let closing = profiler.clone();
        let path = config.out_dir.join(TRACE_PATH);
        cx.on_closed(move || match closing.export(&path) {
            Ok(frames) => eprintln!("zgx-profiler: wrote {} ({frames} frames)", path.display()),
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
        self.0.mark(name);
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
        self.0.began.set(Instant::now());
        self.0.markers.borrow_mut().clear();
        self.0.last_app_damage.borrow_mut().clear();
        self.0.flashes.borrow_mut().clear();
        self.0.trace_index.set(zgx::trace::frame_index());
    }
    /// Write the frames kept as a Chrome trace (`chrome://tracing`,
    /// `ui.perfetto.dev`): each frame's phases, its damage, its reactive work,
    /// its costliest effects, the marker-delimited segments of the capture and
    /// the markers taken. Returns how many frames were written.
    pub fn export(&self, path: impl AsRef<Path>) -> io::Result<usize> {
        let history = self.0.history.borrow();
        let markers = self.0.markers.borrow();
        chrome::write(
            path,
            history.frames(),
            &markers,
            &history.segments(&markers),
            history.worst(),
        )
    }

    /// The marker-delimited stretches of the recording, in order: each marker
    /// begins a segment that lasts until the next marker or the last frame, so
    /// `mark("scenario: a")` and `mark("scenario: b")` name two stretches to
    /// compare (`[`Segment`]`'s fps, pacing, damage and phase split).
    pub fn segments(&self) -> Vec<Segment> {
        let history = self.0.history.borrow();
        history.segments(&self.0.markers.borrow())
    }

    fn key(&self, cx: &mut EventContext) {
        if cx.phase == EventPhase::Bubble {
            return;
        }
        let InputEvent::KeyDown {
            key: Key::Function(number),
            modifiers,
            repeat: false,
        } = cx.event
        else {
            return;
        };
        if modifiers != Modifiers::default() {
            return;
        }
        match number {
            12 => self.toggle(),
            11 => self.toggle_damage(),
            10 => self.clear(),
            _ => return,
        }
        cx.prevent_default();
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

    /// Open or close the dev menu.
    pub fn toggle_menu(&self) {
        self.0.menu_open.set(!self.0.menu_open.get());
        self.0.refresh();
    }
    pub fn menu_open(&self) -> bool {
        self.0.menu_open.get()
    }

    /// Flash the areas the app repaints, pulsing pink as they fade (the dev
    /// menu's first row). The damage figures keep counting the app's own
    /// repaints: the overlay's are excluded.
    pub fn toggle_flash(&self) {
        self.0.toggle_flash();
        self.0.window.request_redraw();
    }
    pub fn flashing(&self) -> bool {
        self.0.flash.get()
    }
    /// Write every app frame to `zgx-profile-log.txt` as it happens: its
    /// pacing, each phase's cost, its damage, reactive work and memory (the
    /// dev menu's second row). Toggling off writes the totals and flushes.
    pub fn toggle_log(&self) {
        self.0.toggle_log();
        self.0.window.request_redraw();
    }
    pub fn logging(&self) -> bool {
        self.0.log_on.get()
    }
    /// Run the stress suite (the dev menu's third row): three marked
    /// stretches — a redraw storm, a synthetic pointer sweep, scrolls —
    /// each reporting its fps, pacing, damage and phase split, to stderr and
    /// `zgx-stress-report.txt`, and to the trace's segments.
    pub fn run_stress(&self) {
        self.0.start_stress();
    }
    pub fn stressing(&self) -> bool {
        self.0.stressing.get()
    }
    /// Pin the process to one core (the dev menu's fifth row): every thread
    /// through one CPU, so what parallelism hides shows as latency. Off
    /// restores the mask the process had. Now serial.
    pub fn toggle_serial(&self) {
        self.0.toggle_serial();
        self.0.window.request_redraw();
    }
    pub fn serial(&self) -> bool {
        self.0.serial.get()
    }
    /// Write the profiler's whole retention — every frame kept, with the
    /// engine's own trace of writes and effect runs — to
    /// `zgx-crash-dump.txt`, then panic (the dev menu's fourth row). The dump
    /// is the last thing that happens.
    pub fn break_now(&self) {
        self.0.pending_break.set(true);
        self.0.window.request_redraw();
    }
}

impl Inner {
    fn mark(&self, name: impl Into<String>) {
        let at = self.began.get().elapsed().as_secs_f64() * 1e3;
        self.markers.borrow_mut().push((at, name.into()));
    }

    fn refresh(&self) {
        self.last_draw.set(None);
        self.window.request_redraw();
    }

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

    /// A dev-menu row was pressed.
    fn menu_clicked(&self, index: usize) {
        match index {
            0 => self.toggle_flash(),
            1 => self.toggle_log(),
            2 => self.start_stress(),
            3 => {
                self.pending_break.set(true);
                self.window.request_redraw();
            }
            _ => self.toggle_serial(),
        }
        self.refresh();
    }

    fn toggle_flash(&self) {
        let on = !self.flash.get();
        self.flash.set(on);
        self.mark(if on {
            "flash repaints on"
        } else {
            "flash repaints off"
        });
        if !on {
            self.flashes.borrow_mut().clear();
        }
    }

    /// Wake the window while flashes have life left in them, so the flash
    /// overlay can fade: one timer a tick while it runs, and nothing once the
    /// app falls quiet again — it restarts when damage does.
    fn start_flash_ticker(&self) {
        if self.flash_ticker.replace(true) {
            return;
        }
        let Some(inner) = self.weak.borrow().upgrade() else {
            self.flash_ticker.set(false);
            return;
        };
        self.tasks.spawn(async move {
            loop {
                zgx::timer::sleep(Duration::from_millis(33)).await;
                let live = inner
                    .flashes
                    .borrow()
                    .iter()
                    .any(|(_, at)| at.elapsed().as_secs_f32() * 1e3 < hud::FLASH_MS * 1.5);
                if !inner.flash.get() || !live {
                    break;
                }
                inner.window.request_redraw();
            }
            inner.flash_ticker.set(false);
        });
    }

    fn toggle_log(&self) {
        let mut log = self.log.borrow_mut();
        if let Some(mut writer) = log.take() {
            drop(log);
            let summary = self.history.borrow().summary(IDLE_AFTER);
            match writer.close(&summary) {
                Ok(lines) => eprintln!("zgx-profiler: wrote {LOG_PATH} ({lines} frames)"),
                Err(error) => eprintln!("zgx-profiler: could not write {LOG_PATH}: {error}"),
            }
            self.log_on.set(false);
            self.mark("frame log off");
            return;
        }
        drop(log);
        match FrameLog::open(&self.out_dir.join(LOG_PATH)) {
            Ok(writer) => {
                *self.log.borrow_mut() = Some(writer);
                self.log_on.set(true);
                self.mark("frame log on");
            }
            Err(error) => eprintln!("zgx-profiler: could not open {LOG_PATH}: {error}"),
        }
    }

    fn toggle_serial(&self) {
        if self.serial.get() {
            let mask = self.serial_mask.borrow_mut().take();
            let restored = mask.map(serial::restore).unwrap_or(false);
            self.serial.set(false);
            self.mark(if restored {
                "serial: all cores"
            } else {
                "serial: restore failed"
            });
            return;
        }
        match serial::pin_to_one_core() {
            Some(mask) => {
                *self.serial_mask.borrow_mut() = Some(mask);
                self.serial.set(true);
                self.mark("serial: one core");
            }
            None => {
                self.serial.set(false);
                eprintln!("zgx-profiler: serial mode is not available here");
            }
        }
    }

    fn start_stress(&self) {
        if self.stressing.replace(true) {
            return;
        }
        let Some(ui) = self.ui.upgrade() else {
            self.stressing.set(false);
            return;
        };
        let scene = ui.scene.clone();
        let input = ui.input.clone();
        let Some(inner) = self.weak.borrow().upgrade() else {
            self.stressing.set(false);
            return;
        };
        self.tasks.spawn(async move {
            let viewport = {
                let scene = scene.borrow();
                scene.bounds(scene.root())
            };
            inner.mark("stress: redraw storm");
            for _ in 0..750 {
                inner.window.request_redraw();
                zgx::timer::sleep(Duration::from_millis(4)).await;
            }
            inner.mark("stress: pointer sweep");
            let began = Instant::now();
            while began.elapsed() < Duration::from_secs(4) {
                let t = began.elapsed().as_secs_f32();
                let x = viewport.width * (0.5 + 0.45 * (t * 2.1).sin());
                let y = viewport.height * (0.5 + 0.45 * (t * 2.7).cos());
                input.dispatch(&scene, InputEvent::PointerMove { x, y });
                inner.window.request_redraw();
                zgx::timer::sleep(Duration::from_millis(8)).await;
            }
            inner.mark("stress: scroll");
            let began = Instant::now();
            let mut flip = false;
            while began.elapsed() < Duration::from_secs(3) {
                let t = began.elapsed().as_secs_f32();
                flip = !flip;
                input.dispatch(
                    &scene,
                    InputEvent::Scroll {
                        x: viewport.width * (0.5 + 0.3 * (t * 0.9).sin()),
                        y: viewport.height * 0.5,
                        delta_x: 0.,
                        delta_y: if flip { 60. } else { -60. },
                    },
                );
                inner.window.request_redraw();
                zgx::timer::sleep(Duration::from_millis(16)).await;
            }
            inner.mark("stress: done");
            inner.report_stress();
            inner.stressing.set(false);
        });
    }

    /// The stress suite's report: the segments it marked, to stderr and
    /// `zgx-stress-report.txt`.
    fn report_stress(&self) {
        let history = self.history.borrow();
        let markers = self.markers.borrow();
        let segments: Vec<Segment> = history
            .segments(&markers)
            .into_iter()
            .filter(|segment| segment.name.starts_with("stress:"))
            .collect();
        drop(history);
        drop(markers);
        let lines: Vec<String> = segments.iter().map(segment_line).collect();
        for line in &lines {
            eprintln!("zgx-profiler: {line}");
        }
        match File::create(self.out_dir.join(STRESS_PATH))
            .map(BufWriter::new)
            .and_then(|mut out| {
                for line in &lines {
                    writeln!(out, "{line}")?;
                }
                out.flush()
            }) {
            Ok(()) => eprintln!("zgx-profiler: wrote {STRESS_PATH}"),
            Err(error) => eprintln!("zgx-profiler: could not write {STRESS_PATH}: {error}"),
        }
    }

    /// Dump the whole retention and panic: the dev menu's break. Runs between
    /// frames, where no borrow is held.
    fn break_app(&self) -> ! {
        self.pending_break.set(false);
        let history = self.history.borrow();
        let markers = self.markers.borrow();
        let summary = history.summary(IDLE_AFTER);
        match dump::write(&self.out_dir.join(DUMP_PATH), &history, &markers, &summary) {
            Ok(frames) => eprintln!("zgx-profiler: wrote {DUMP_PATH} ({frames} frames)"),
            Err(error) => eprintln!("zgx-profiler: could not write {DUMP_PATH}: {error}"),
        }
        drop(markers);
        drop(history);
        if let Some(log) = self.log.borrow_mut().as_mut() {
            let _ = log.flush_line();
        }
        panic!("zgx-profiler: break requested — the full retention is in {DUMP_PATH}");
    }
    fn observe(&self, sample: &FrameSample) {
        if self.pending_break.get() {
            self.break_app();
        }
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let at = sample.at.unwrap_or_else(Instant::now);
        let traced = zgx::trace::frames_since(self.trace_index.get());
        if let Some(last) = traced.last() {
            self.trace_index.set(last.index);
        }
        let mut phases: Vec<PhaseCost> = Vec::new();
        let mut writes = 0u32;
        let mut effects = 0u32;
        let mut per_cause: HashMap<usize, u32> = HashMap::new();
        let mut thrashing: Vec<(usize, u32)> = Vec::new();
        let mut slow: Vec<(String, f32, Option<usize>)> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let mut gpu_micros = None;
        for frame in &traced {
            for (name, instructions, micros) in &frame.phases {
                match phases.iter_mut().find(|known| known.name == *name) {
                    Some(total) => {
                        total.micros += micros;
                        total.instructions += instructions;
                    }
                    None => phases.push(PhaseCost {
                        name,
                        micros: *micros,
                        instructions: *instructions,
                    }),
                }
            }
            for (signal, count) in &frame.writes {
                writes += count;
                if *count > THRASH_WRITES {
                    thrashing.push((*signal, *count));
                }
            }
            effects += frame.runs.len() as u32;
            for run in &frame.runs {
                if let Some(cause) = run.cause {
                    *per_cause.entry(cause).or_default() += 1;
                }
                if let Some(name) = &run.name {
                    match slow.iter_mut().find(|(known, ..)| known == &**name) {
                        Some((_, micros, _)) => *micros += run.micros,
                        None => slow.push(((**name).to_string(), run.micros, run.cause)),
                    }
                }
            }
            for note in &frame.notes {
                if notes.len() < NOTES && !notes.iter().any(|kept| kept == &**note) {
                    notes.push(note.to_string());
                }
            }
            if let Some(micros) = frame.gpu_micros {
                gpu_micros = Some(micros);
            }
        }
        slow.sort_by(|a, b| b.1.total_cmp(&a.1));
        let slow: Vec<_> = slow
            .into_iter()
            .take(SLOW_RUNS)
            .map(|(name, micros, cause)| SlowRun {
                name,
                micros,
                cause,
            })
            .collect();
        let instructions = phases.iter().map(|phase| phase.instructions).sum();
        let overlays = self.overlay_rects.take();
        let quiet = writes == 0 && effects == 0;
        let inside_overlay = |rect: &Rect| {
            overlays.iter().any(|overlay| {
                rect.intersection(*overlay)
                    .is_some_and(|inside| inside == *rect)
            })
        };
        let hud_only = quiet && sample.damage.iter().all(inside_overlay);
        let app_damage: Vec<Rect> = sample
            .damage
            .iter()
            .filter(|rect| !inside_overlay(rect))
            .copied()
            .collect();
        let interval = if hud_only {
            0.
        } else {
            self.history
                .borrow_mut()
                .interval(sample.at, IDLE_AFTER)
                .unwrap_or(0.)
        };
        let app_area: f32 = app_damage
            .iter()
            .map(|rect| rect.width.max(0.) * rect.height.max(0.))
            .sum();
        if !hud_only {
            self.last_app_damage.borrow_mut().clone_from(&app_damage);
        }
        self.last_frame_at.set(Some(at));
        if self
            .rss_at
            .get()
            .is_none_or(|read| read.elapsed() > RSS_EVERY)
        {
            self.rss_at.set(Some(at));
            self.history.borrow_mut().set_rss(resident::mib());
        }
        let record = FrameRecord {
            at_ms: (at - self.began.get()).as_secs_f64() * 1e3,
            rss_mib: self.history.borrow().rss_mib(),
            interval_ms: interval,
            damage: app_damage.clone(),
            damage_area: app_area,
            phases,
            instructions,
            writes,
            effects,
            cascade: per_cause.values().copied().max().unwrap_or(0) as f32,
            thrashing,
            slow,
            notes,
            gpu_micros,
            hud_only,
            layout_nodes: sample.layout_nodes,
            paint_nodes: sample.paint_nodes,
            composite_nodes: sample.composite_nodes,
            full: sample.full,
        };
        if !hud_only {
            if let Some(log) = self.log.borrow_mut().as_mut() {
                log.frame(&record);
            }
            if self.flash.get() {
                let mut flashes = self.flashes.borrow_mut();
                flashes.extend(app_damage.iter().copied().map(|rect| (rect, at)));
                let spare = flashes.len().saturating_sub(FLASH_MEMORY);
                flashes.drain(..spare);
                drop(flashes);
                self.start_flash_ticker();
            }
        }
        self.history.borrow_mut().push(record, at);
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
        let flashes: Vec<(Rect, f32)> = self
            .flashes
            .borrow()
            .iter()
            .rev()
            .take(FLASH_MEMORY)
            .map(|(rect, at)| (*rect, at.elapsed().as_secs_f32() * 1e3))
            .filter(|(_, age)| *age < hud::FLASH_MS)
            .collect();
        let model = Model {
            summary: &summary,
            graph: history.bars(),
            thrashing,
            damage: damage.as_slice(),
            flashes: flashes.as_slice(),
            damage_overlay: self.damage_overlay.get(),
            visible: self.visible.get(),
        };
        let mut overlays = {
            let mut hud = self.hud.borrow_mut();
            let drawn = hud.update(ui, &model);
            self.hud_rect.set(hud.bounds);
            drawn
        };
        if let Some(me) = self.weak.borrow().upgrade() {
            let mut menu = self.menu.borrow_mut();
            menu.update(ui, &me);
            if let Some(bounds) = menu.bounds {
                overlays.push(bounds);
            }
        }
        self.order_overlays(ui);
        *self.overlay_rects.borrow_mut() = overlays;
        self.live_shown.set(!summary.idle);
    }

    /// The dev overlays sit above the app, the menu above the HUD: one
    /// ordering of the scene root's children, after both are drawn, so the
    /// two roots never fight over the last place.
    fn order_overlays(&self, ui: &zgx::widgets::Ui) {
        let hud_root = self.hud.borrow().root();
        let Some(hud_root) = hud_root else {
            return;
        };
        let menu_root = self.menu.borrow().root();
        let mut scene = ui.scene.borrow_mut();
        let scene_root = scene.root();
        let children: Vec<_> = scene.children(scene_root).to_vec();
        let mut order: Vec<_> = children
            .iter()
            .copied()
            .filter(|id| *id != hud_root && Some(*id) != menu_root)
            .collect();
        if order.len() + 1 + usize::from(menu_root.is_some()) != children.len() {
            return;
        }
        order.push(hud_root);
        if let Some(menu_root) = menu_root {
            order.push(menu_root);
        }
        if order != children {
            scene.reorder_children(scene_root, &order);
        }
    }
}

/// One frame of the log: when it came, how long after the one before, what
/// each phase took, what it repainted, what ran, and what memory held.
fn frame_log_line(record: &FrameRecord) -> String {
    let phases = record
        .phases
        .iter()
        .map(|phase| format!("{} {:.0}", phase.name, phase.micros))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{:9.1}ms {:+7.1}ms  {:40}  {:3}r {:6.0}px  w{:2} e{:3} x{:.1}  {}  {}n  {:.0}MiB",
        record.at_ms,
        record.interval_ms,
        if phases.is_empty() {
            "-".to_string()
        } else {
            phases
        },
        record.damage.len(),
        record.damage_area,
        record.writes,
        record.effects,
        record.cascade,
        record
            .gpu_micros
            .map_or_else(|| "-".to_string(), |micros| format!("{micros:.0}us")),
        record.layout_nodes,
        record.rss_mib,
    )
}

/// One segment of the stress report, as it prints and as it files.
fn segment_line(segment: &Segment) -> String {
    let phases = segment
        .phase_micros
        .iter()
        .take(4)
        .map(|phase| format!("{} {:.0}", phase.name, phase.micros))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{} — {} frames, {:.0} fps, p50 {:.1} p95 {:.1} ms, damage {:.0} px², {}",
        segment.name,
        segment.frames,
        segment.fps,
        segment.p50_ms,
        segment.p95_ms,
        segment.damage_area,
        if phases.is_empty() {
            "-".to_string()
        } else {
            phases
        },
    )
}

/// The frame log: one line an app frame, written as the frames happen.
struct FrameLog {
    out: BufWriter<File>,
    lines: usize,
}
impl FrameLog {
    fn open(path: &Path) -> io::Result<Self> {
        let mut out = BufWriter::new(File::create(path)?);
        writeln!(
            out,
            "# zgx-profiler frame log: when each app frame came, how long after the one before, each phase's microseconds, damage, reactive work, nodes, memory"
        )?;
        Ok(Self { out, lines: 0 })
    }
    fn frame(&mut self, record: &FrameRecord) {
        if writeln!(self.out, "{}", frame_log_line(record)).is_ok() {
            self.lines += 1;
        }
        if self.lines.is_multiple_of(60) {
            let _ = self.out.flush();
        }
    }
    /// Flush the line in progress, for the break dump's last words.
    fn flush_line(&mut self) -> io::Result<()> {
        self.out.flush()
    }
    /// The totals, then the file: what the log said as a whole.
    fn close(&mut self, summary: &Summary) -> io::Result<usize> {
        writeln!(
            self.out,
            "# {} frames over {:.0}s: {:.0} fps, p50 {:.1} p95 {:.1} p99 {:.1} ms, {} late, {} writes, {} effects, {:.0} MiB",
            summary.app_frames,
            summary.seconds,
            summary.fps,
            summary.p50_ms,
            summary.p95_ms,
            summary.p99_ms,
            summary.dropped,
            summary.total_writes,
            summary.total_effects,
            summary.rss_mib,
        )?;
        self.out.flush()?;
        Ok(self.lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use record::PhaseCost;

    fn record(phases: &[(&'static str, f32, u64)]) -> FrameRecord {
        FrameRecord {
            at_ms: 8234.5,
            interval_ms: 7.2,
            damage: vec![Rect::new(10., 20., 30., 40.)],
            damage_area: 1200.,
            phases: phases
                .iter()
                .map(|&(name, micros, instructions)| PhaseCost {
                    name,
                    micros,
                    instructions,
                })
                .collect(),
            instructions: 12_000,
            writes: 1,
            effects: 29,
            cascade: 24.,
            gpu_micros: Some(120.),
            layout_nodes: 40,
            ..Default::default()
        }
    }

    #[test]
    fn a_frame_log_line_carries_the_frame() {
        let line = frame_log_line(&record(&[("paint", 320., 0), ("layout", 12., 0)]));
        for needed in [
            "8234.5ms",
            "+7.2ms",
            "paint 320",
            "layout 12",
            "1r",
            "w 1",
            "e 29",
        ] {
            assert!(line.contains(needed), "missing {needed:?} in {line}");
        }
    }

    #[test]
    fn a_frame_log_line_survives_a_quiet_frame() {
        let line = frame_log_line(&FrameRecord::default());
        assert!(line.contains("0.0ms"));
    }

    #[test]
    fn a_segment_line_reads_as_a_sentence() {
        let segment = Segment {
            name: "stress: scroll".into(),
            frames: 180,
            fps: 132.,
            p50_ms: 6.9,
            p95_ms: 13.4,
            damage_area: 2008.,
            phase_micros: vec![PhaseCost {
                name: "wait",
                micros: 3802.,
                instructions: 0,
            }],
            ..Default::default()
        };
        let line = segment_line(&segment);
        for needed in [
            "stress: scroll",
            "180 frames",
            "132 fps",
            "p50 6.9",
            "wait 3802",
        ] {
            assert!(line.contains(needed), "missing {needed:?} in {line}");
        }
    }

    #[test]
    fn flash_fill_fades_out_and_pulses() {
        assert!(hud::flash_fill(-1.).is_none());
        assert!(hud::flash_fill(hud::FLASH_MS).is_none());
        let (young, _) = hud::flash_fill(30.).expect("lit while young");
        let (old, _) = hud::flash_fill(500.).expect("lit while fading");
        assert!(young.3 > old.3, "the fill must dim as it ages");
        let mut brightest = 0;
        for step in 0..40 {
            if let Some((fill, _)) = hud::flash_fill(step as f32 * 10.) {
                brightest = brightest.max(fill.3);
            }
        }
        assert!(
            brightest > 60,
            "a flash must actually light up: {brightest}"
        );
    }
}
