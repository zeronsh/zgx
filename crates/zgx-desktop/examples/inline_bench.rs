//! Chips in a transcript: a virtualized list of paragraphs of words and
//! about 50 chips each (an icon, a label, a rounded fill, a click), written
//! as spans with icons (`MODE=spans`) or as inline boxes (`MODE=boxes`, each
//! chip an ordinary row in the line); `MODE=rows` has the same chips outside
//! a paragraph, a control. What an operation costs through to a
//! frame ready to present:
//!
//! - `OP=scroll`: the list scrolls 12 points, as a transcript reads.
//! - `OP=resize`: one chip's label changes size, in the view.
//! - `OP=none`: nothing: what mounting and the first frame cost, to subtract.
//!
//!   MODE=boxes OP=scroll OPS=2000 cargo run --release -p zgx-desktop --example inline_bench
//!
//! `RENDER=0` measures text as a renderer would and draws nothing, as CI's
//! instruction gate counts it. Prints what it did and its CPU time; run it
//! under `/usr/bin/time -l` (`perf stat` on Linux) for instructions and peak
//! memory. It also says, as [footprint, live heap] in MB, what it holds as it
//! starts, working, trimmed at rest as a window's host trims, and with the
//! list gone.
use std::sync::Arc;
use zgx::{
    compose::{SpanIcon, VariableHeights, measured_virtual_list, prelude::*},
    reactive::Signal,
    scene::Color,
    svg::SvgData,
    widgets::Ui,
};
use zgx_gpu::{GpuRenderer, TextServices};

#[path = "support/headless_font.rs"]
mod headless_font;

const WIDTH: f32 = 800.;
const HEIGHT: f32 = 900.;
const ROWS: usize = 2000;
const CHIPS: usize = 50;
const STEP: f32 = 12.;
const ESTIMATE: f32 = 400.;
const ICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 12 12"><path d="M2 1h5l3 3v7H2z" fill="#9ab"/></svg>"##;
const FILL: Color = Color(40, 60, 90, 255);

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn label(row: usize, chip: usize) -> String {
    format!("file_{}.rs", (row * 31 + chip * 7) % 1000)
}

/// Chips as spans: each a padded span with an icon and a fill.
fn spans_row(row_index: usize, svg: Arc<SvgData>, first: Signal<String>) -> View {
    rich_text_signal(move || {
        let mut spans = Vec::with_capacity(2 * CHIPS);
        for chip in 0..CHIPS {
            spans.push(text_span("see the change in "));
            let name = match (row_index, chip) {
                (0, 0) => first.get(),
                _ => label(row_index, chip),
            };
            spans.push(
                text_span(name)
                    .icon(SpanIcon {
                        svg: svg.clone(),
                        size: 12.,
                        gap: 4.,
                        tint: None,
                        bg: None,
                        radius: 0.,
                        pad: 0.,
                    })
                    .padding(4., 1.)
                    .radius(4.)
                    .background(FILL)
                    .on_click(|| {})
                    .no_underline(),
            );
        }
        spans
    })
    .w_full()
    .text_wrap(true)
}

/// One chip, an ordinary row: an icon, a label, a rounded fill, a click.
/// (`NO_CLICK=1`, `NO_ICON=1`: without one, to see what each holds.)
fn chip(icon: &Arc<SvgData>, name: View) -> View {
    let mut chip = row()
        .items_center()
        .gap(4.)
        .px(4.)
        .py(1.)
        .rounded(4.)
        .bg(FILL);
    if env("NO_CLICK", 0) == 0 {
        chip = chip.on_click(|| {});
    }
    if env("NO_ICON", 0) == 0 {
        chip = chip.child(zgx::compose::svg("file", icon.clone()).w(12.).h(12.));
    }
    chip.child(name)
}

fn name_of(row_index: usize, chip: usize, first: &Signal<String>) -> View {
    match (row_index, chip) {
        (0, 0) => {
            let first = first.clone();
            text_signal(move || first.get())
        }
        _ => text(label(row_index, chip)),
    }
}

