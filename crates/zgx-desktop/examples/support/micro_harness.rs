//! The microbenchmarks' harness (`micro_bench`, and `micro_luau_bench`
//! for the same scenes in Luau): each scene timed per operation, for the
//! operation alone ("write") and through to a frame ready to present
//! ("frame"). See `micro_bench.rs`.
use zgx::{compose::prelude::*, reactive::Signal, widgets::Ui};
use zgx_gpu::{GpuRenderer, TextServices};
use zgx_workload::{
    micro::{self, *},
    vlist,
};

#[path = "headless_font.rs"]
mod headless_font;
/// The variable list's rows (its estimate, for another engine's).
#[path = "vlist_view.rs"]
pub mod vlist_view;

/// Another engine's scene (`micro_luau_bench`'s): mounted on the window's
/// `Ui` for `SCENE`, with what it keeps alive and what one operation does.
pub type Engine<'a> = &'a dyn Fn(&mut Ui, &str) -> (Box<dyn std::any::Any>, Box<dyn FnMut(usize)>);

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The process's CPU time (all its threads), in nanoseconds.
#[cfg(unix)]
fn cpu_ns() -> u64 {
    #[repr(C)]
    struct Timespec {
        seconds: i64,
        nanoseconds: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, time: *mut Timespec) -> i32;
    }
    // CLOCK_PROCESS_CPUTIME_ID, or with `CLOCK=thread` CLOCK_THREAD_CPUTIME_ID
    // (this thread alone: what the GPU driver's threads add is the difference).
    let thread = std::env::var("CLOCK").is_ok_and(|v| v == "thread");
    let clock: i32 = match (cfg!(target_os = "macos"), thread) {
        (true, false) => 12,
        (true, true) => 16,
        (false, false) => 2,
        (false, true) => 3,
    };
    let mut time = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    // SAFETY: a valid clock id and a timespec to write.
    unsafe { clock_gettime(clock, &mut time) };
    time.seconds as u64 * 1_000_000_000 + time.nanoseconds as u64
}
/// Elsewhere, wall time (the figures say so).
#[cfg(not(unix))]
fn cpu_ns() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u64
}

/// What one operation changes, and how a frame is made of it.
struct Bench {
    ui: Ui,
    gpu: Option<GpuRenderer>,
    _text: TextServices,
    step: Box<dyn FnMut(usize)>,
    /// What another engine's scene keeps alive while it runs.
    _kept: Option<Box<dyn std::any::Any>>,
}
impl Bench {
    fn new(scene: &str, render: bool, engine: Option<Engine>) -> Self {
        let (width, height) = if scene == "state" {
            (zgx_workload::WIDTH, zgx_workload::HEIGHT)
        } else {
            (vlist::WIDTH, VIEWPORT)
        };
        let mut ui = Ui::new(width, height);
        let text = TextServices::new();
        let gpu = render.then(|| {
            let scale = 2.;
            let mut gpu =
                GpuRenderer::new((width * scale) as u32, (height * scale) as u32).expect("GPU");
            gpu.set_scale_factor(scale);
            headless_font::font().install(&mut gpu.text_system().borrow_mut());
            gpu.install_text(&mut ui.scene.borrow_mut());
            gpu
        });
        if gpu.is_none() {
            headless_font::font().install(&mut text.text_system().borrow_mut());
            text.install(&mut ui.scene.borrow_mut());
        }
        let mut kept = None;
        let step: Box<dyn FnMut(usize)> = match (scene, engine) {
            (_, Some(engine)) => {
                let (keep, step) = engine(&mut ui, scene);
                kept = Some(keep);
                step
            }
            ("state", None) => {
                let counter = ui.signal(0_u64);
                ui.mount(state_view(counter.clone()));
                Box::new(move |op| {
                    counter.set(op as u64 + 1);
                })
            }
            ("vscroll-fixed" | "vscroll-variable", None) => {
                let offset = ui.signal(0_f32);
                // Where the list ends: fixed, or as far as the rows' heights
                // (measured as they mount, estimated otherwise) reach.
                let (view, end): (View, Box<dyn Fn() -> f32>) = if scene == "vscroll-fixed" {
                    (
                        fixed_list(offset.clone()),
                        Box::new(|| ROWS as f32 * FIXED_ROW - VIEWPORT),
                    )
                } else {
                    let heights = VariableHeights::new(&ui.runtime, ROWS, vlist_view::ESTIMATE);
                    let list = measured_virtual_list(
                        offset.clone(),
                        heights.clone(),
                        2,
                        |i| i,
                        |_, index, _| vlist_view::chat_row(index),
                    )
                    .w_full()
                    .h_full();
                    (
                        vlist_view::frame(list),
                        Box::new(move || heights.content_height() - VIEWPORT),
                    )
                };
                ui.mount(view);
                Box::new(move |_| {
                    // Past the end, back to the top.
                    let next = offset.get() + STEP;
                    offset.set(if next > end() { 0. } else { next });
                })
            }
            (other, None) => panic!("SCENE {other}: state, vscroll-fixed or vscroll-variable"),
        };
        let mut bench = Self {
            ui,
            gpu,
            _text: text,
            step,
            _kept: kept,
        };
        bench.frame();
        bench
    }
    /// Layout, damage and (with a renderer) encode and submit: a frame ready to present.
    fn frame(&mut self) {
        self.ui.prepare_frame();
        let report = self.ui.scene.borrow_mut().flush();
        if zgx_fingerprint::on() {
            zgx_fingerprint::frame(&self.ui.scene.borrow(), &report);
        }
        if let Some(gpu) = &mut self.gpu {
            gpu.render(&self.ui.scene.borrow(), &report.damage)
                .expect("render");
        }
    }
    /// Settles the GPU between batches (untimed): a window's presents never
    /// let submitted work pile up.
    fn settle(&mut self) {
        if let Some(gpu) = &mut self.gpu {
            gpu.readback().expect("readback");
        }
    }
}

