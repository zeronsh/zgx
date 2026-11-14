//! (flush, local probe) scroll_glass_bench's scene in a window, for real
//! wheel notches (`~/sol-tools/scroll-ref/post`): `GROUPS` open groups of
//! `ROWS` deep rows under a frosted composer with a shadow, the list fading
//! out over its last 61 points. Put at `AT_X`, `AT_Y` (the second display)
//! and ordered front without activating, it prints its pid, then once a
//! second its CPU time, each thread's (name, user and system ms), and the
//! scroll offset, for `SECONDS`. Run with ZGX_PRESENT_LOG=1 ZGX_GPU_STATS=1.
use std::{cell::RefCell, rc::Rc, time::Duration};
use zgx::{
    widgets::WheelTape,
    compose::prelude::*,
    reactive::Signal,
    scene::{BoxShadow, Color, Rect, Transform},
    style::rgb,
};
use zgx_desktop::{Application, WindowOptions};

const WIDTH: f32 = 1240.;
const HEIGHT: f32 = 800.;
/// The composer's height: with its shadow and blur, a band of 133 points
/// repaints under it (120 made 188).
const COMPOSER: f32 = 65.;

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// What the observing nodes watch, kept alive (each a signal a mounted
/// node republishes: as a Luau `on_resize`, a `ref.width` read, an
/// `on_visible` with a margin).
#[derive(Default)]
struct Watched {
    bounds: Vec<Signal<Rect>>,
    sizes: Vec<Signal<(f32, f32)>>,
    visible: Vec<Signal<bool>>,
}
type Kept = Rc<RefCell<Watched>>;

fn watch_size(view: View, observe: bool, kept: &Kept) -> View {
    if !observe {
        return view;
    }
    let kept = kept.clone();
    view.on_mount(move |handle| kept.borrow_mut().sizes.extend(handle.observe_size()))
}
fn watch_bounds(view: View, observe: bool, kept: &Kept) -> View {
    if !observe {
        return view;
    }
    let kept = kept.clone();
    view.on_mount(move |handle| kept.borrow_mut().bounds.extend(handle.observe_bounds()))
}
fn watch_visible(view: View, observe: bool, kept: &Kept) -> View {
    if !observe {
        return view;
    }
    let kept = kept.clone();
    view.on_mount(move |handle| {
        kept.borrow_mut()
            .visible
            .extend(handle.observe_visibility_within(200.))
    })
}

/// A tool row: a dozen nodes (an icon, a title, a detail clipped to fade
/// out, a status badge, a time, a dot and their boxes), its column's size
/// watched, its title's and detail clip's bounds read, its detail's
/// visibility watched within a margin.
static ROWS_BUILT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn tool_row(index: usize, observe: bool, kept: &Kept) -> View {
    ROWS_BUILT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let title = watch_bounds(
        text(format!("tool {index}: read_file")).text_size(13.),
        observe,
        kept,
    );
    let detail = watch_visible(
        text(format!("crates/zgx/src/scene_{index}.rs, lines 1 to 240"))
            .text_size(12.)
            .text_color(rgb(0xa9a9ae)),
        observe,
        kept,
    );
    let clip = watch_bounds(div().grow().child(detail), observe, kept);
    // Each part in the column a `ui.show` wraps it in, as a transcript's
    // conditional parts are.
    let shown = |view: View| column().child(view);
    let row = row()
        .h(28.)
        .px(8.)
        .gap(8.)
        .items_center()
        .child(shown(div().size(16., 16.).rounded(4.).bg(rgb(0x3b82f6))))
        .child(shown(title))
        .child(shown(clip).grow())
        .child(shown(
            div()
                .px(6.)
                .rounded(8.)
                .bg(rgb(0x1f3a2a))
                .child(text("done").text_size(11.).text_color(rgb(0x6ee7a0))),
        ))
        .child(shown(
            text("0.4 s").text_size(11.).text_color(rgb(0x8a8a90)),
        ))
        .child(shown(div().size(8., 8.).rounded(4.).bg(rgb(0x6ee7a0))));
    watch_size(column().w_full().child(row), observe, kept)
}

/// The levels a transcript nests a row in below its group's list (a
/// message, its parts, a tool group, its body...): `depth` in all, the
/// last six single-child boxes the size of what they hold.
fn nested(mut view: View, depth: usize) -> View {
    for level in 0..depth {
        view = match level < 6 {
            true => div().w_full().child(view),
            false => div().w_full().pl(1.).child(view),
        };
    }
    view
}

