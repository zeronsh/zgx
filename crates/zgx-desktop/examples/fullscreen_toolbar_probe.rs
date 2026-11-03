//! An inset title bar in full screen (macOS): AppKit moves a window's
//! toolbar into a window of its own above it (NSToolbarFullScreenWindow),
//! where the inset title bar's empty toolbar showed as an opaque 40-point
//! bar over the content's top. `scripts/macos_fullscreen_toolbar_probe.py`
//! runs it and checks the lines.
//!
//! `--via green` goes in and out of full screen as the green button does
//! (`toggleFullScreen:`); `--via borderless` with zgx's
//! `WindowHandle::set_fullscreen` (winit's borderless full screen). Before,
//! in and after full screen it prints:
//! - `STRIP <when> height=<pt> alpha=<a> visible=<bool>`: each toolbar
//!   full-screen window;
//! - `WINDOW <when> fullscreen=<bool> frame_h=<pt> layout_h=<pt>
//!   buttons_hidden=<bool> toolbar_visible=<bool>`: the window, its content
//!   layout rect's height (the window's whole height when nothing covers
//!   its top) and its traffic lights.
//!
//! Then `FULLSCREEN_TOOLBAR_DONE`.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx_desktop::{Application, Titlebar, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let green = args
        .iter()
        .position(|arg| arg == "--via")
        .and_then(|i| args.get(i + 1))
        .is_none_or(|via| via == "green");
    Application::new()
        .window(WindowOptions {
            title: "Fullscreen toolbar".into(),
            width: 640.,
            height: 400.,
            titlebar: Titlebar::Inset,
            ..Default::default()
        })
        .run(move |cx| {
            cx.render(div().w_full().h_full().bg(rgba(0x3060a0ff)));
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let pause = |ms| zgx::timer::sleep(Duration::from_millis(ms));
                pause(1500).await;
                state::report("before");
                if green {
                    state::toggle();
                } else {
                    window.set_fullscreen(true);
                }
                pause(3000).await;
                state::report("full");
                if green {
                    state::toggle();
                } else {
                    window.set_fullscreen(false);
                }
                pause(3000).await;
                state::report("after");
                println!("FULLSCREEN_TOOLBAR_DONE");
                window.close();
            });
        })
}

#[cfg(target_os = "macos")]
mod state {
    use objc2::{MainThreadMarker, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSApplication, NSWindow, NSWindowStyleMask};
    use objc2_foundation::NSString;

    fn ours() -> Option<Retained<NSWindow>> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
        app.windows()
            .iter()
            .find(|window| window.title().to_string() == "Fullscreen toolbar")
    }

    /// What the green button does.
    pub fn toggle() {
        if let Some(window) = ours() {
            window.toggleFullScreen(None);
        } else {
            println!("NO_WINDOW");
        }
    }

    pub fn report(when: &str) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        for window in app.windows().iter() {
            let class = window.class().name().to_string_lossy().into_owned();
            if class.contains("ToolbarFullScreenWindow") {
                println!(
                    "STRIP {when} height={:.0} alpha={:.2} visible={}",
                    window.frame().size.height,
                    window.alphaValue(),
                    window.isVisible()
                );
            }
        }
        let Some(window) = ours() else {
            println!("NO_WINDOW");
            return;
        };
        // SAFETY: `-standardWindowButton:` takes an NSUInteger and returns
        // a view or nil, retained here; `-toolbar` the NSToolbar or nil;
        // `-isVisible` a BOOL.
        let (buttons_hidden, toolbar_visible) = unsafe {
            let close: Option<Retained<AnyObject>> =
                msg_send![&*window, standardWindowButton: 0_usize];
            let hidden = close.is_none_or(|button| {
                let hidden: bool = msg_send![&*button, isHidden];
                hidden
            });
            let toolbar: Option<Retained<AnyObject>> = msg_send![&*window, toolbar];
            let visible = toolbar.is_some_and(|toolbar| {
                let id: Option<Retained<NSString>> = msg_send![&*toolbar, identifier];
                let visible: bool = msg_send![&*toolbar, isVisible];
                id.is_some() && visible
            });
            (hidden, visible)
        };
        println!(
            "WINDOW {when} fullscreen={} frame_h={:.0} layout_h={:.0} buttons_hidden={buttons_hidden} toolbar_visible={toolbar_visible}",
            window.styleMask().contains(NSWindowStyleMask::FullScreen),
            window.frame().size.height,
            window.contentLayoutRect().size.height,
        );
    }
}

#[cfg(not(target_os = "macos"))]
mod state {
    pub fn toggle() {}
    pub fn report(_: &str) {
        println!("SKIP macOS only");
    }
}
