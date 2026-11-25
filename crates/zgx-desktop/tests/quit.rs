//! macOS: each way an app is told to quit runs every window's `on_closed`,
//! once, and asks every `on_close_requested` (a session's end none). Each is
//! a process of its own (this one again, with an arm to run), offscreen, two
//! windows with two handlers each; quit events are dispatched in the process
//! and sent nowhere.
#[cfg(not(target_os = "macos"))]
fn main() {}
#[cfg(target_os = "macos")]
fn main() {
    // `cargo test` lists and filters what it runs: this is one test.
    if std::env::args().any(|arg| arg == "--list") {
        return;
    }
    match std::env::var("ZGX_QUIT_ARM") {
        Ok(arm) => app::run(&arm),
        Err(_) => checks(),
    }
}
/// What the process of `arm` printed, sorted but for who was asked first, and
/// its exit status.
#[cfg(target_os = "macos")]
fn quit(arm: &str) -> (Vec<String>, Option<i32>) {
    use std::time::{Duration, Instant};
    let out = std::env::temp_dir().join(format!("zgx-quit-{}-{arm}", std::process::id()));
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .env("ZGX_QUIT_ARM", arm)
        .env("ZGX_OFFSCREEN", "1")
        .stdout(std::fs::File::create(&out).unwrap())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let status = loop {
        match child.try_wait().unwrap() {
            Some(status) => break status.code(),
            None if start.elapsed() > Duration::from_secs(60) => {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{arm} hangs");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let printed = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    (printed.lines().map(str::to_owned).collect(), status)
}
#[cfg(target_os = "macos")]
fn checks() {
    let closed = ["closed a", "closed a 2", "closed b", "closed b 2"];
    let asked = ["asked a", "asked b"];
    let check = |arm: &str, asks: &[&str], closes: &[&str], status: i32| {
        let (printed, got) = quit(arm);
        let of = |what: &str| -> Vec<&str> {
            let mut lines: Vec<_> = printed
                .iter()
                .map(String::as_str)
                .filter(|line| line.starts_with(what))
                .collect();
            lines.sort_unstable();
            lines
        };
        assert_eq!(of("asked"), asks, "{arm}: {printed:?}");
        assert_eq!(of("closed"), closes, "{arm}: {printed:?}");
        assert_eq!(got, Some(status), "{arm}: {printed:?}");
        println!("quit: {arm}");
    };
    for from in ["outside", "inside", "early"] {
        check(&format!("{from}-plain"), &[], &closed, 0);
        // One whose `on_closed` panics loses no other handler.
        check(&format!("{from}-plain-panics"), &[], &closed, 0);
        check(&format!("{from}-asks"), &asked, &closed, 0);
        // The first asked refuses: it's the only one asked, and none closes.
        let (printed, status) = quit(&format!("{from}-asks-refuses"));
        assert_eq!(status, Some(3), "{from}: {printed:?}");
        assert!(
            printed.len() == 1 && printed[0].starts_with("asked"),
            "{from}: {printed:?}"
        );
        check(&format!("{from}-logout-asks-refuses"), &[], &closed, 0);
    }
    // The app's own quit asks nothing.
    check("app-asks", &[], &closed, 0);
}
#[cfg(target_os = "macos")]
mod app {
    use objc2::{class, msg_send, runtime::AnyObject};
    use std::{ffi::c_void, time::Duration};
    use zgx::compose::prelude::*;
    use zgx_desktop::{Application, WindowContext, WindowOptions};
    unsafe extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(q: *const c_void, cx: *mut c_void, f: extern "C" fn(*mut c_void));
    }
    pub fn run(arm: &str) {
        let has = |part: &str| arm.split('-').any(|p| p == part);
        let (asks, refuses, panics, logout) =
            (has("asks"), has("refuses"), has("panics"), has("logout"));
        let from = arm.split('-').next().unwrap().to_owned();
        let build = move |cx: &mut WindowContext, name: &'static str| {
            cx.render(div().w_full().h_full().bg(rgb(0x203040)));
            if asks {
                cx.on_close_requested(move || {
                    println!("asked {name}");
                    !refuses
                });
            }
            cx.on_closed(move || {
                println!("closed {name}");
                assert!(!(panics && name == "a"), "on purpose");
            });
            cx.on_closed(move || println!("closed {name} 2"));
        };
        let options = |title: &str| WindowOptions {
            title: title.into(),
            width: 300.,
            height: 200.,
            ..Default::default()
        };
        Application::new()
            .window(options("a"))
            .run(move |cx| {
                build(cx, "a");
                cx.windows.open(options("b"), move |cx| build(cx, "b"));
                let windows = cx.windows.clone();
                cx.tasks.spawn(async move {
                    // Both windows open.
                    zgx::timer::sleep(Duration::from_millis(800)).await;
                    match (from.as_str(), logout) {
                        ("app", _) => windows.quit(),
                        // From this task's turn: inside winit's handler.
                        ("inside", false) => terminate(std::ptr::null_mut()),
                        ("inside" | "early", true) => logout_event(std::ptr::null_mut()),
                        // ⌘Q before the menu bar is made, as the host offers it.
                        ("early", false) => {
                            let _ = zgx_desktop::app_menu::debug_key_before_menu_bar(&[], "q");
                        }
                        (_, logout) => {
                            let f = if logout { logout_event } else { terminate };
                            unsafe {
                                dispatch_async_f(
                                    &raw const _dispatch_main_q,
                                    std::ptr::null_mut(),
                                    f,
                                )
                            }
                        }
                    }
                    zgx::timer::sleep(Duration::from_secs(3)).await;
                    // (Held, or nothing quit it.)
                    std::process::exit(3);
                });
            })
            .unwrap();
    }
    extern "C" fn terminate(_: *mut c_void) {
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let none: *const AnyObject = std::ptr::null();
            let _: () = msg_send![app, terminate: none];
        }
    }
    /// A quit for a logout (`kAEReallyLogOut`), given to this process alone.
    extern "C" fn logout_event(_: *mut c_void) {
        use objc2_foundation::{NSAppleEventDescriptor as D, NSAppleEventManager as M};
        let code = |c: &[u8; 4]| u32::from_be_bytes(*c);
        unsafe {
            let event = D::appleEventWithEventClass_eventID_targetDescriptor_returnID_transactionID(
                code(b"aevt"),
                code(b"quit"),
                None,
                -1,
                0,
            );
            let reason = D::descriptorWithTypeCode(code(b"rlgo"));
            event.setAttributeDescriptor_forKeyword(&reason, code(b"why?"));
            let reply = D::nullDescriptor();
            M::sharedAppleEventManager().dispatchRawAppleEvent_withRawReply_handlerRefCon(
                std::ptr::NonNull::new(event.aeDesc().cast_mut()).unwrap(),
                std::ptr::NonNull::new(reply.aeDesc().cast_mut()).unwrap(),
                std::ptr::null_mut::<c_void>() as _,
            );
        }
    }
}
