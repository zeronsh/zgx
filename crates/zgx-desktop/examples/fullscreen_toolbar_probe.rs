//! An inset title bar in full screen (macOS): AppKit moves a window's
//! toolbar into a window of its own above it (NSToolbarFullScreenWindow),
//! where the inset title bar's empty toolbar showed as an opaque 40-point
//! bar over the content's top. `scripts/macos_fullscreen_toolbar_probe.py`
//! runs it and checks the lines.
//!
//! `--via green` goes in and out of full screen as the green button does
//! (`toggleFullScreen:`), `--via button` in by clicking it (its
//! `performClick:`; full screen hides it, so out as the green button
//! does); `--via borderless` with zgx's
//! `WindowHandle::set_fullscreen` (winit's borderless full screen);
//! `--via failed` posts AppKit's will-enter-full-screen notification for the
//! window without entering, then tells its delegate the transition failed
//! (`windowDidFailToEnterFullScreen:`), as a transition that fails after it
//! began looks to zgx (the toolbar hidden, then back once the window is plainly not in
//! full screen). Before, in and after full screen it prints:
//! - `STRIP <when> height=<pt> alpha=<a> visible=<bool>`: each toolbar
//!   full-screen window;
//! - `WINDOW <when> fullscreen=<bool> frame_h=<pt> layout_h=<pt>
//!   buttons_hidden=<bool> toolbar_visible=<bool>`: the window, its content
//!   layout rect's height (the window's whole height when nothing covers
//!   its top) and its traffic lights.
//!
//! - `STATE <ms> full=<bool> transition=<entering|leaving|none> <seconds>
//!   controls=<x or none>`: as each changes, what the window says of its
//!   full screen (`WindowInfo::fullscreen`,
//!   `WindowContext::full_screen_transition`) and its traffic lights
//!   (`WindowInfo::controls`: none in full screen).
//!
//! - with `--animate`, a bar that grows a step each display frame, and
//!   `FRAME <ms>` as each frame comes (what the window draws through a
//!   transition); `OCCLUSION <ms> visible=<bool>` as AppKit's occlusion
//!   state of the window changes.
//!
//! `--via twice` goes in and out twice, as `green` does (the second says how
//! long the first took). `--via hidden` minimizes the window and brings it
//! back, `--via hidden-during` the same but minimized as a transition (that
//! fails) begins: `MARK <ms> hidden|back` around it, with `--animate`, for
//! what it draws meanwhile.
//!
//! The window moves clear of the pointer first: resting on the green button,
//! the pointer opens AppKit's tiling menu, whose event loop runs until someone
//! dismisses it, and a probe has no one. `--stay` leaves it where it opened
//! (`POINTER over_button=<bool>` says, after the last step).
//!
//! `--via manual` runs no steps: the window stays until it is closed, for going
//! in and out by hand (the checklist on a change to full screen).
//!
//! `--close-button` closes through the red button (`performClick:`), not
//! `WindowHandle::close`.
//!
//! `--linger <s>` stays open that long after the last step, `BEAT <ms>` four
//! times a second, then says `LINGERED`: a stall shows when it began.
//!
//! Then `FULLSCREEN_TOOLBAR_DONE`.
use std::time::Duration;
use zgx::compose::prelude::*;
use zgx_desktop::{Application, Titlebar, WindowOptions};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let via = args
        .iter()
        .position(|arg| arg == "--via")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "green".into());
    let button = via == "button";
    let green = via == "green" || button;
    let failed = via == "failed";
    let animate = args.iter().any(|arg| arg == "--animate");
    let live_full_screen = args.iter().any(|arg| arg == "--live");
    let via_manual = via == "manual";
    let stay = args.iter().any(|arg| arg == "--stay");
    let close_button = args.iter().any(|arg| arg == "--close-button");
    let linger: u64 = args
        .iter()
        .position(|arg| arg == "--linger")
        .and_then(|i| args.get(i + 1))
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(0);
    let twice = via == "twice";
    let green = green || twice;
    let hidden = via == "hidden";
    let hidden_during = via == "hidden-during";
    Application::new()
        .window(WindowOptions {
            title: "Fullscreen toolbar".into(),
            width: 640.,
            height: 400.,
            titlebar: Titlebar::Inset,
            live_full_screen,
            ..Default::default()
        })
        .run(move |cx| {
            let start = std::time::Instant::now();
            let step = cx.ui.signal(0_u32);
            cx.render(div().w_full().h_full().bg(rgba(0x3060a0ff)).child(
                div().h(24.).bg(rgba(0xf0c040ff)).reactive_style({
                    let step = step.clone();
                    move || Styles::new().w((step.get() % 400) as f32)
                }),
            ));
            if animate {
                state::watch_occlusion(start);
                let frames = cx.frames.clone();
                let viewport = cx.viewport.clone();
                cx.tasks.spawn(async move {
                    let mut n = 0;
                    loop {
                        frames.next().await;
                        n += 1;
                        step.set(n);
                        let (drawn_w, drawn_h) = viewport.with_untracked(|size| *size);
                        let (w, h) = state::window_size();
                        println!(
                            "FRAME {} window={w:.0}x{h:.0} drawn={drawn_w:.0}x{drawn_h:.0}",
                            start.elapsed().as_millis()
                        );
                    }
                });
            }
            let (info, transition) = (cx.window_info.clone(), cx.full_screen_transition.clone());
            let states = cx.ui.runtime.effect(move || {
                let (moving, seconds) = match transition.get() {
                    Some(zgx_desktop::FullScreenTransition::Entering(d)) => ("entering", d),
                    Some(zgx_desktop::FullScreenTransition::Leaving(d)) => ("leaving", d),
                    None => ("none", Duration::ZERO),
                };
                let controls = info
                    .controls
                    .get()
                    .map_or("none".to_owned(), |rect| format!("{:.0}", rect.x));
                println!(
                    "STATE {} full={} transition={moving} {:.3} controls={controls}",
                    start.elapsed().as_millis(),
                    info.fullscreen.get(),
                    seconds.as_secs_f64(),
                );
            });
            cx.on_closed(move || drop(states));
            let window = cx.window.clone();
            cx.tasks.spawn(async move {
                let pause = |ms| zgx::timer::sleep(Duration::from_millis(ms));
                if via_manual {
                    return;
                }
                if !stay {
                    state::clear_of_pointer();
                }
                pause(1500).await;
                state::report("before");
                if hidden || hidden_during {
                    if hidden_during {
                        state::will_enter();
                    }
                    println!("MARK {} hidden", start.elapsed().as_millis());
                    state::minimize(true);
                    if hidden_during {
                        pause(300).await;
                        state::fail_to_enter();
                    }
                    pause(4000).await;
                    println!("MARK {} back", start.elapsed().as_millis());
                    state::minimize(false);
                    pause(1500).await;
                    println!("FULLSCREEN_TOOLBAR_DONE");
                    window.close();
                    return;
                }
                if failed {
                    state::will_enter();
                    pause(300).await;
                    state::fail_to_enter();
                    pause(200).await;
                    state::report("full");
                    pause(3000).await;
                    state::report("after");
                    println!("FULLSCREEN_TOOLBAR_DONE");
                    window.close();
                    return;
                }
                if button {
                    state::click();
                } else if green {
                    state::toggle();
                } else {
                    window.set_fullscreen(true);
                }
                pause(3000).await;
                state::report("full");
                // Full screen hides the traffic lights: a click on the
                // hidden green button does nothing.
                if green {
                    state::toggle();
                } else {
                    window.set_fullscreen(false);
                }
                pause(3000).await;
                state::report("after");
                if twice {
                    state::toggle();
                    pause(3000).await;
                    state::toggle();
                    pause(3000).await;
                }
                state::report_pointer();
                for _ in 0..linger * 4 {
                    pause(250).await;
                    println!("BEAT {}", start.elapsed().as_millis());
                }
                if linger > 0 {
                    println!("LINGERED");
                }
                println!("FULLSCREEN_TOOLBAR_DONE");
                if close_button {
                    state::click_close();
                } else {
                    window.close();
                }
            });
        })
}

