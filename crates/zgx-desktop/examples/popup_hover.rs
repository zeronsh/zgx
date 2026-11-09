//! A tooltip and a hover card, for `scripts/macos_popup_hover` to hover from
//! outside with another app in front: does hovering a window at the back
//! open them, activate the app, make a window key, or raise the window?
//!
//! Besides zgx's own tooltip paths, `row` opens a menu and its submenu from the
//! app's hover state, and `overlay` shows a box in the window from it.
//!
//! `POPUP_HOVER_ACT` (move, resize, mini, full, close, space) does that to the
//! window, or for space puts its other window in full screen, a second after a
//! popup shows, for the probe to read what becomes of the popup.
//!
//! Prints `AT name x y` (the window's content, in points) for what to hover,
//! `HOVER name enter|leave` as the app hears it, and on macOS `TARGET name x
//! y` (the screen's points, from its top left) and a `STATE` line each time the app's state changes: whether it is active,
//! and each window's class, number, level, parent, and whether it is key and
//! visible.
use zgx::{
    compose::{Tooltip, prelude::*},
    input::InputEvent,
};
use zgx_desktop::{Application, WindowOptions};

/// `view`, saying as the pointer enters and leaves, and running `then` with
/// whether it is in.
fn heard(view: View, name: &'static str, then: impl Fn(bool) + 'static) -> View {
    view.on_event(move |cx| {
        if cx.target != cx.current_target {
            return;
        }
        let inside = match cx.event {
            InputEvent::PointerEnter => true,
            InputEvent::PointerLeave => false,
            _ => return,
        };
        println!("HOVER {name} {}", if inside { "enter" } else { "leave" });
        then(inside);
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    Application::new()
        .window(WindowOptions {
            title: "Popup hover".into(),
            width: 420.,
            height: 300.,
            // `POPUP_HOVER_AT=x,y` in physical pixels: where the probe wants it.
            bounds: std::env::var("POPUP_HOVER_AT").ok().and_then(|at| {
                let (x, y) = at.split_once(',')?;
                let (x, y) = (x.parse().ok()?, y.parse().ok()?);
                Some(zgx_desktop::WindowBounds::new(840, 600).at(x, y))
            }),
            ..Default::default()
        })
        .run(|cx| {
            let (menu_open, sub_open) = (cx.ui.signal(false), cx.ui.signal(false));
            let shown = cx.ui.signal(false);
            let row = heard(
                div()
                    .id("row")
                    .size(120., 28.)
                    .bg(rgb(0x445566))
                    .child(text("Menu row")),
                "row",
                {
                    let (menu_open, sub_open) = (menu_open.clone(), sub_open.clone());
                    move |inside| {
                        if inside {
                            menu_open.set(true);
                            sub_open.set(true);
                        }
                    }
                },
            );
            let overlay_shown = shown.clone();
            let view = cx.render(
                column()
                    .p(16.)
                    .gap(12.)
                    .child(
                        heard(
                            div()
                                .id("tip")
                                .size(120., 28.)
                                .bg(rgb(0x445566))
                                .child(text("Tooltip")),
                            "tip",
                            |_| {},
                        )
                        .tooltip("A tooltip"),
                    )
                    .child(
                        heard(
                            div()
                                .id("card")
                                .size(120., 28.)
                                .bg(rgb(0x445566))
                                .child(text("Hover card")),
                            "card",
                            |_| {},
                        )
                        .tooltip(
                            Tooltip::from(
                                column()
                                    .size(160., 80.)
                                    .bg(rgb(0x4a3768))
                                    .child(text("Card")),
                            )
                            .interactive(true),
                        ),
                    )
                    .child(
                        menu("Row menu", menu_open, row)
                            .w(180.)
                            .child(menu_item("Item").h(28.))
                            .child(
                                submenu("More", sub_open)
                                    .w(180.)
                                    .child(menu_item("Sub item").h(28.)),
                            ),
                    )
                    .child(heard(
                        div()
                            .id("overlay")
                            .size(120., 28.)
                            .bg(rgb(0x445566))
                            .child(text("Overlay")),
                        "overlay",
                        move |inside| {
                            shown.set(inside);
                        },
                    ))
                    .child(div().size(120., 28.).reactive_style(move || {
                        if overlay_shown.get() {
                            Styles::new().bg(rgb(0xee3322))
                        } else {
                            Styles::new()
                        }
                    })),
            );
            // `POPUP_HOVER_SECOND`: another of its windows, active, over the
            // first's lower part: a tooltip in a window not active shows, and
            // showing raises nothing over it.
            if std::env::var_os("POPUP_HOVER_SECOND").is_some() {
                let at = std::env::var("POPUP_HOVER_AT").ok().and_then(|at| {
                    let (x, y) = at.split_once(',')?;
                    Some((x.parse::<i32>().ok()?, y.parse::<i32>().ok()?))
                });
                let bounds =
                    at.map(|(x, y)| zgx_desktop::WindowBounds::new(840, 400).at(x - 40, y + 300));
                cx.windows.open(
                    WindowOptions {
                        title: "Popup hover, second".into(),
                        width: 420.,
                        height: 200.,
                        bounds,
                        ..Default::default()
                    },
                    |cx| {
                        cx.render(column().size(420., 200.).bg(rgb(0x204060)));
                    },
                );
            }
            let ui = cx.ui.downgrade();
            cx.tasks.spawn(async move {
                zgx::timer::sleep(std::time::Duration::from_millis(500)).await;
                let Some(ui) = ui.upgrade() else {
                    return;
                };
                #[cfg(target_os = "macos")]
                appkit::move_across();
                for name in ["tip", "card", "row", "overlay"] {
                    if let Some(node) = view.find(name) {
                        let b = ui.scene.borrow().bounds(node);
                        let (x, y) = (b.x + b.width / 2., b.y + b.height / 2.);
                        println!("AT {name} {x} {y}");
                        #[cfg(target_os = "macos")]
                        {
                            let (x, y) = appkit::on_screen(x, y);
                            println!("TARGET {name} {x} {y}");
                        }
                    }
                }
                println!("READY");
                drop(ui);
                #[cfg(target_os = "macos")]
                {
                    let act = std::env::var("POPUP_HOVER_ACT").ok();
                    let (mut last, mut seen, mut acted) = (String::new(), 0, false);
                    loop {
                        zgx::timer::sleep(std::time::Duration::from_millis(100)).await;
                        let now = appkit::state();
                        if now != last {
                            println!("STATE {now}");
                            last = now;
                        }
                        if appkit::panel_showing() {
                            seen += 1;
                        }
                        if let Some(act) = act.as_deref().filter(|_| seen >= 10 && !acted) {
                            acted = true;
                            println!("ACT {act}");
                            appkit::act(act);
                        }
                    }
                }
            });
        })
}

#[cfg(target_os = "macos")]
mod appkit {
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSApplication, NSScreen, NSWindow};

    fn main_window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
        let windows = NSApplication::sharedApplication(mtm).windows();
        windows
            .iter()
            .find(|w| w.parentWindow().is_none() && w.level() == 0)
    }
    /// A point in the window's content as the screen's, from its top left.
    pub fn on_screen(x: f32, y: f32) -> (f64, f64) {
        let mtm = MainThreadMarker::new().unwrap();
        let window = main_window(mtm).unwrap();
        let content = window.contentRectForFrameRect(window.frame());
        let height = NSScreen::screens(mtm)
            .firstObject()
            .unwrap()
            .frame()
            .size
            .height;
        (
            content.origin.x + x as f64,
            height - (content.origin.y + content.size.height) + y as f64,
        )
    }
    /// `POPUP_HOVER_X`: its windows moved that many points, to another display.
    pub fn move_across() {
        let Some(dx) = std::env::var("POPUP_HOVER_X")
            .ok()
            .and_then(|x| x.parse::<f64>().ok())
        else {
            return;
        };
        let mtm = MainThreadMarker::new().unwrap();
        for w in NSApplication::sharedApplication(mtm).windows().iter() {
            if w.isVisible() {
                let at = w.frame().origin;
                w.setFrameOrigin(objc2_foundation::NSPoint::new(at.x + dx, at.y));
            }
        }
    }
    pub fn panel_showing() -> bool {
        let mtm = MainThreadMarker::new().unwrap();
        NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .any(|w| w.level() > 0 && w.isVisible())
    }
    pub fn act(what: &str) {
        let mtm = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(mtm);
        let window = main_window(mtm).unwrap();
        match what {
            "move" => {
                let at = window.frame().origin;
                window.setFrameOrigin(objc2_foundation::NSPoint::new(at.x + 60., at.y + 40.));
            }
            "resize" => {
                let mut frame = window.frame();
                frame.size.width += 80.;
                frame.size.height += 60.;
                window.setFrame_display(frame, true);
            }
            "mini" => window.miniaturize(None),
            "full" => window.toggleFullScreen(None),
            "close" => window.performClose(None),
            "space" => {
                let other = app.windows().iter().find(|w| {
                    w.level() == 0
                        && w.parentWindow().is_none()
                        && w.isVisible()
                        && w.windowNumber() != window.windowNumber()
                });
                if let Some(other) = other {
                    other.toggleFullScreen(None);
                }
            }
            _ => {}
        }
    }
    pub fn state() -> String {
        let mtm = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(mtm);
        let mut line = format!("active={}", app.isActive());
        for w in app.windows().iter() {
            let parent = w.parentWindow().map_or(0, |p| p.windowNumber());
            let at = w.frame().origin;
            line += &format!(
                " | {} #{} level={} parent={} key={} visible={} at={},{}",
                w.class().name().to_string_lossy(),
                w.windowNumber(),
                w.level(),
                parent,
                w.isKeyWindow(),
                w.isVisible(),
                at.x,
                at.y,
            );
        }
        line
    }
}