/// Chips as inline boxes: each in the line.
fn boxes_row(row_index: usize, icon: Arc<SvgData>, first: Signal<String>) -> View {
    let mut paragraph = rich_text().w_full().text_wrap(true);
    for at in 0..CHIPS {
        paragraph = paragraph
            .child(text_span("see the change in "))
            .inline(chip(&icon, name_of(row_index, at, &first)));
    }
    paragraph.into()
}

/// The same chips and words as rows in a wrapping row: no paragraph, what
/// `boxes` holds that isn't the inline boxes' own.
fn rows_row(row_index: usize, icon: Arc<SvgData>, first: Signal<String>) -> View {
    let mut holder = row().w_full().flex_wrap();
    for at in 0..CHIPS {
        holder = holder
            .child(text("see the change in "))
            .child(chip(&icon, name_of(row_index, at, &first)));
    }
    holder
}

/// The process's CPU time, in seconds (its wall clock on Windows, whose
/// process clock takes more than a bench's measuring needs).
#[cfg(unix)]
fn cpu_seconds() -> f64 {
    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, time: *mut Timespec) -> i32;
    }
    // CLOCK_PROCESS_CPUTIME_ID.
    let clock = if cfg!(target_os = "macos") { 12 } else { 2 };
    let mut time = Timespec { sec: 0, nsec: 0 };
    // SAFETY: `time` is a valid place for the clock to write.
    unsafe { clock_gettime(clock, &mut time) };
    time.sec as f64 + time.nsec as f64 * 1e-9
}
#[cfg(not(unix))]
fn cpu_seconds() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// What the process holds now, in bytes (macOS's footprint; 0 elsewhere,
/// where `/usr/bin/time` or a peak has to do).
fn footprint() -> u64 {
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct Rusage {
            uuid: [u8; 16],
            fields: [u64; 10],
        }
        unsafe extern "C" {
            fn getpid() -> i32;
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut std::ffi::c_void) -> i32;
        }
        let mut usage = Rusage {
            uuid: [0; 16],
            fields: [0; 10],
        };
        // SAFETY: flavor 0 (RUSAGE_INFO_V0) fills a `rusage_info_v0`: a
        // 16-byte uuid then ten u64s (`phys_footprint` the eighth).
        let filled = unsafe { proc_pid_rusage(getpid(), 0, (&mut usage as *mut Rusage).cast()) };
        if filled == 0 { usage.fields[7] } else { 0 }
    }
    #[cfg(not(target_os = "macos"))]
    0
}

/// What the heap holds live now, in bytes: the footprint less what is freed
/// but not given back (macOS; 0 elsewhere).
fn live_heap() -> u64 {
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Statistics {
            blocks: u32,
            in_use: usize,
            most_in_use: usize,
            allocated: usize,
        }
        unsafe extern "C" {
            fn malloc_zone_statistics(zone: *mut std::ffi::c_void, stats: *mut Statistics);
        }
        let mut stats = Statistics::default();
        // SAFETY: a null zone sums every zone into `stats`.
        unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut stats) };
        stats.in_use as u64
    }
    #[cfg(not(target_os = "macos"))]
    0
}

