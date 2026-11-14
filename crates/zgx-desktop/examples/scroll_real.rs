//! (flush, local) The benchmark's scroll list (`ROWS` keyed rows, 28 pt, a
//! text each, all mounted, in a 1000×700 window) for real wheel events posted
//! through the window server (`~/sol-tools/scroll-ref/post`): put at `AT_X`,
//! `AT_Y` (AppKit's screen points: on the second display), it prints its pid, then once a
//! second its CPU time, for `SECONDS`. Run with ZGX_PRESENT_LOG=1.
use std::time::Duration;
use zgx::{compose::prelude::*, scene::Color};
use zgx_desktop::{Application, WindowOptions};

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rows: usize = env("ROWS", 7000);
    let seconds: u64 = env("SECONDS", 20);
    let at: (f64, f64) = (env("AT_X", -2100.), env("AT_Y", 500.));
    Application::new()
        .window(WindowOptions {
            title: "Scroll real".into(),
            width: 1000.,
            height: 700.,
            ..Default::default()
        })
        .run(move |cx| {
            let offset = cx.ui.signal(0_f32);
            let keys = cx.ui.signal((0..rows).collect::<Vec<usize>>());
            let list = keyed(
                move || keys.get(),
                |index, _| {
                    row()
                        .w_full()
                        .h(28.)
                        .px(12.)
                        .items_center()
                        .bg(if index % 2 == 0 {
                            Color(25, 34, 48, 255)
                        } else {
                            Color(29, 40, 56, 255)
                        })
                        .child(text(format!("Row {index}: some text to shape")))
                },
            );
            cx.render(
                scroll(offset.clone())
                    .w_full()
                    .h_full()
                    .child(column().w_full().child(list)),
            );
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                zgx::timer::sleep(Duration::from_millis(300)).await;
                place(at);
                eprintln!("scroll_real pid={} ready", std::process::id());
                for _ in 0..seconds {
                    zgx::timer::sleep(Duration::from_secs(1)).await;
                    zgx_gpu::present_log::log(|| {
                        format!(
                            "probe cpu_ms={:.1} offset={:.0}",
                            cpu_time().as_secs_f64() * 1e3,
                            offset.get()
                        )
                    });
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
            // In front of other windows there, without activating the app
            // or taking key focus (the poster posts only onto its own).
            w.orderFrontRegardless();
        }
    }
}

/// This process's CPU time (user and system).
fn cpu_time() -> Duration {
    #[repr(C)]
    struct Timeval {
        sec: i64,
        usec: i32,
    }
    #[repr(C)]
    struct Rusage {
        user: Timeval,
        system: Timeval,
        rest: [i64; 14],
    }
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
    let mut usage = std::mem::MaybeUninit::<Rusage>::zeroed();
    // SAFETY: fills the struct it's given (`RUSAGE_SELF` is 0).
    unsafe { getrusage(0, usage.as_mut_ptr()) };
    // SAFETY: filled above, or zeroed if it failed.
    let usage = unsafe { usage.assume_init() };
    let time = |t: &Timeval| Duration::from_secs(t.sec as u64) + Duration::from_micros(t.usec as u64);
    time(&usage.user) + time(&usage.system)
}
