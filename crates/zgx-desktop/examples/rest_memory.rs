//! What a window holds at rest after a list scrolled, as Activity Monitor sees
//! it: a virtual list of rows of chips and words scrolls for `SCROLL` seconds,
//! then the window is left alone, then the list goes. It prints each point and
//! holds still there, so `footprint -p` can read it from outside
//! (`scripts/rest_memory.py` does). With it, the engine's own memory report,
//! and whether the window was presenting: one that another covered holds what
//! it would give back, and says nothing of a window at rest.
//!
//! Build it with the allocator apps have, or the numbers are the system
//! allocator's, which keeps tens of MB more of what's been freed:
//!
//!   cargo build --release -p zgx-desktop --example rest_memory --features mimalloc
//!   python3 scripts/rest_memory.py target/release/examples/rest_memory
//!
//! `LIST=0`: the same window with no list. `HIDE=1`: the list goes while the
//! window is hidden. `NO_CLICK=1`, `NO_ICON=1`: chips without those.
use std::sync::Arc;
use zgx::{
    compose::{VariableHeights, measured_virtual_list, prelude::*},
    reactive::Signal,
    scene::Color,
    svg::SvgData,
};

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zgx_desktop::{Application, WindowOptions};
    if !cfg!(feature = "mimalloc") {
        eprintln!(
            "rest_memory: built without `--features mimalloc`: the system allocator's \
             numbers, not an app's"
        );
    }
    let scroll: f64 = env("SCROLL", 15.);
    // Rows built, so a run that drew nothing doesn't pass for a measurement.
    let built = std::rc::Rc::new(std::cell::Cell::new(0_usize));
    let counted = built.clone();
    let report = move |at: &str,
                       scene: &std::rc::Rc<std::cell::RefCell<zgx::scene::Scene>>,
                       weak: &zgx::widgets::WeakUi| {
        use std::io::Write;
        if let Some(ui) = weak.upgrade() {
            println!(
                "MEM {at} | host | presented {} reclaiming {} | None | 0.00 MB",
                ui.is_presented(),
                ui.reclaiming()
            );
        }
        let mut lines = zgx::memory::Report::default();
        scene.borrow().memory(&mut lines);
        for line in &lines.lines {
            println!(
                "MEM {at} | {} | {} | {:?} | {:.2} MB",
                line.area,
                line.name,
                line.entries,
                line.bytes.unwrap_or(0) as f64 / 1e6
            );
        }
        println!(
            "POINT {at}: footprint {:.1} rows {}",
            footprint() as f64 / 1e6,
            counted.get()
        );
        let _ = std::io::stdout().flush();
    };
    Application::new()
        .window(WindowOptions {
            title: "Inline bench".into(),
            width: WIDTH as f64,
            height: HEIGHT as f64,
            ..Default::default()
        })
        .run(move |cx| {
            let svg = Arc::new(SvgData::new(ICON).expect("the icon"));
            let first = cx.ui.signal(label(0, 0));
            let offset = cx.ui.signal(0_f32);
            let shown = cx.ui.signal(true);
            let heights = VariableHeights::new(&cx.ui.runtime, ROWS, ESTIMATE);
            let (list_offset, list_heights) = (offset.clone(), heights.clone());
            let (list_svg, list_first) = (svg.clone(), first.clone());
            let holder = shown.clone();
            // `LIST=0`: the same window with no list: what an empty one holds.
            let list = env("LIST", 1) != 0;
            let scene = cx.ui.scene.clone();
            let window = cx.window.clone();
            let weak = cx.ui.downgrade();
            cx.render(
                column()
                    .w_full()
                    .h_full()
                    .text_size(14.)
                    .child(zgx::compose::switch(
                        move || holder.get(),
                        move |shown, _| {
                            if !shown || !list {
                                return div();
                            }
                            let (icon, name) = (list_svg.clone(), list_first.clone());
                            let built = built.clone();
                            measured_virtual_list(
                                list_offset.clone(),
                                list_heights.clone(),
                                2,
                                |i| i,
                                move |_, index, _| {
                                    built.set(built.get() + 1);
                                    column().w_full().p(8.).child(rows_row(
                                        index,
                                        icon.clone(),
                                        name.clone(),
                                    ))
                                },
                            )
                            .w_full()
                            .h(HEIGHT)
                        },
                    )),
            );
            cx.tasks.spawn(async move {
                let wait = |s: f64| zgx::timer::sleep(std::time::Duration::from_secs_f64(s));
                async fn host_lines(at: &str) {
                    let report = zgx::memory::from_host().await;
                    for line in report.lines {
                        println!(
                            "MEM {at} | {} | {} | {:?} | {:.2} MB",
                            line.area,
                            line.name,
                            line.entries,
                            line.bytes.unwrap_or(0) as f64 / 1e6
                        );
                    }
                }
                const HOLD: f64 = 20.;
                wait(2.).await;
                host_lines("mounted").await;
                report("mounted", &scene, &weak);
                wait(HOLD).await;
                let started = std::time::Instant::now();
                let mut read = false;
                while started.elapsed().as_secs_f64() < scroll {
                    let next = offset.get() + STEP;
                    let end = heights.content_height() - HEIGHT;
                    offset.set(if next > end { 0. } else { next });
                    if !read && started.elapsed().as_secs_f64() >= scroll / 3. {
                        read = true;
                        host_lines("scrolling").await;
                        report("scrolling", &scene, &weak);
                    }
                    wait(0.008).await;
                }
                wait(5.).await;
                host_lines("idle 5 s").await;
                report("idle 5 s", &scene, &weak);
                wait(30.).await;
                host_lines("idle 35 s").await;
                report("idle 35 s", &scene, &weak);
                wait(HOLD).await;
                // `HIDE=1`: the list goes while the window isn't presenting.
                if env("HIDE", 0) != 0 {
                    window.set_visible(false);
                    wait(1.).await;
                }
                shown.set(false);
                wait(5.).await;
                host_lines("gone 5 s").await;
                report("gone 5 s", &scene, &weak);
                wait(30.).await;
                host_lines("gone 35 s").await;
                report("gone 35 s", &scene, &weak);
                wait(HOLD).await;
                std::process::exit(0);
            });
        })?;
    Ok(())
}