#[cfg(target_os = "macos")]
mod state {
    use objc2::{MainThreadMarker, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSApplication, NSEvent, NSWindow, NSWindowButton, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSString};

    /// Its window isn't there: nothing that follows would mean anything.
    fn no_window() -> ! {
        println!("NO_WINDOW");
        std::process::exit(3);
    }

    fn ours() -> Option<Retained<NSWindow>> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
        app.windows()
            .iter()
            .find(|window| window.title().to_string() == "Fullscreen toolbar")
    }

    /// AppKit's will-enter-full-screen notification for the window, with no
    /// full screen after it: a transition that failed.
    pub fn will_enter() {
        use objc2_foundation::NSNotificationCenter;
        let Some(window) = ours() else {
            no_window();
        };
        // SAFETY: AppKit's own notification name (a constant static), posted
        // on the main thread with the window as its object, as AppKit does.
        unsafe {
            let name = objc2_app_kit::NSWindowWillEnterFullScreenNotification;
            NSNotificationCenter::defaultCenter().postNotificationName_object(name, Some(&window));
        }
    }

    /// A click on the green button (its `performClick:`, the action a
    /// press sends; no input posted).
    pub fn click() {
        let Some(window) = ours() else {
            no_window();
        };
        // SAFETY: `-standardWindowButton:` takes an NSUInteger (2: the zoom
        // button) and returns a button or nil; `-performClick:` takes a
        // sender, which may be nil.
        unsafe {
            let zoom: *mut AnyObject = msg_send![&*window, standardWindowButton: 2usize];
            if zoom.is_null() {
                println!("NO_BUTTON");
                return;
            }
            let _: () = msg_send![zoom, performClick: std::ptr::null::<AnyObject>()];
        }
    }

    /// `OCCLUSION <ms> visible=<bool>` as AppKit's occlusion state of the
    /// window changes (`NSWindowDidChangeOcclusionStateNotification`).
    pub fn watch_occlusion(start: std::time::Instant) {
        use objc2_app_kit::{NSWindowDidChangeOcclusionStateNotification, NSWindowOcclusionState};
        use objc2_foundation::{NSNotification, NSNotificationCenter};
        let block = block2::RcBlock::new(move |note: std::ptr::NonNull<NSNotification>| {
            // SAFETY: AppKit passes a valid notification for the block's
            // duration; a window's notification's object is that NSWindow.
            let note = unsafe { note.as_ref() };
            let Some(window) = note
                .object()
                .and_then(|object| object.downcast::<NSWindow>().ok())
            else {
                return;
            };
            if window.title().to_string() != "Fullscreen toolbar" {
                return;
            }
            println!(
                "OCCLUSION {} visible={}",
                start.elapsed().as_millis(),
                window
                    .occlusionState()
                    .contains(NSWindowOcclusionState::Visible)
            );
        });
        // SAFETY: AppKit's own notification name (a constant static); with
        // no queue the block runs on the main thread, where it is posted;
        // the observer lives as long as the probe.
        let observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSWindowDidChangeOcclusionStateNotification),
                None,
                None,
                &block,
            )
        };
        std::mem::forget(observer);
    }

    /// What AppKit tells the window's delegate (winit's) as a transition
    /// fails: `windowDidFailToEnterFullScreen:`, which winit undoes its
    /// full screen for. No notification says so.
    pub fn fail_to_enter() {
        let Some(window) = ours() else {
            no_window();
        };
        let Some(delegate) = window.delegate() else {
            println!("NO_DELEGATE");
            return;
        };
        // SAFETY: the window's delegate (winit's) answers
        // `-windowDidFailToEnterFullScreen:`, which takes the window.
        unsafe {
            let _: () = msg_send![&*delegate, windowDidFailToEnterFullScreen: &*window];
        }
    }

    /// The window moved so the pointer isn't over it.
    pub fn clear_of_pointer() {
        let Some(window) = ours() else {
            no_window();
        };
        let pointer = NSEvent::mouseLocation();
        let frame = window.frame();
        if pointer.x < frame.origin.x
            || pointer.x > frame.origin.x + frame.size.width
            || pointer.y < frame.origin.y
            || pointer.y > frame.origin.y + frame.size.height
        {
            return;
        }
        let screen = window
            .screen()
            .map_or(frame, |screen| screen.visibleFrame());
        // Below it if there is room, else above it.
        let y = if pointer.y - frame.size.height - 40. >= screen.origin.y {
            pointer.y - frame.size.height - 40.
        } else {
            pointer.y + 40.
        };
        window.setFrameOrigin(NSPoint::new(frame.origin.x, y));
    }

    /// `POINTER over_button=<bool>`: whether the pointer rests on the green
    /// button (where AppKit opens its tiling menu after a moment).
    pub fn report_pointer() {
        let Some(window) = ours() else {
            return;
        };
        let pointer = NSEvent::mouseLocation();
        let over = window
            .standardWindowButton(NSWindowButton::ZoomButton)
            .is_some_and(|button| {
                let rect =
                    window.convertRectToScreen(button.convertRect_toView(button.bounds(), None));
                pointer.x >= rect.origin.x
                    && pointer.x <= rect.origin.x + rect.size.width
                    && pointer.y >= rect.origin.y
                    && pointer.y <= rect.origin.y + rect.size.height
            });
        println!("POINTER over_button={over}");
    }

    /// A click on the red button (its `performClick:`).
    pub fn click_close() {
        let Some(window) = ours() else {
            no_window();
        };
        // SAFETY: `-standardWindowButton:` takes an NSUInteger (0: the close
        // button) and returns a button or nil; `-performClick:` takes a
        // sender, which may be nil.
        unsafe {
            let close: *mut AnyObject = msg_send![&*window, standardWindowButton: 0usize];
            if !close.is_null() {
                let _: () = msg_send![close, performClick: std::ptr::null::<AnyObject>()];
            }
        }
    }

    /// The window's frame size, in points.
    pub fn window_size() -> (f64, f64) {
        ours().map_or((0., 0.), |window| {
            let size = window.frame().size;
            (size.width, size.height)
        })
    }

    /// The window minimized, or back.
    pub fn minimize(hide: bool) {
        let Some(window) = ours() else {
            no_window();
        };
        if hide {
            window.miniaturize(None);
        } else {
            window.deminiaturize(None);
        }
    }

    /// What the green button does.
    pub fn toggle() {
        if let Some(window) = ours() {
            window.toggleFullScreen(None);
        } else {
            no_window();
        }
    }

    pub fn report(when: &str) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        for window in app.windows().iter() {
            let class = window.class().name().to_string_lossy().into_owned();
            if class.contains("ToolbarFullScreenWindow") {
                println!(
                    "STRIP {when} height={:.0} alpha={:.2} visible={}",
                    window.frame().size.height,
                    window.alphaValue(),
                    window.isVisible()
                );
            }
        }
        let Some(window) = ours() else {
            no_window();
        };
        // SAFETY: `-standardWindowButton:` takes an NSUInteger and returns
        // a view or nil, retained here; `-toolbar` the NSToolbar or nil;
        // `-isVisible` a BOOL.
        let (buttons_hidden, toolbar_visible) = unsafe {
            let close: Option<Retained<AnyObject>> =
                msg_send![&*window, standardWindowButton: 0_usize];
            let hidden = close.is_none_or(|button| {
                let hidden: bool = msg_send![&*button, isHidden];
                hidden
            });
            let toolbar: Option<Retained<AnyObject>> = msg_send![&*window, toolbar];
            let visible = toolbar.is_some_and(|toolbar| {
                let id: Option<Retained<NSString>> = msg_send![&*toolbar, identifier];
                let visible: bool = msg_send![&*toolbar, isVisible];
                id.is_some() && visible
            });
            (hidden, visible)
        };
        println!(
            "WINDOW {when} fullscreen={} frame_h={:.0} layout_h={:.0} buttons_hidden={buttons_hidden} toolbar_visible={toolbar_visible}",
            window.styleMask().contains(NSWindowStyleMask::FullScreen),
            window.frame().size.height,
            window.contentLayoutRect().size.height,
        );
    }
}

#[cfg(not(target_os = "macos"))]
mod state {
    pub fn will_enter() {}
    pub fn click_close() {}
    pub fn clear_of_pointer() {}
    pub fn report_pointer() {}
    pub fn window_size() -> (f64, f64) {
        (0., 0.)
    }
    pub fn minimize(_: bool) {}
    pub fn watch_occlusion(_: std::time::Instant) {}
    pub fn fail_to_enter() {}
    pub fn click() {}
    pub fn toggle() {}
    pub fn report(_: &str) {
        println!("SKIP macOS only");
    }
}