fn state_view(counter: Signal<u64>) -> View {
    let mut grid = column().gap(2.);
    for r in 0..STATIC_ROWS {
        let mut line = row().gap(2.);
        for c in 0..STATIC_COLUMNS {
            line = line.child(
                text(static_label(r * STATIC_COLUMNS + c))
                    .w(CELL.0)
                    .h(CELL.1),
            );
        }
        grid = grid.child(line);
    }
    column()
        .w_full()
        .h_full()
        .p(8.)
        .gap(6.)
        .bg(rgb(vlist::BACKGROUND))
        .font_family(zgx_workload::FONT)
        .text_size(micro::FONT_SIZE)
        .text_color(rgb(vlist::TEXT))
        .child(text_signal(move || counter_label(counter.get())).h(20.))
        .child(grid)
}

fn fixed_list(offset: Signal<f32>) -> View {
    let rows = virtual_list(
        offset,
        FIXED_ROW,
        2,
        || ROWS,
        |index| index,
        |_, index, _| {
            row()
                .w_full()
                .h(FIXED_ROW)
                .px(12.)
                .items_center()
                .bg(rgb(vlist::ROW_COLORS[index % 2]))
                .child(text(fixed_label(index)))
        },
    )
    .w_full()
    .h_full();
    column()
        .w_full()
        .h_full()
        .bg(rgb(vlist::BACKGROUND))
        .font_family(zgx_workload::FONT)
        .text_size(vlist::FONT_SIZE)
        .text_color(rgb(vlist::TEXT))
        .child(rows)
}

/// Median, p10, p90, min and max of `samples` (ns per operation).
fn summary(samples: &mut [f64]) -> String {
    samples.sort_by(f64::total_cmp);
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
    format!(
        "\"median_ns\":{:.1},\"p10_ns\":{:.1},\"p90_ns\":{:.1},\"min_ns\":{:.1},\"max_ns\":{:.1},\"batches\":{}",
        at(0.5),
        at(0.1),
        at(0.9),
        samples[0],
        samples[samples.len() - 1],
        samples.len()
    )
}

/// Times `ops` operations in batches of `batch` (after `WARMUP` untimed),
/// each through `run`, settling the GPU between batches.
fn measure(bench: &mut Bench, ops: usize, batch: usize, frame: bool, next: &mut usize) -> Vec<f64> {
    let one = |bench: &mut Bench, next: &mut usize| {
        (bench.step)(*next);
        *next += 1;
        if frame {
            bench.frame();
        }
    };
    for _ in 0..env("WARMUP", WARMUP) {
        one(bench, next);
    }
    bench.settle();
    let mut samples = Vec::with_capacity(ops / batch);
    for _ in 0..ops / batch {
        let started = cpu_ns();
        for _ in 0..batch {
            one(bench, next);
        }
        samples.push((cpu_ns() - started) as f64 / batch as f64);
        bench.settle();
    }
    // A write alone leaves the frame to do: catch up, untimed.
    if !frame {
        bench.frame();
        bench.settle();
    }
    samples
}

/// Runs `SCENE` (in Rust, or in `engine`), printing its figures as
/// `framework`'s.
pub fn main(framework: &str, engine: Option<Engine>) {
    let scene: String = env("SCENE", "state".to_string());
    let ops: usize = env("OPS", micro::OPS);
    let render = env("RENDER", 1) != 0;
    let clock = match (cfg!(unix), std::env::var("CLOCK").as_deref()) {
        (true, Ok("thread")) => "thread_cpu",
        (true, _) => "process_cpu",
        _ => "wall",
    };
    let mut bench = Bench::new(&scene, render, engine);
    let mut next = 0;
    // The write alone (whatever it runs at once), then write and frame.
    for (figure, frame, batch) in [
        ("write", false, env("WRITE_BATCH", 50)),
        ("frame", true, env("FRAME_BATCH", 10)),
    ] {
        let mut samples = measure(&mut bench, ops, batch, frame, &mut next);
        println!(
            "{{\"framework\":\"{framework}\",\"scene\":\"{scene}\",\"figure\":\"{figure}\",\"render\":{render},\"clock\":\"{clock}\",\"ops\":{ops},\"batch\":{batch},{}}}",
            summary(&mut samples)
        );
    }
}