/// An open tool group: a header (a message's parts, its size and its
/// header's visibility watched) and its rows, a list built as the
/// transcript's scroll shows them (`virtualize_in_ancestor`).
fn group(
    group: usize,
    rows: usize,
    depth: usize,
    observe: bool,
    kept: &Kept,
    list_offset: Signal<f32>,
) -> View {
    let header = watch_visible(
        row()
            .h(30.)
            .px(12.)
            .items_center()
            .child(text(format!("Ran {rows} tools")).text_size(13.)),
        observe,
        kept,
    );
    let row_kept = kept.clone();
    let list = virtual_list(
        list_offset,
        30.,
        4,
        move || rows,
        move |index| group * 1000 + index,
        move |_, key, _| nested(tool_row(key, observe, &row_kept), depth),
    )
    .virtualize_in_ancestor()
    .w_full();
    let inner = watch_size(
        column().gap(2.).px(12.).py(4.).w_full().child(list),
        observe,
        kept,
    );
    watch_size(
        column()
            .w_full()
            .rounded(10.)
            .bg(rgb(0x16181d))
            .child(header)
            .child(inner),
        observe,
        kept,
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let groups = env("GROUPS", 60);
    let rows = env("ROWS", 20);
    let depth = env("DEPTH", 28);
    let observe = env("OBSERVE", 1) == 1;
    let seconds = env("SECONDS", 14) as u64;
    let at: (f64, f64) = (
        std::env::var("AT_X").ok().and_then(|v| v.parse().ok()).unwrap_or(-2300.),
        std::env::var("AT_Y").ok().and_then(|v| v.parse().ok()).unwrap_or(400.),
    );
    Application::new()
        .window(WindowOptions {
            title: "Scroll glass".into(),
            width: f64::from(WIDTH),
            height: f64::from(HEIGHT),
            ..Default::default()
        })
        .run(move |cx| {
            let offset = cx.ui.signal(0_f32);
            let kept: Kept = Rc::default();
            let mut transcript = column().w_full().gap(10.).px(24.).pt(24.).pb(160.);
            for g in 0..groups {
                transcript =
                    transcript.child(group(g, rows, depth, observe, &kept, cx.ui.signal(0_f32)));
            }
            let composer = div()
        .absolute()
        .ml(120.)
        .mt(HEIGHT - COMPOSER - 20.)
        .size(WIDTH - 240., COMPOSER)
        .rounded(18.)
        .bg(Color(240, 240, 245, 40))
        .backdrop_blur(if env("BLUR", 1) == 1 { 16. } else { 0. })
        .shadow(BoxShadow {
            color: Color(0, 0, 0, 90),
            offset: Transform { x: 0., y: 8. },
            blur_radius: 24.,
            spread: 0.,
        })
        .child(
            text("Ask anything")
                .text_size(14.)
                .text_color(rgb(0xa9a9ae)),
        );
            cx.render(
                div()
                    .w_full()
                    .h_full()
                    .bg(rgb(0x0f1115))
                    .text_color(rgb(0xe8e8ea))
                    .child(
                        scroll(offset.clone())
                            .w_full()
                            .h_full()
                            .fade_edges(0., if env("FADE", 1) == 1 { 61. } else { 0. })
                            .child(transcript),
                    )
                    .child(div().absolute().w_full().h_full().child(composer)),
            );
            let window = cx.window.clone();
            let ui = cx.ui.downgrade();
            cx.tasks.spawn(async move {
                let _kept = kept;
                zgx::timer::sleep(Duration::from_millis(300)).await;
                // `TAPE=1`: the window's own wheel tape (the benchmark's
                // notches, 144 Hz, reversing each 0.5 s, 9 s) instead of
                // real events; left where it opened, behind other windows
                // (run with ZGX_RENDER_WHEN_COVERED=1), the pointer untouched.
                if env("TAPE", 0) == 1 {
                    let mut tape = String::from("# zgx wheel tape 1\n");
                    for i in 0..1296 {
                        let down = (i / 72) % 2 == 0;
                        tape.push_str(&format!(
                            "{:.3} lines 0 {} moved\n",
                            i as f64 * 1000. / 144.,
                            if down { -1 } else { 1 }
                        ));
                    }
                    zgx::timer::sleep(Duration::from_millis(700)).await;
                    if let Some(ui) = ui.upgrade() {
                        let _ = ui.wheel_tape(WheelTape::Play { tape, at: (620., 300.) });
                    }
                    zgx_gpu::present_log::log(|| "tl tape started".into());
                } else {
                    place(at);
                }
                eprintln!("scroll_real pid={} ready", std::process::id());
                for _ in 0..seconds {
                    zgx::timer::sleep(Duration::from_secs(1)).await;
                    let threads = threads();
                    zgx_gpu::present_log::log(|| {
                        format!("probe cpu_ms={:.1} offset={:.0}", cpu_ms(&threads), offset.get())
                    });
                    zgx_gpu::present_log::log(|| format!("threads {}", threads.join(" ")));
                    zgx_gpu::present_log::log(|| {
                        format!("rows_built {}", ROWS_BUILT.swap(0, std::sync::atomic::Ordering::Relaxed))
                    });
                    zgx_gpu::present_log::log(|| format!("instructions {}", instructions()));
                }
                window.close();
            });
        })?;
    Ok(())
}

fn place((x, y): (f64, f64)) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let mtm = MainThreadMarker::new().unwrap();
    for w in NSApplication::sharedApplication(mtm).windows().iter() {
        if w.isVisible() {
            w.setFrameOrigin(objc2_foundation::NSPoint::new(x, y));
            // In front there, without activating the app or taking key focus.
            w.orderFrontRegardless();
        }
    }
}

