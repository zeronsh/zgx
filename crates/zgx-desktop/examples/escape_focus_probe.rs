//! Where focus goes, and whether its ring shows, as Escape closes a menu
//! shown in a popup window of its own (macOS): opened with nothing focused,
//! nothing is focused after it (not the window's first button); opened after
//! a button was clicked, that button has focus back without a ring; opened
//! and moved through from the keyboard, its trigger has focus with its ring.
//! Synthetic AppKit events posted to this process's own windows (the user's
//! pointer is never moved, nothing outside the process is touched). Prints
//! one line per step for `scripts/macos_escape_focus_probe.py`.
use zgx::compose::prelude::*;
use zgx_desktop::{Application, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "Escape focus probe".into(),
            width: 420.,
            height: 160.,
            bounds: Some(zgx_desktop::WindowBounds::new(840, 320).at(400, 300)),
            ..Default::default()
        })
        .run(|cx| {
            let open = cx.ui.signal(false);
            // The menu hangs past the window's bottom edge: in a popup
            // window of its own.
            let view = cx.render(
                column()
                    .w_full()
                    .h_full()
                    .p(16.)
                    .gap(8.)
                    .child(button().id("first").w(100.).h(24.).child(text("New chat")))
                    .child(button().id("other").w(100.).h(24.).child(text("Other")))
                    .child(
                        menu(
                            "Actions",
                            open.clone(),
                            button().id("anchor").w(100.).h(24.).child(text("Actions")),
                        )
                        .w(180.)
                        .h(220.)
                        .children([menu_item("Alpha").id("alpha"), menu_item("Beta")]),
                    ),
            );
            #[cfg(target_os = "macos")]
            {
                let window = cx.window.clone();
                let ui = cx.ui.downgrade();
                cx.tasks.spawn(async move {
                    macos::run(ui, view, open).await;
                    window.close();
                });
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (view, open);
                println!("macOS only");
                cx.window.close();
            }
        })
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
    use objc2_foundation::{NSPoint, NSString};
    use std::time::Duration;
    use zgx::{compose::ViewHandle, reactive::Signal, scene::Rect, widgets::WeakUi};
    use zgx_gpu::present_log::now_ns;

    async fn sleep(ms: u64) {
        zgx::timer::sleep(Duration::from_millis(ms)).await;
    }
    /// The app's own window and how many popup windows show.
    fn windows() -> (Retained<NSWindow>, usize) {
        let mtm = MainThreadMarker::new().unwrap();
        let mut main = None;
        let mut popups = 0;
        for window in NSApplication::sharedApplication(mtm).windows().iter() {
            if !window.isVisible() {
                continue;
            }
            if window.title().to_string() == "Escape focus probe" {
                main = Some(window);
            } else {
                popups += 1;
            }
        }
        (main.expect("probe window"), popups)
    }
    fn post(event: Retained<NSEvent>) {
        let mtm = MainThreadMarker::new().unwrap();
        NSApplication::sharedApplication(mtm).postEvent_atStart(&event, false);
    }
    /// A click at `(x, y)` in the main window's content (top-down).
    async fn click(main: &NSWindow, (x, y): (f64, f64)) {
        let height = main.contentView().unwrap().frame().size.height;
        for kind in [
            NSEventType::MouseMoved,
            NSEventType::LeftMouseDown,
            NSEventType::LeftMouseUp,
        ] {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                NSPoint::new(x, height - y),
                NSEventModifierFlags::empty(),
                now_ns() as f64 / 1e9,
                main.windowNumber(),
                None,
                0,
                1,
                if kind == NSEventType::LeftMouseDown { 1. } else { 0. },
            )
            .unwrap();
            post(event);
            sleep(60).await;
        }
        sleep(300).await;
    }
    /// A key pressed and released in the main window (the key window, which
    /// the popup's content takes keys through).
    async fn key(main: &NSWindow, characters: &str, code: u16) {
        let characters = NSString::from_str(characters);
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::new(0., 0.),
                NSEventModifierFlags::empty(),
                now_ns() as f64 / 1e9,
                main.windowNumber(),
                None,
                &characters,
                &characters,
                false,
                code,
            )
            .unwrap();
            post(event);
            sleep(40).await;
        }
        sleep(300).await;
    }
    fn center(rect: Rect) -> (f64, f64) {
        (
            (rect.x + rect.width / 2.) as f64,
            (rect.y + rect.height / 2.) as f64,
        )
    }
    pub async fn run(ui: WeakUi, view: ViewHandle, open: Signal<bool>) {
        sleep(3000).await;
        let bounds = |id: &str| {
            let ui = ui.upgrade().unwrap();
            ui.scene.borrow().bounds(view.find(id).unwrap())
        };
        let report = |step: &str, popups: usize| {
            let ui = ui.upgrade().unwrap();
            let focused = ui.input.focused().map_or("none".to_owned(), |node| {
                ["first", "other", "anchor", "alpha"]
                    .into_iter()
                    .find(|id| view.find(id) == Some(node))
                    .unwrap_or("another")
                    .to_owned()
            });
            println!(
                "{step} open={} popups={popups} focused={focused} ring={}",
                open.get(),
                ui.input.focused().is_some() && ui.input.focus_visible_now(),
            );
        };
        let (main, _) = windows();
        main.makeKeyAndOrderFront(None);
        sleep(300).await;
        let content = main.contentView().unwrap().frame().size;
        let blank = (content.width - 20., 20.);
        // Nothing focused (a click on blank space), the menu opened as a
        // shortcut would: Escape leaves nothing focused.
        click(&main, blank).await;
        open.set(true);
        sleep(500).await;
        report("OPEN_BLANK", windows().1);
        key(&main, "\u{1b}", 53).await;
        report("ESCAPE_BLANK", windows().1);
        // A clicked button, then the menu: Escape gives it focus back,
        // without a ring.
        click(&main, center(bounds("other"))).await;
        open.set(true);
        sleep(500).await;
        key(&main, "\u{1b}", 53).await;
        report("ESCAPE_CLICKED", windows().1);
        // From the keyboard: Tab to the trigger, Return, an arrow, Escape.
        key(&main, "\t", 48).await;
        key(&main, "\r", 36).await;
        key(&main, "\u{f701}", 125).await;
        report("KEYBOARD_OPEN", windows().1);
        key(&main, "\u{1b}", 53).await;
        report("ESCAPE_KEYBOARD", windows().1);
    }
}
