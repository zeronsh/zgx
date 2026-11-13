//! An event loop asked to exit as it starts ends at once (Windows, zgx's
//! winit). Winit delivers `resumed` as the loop first goes idle, then waited
//! for a message before reading the exit: where none came (an idle desktop),
//! it never returned. A process of its own: winit makes one loop a process.

/// Exits the process if `done` isn't set within `limit`: a loop that never
/// wakes fails the run rather than hanging it.
#[cfg(target_os = "windows")]
fn watchdog(done: std::sync::Arc<std::sync::atomic::AtomicBool>, limit: std::time::Duration) {
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        while !done.load(std::sync::atomic::Ordering::Acquire) {
            if start.elapsed() >= limit {
                eprintln!("the event loop asked to exit didn't return in {limit:?}");
                std::process::exit(101);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
}

#[cfg(target_os = "windows")]
#[test]
fn an_exit_asked_for_as_the_loop_starts_ends_it_without_a_message() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use winit::{
        application::ApplicationHandler,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop},
        platform::windows::EventLoopBuilderExtWindows,
        window::WindowId,
    };
    #[derive(Default)]
    struct ExitAtOnce {
        resumed: bool,
    }
    impl ApplicationHandler for ExitAtOnce {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.resumed = true;
            event_loop.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let done = Arc::new(AtomicBool::new(false));
    watchdog(done.clone(), std::time::Duration::from_secs(20));
    let event_loop = EventLoop::builder().with_any_thread(true).build().unwrap();
    let mut app = ExitAtOnce::default();
    event_loop.run_app(&mut app).unwrap();
    done.store(true, Ordering::Release);
    assert!(app.resumed);
}
