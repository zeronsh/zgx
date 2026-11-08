//! A standard Edit item (Copy), an ordinary one and the standard full screen
//! item, chosen as the rendered menu bar chooses them
//! (`WindowHandle::dispatch_menu_action`). Outside macOS the first two reach
//! the app's `on_menu_action` while nothing focused takes the edit, and full
//! screen toggles the window instead: `--smoke-test` chooses each, checks
//! the first two arrive in order, and that full screen goes in and out
//! without reaching the app.
use std::{cell::RefCell, rc::Rc, time::Duration};
use zgx::{compose::prelude::*, timer::sleep};
use zgx_desktop::{AppMenu, AppMenuEntry, Application, StandardItem};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let smoke = std::env::args().any(|arg| arg == "--smoke-test");
    let menus = vec![
        AppMenu::new(
            "Edit",
            [
                AppMenuEntry::Standard(StandardItem::Copy),
                AppMenuEntry::item("Control", "control"),
            ],
        ),
        AppMenu::new(
            "View",
            [AppMenuEntry::Standard(StandardItem::ToggleFullScreen)],
        ),
    ];
    Application::new().menus(menus)?.run(move |cx| {
        cx.window.set_title("zgx standard menu items");
        let heard = Rc::new(RefCell::new(Vec::<String>::new()));
        let log = heard.clone();
        cx.on_menu_action(move |action| {
            println!("MENU {}", action.0);
            log.borrow_mut().push(action.0);
        });
        let window = cx.window.clone();
        cx.render(
            column()
                .p(12.)
                .gap(8.)
                .child("Edit → Copy and Edit → Control reach the app.")
                .child(
                    button()
                        .child("Copy")
                        .on_click(move || window.dispatch_menu_action(StandardItem::Copy.action())),
                ),
        );
        if smoke {
            let window = cx.window.clone();
            let fullscreen = cx.window_info.fullscreen.clone();
            cx.tasks.spawn(async move {
                sleep(Duration::from_millis(500)).await;
                window.dispatch_menu_action(StandardItem::Copy.action());
                window.dispatch_menu_action("control");
                sleep(Duration::from_millis(500)).await;
                let got = heard.borrow().clone();
                let expected = if cfg!(target_os = "macos") {
                    // AppKit's own item there, sent to the first responder.
                    vec!["control".to_string()]
                } else {
                    vec!["standard:copy".to_string(), "control".to_string()]
                };
                if got != expected {
                    eprintln!(
                        "standard menu items smoke FAILED: heard {got:?}, expected {expected:?}"
                    );
                    std::process::exit(1);
                }
                // Full screen toggles the window, in and out, and never
                // reaches the app (on macOS AppKit's item, as Copy is).
                for want in [true, false]
                    .into_iter()
                    .filter(|_| !cfg!(target_os = "macos"))
                {
                    window.dispatch_menu_action(StandardItem::ToggleFullScreen.action());
                    let mut waited = 0;
                    while fullscreen.with_untracked(|on| *on) != want && waited < 5000 {
                        sleep(Duration::from_millis(50)).await;
                        waited += 50;
                    }
                    if fullscreen.with_untracked(|on| *on) != want {
                        eprintln!("standard menu items smoke FAILED: full screen not {want}");
                        std::process::exit(1);
                    }
                }
                let got = heard.borrow().clone();
                if got != expected {
                    eprintln!(
                        "standard menu items smoke FAILED: full screen reached the app: {got:?}"
                    );
                    std::process::exit(1);
                }
                println!("standard menu items smoke passed: {got:?}, full screen in and out");
                window.close();
            });
        }
    })?;
    Ok(())
}