/// `WINDOW=1`: in a real window, as Activity Monitor sees one: the list scrolls
/// for `SCROLL` seconds, then the window is left alone so its host trims it,
/// with footprint and live heap (in MB) at each point; then the list goes.
fn window() -> Result<(), Box<dyn std::error::Error>> {
    use zgx_desktop::{Application, WindowOptions};
    let mode: String = env("MODE", "boxes".to_string());
    let scroll: f64 = env("SCROLL", 15.);
    // Rows built, so a run that drew nothing doesn't pass for a measurement.
    let built = std::rc::Rc::new(std::cell::Cell::new(0_usize));
    let counted = built.clone();
    let report = move |at: &str| {
        println!(
            "{mode} {at}: footprint {:.1} live {:.1} rows {}",
            footprint() as f64 / 1e6,
            live_heap() as f64 / 1e6,
            counted.get()
        );
    };
    Application::new()
        .window(WindowOptions {
            title: "Inline bench".into(),
            width: WIDTH as f64,
            height: HEIGHT as f64,
            ..Default::default()
        })
        .run(move |cx| {
            let which: String = env("MODE", "boxes".to_string());
            let svg = Arc::new(SvgData::new(ICON).expect("the icon"));
            let first = cx.ui.signal(label(0, 0));
            let offset = cx.ui.signal(0_f32);
            let shown = cx.ui.signal(true);
            let heights = VariableHeights::new(&cx.ui.runtime, ROWS, ESTIMATE);
            let (list_offset, list_heights) = (offset.clone(), heights.clone());
            let (list_svg, list_first) = (svg.clone(), first.clone());
            let holder = shown.clone();
            cx.render(
                column()
                    .w_full()
                    .h_full()
                    .text_size(14.)
                    .child(zgx::compose::switch(
                        move || holder.get(),
                        move |shown, _| {
                            if !shown {
                                return div();
                            }
                            let (icon, name, which) =
                                (list_svg.clone(), list_first.clone(), which.clone());
                            let built = built.clone();
                            measured_virtual_list(
                                list_offset.clone(),
                                list_heights.clone(),
                                2,
                                |i| i,
                                move |_, index, _| {
                                    built.set(built.get() + 1);
                                    let row = match which.as_str() {
                                        "boxes" => boxes_row(index, icon.clone(), name.clone()),
                                        "rows" => rows_row(index, icon.clone(), name.clone()),
                                        _ => spans_row(index, icon.clone(), name.clone()),
                                    };
                                    column().w_full().p(8.).child(row)
                                },
                            )
                            .w_full()
                            .h_full()
                        },
                    )),
            );
            cx.tasks.spawn(async move {
                let wait = |s: f64| zgx::timer::sleep(std::time::Duration::from_secs_f64(s));
                wait(2.).await;
                report("mounted");
                let started = std::time::Instant::now();
                while started.elapsed().as_secs_f64() < scroll {
                    let next = offset.get() + STEP;
                    let end = heights.content_height() - HEIGHT;
                    offset.set(if next > end { 0. } else { next });
                    wait(0.008).await;
                }
                report("scrolled");
                // Past the host's trim, then past its atlas's.
                wait(5.).await;
                report("idle 5 s");
                wait(30.).await;
                report("idle 35 s");
                shown.set(false);
                wait(5.).await;
                report("gone 5 s");
                wait(30.).await;
                report("gone 35 s");
                std::process::exit(0);
            });
        })?;
    Ok(())
}

