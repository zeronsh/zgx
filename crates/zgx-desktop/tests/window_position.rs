//! macOS, on the main thread (`fn main`): a window asked for at a position
//! is made there, on whichever screen that is. AppKit reads a new window's
//! content rect relative to the key window's screen unless it is given
//! one, and moved a window asked for on another screen onto that one, its
//! height mirrored: a window restored to a second display opened on the
//! first, far from where it was. The windows here are never shown (AppKit
//! placed them wrong as they were made); with one screen this checks only
//! that one, with two or more at least one is not the key window's.
fn main() {
    #[cfg(target_os = "macos")]
    {
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
                            "asked for {asked:?} on the screen at {at:?} ({size:?}): made at {got:?}"
                        ));
                    }
                    println!("window_position: {asked:?} on the screen at {at:?}: made at {got:?}");
                }
                event_loop.exit();
            }
            fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
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
