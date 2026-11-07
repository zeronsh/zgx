//! A double click on a `window_drag` region zooming the window (macOS): each
//! size the animation passes through reaches the app while the window has it,
//! so a frame of that size can show with it. Reported after the animation, the
//! window's last frame stays stretched over it meanwhile. A synthetic AppKit
//! press posted to this process's own window (the user's pointer is never
//! moved, nothing outside the process is touched). Prints one line for
//! `scripts/macos_double_click_zoom_probe.py`.
use zgx::compose::prelude::*;
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "Double click zoom probe".into(),
            width: 640.,
            height: 420.,
            bounds: Some(zgx_desktop::WindowBounds::new(1280, 840).at(200, 200)),
            ..Default::default()
        })
        .run(|cx| {
            cx.render(div().w_full().h_full().window_drag());
            #[cfg(target_os = "macos")]
            {
                let steps = macos::Steps::default();
                let viewport = cx.viewport.clone();
                let record = steps.clone();
                let trace = cx
                    .ui
                    .runtime
                    .effect(move || macos::record(&record, viewport.get()));
                let window = cx.window.clone();
                cx.tasks.spawn(async move {
                    macos::run(steps).await;
                    drop(trace);
                    window.close();
                });
            }
            #[cfg(not(target_os = "macos"))]
            {
                println!("macOS only");
                cx.window.close();
            }
        })
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
    use objc2_foundation::NSPoint;
    use std::{cell::RefCell, rc::Rc, time::Duration};
    use zgx_gpu::present_log::now_ns;

    /// What the app was told, what the window had then (points), and when.
    pub type Steps = Rc<RefCell<Vec<((f32, f32), (f64, f64), u64)>>>;

    fn window() -> Option<Retained<NSWindow>> {
        let mtm = MainThreadMarker::new()?;
        NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .find(|window| window.title().to_string() == "Double click zoom probe")
    }
    fn content(window: &NSWindow) -> (f64, f64) {
        let size = window.contentView().unwrap().frame().size;
        (size.width, size.height)
    }
    pub fn record(steps: &Steps, viewport: (f32, f32)) {
        if let Some(window) = window() {
            steps
                .borrow_mut()
                .push((viewport, content(&window), now_ns()));
        }
    }
    fn post(window: &NSWindow, kind: NSEventType, pressure: f32) {
        let height = content(window).1;
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind,
            NSPoint::new(320., height - 200.),
            NSEventModifierFlags::empty(),
            now_ns() as f64 / 1e9,
            window.windowNumber(),
            None,
            0,
            2,
            pressure,
        )
        .unwrap();
        let mtm = MainThreadMarker::new().unwrap();
        NSApplication::sharedApplication(mtm).postEvent_atStart(&event, false);
    }
    pub async fn run(steps: Steps) {
        zgx::timer::sleep(Duration::from_millis(3000)).await;
        let window = window().expect("probe window");
        window.makeKeyAndOrderFront(None);
        zgx::timer::sleep(Duration::from_millis(300)).await;
        let from = content(&window);
        let pressed = now_ns();
        steps.borrow_mut().clear();
        post(&window, NSEventType::LeftMouseDown, 1.);
        post(&window, NSEventType::LeftMouseUp, 0.);
        zgx::timer::sleep(Duration::from_millis(1500)).await;
        let to = content(&window);
        let steps = steps.borrow();
        let stale = steps
            .iter()
            .filter(|((width, height), (shown_width, shown_height), _)| {
                (f64::from(*width) - shown_width).abs() > 1.
                    || (f64::from(*height) - shown_height).abs() > 1.
            })
            .count();
        let ms = |at: u64| at.saturating_sub(pressed) as f64 / 1e6;
        println!(
            "ZOOM steps={} stale={stale} grew={} first_ms={:.1} last_ms={:.1}",
            steps.len(),
            to.0 > from.0 && to.1 > from.1,
            steps.first().map_or(0., |step| ms(step.2)),
            steps.last().map_or(0., |step| ms(step.2)),
        );
    }
}
