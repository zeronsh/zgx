//! A popup window under a zoomed window (macOS): `--zoom Z` zooms the
//! window, then opens a menu taller than it, which shows in a window of its
//! own. It prints where that window is, against the content's top-left in
//! window points, beside where the scene puts the menu times the zoom:
//! `PLACED got=x,y,w,h want=x,y,w,h`. Then a click is posted to the
//! popup window's second item, in the app's own queue (nothing else hears
//! it): `CLICKED <item>`. Then `POPUP_ZOOM_DONE`.
//! `scripts/macos_popup_zoom_probe.py` runs it at 90% and 125%.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let zoom: f32 = args
        .iter()
        .position(|arg| arg == "--zoom")
        .and_then(|i| args.get(i + 1))
        .and_then(|z| z.parse().ok())
        .unwrap_or(0.9);
    Application::new()
        .window(WindowOptions {
            title: "Popup zoom".into(),
            width: 420.,
            height: 200.,
            ..Default::default()
        })
        .run(move |cx| {
            cx.ui.set_zoom(zoom as f64);
            let open = cx.ui.signal(false);
            let items = [
                "First", "Second", "Third", "Fourth", "Fifth", "Sixth", "Seventh",
            ];
            let view = cx.render(
                column().p(20.).child(
                    menu(
                        "Actions",
                        open.clone(),
                        button().size(100., 24.).child(text("Actions")),
                    )
                    .children(items.map(|item| {
                        menu_item(item)
                            .id(item)
                            .on_click(move || println!("CLICKED {item}"))
                    })),
                ),
            );
            let ui = cx.ui.downgrade();
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let pause = |ms| zgx::timer::sleep(Duration::from_millis(ms));
                pause(1500).await;
                open.set(true);
                pause(1000).await;
                let Some(ui) = ui.upgrade() else {
                    return;
                };
                let (bounds, second) = {
                    let scene = ui.scene.borrow();
                    let root = scene.popups().first().copied();
                    let bounds = root.and_then(|root| scene.popup_bounds(root));
                    let second = view.find("Second").map(|item| scene.bounds(item));
                    (bounds, second)
                };
                match (bounds, second) {
                    (Some(bounds), Some(second)) => {
                        native::report(bounds, zoom);
                        // The second item's middle, in the popup window's
                        // own points (from its top-left).
                        let local = (
                            (second.x + second.width / 2. - bounds.x) * zoom,
                            (second.y + second.height / 2. - bounds.y) * zoom,
                        );
                        native::click(local);
                    }
                    _ => println!("NO_POPUP"),
                }
                drop(ui);
                pause(800).await;
                println!("POPUP_ZOOM_DONE");
                window.close();
            });
        })
}

#[cfg(target_os = "macos")]
mod native {
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
    use objc2_foundation::{NSPoint, NSProcessInfo};
    use zgx::scene::Rect;

    fn windows() -> (Option<Retained<NSWindow>>, Option<Retained<NSWindow>>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return (None, None);
        };
        let all = NSApplication::sharedApplication(mtm).windows();
        let ours = all
            .iter()
            .find(|window| window.title().to_string() == "Popup zoom");
        let popup = all.iter().find(|window| {
            // zgx's popup view is its panel's content's subview.
            window.isVisible()
                && window.contentView().is_some_and(|content| {
                    content
                        .subviews()
                        .iter()
                        .any(|view| view.class().name().to_string_lossy() == "ZgxPopupView")
                })
        });
        (ours, popup)
    }

    pub fn report(bounds: Rect, zoom: f32) {
        let (Some(ours), Some(popup)) = windows() else {
            println!("NO_POPUP_WINDOW");
            return;
        };
        let Some(content) = ours.contentView() else {
            return;
        };
        let area = ours.convertRectToScreen(content.convertRect_toView(content.bounds(), None));
        let frame = popup.frame();
        // Against the content's top-left, top-down, in window points.
        let got = (
            frame.origin.x - area.origin.x,
            (area.origin.y + area.size.height) - (frame.origin.y + frame.size.height),
            frame.size.width,
            frame.size.height,
        );
        let want = (
            bounds.x * zoom,
            bounds.y * zoom,
            bounds.width * zoom,
            bounds.height * zoom,
        );
        println!(
            "PLACED got={:.1},{:.1},{:.1},{:.1} want={:.1},{:.1},{:.1},{:.1}",
            got.0, got.1, got.2, got.3, want.0, want.1, want.2, want.3
        );
    }

    /// A left click at `local` (the popup window's points from its
    /// top-left), posted to this app's own queue.
    pub fn click(local: (f32, f32)) {
        let (_, Some(popup)) = windows() else {
            return;
        };
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        // The window's own coordinates run bottom-up.
        let at = NSPoint::new(local.0 as f64, popup.frame().size.height - local.1 as f64);
        for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                at,
                NSEventModifierFlags::empty(),
                NSProcessInfo::processInfo().systemUptime(),
                popup.windowNumber(),
                None,
                0,
                1,
                1.,
            );
            if let Some(event) = event {
                app.postEvent_atStart(&event, false);
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub fn report(_: zgx::scene::Rect, _: f32) {
        println!("SKIP macOS only");
    }
    pub fn click(_: (f32, f32)) {}
}
