//! An inset title bar's traffic lights placed by the app (macOS,
//! `WindowOptions::traffic_lights`): the close button's top left at
//! `--at X,Y` content points (18,22 unless given), beside a mark in a
//! 60-point title bar row. `scripts/macos_traffic_lights_probe.py` runs it
//! and checks the lines.
//!
//! It resizes the window, gives it a dark and then a light appearance,
//! hides and shows the app (the window losing focus and getting it back),
//! zooms it to 125% and back, and goes into full screen and out (as the
//! green button does), printing after each
//! `CLOSE <when> x=<pt> y=<pt> h=<pt> hidden=<bool> zoom=<z>
//! controls=<x,y,w,h or none> area_h=<pt or none>`: the close button's top
//! left in the window's points from its top left, as AppKit has it, and
//! what the window says of its buttons and title bar strip
//! (`WindowInfo::controls`, `titlebar_area`, in the content's points).
//! `WINDOW <number>` first says which it is (its `CGWindowID`, for
//! `screencapture -l`). `--hold` keeps the window open at the end, for
//! looking at it. Then `TRAFFIC_LIGHTS_DONE`.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx_desktop::{Application, Titlebar, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let at = args
        .iter()
        .position(|arg| arg == "--at")
        .and_then(|i| args.get(i + 1))
        .and_then(|at| {
            let (x, y) = at.split_once(',')?;
            Some((x.parse().ok()?, y.parse().ok()?))
        })
        .unwrap_or((18., 22.));
    let hold = args.iter().any(|arg| arg == "--hold");
    Application::new()
        .window(WindowOptions {
            title: "Traffic lights".into(),
            width: 640.,
            height: 400.,
            titlebar: Titlebar::Inset,
            traffic_lights: Some(at),
            ..Default::default()
        })
        .run(move |cx| {
            cx.render(
                column().w_full().h_full().bg(rgba(0x3060a0ff)).child(
                    row()
                        .w_full()
                        .h(60.)
                        .shrink_0()
                        .items_center()
                        .pl(90.)
                        .bg(rgba(0x204070ff))
                        .window_drag()
                        .child(div().w(28.).h(28.).rounded(7.).bg(rgb(0xf0c040))),
                ),
            );
            let (window, info, ui) = (cx.window.clone(), cx.window_info.clone(), cx.ui.downgrade());
            cx.tasks.spawn(async move {
                let pause = |ms| zgx::timer::sleep(Duration::from_millis(ms));
                let report = |when: &str| {
                    let zoom = ui.upgrade().map_or(1., |ui| ui.zoom());
                    let rect = |r: Option<zgx::scene::Rect>| {
                        r.map_or("none".to_owned(), |r| {
                            format!("{:.1},{:.1},{:.1},{:.1}", r.x, r.y, r.width, r.height)
                        })
                    };
                    let area_h = info
                        .titlebar_area
                        .with_untracked(|r| *r)
                        .map_or("none".to_owned(), |r| format!("{:.1}", r.height));
                    println!(
                        "CLOSE {when} {} zoom={zoom:.2} controls={} area_h={area_h}",
                        state::close(),
                        rect(info.controls.with_untracked(|r| *r)),
                    );
                };
                pause(1500).await;
                println!("WINDOW {}", state::number());
                report("before");
                window.set_inner_size(720., 460.);
                pause(800).await;
                report("resized");
                window.set_theme(Some(zgx::theme::Appearance::Dark));
                pause(600).await;
                report("dark");
                window.set_theme(Some(zgx::theme::Appearance::Light));
                pause(600).await;
                report("light");
                window.set_theme(None);
                state::hide(true);
                pause(800).await;
                state::hide(false);
                pause(800).await;
                report("refocused");
                if let Some(ui) = ui.upgrade() {
                    ui.set_zoom(1.25);
                }
                pause(800).await;
                report("zoomed");
                if let Some(ui) = ui.upgrade() {
                    ui.set_zoom(1.);
                }
                pause(800).await;
                report("unzoomed");
                state::toggle();
                pause(3000).await;
                report("full");
                state::toggle();
                pause(3000).await;
                report("after");
                println!("TRAFFIC_LIGHTS_DONE");
                if !hold {
                    window.close();
                }
            });
        })
}

#[cfg(target_os = "macos")]
mod state {
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSApplication, NSWindow, NSWindowButton};

    fn ours() -> Option<Retained<NSWindow>> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
        app.windows()
            .iter()
            .find(|window| window.title().to_string() == "Traffic lights")
    }

    /// The window's number (its `CGWindowID`).
    pub fn number() -> isize {
        ours().map_or(0, |window| window.windowNumber())
    }

    /// The close button's top left below and right of the window's top
    /// left, its height, and whether it is hidden.
    pub fn close() -> String {
        let Some(window) = ours() else {
            return "x=none y=none h=none hidden=none".into();
        };
        let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
            return "x=none y=none h=none hidden=none".into();
        };
        let frame = close.convertRect_toView(close.bounds(), None);
        let top = window.frame().size.height - (frame.origin.y + frame.size.height);
        format!(
            "x={:.1} y={:.1} h={:.1} hidden={}",
            frame.origin.x,
            top,
            frame.size.height,
            close.isHidden()
        )
    }

    /// The app hidden, or shown again: its window resigns key and becomes
    /// key again.
    pub fn hide(hide: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        if hide {
            app.hide(None);
        } else {
            app.unhide(None);
            if let Some(window) = ours() {
                window.makeKeyAndOrderFront(None);
            }
        }
    }

    /// What the green button does.
    pub fn toggle() {
        if let Some(window) = ours() {
            window.toggleFullScreen(None);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod state {
    pub fn close() -> String {
        "x=none y=none h=none hidden=none".into()
    }
    pub fn number() -> isize {
        0
    }
    pub fn hide(_: bool) {}
    pub fn toggle() {}
}