fn main() {
    if env("WINDOW", 0) != 0 {
        window().expect("the window");
        return;
    }
    let mode: String = env("MODE", "boxes".to_string());
    let op: String = env("OP", "scroll".to_string());
    let ops: usize = env("OPS", 1000);
    let render = env("RENDER", 1) != 0;
    let mut ui = Ui::new(WIDTH, HEIGHT);
    let text_services = TextServices::new();
    let mut gpu = render.then(|| {
        let gpu = GpuRenderer::new(WIDTH as u32, HEIGHT as u32).expect("GPU");
        headless_font::font().install(&mut gpu.text_system().borrow_mut());
        gpu.install_text(&mut ui.scene.borrow_mut());
        gpu
    });
    if gpu.is_none() {
        headless_font::font().install(&mut text_services.text_system().borrow_mut());
        text_services.install(&mut ui.scene.borrow_mut());
    }
    let baseline = footprint();
    let live_start = live_heap();
    let svg = Arc::new(SvgData::new(ICON).expect("the icon"));
    let first = ui.signal(label(0, 0));
    let offset = ui.signal(0_f32);
    let heights = VariableHeights::new(&ui.runtime, ROWS, ESTIMATE);
    let which = mode.clone();
    let (icon, name) = (svg.clone(), first.clone());
    let list = measured_virtual_list(
        offset.clone(),
        heights.clone(),
        2,
        |i| i,
        move |_, index, _| {
            let row = match which.as_str() {
                "boxes" => boxes_row(index, icon.clone(), name.clone()),
                "rows" => rows_row(index, icon.clone(), name.clone()),
                _ => spans_row(index, icon.clone(), name.clone()),
            };
            column().w_full().p(8.).child(row)
        },
    )
    .w_full()
    .h_full();
    let handle = ui.mount(
        column()
            .w_full()
            .h_full()
            .font_family(zgx_workload::FONT)
            .text_size(14.)
            .child(list),
    );
    let frame = |ui: &mut Ui, gpu: &mut Option<GpuRenderer>| {
        ui.prepare_frame();
        let report = ui.scene.borrow_mut().flush();
        if let Some(gpu) = gpu {
            gpu.render(&ui.scene.borrow(), &report.damage)
                .expect("render");
        }
    };
    frame(&mut ui, &mut gpu);
    let nodes = ui.scene.borrow().len();
    let first_height = heights.content_height();
    let started = cpu_seconds();
    for at in 0..ops {
        match op.as_str() {
            "scroll" => {
                let next = offset.get() + STEP;
                let end = heights.content_height() - HEIGHT;
                offset.set(if next > end { 0. } else { next });
            }
            "resize" => {
                first.set(if at % 2 == 0 {
                    "a_longer_file_name.rs".to_string()
                } else {
                    label(0, 0)
                });
            }
            _ => {}
        }
        frame(&mut ui, &mut gpu);
        if at % 64 == 63
            && let Some(gpu) = &mut gpu
        {
            gpu.readback().expect("readback");
        }
    }
    let spent = cpu_seconds() - started;
    let view = ui.scene.borrow().bounds(handle.node()).height;
    let height = heights.content_height();
    let working = (footprint(), live_heap());
    // Left alone, as a window's host trims one at rest, its list mounted;
    // then the list gone, and trimmed the same.
    let at_rest = |ui: &mut Ui, gpu: &mut Option<GpuRenderer>| {
        zgx::idle::trim(&ui.runtime);
        if let Some(gpu) = gpu {
            gpu.trim();
            gpu.trim_atlas();
        }
        text_services.release_unused();
        ui.scene.borrow_mut().rest();
        ui.semantics.borrow_mut().rest();
        zgx::theme::Themes::of(&ui.runtime).rest();
        // What the allocator holds freed goes back.
        #[cfg(target_os = "macos")]
        {
            unsafe extern "C" {
                fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
            }
            // SAFETY: a null zone is every zone; it only releases what's free.
            unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
        }
        (footprint(), live_heap())
    };
    let idle = at_rest(&mut ui, &mut gpu);
    ui.remove(handle.node());
    for _ in 0..4 {
        frame(&mut ui, &mut gpu);
    }
    while ui.reclaim(std::time::Duration::from_secs(1)) {}
    let gone = at_rest(&mut ui, &mut gpu);
    let left = ui.scene.borrow().len();
    // `MEMORY=1`: what the renderer says it holds, line by line.
    if env("MEMORY", 0) != 0
        && let Some(gpu) = &gpu
    {
        let mut report = zgx::memory::Report::default();
        gpu.memory(&mut report);
        for line in &report.lines {
            let (name, entries, bytes) = (&line.name, line.entries, line.bytes);
            eprintln!("{:<10} {name:<40} {entries:?} {bytes:?}", line.area);
        }
    }
    let mb = |bytes: u64| bytes as f64 / 1e6;
    println!(
        "{{\"mode\":\"{mode}\",\"op\":\"{op}\",\"render\":{render},\"ops\":{ops},\"nodes\":{nodes},\"us_per_op\":{:.1},\"list_first\":{first_height:.0},\"list_last\":{height:.0},\"view\":{view:.0},\"nodes_rest\":{left},\"start\":[{:.1},{:.1}],\"working\":[{:.1},{:.1}],\"idle\":[{:.1},{:.1}],\"gone\":[{:.1},{:.1}]}}",
        spent * 1e6 / ops.max(1) as f64,
        mb(baseline),
        mb(live_start),
        mb(working.0),
        mb(working.1),
        mb(idle.0),
        mb(idle.1),
        mb(gone.0),
        mb(gone.1),
    );
}
