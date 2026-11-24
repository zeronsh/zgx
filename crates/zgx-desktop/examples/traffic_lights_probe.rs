//! An inset title bar's traffic lights placed by the app (macOS,
//! `WindowOptions::traffic_lights`): the close button's top left at
//! `--at X,Y` content points (18,22 unless given), beside a mark in a
//! 60-point title bar row. `scripts/macos_traffic_lights_probe.py` runs it
//! and checks the lines.
//!
//! It resizes the window, gives it a dark and then a light appearance,
//! hides and shows the app (the window losing focus and getting it back),
//! zooms it to fill the screen (`zoom:`, as a double click on the title
//! bar does) and back,
//! moves the close button off its place (as AppKit's layout does) and reads it
//! back at once, zooms it to 125% and back, and goes into full screen and out (as the
//! green button does), printing after each
//! `CLOSE <when> x=<pt> y=<pt> h=<pt> hidden=<bool> zoom=<z> buttons=<x:y,x:y,x:y>
//! controls=<x,y,w,h or none> area_h=<pt or none>`: the close button's top
//! left in the window's points from its top left, as AppKit has it (and
//! each of the three buttons'), and
//! what the window says of its buttons and title bar strip
//! (`WindowInfo::controls`, `titlebar_area`, in the content's points).
//! `WINDOW <number>` first says which it is (its `CGWindowID`, for
//! `screencapture -l`). `--hold` keeps the window open at the end, for
//! looking at it. Then `TRAFFIC_LIGHTS_DONE`.
//!
//! After `resized` it resizes the window in 90 steps while counting, after each
//! frame Core Animation commits, the frames where the close button was not at
//! `--at`, and those where the three buttons were not in one row, evenly apart:
//! `LIVE_RESIZE samples=<n> wrong=<n> first_wrong=<x,y> row_wrong=<n>
//! first_row_wrong=<x:y,x:y,x:y>`. Then `BURST`: 30 `setFrame:display:`
//! back to back, the run loop never turning between them, the buttons read
//! after each (`steps=<n> row_wrong=<n> first=<…>`), and `TRACKING`: 30 steps
//! in the event-tracking run loop mode a live resize's drag runs in, a loop
//! pass each, read after each pass (`… in_live_resize=<bool>`).
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
    let busy = args.iter().any(|arg| arg == "--busy");
    let zoom_arg: f64 = args
        .iter()
        .position(|arg| arg == "--zoom")
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(1.);
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
            if busy {
                // A main thread kept busy, as a heavy app's is: the resize
                // events wait behind it, while AppKit lays out and draws.
                cx.tasks.spawn(async {
                    loop {
                        zgx::timer::sleep(Duration::from_millis(4)).await;
                        let until = std::time::Instant::now() + Duration::from_millis(12);
                        while std::time::Instant::now() < until {
                            std::hint::spin_loop();
                        }
                    }
                });
            }
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
                // AppKit's layout moving the button: its frame change must put it back at once.
                state::nudge();
                report("nudged");
                if let Some(ui) = ui.upgrade() {
                    ui.set_zoom(zoom_arg);
                }
                pause(800).await;
                let at = (at.0 * zoom_arg, (at.1 + 7.) * zoom_arg - 7.);
                state::watch_frames(at);
                for step in 0..90 {
                    let grow = if step < 45 { step } else { 90 - step } as f64;
                    window.set_inner_size(640. + grow * 4., 400. + grow * 2.);
                    pause(16).await;
                }
                pause(200).await;
                println!("{}", state::watched());
                println!("BURST {}", state::burst(30));
                pause(300).await;
                println!("TRACKING {}", state::tracking(30));
                pause(300).await;
                state::watch_frames(at);
                for size in [(1000., 700.), (560., 360.), (900., 600.)] {
                    state::animate_resize(size);
                    pause(300).await;
                }
                println!("ANIMATED {}", state::watched());
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
                state::fill_screen();
                pause(1200).await;
                report("filled");
                state::fill_screen();
                pause(1200).await;
                report("unfilled");
                state::relayout_titlebar();
                pause(600).await;
                report("relaid");
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
                if hold {
                    state::reset_counts();
                    // Time to drag the window's edge by hand: the counts keep running.
                    for _ in 0..90 {
                        pause(1000).await;
                        println!("HELD {}", state::watched());
                    }
                }
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
    use std::{
        ffi::c_void,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

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
        let height = window.frame().size.height;
        let buttons: Vec<String> = [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ]
        .into_iter()
        .filter_map(|kind| window.standardWindowButton(kind))
        .map(|button| {
            let frame = button.convertRect_toView(button.bounds(), None);
            let top = height - (frame.origin.y + frame.size.height);
            format!("{:.1}:{top:.1}", frame.origin.x)
        })
        .collect();
        format!(
            "x={:.1} y={:.1} h={:.1} hidden={} buttons={} window={:.0}x{:.0}",
            frame.origin.x,
            top,
            frame.size.height,
            close.isHidden(),
            buttons.join(","),
            window.frame().size.width,
            height
        )
    }

    static SAMPLES: AtomicUsize = AtomicUsize::new(0);
    static WRONG: AtomicUsize = AtomicUsize::new(0);
    static FIRST_WRONG: Mutex<String> = Mutex::new(String::new());
    static ROW_WRONG: AtomicUsize = AtomicUsize::new(0);
    static FIRST_ROW_WRONG: Mutex<String> = Mutex::new(String::new());

    /// The three buttons' top lefts (`x:y`, the window's points from its top
    /// left), and whether they stand in one row, evenly apart.
    fn row() -> Option<(String, bool)> {
        let window = ours()?;
        let height = window.frame().size.height;
        let at: Vec<(f64, f64)> = [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ]
        .into_iter()
        .filter_map(|kind| window.standardWindowButton(kind))
        .map(|button| {
            let frame = button.convertRect_toView(button.bounds(), None);
            (
                frame.origin.x,
                height - (frame.origin.y + frame.size.height),
            )
        })
        .collect();
        let text = at
            .iter()
            .map(|(x, y)| format!("{x:.1}:{y:.1}"))
            .collect::<Vec<_>>()
            .join(",");
        let even = at.len() == 3
            && (at[1].1 - at[0].1).abs() <= 0.5
            && (at[2].1 - at[0].1).abs() <= 0.5
            && ((at[2].0 - at[1].0) - (at[1].0 - at[0].0)).abs() <= 0.5;
        Some((text, even))
    }

    /// Counts a row read: whether it was wrong, the first wrong one kept.
    fn count_row(wrong: &AtomicUsize, first: &Mutex<String>) {
        if let Some((text, false)) = row()
            && wrong.fetch_add(1, Ordering::Relaxed) == 0
        {
            *first.lock().unwrap() = text;
        }
    }

    /// The window's frame for a content size, its top left kept.
    fn frame_for(window: &NSWindow, size: (f64, f64)) -> objc2_foundation::NSRect {
        let mut frame = window.frame();
        let content = window.contentRectForFrameRect(frame);
        let height = frame.size.height;
        frame.size = objc2_foundation::NSSize::new(
            frame.size.width - content.size.width + size.0,
            frame.size.height - content.size.height + size.1,
        );
        frame.origin.y -= frame.size.height - height;
        frame
    }

    /// `steps` frame changes back to back (`setFrame:display:`), the run loop
    /// never turning between them, the buttons read after each.
    pub fn burst(steps: usize) -> String {
        let Some(window) = ours() else {
            return "steps=0 row_wrong=0 first=".into();
        };
        let (wrong, first) = (AtomicUsize::new(0), Mutex::new(String::new()));
        for step in 0..steps {
            let grow = (step % 15) as f64;
            window.setFrame_display(
                frame_for(&window, (700. + grow * 6., 440. + grow * 3.)),
                true,
            );
            count_row(&wrong, &first);
        }
        format!(
            "steps={steps} row_wrong={} first={}",
            wrong.load(Ordering::Relaxed),
            first.lock().unwrap()
        )
    }

    /// `steps` frame changes in the event-tracking run loop mode, as a live
    /// resize's drag makes them: each in a loop pass of its own (its blocks
    /// and Core Animation's commit run), the buttons read after it.
    pub fn tracking(steps: usize) -> String {
        let Some(window) = ours() else {
            return "steps=0 row_wrong=0 first= in_live_resize=false".into();
        };
        let (wrong, first) = (AtomicUsize::new(0), Mutex::new(String::new()));
        let mut live = false;
        // SAFETY: NSRunLoop's +currentRunLoop and -runMode:beforeDate: on the
        // main thread, with AppKit's tracking mode's name and a date now.
        unsafe {
            use objc2::{msg_send, runtime::AnyObject};
            let run_loop: Retained<AnyObject> = msg_send![objc2::class!(NSRunLoop), currentRunLoop];
            let mode = objc2_foundation::NSString::from_str("NSEventTrackingRunLoopMode");
            for step in 0..steps {
                let grow = (step % 15) as f64;
                window.setFrame_display(
                    frame_for(&window, (700. - grow * 6., 440. - grow * 3.)),
                    true,
                );
                live |= window.inLiveResize();
                let now: Retained<AnyObject> = msg_send![objc2::class!(NSDate), date];
                let _: bool = msg_send![&*run_loop, runMode: &*mode, beforeDate: &*now];
                count_row(&wrong, &first);
            }
        }
        format!(
            "steps={steps} row_wrong={} first={} in_live_resize={live}",
            wrong.load(Ordering::Relaxed),
            first.lock().unwrap()
        )
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopCommonModes: *const c_void;
        fn CFRunLoopGetMain() -> *mut c_void;
        fn CFRunLoopObserverCreateWithHandler(
            allocator: *const c_void,
            activities: usize,
            repeats: u8,
            order: isize,
            block: *const c_void,
        ) -> *mut c_void;
        fn CFRunLoopAddObserver(run_loop: *mut c_void, observer: *mut c_void, mode: *const c_void);
    }

    /// Samples the close button after each frame Core Animation commits
    /// (a run-loop observer just after its own, before the loop waits):
    /// what a frame shows is where the button stood then. Counts the
    /// frames where it was not at `at`.
    pub fn watch_frames(at: (f64, f64)) {
        let block = block2::RcBlock::new(move |_observer: *mut c_void, _activity: usize| {
            let Some((x, y)) = close_xy() else {
                return;
            };
            SAMPLES.fetch_add(1, Ordering::Relaxed);
            if ((x - at.0).abs() > 0.5 || (y - at.1).abs() > 0.5)
                && WRONG.fetch_add(1, Ordering::Relaxed) == 0
            {
                *FIRST_WRONG.lock().unwrap() = format!("{x:.1},{y:.1}");
            }
            count_row(&ROW_WRONG, &FIRST_ROW_WRONG);
        });
        // kCFRunLoopBeforeWaiting, repeating, after Core Animation's commit (order 2000000).
        // SAFETY: CoreFoundation's observer API with a block that lives as long as the process.
        unsafe {
            let observer = CFRunLoopObserverCreateWithHandler(
                std::ptr::null(),
                1 << 5,
                1,
                2_000_001,
                &*block as *const _ as *const c_void,
            );
            CFRunLoopAddObserver(CFRunLoopGetMain(), observer, kCFRunLoopCommonModes);
        }
        std::mem::forget(block);
    }

    pub fn reset_counts() {
        SAMPLES.store(0, Ordering::Relaxed);
        WRONG.store(0, Ordering::Relaxed);
        FIRST_WRONG.lock().unwrap().clear();
        ROW_WRONG.store(0, Ordering::Relaxed);
        FIRST_ROW_WRONG.lock().unwrap().clear();
    }

    pub fn watched() -> String {
        format!(
            "LIVE_RESIZE samples={} wrong={} first_wrong={} row_wrong={} first_row_wrong={}",
            SAMPLES.load(Ordering::Relaxed),
            WRONG.load(Ordering::Relaxed),
            FIRST_WRONG.lock().unwrap(),
            ROW_WRONG.load(Ordering::Relaxed),
            FIRST_ROW_WRONG.lock().unwrap()
        )
    }

    /// The window resized as AppKit animates a frame change (its own loop
    /// runs the frames, as a drag's does), to a content size.
    pub fn animate_resize(size: (f64, f64)) {
        use objc2_foundation::{NSRect, NSSize};
        let Some(window) = ours() else {
            return;
        };
        let mut frame = window.frame();
        let content = window.contentRectForFrameRect(frame);
        frame.size = NSSize::new(
            frame.size.width - content.size.width + size.0,
            frame.size.height - content.size.height + size.1,
        );
        frame.origin.y -= frame.size.height - window.frame().size.height;
        let _: NSRect = frame;
        window.setFrame_display_animate(frame, true, true);
    }

    fn close_xy() -> Option<(f64, f64)> {
        let window = ours()?;
        let close = window.standardWindowButton(NSWindowButton::CloseButton)?;
        let frame = close.convertRect_toView(close.bounds(), None);
        Some((
            frame.origin.x,
            window.frame().size.height - (frame.origin.y + frame.size.height),
        ))
    }

    /// The close button set off its place, as AppKit's own layout does: the
    /// frame change must have it back where the app asked before this returns.
    pub fn nudge() {
        let Some(window) = ours() else {
            return;
        };
        if let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) {
            close.setFrameOrigin(objc2_foundation::NSPoint::new(3., 3.));
        }
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

    /// The window zoomed to fill the screen, or back: AppKit's `zoom:`, as
    /// a double click on the title bar (or Option and the green button) does.
    pub fn fill_screen() {
        if let Some(window) = ours() {
            window.zoom(None);
        }
    }

    /// AppKit's title bar laid out again with the window's size unchanged, as
    /// a new title has it: its constraints set every button's frame, the zoom
    /// button's last, and no resize follows to place them again.
    pub fn relayout_titlebar() {
        let Some(window) = ours() else {
            return;
        };
        let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
            return;
        };
        // SAFETY: a set superview back pointer is a live view (main thread).
        let Some(container) =
            (unsafe { close.superview() }).and_then(|bar| unsafe { bar.superview() })
        else {
            return;
        };
        // A new title, then the probe's own back (`ours` finds the window by it).
        for title in ["Traffic lights, retitled", "Traffic lights"] {
            window.setTitle(&objc2_foundation::NSString::from_str(title));
            container.layoutSubtreeIfNeeded();
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
    pub fn nudge() {}
    pub fn hide(_: bool) {}
    pub fn toggle() {}
    pub fn fill_screen() {}
    pub fn relayout_titlebar() {}
    pub fn watch_frames(_: (f64, f64)) {}
    pub fn animate_resize(_: (f64, f64)) {}
    pub fn reset_counts() {}
    pub fn watched() -> String {
        "LIVE_RESIZE samples=0 wrong=0 first_wrong= row_wrong=0 first_row_wrong=".into()
    }
    pub fn burst(_: usize) -> String {
        "steps=0 row_wrong=0 first=".into()
    }
    pub fn tracking(_: usize) -> String {
        "steps=0 row_wrong=0 first= in_live_resize=false".into()
    }
}