/// The process's CPU ms, summed from `threads`.
fn cpu_ms(threads: &[String]) -> f64 {
    threads
        .iter()
        .filter_map(|t| t.rsplit('=').next()?.split_once('+'))
        .map(|(u, s)| u.parse::<f64>().unwrap_or(0.) + s.parse::<f64>().unwrap_or(0.))
        .sum()
}

/// Each live thread as `name=user_ms+system_ms` (unnamed: `#index`), from
/// Mach's `THREAD_EXTENDED_INFO`.
fn threads() -> Vec<String> {
    #[repr(C)]
    struct Extended {
        user: u64,
        system: u64,
        cpu_usage: i32,
        policy: i32,
        run_state: i32,
        flags: i32,
        sleep_time: i32,
        curpri: i32,
        priority: i32,
        maxpriority: i32,
        name: [u8; 64],
    }
    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_threads(task: u32, list: *mut *mut u32, count: *mut u32) -> i32;
        fn thread_info(thread: u32, flavor: i32, info: *mut Extended, count: *mut u32) -> i32;
        fn mach_port_deallocate(task: u32, name: u32) -> i32;
        fn vm_deallocate(task: u32, address: usize, size: usize) -> i32;
    }
    let mut out = Vec::new();
    // SAFETY: Mach's calls as declared; the list and ports it returns are
    // given back below.
    unsafe {
        let task = mach_task_self_;
        let mut list: *mut u32 = std::ptr::null_mut();
        let mut count = 0_u32;
        if task_threads(task, &mut list, &mut count) != 0 {
            return out;
        }
        for i in 0..count as usize {
            let thread = *list.add(i);
            let mut info = std::mem::zeroed::<Extended>();
            let mut words = (std::mem::size_of::<Extended>() / 4) as u32;
            if thread_info(thread, 5, &mut info, &mut words) == 0 {
                let len = info.name.iter().position(|b| *b == 0).unwrap_or(64);
                let name = String::from_utf8_lossy(&info.name[..len]).replace([' ', '='], "_");
                let name = if name.is_empty() { format!("#{i}") } else { name };
                out.push(format!(
                    "{name}={:.1}+{:.1}",
                    info.user as f64 / 1e6,
                    info.system as f64 / 1e6
                ));
            }
            mach_port_deallocate(task, thread);
        }
        vm_deallocate(task, list as usize, count as usize * 4);
    }
    out
}

/// The process's retired instructions so far (`proc_pid_rusage`,
/// RUSAGE_INFO_V4's `ri_instructions`: the 30th count after its 16-byte uuid).
fn instructions() -> u64 {
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
    }
    let mut info = [0_u64; 64];
    // SAFETY: room for a rusage_info_v4 (under 64 counts), for this process.
    if unsafe { proc_pid_rusage(std::process::id() as i32, 4, info.as_mut_ptr()) } != 0 {
        return 0;
    }
    info[2 + 29]
}
