//! macOS, on the main thread (`fn main`): a window asked for at a position
//! is made there, on whichever screen that is. AppKit reads a new window's
//! content rect relative to the key window's screen unless it is given
//! one, and moved a window asked for on another screen onto that one, its
//! height mirrored: a window restored to a second display opened on the
//! first, far from where it was. The windows here are never shown (AppKit
//! placed them wrong as they were made), but the process is a foreground
//! app while it runs, with a Dock icon. With one screen this checks only
//! that one; with two or more, at least one is not the key window's. A
//! failure names the screens as AppKit and Core Graphics have them.
//! `screen_placement` checks the arithmetic for other arrangements.
//!
//! It makes its windows straight from winit, so `ZGX_OFFSCREEN` cannot hide
//! them: with it set the test is skipped, and puts nothing on the screen.
fn main() {
    let offscreen = std::env::var_os("ZGX_OFFSCREEN").is_some_and(|v| !v.is_empty() && v != "0");
    if offscreen {
        println!("window_position: skipped, ZGX_OFFSCREEN is set (its windows are real ones)");
    }
    #[cfg(target_os = "macos")]
    if !offscreen {
        use winit::{
            application::ApplicationHandler,
            dpi::{PhysicalPosition, PhysicalSize},
            event::WindowEvent,
            event_loop::{ActiveEventLoop, EventLoop},
            window::{Window, WindowId},
        };
        struct Placing(Vec<String>);
        impl ApplicationHandler for Placing {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                for monitor in event_loop.available_monitors() {
                    // A quarter in from the screen's top left.
                    let (at, size) = (monitor.position(), monitor.size());
                    let asked = PhysicalPosition::new(
                        at.x + size.width as i32 / 4,
                        at.y + size.height as i32 / 4,
                    );
                    let window = event_loop
                        .create_window(
                            Window::default_attributes()
                                .with_visible(false)
                                .with_position(asked)
                                .with_inner_size(PhysicalSize::new(600, 400)),
                        )
                        .expect("a window");
                    let got = window.inner_position().expect("its position");
                    let scale = window.scale_factor();
                    // Within a point, as it rounds to points.
                    let near = |a: i32, b: i32| (a - b).abs() <= scale.ceil() as i32;
                    if !(near(got.x, asked.x) && near(got.y, asked.y)) {
                        self.0.push(format!(
                            "asked for {asked:?} on the screen at {at:?} ({size:?}): made at {got:?} \
                             (scale {scale}; {})",
                            screens()
                        ));
                    }
                    println!("window_position: {asked:?} on the screen at {at:?}: made at {got:?}");
                }
                event_loop.exit();
            }
            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
        }
        /// The screens as AppKit and Core Graphics have them, for a failure
        /// to say which arrangement misplaced it: winit's positions flip
        /// about the main display's height, AppKit's frames start at the
        /// first screen's bottom left, the key window's screen is `main`.
        fn screens() -> String {
            use objc2_app_kit::NSScreen;
            use objc2_core_foundation::CGRect;
            #[link(name = "CoreGraphics", kind = "framework")]
            unsafe extern "C" {
                fn CGMainDisplayID() -> u32;
                fn CGDisplayBounds(display: u32) -> CGRect;
            }
            let mtm = objc2::MainThreadMarker::new().expect("the main thread");
            let rect = |r: CGRect| {
                format!(
                    "({}, {}, {}×{})",
                    r.origin.x, r.origin.y, r.size.width, r.size.height
                )
            };
            let frames: Vec<String> = NSScreen::screens(mtm)
                .iter()
                .map(|screen| {
                    format!(
                        "{} visible {} at {}x",
                        rect(screen.frame()),
                        rect(screen.visibleFrame()),
                        screen.backingScaleFactor()
                    )
                })
                .collect();
            let main = NSScreen::mainScreen(mtm).map(|screen| rect(screen.frame()));
            // SAFETY: plain Core Graphics queries.
            let cg_main = unsafe { CGDisplayBounds(CGMainDisplayID()) };
            format!(
                "NSScreens {}; key window's screen {}; main display {}",
                frames.join(", "),
                main.unwrap_or_else(|| "none".into()),
                rect(cg_main)
            )
        }
        let mut placing = Placing(Vec::new());
        EventLoop::new()
            .expect("an event loop (not the main thread?)")
            .run_app(&mut placing)
            .expect("run");
        assert!(placing.0.is_empty(), "{}", placing.0.join("\n"));
        println!("window_position: each window was made where it was asked to be");
    }
}
