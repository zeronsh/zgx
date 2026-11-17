//! `ZGX_OFFSCREEN=1` (macOS): no window of the session is ever ordered in,
//! by any way an app shows one: its windows opened, shown, focused, restored,
//! made fullscreen or zoomed, a second window, a popover and a native
//! context menu, a prompt and a file panel. AppKit has none visible and
//! WindowServer lists none of this process's windows on a screen, while the
//! window goes on drawing frames. A process of its own (`harness = false`):
//! the flag is read once.
//!
//! Were it to fail, a window would show on this Mac's screen: it runs only
//! when no one has used the Mac for 20 minutes (or on CI).
fn main() {
    #[cfg(target_os = "macos")]
    mac::run();
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use std::time::Duration;
    use zgx::compose::prelude::*;
    use zgx_desktop::{Application, WindowOptions};

    /// Seconds since anyone used the keyboard or pointer.
    fn idle_seconds() -> f64 {
        let out = std::process::Command::new("ioreg")
            .args(["-c", "IOHIDSystem"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .unwrap_or_default();
        out.lines()
            .find(|line| line.contains("\"HIDIdleTime\""))
            .and_then(|line| line.rsplit('=').next())
            .and_then(|n| n.trim().parse::<f64>().ok())
            .map_or(0., |ns| ns / 1e9)
    }

    /// This process's windows WindowServer has on a screen.
    fn on_screen() -> usize {
        use std::ffi::c_void;
        type Ref = *const c_void;
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> Ref;
            static kCGWindowOwnerPID: Ref;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFArrayGetCount(array: Ref) -> isize;
            fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
            fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
            fn CFNumberGetValue(number: Ref, kind: isize, out: *mut c_void) -> bool;
            fn CFRelease(object: Ref);
        }
        const ON_SCREEN_ONLY: u32 = 1;
        const SINT32: isize = 3;
        let me = std::process::id() as i32;
        // SAFETY: the list is owned here and released once; each entry and
        // value is borrowed from it while it lives, the pid read into an i32.
        unsafe {
            let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, 0);
            if list.is_null() {
                return 0;
            }
            let mut mine = 0;
            for at in 0..CFArrayGetCount(list) {
                let window = CFArrayGetValueAtIndex(list, at);
                let pid = CFDictionaryGetValue(window, kCGWindowOwnerPID);
                let mut owner = 0i32;
                if !pid.is_null() {
                    CFNumberGetValue(pid, SINT32, (&raw mut owner).cast());
                }
                mine += usize::from(owner == me);
            }
            CFRelease(list);
            mine
        }
    }

    /// Fail at once, before anything else could show.
    fn check(step: &str) {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        let windows = app.windows();
        let visible: Vec<String> = windows
            .iter()
            .filter(|window| window.isVisible())
            .map(|window| window.title().to_string())
            .collect();
        let listed = on_screen();
        if !visible.is_empty() || listed > 0 || app.isActive() {
            println!(
                "offscreen: FAILED after {step}: visible {visible:?}, {listed} on a screen, \
                 active {}",
                app.isActive()
            );
            std::process::exit(1);
        }
        println!(
            "offscreen: after {step}: {} windows, none visible or on a screen",
            windows.len()
        );
    }

    pub(super) fn run() {
        let idle = idle_seconds();
        println!("offscreen: HIDIdleTime {idle:.0} s");
        if idle < 1200. && std::env::var_os("CI").is_none() {
            println!("offscreen: skipped: the Mac was used in the last 20 minutes");
            return;
        }
        // SAFETY: before any thread is made.
        unsafe { std::env::set_var("ZGX_OFFSCREEN", "1") };
        let options = |title: &str| WindowOptions {
            title: title.into(),
            width: 320.,
            height: 200.,
            ..Default::default()
        };
        Application::new()
            .window(options("Offscreen"))
            .run(move |cx| {
                let open = cx.ui.signal(false);
                cx.render(
                    column().p(20.).child(text("Offscreen")).child(
                        popover("Popover", open.clone(), button().child(text("Anchor")))
                            .child(text("In the popover")),
                    ),
                );
                let (window, windows, dialogs, frames) = (
                    cx.window.clone(),
                    cx.windows.clone(),
                    cx.dialogs.clone(),
                    cx.frames.clone(),
                );
                // Frames as they come, the window never shown.
                let drawn = std::rc::Rc::new(std::cell::Cell::new(0u32));
                let counted = drawn.clone();
                cx.tasks.spawn(async move {
                    loop {
                        frames.next().await;
                        counted.set(counted.get() + 1);
                    }
                });
                cx.tasks.spawn(async move {
                    let settle = || zgx::timer::sleep(Duration::from_millis(400));
                    settle().await;
                    check("opening");
                    window.set_visible(true);
                    settle().await;
                    check("set_visible(true)");
                    window.request_focus();
                    settle().await;
                    check("request_focus");
                    window.set_minimized(true);
                    settle().await;
                    window.set_minimized(false);
                    settle().await;
                    check("set_minimized(false)");
                    window.set_fullscreen(true);
                    settle().await;
                    check("set_fullscreen(true)");
                    window.set_fullscreen(false);
                    window.set_maximized(true);
                    settle().await;
                    check("set_maximized(true)");
                    open.set(true);
                    settle().await;
                    check("a popover open");
                    window.show_context_menu(
                        vec![zgx_desktop::AppMenuEntry::item("Item", "item")],
                        20.,
                        20.,
                    );
                    settle().await;
                    check("show_context_menu");
                    let asked = dialogs
                        .prompt(zgx_desktop::PromptOptions::new("Prompt", "Asked offscreen"))
                        .await;
                    println!("offscreen: prompt answered {asked:?}");
                    check("prompt");
                    let picked = dialogs
                        .open_file(zgx_desktop::FileDialogOptions::new("Open"))
                        .await;
                    println!("offscreen: file panel answered {picked:?}");
                    check("open_file");
                    windows.open(options("Second"), |cx| {
                        cx.render(div().p(20.).child(text("Second")));
                    });
                    settle().await;
                    check("a second window");
                    // It goes on drawing: frames come, the window never shown.
                    let before = drawn.get();
                    zgx::timer::sleep(Duration::from_secs(1)).await;
                    let came = drawn.get() - before;
                    if came < 5 {
                        println!("offscreen: FAILED: {came} frames in a second");
                        std::process::exit(1);
                    }
                    check("a second of frames");
                    println!("offscreen: ok");
                    std::process::exit(0);
                });
            })
            .unwrap();
    }
}
