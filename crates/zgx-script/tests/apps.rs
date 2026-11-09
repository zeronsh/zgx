//! Programs scripts start and show (`ui.app`, `ui.surface`): sessions the
//! host keeps across reloads, a plugin's only with `Capability::Apps`, each
//! start asked of the host's vetting hook, and a program ended with its
//! session or its plugin.
#![cfg(all(unix, feature = "apps"))]
mod common;
use common::Harness;
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
use zgx_script::{AppLaunch, AppSpec, Capability, GuardKind, Plugin, ScriptHost};

fn plugin(name: &str, code: &str) -> PathBuf {
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("zgx-apps-{}-{n}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    entry
}

/// Whether process `pid` is alive (and not a zombie: a session reaps it).
fn alive(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .split(") ")
            .nth(1)
            .is_some_and(|rest| rest.starts_with('Z'))
    })
}

/// Settle until `done`, for up to five seconds of the wall clock (a
/// program's exit arrives from a thread of its own).
fn settle_until(h: &mut Harness, mut done: impl FnMut(&mut Harness) -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !done(h) {
        assert!(Instant::now() < until, "timed out");
        h.settle();
        h.frame();
        std::thread::sleep(Duration::from_millis(10));
    }
}

const SLEEPER: &str = r#"
local app = ui.app("sleeper", { command = "sleep", args = { "30" } })
return ui.column {
    ui.text { id = "pid", function() return tostring(app.pid) end },
    ui.text { id = "status", function() return app.status end },
    ui.surface { id = "surface", app = app, w = 200, h = 100, rounded = 8 },
    ui.button { id = "close", "Close", on_press = function() app:close() end },
}
"#;

#[test]
fn a_session_outlives_reloads_and_ends_its_program_as_it_closes() {
    let mut h = Harness::inline(SLEEPER);
    assert!(
        h.find("surface").is_some(),
        "the surface is laid out as a node"
    );
    let pid: i32 = h.text("pid").parse().expect("a pid");
    assert!(alive(pid));
    // `sleep` never says hello: it is starting until it ends.
    assert_eq!(h.text("status"), "starting");
    h.host.reload().unwrap();
    h.settle();
    assert_eq!(
        h.text("pid"),
        pid.to_string(),
        "the same program after a reload"
    );
    h.click("close");
    let until = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(Instant::now() < until, "the program outlived its session");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn on_exit_hears_the_code_and_status_follows() {
    let mut h = Harness::inline(
        r#"
local code = ui.state("code", "none")
local app = ui.app("quick", {
    command = "sh",
    args = { "-c", "exit 3" },
    on_exit = function(c) code:set(tostring(c)) end,
})
return ui.column {
    ui.text { id = "code", function() return code:get() end },
    ui.text { id = "status", function() return `{app.status} {app.exit_code}` end },
}
"#,
    );
    settle_until(&mut h, |h| h.text("code") == "3");
    assert_eq!(h.text("status"), "exited 3");
}

#[test]
fn a_program_that_cannot_start_is_a_failed_session() {
    let mut h = Harness::inline(
        r#"
local app = ui.app("missing", { command = "/nonexistent/zgx-no-such-program" })
return ui.text { id = "status", function() return `{app.status}: {app.error}` end }
"#,
    );
    let status = h.text("status");
    assert!(
        status.starts_with("failed: /nonexistent/zgx-no-such-program"),
        "{status}"
    );
}

#[test]
fn every_start_is_vetted_and_a_plugin_needs_the_grant() {
    let asked: Rc<RefCell<Vec<AppLaunch>>> = Rc::default();
    let granted = plugin(
        "game",
        r#"
local ok, err = pcall(ui.app, "game", { command = "sleep", args = { "30" }, cwd = "/" })
ui.contribute("bar", ui.text { id = "game", ok and "started" or tostring(err) })
"#,
    );
    let refused = plugin(
        "nosy",
        r#"
local ok, err = pcall(ui.app, "x", { command = "sleep", args = { "30" } })
ui.contribute("bar", ui.text { id = "nosy", ok and "started" or tostring(err) })
"#,
    );
    let mut h = Harness::new({
        let asked = asked.clone();
        move |runtime| {
            let host = ScriptHost::inline(
                runtime,
                "app",
                r#"
local ok, err = pcall(ui.app, "mine", { command = "forbidden" })
return ui.column {
    ui.text { id = "mine", ok and "started" or tostring(err) },
    ui.slot("bar", { id = "bar" }),
}
"#,
            );
            host.vet_apps(move |launch| {
                asked.borrow_mut().push(launch.clone());
                match launch.asked.command.as_str() {
                    "forbidden" => Err("not on the list".into()),
                    _ => Ok(launch.asked.clone()),
                }
            });
            host.set_plugins(vec![
                Plugin {
                    name: "game".into(),
                    entry: granted,
                    grants: vec![Capability::Apps],
                },
                Plugin {
                    name: "nosy".into(),
                    entry: refused,
                    grants: vec![],
                },
            ]);
            host
        }
    });
    assert_eq!(h.text("game"), "started");
    let nosy = h.text("nosy");
    assert!(nosy.contains("Capability::Apps"), "{nosy}");
    let mine = h.text("mine");
    assert!(mine.contains("ui.app `mine`: not on the list"), "{mine}");
    let asked = asked.borrow();
    // The plugin without the grant reached no hook.
    let named: Vec<_> = asked
        .iter()
        .map(|l| (l.plugin.clone(), l.asked.command.as_str()))
        .collect();
    assert!(named.contains(&(Some("game".into()), "sleep")), "{named:?}");
    assert!(named.contains(&(None, "forbidden")), "{named:?}");
    assert_eq!(named.len(), 2, "{named:?}");
    let game = asked.iter().find(|l| l.plugin.is_some()).unwrap();
    assert_eq!(
        (game.id.as_str(), game.asked.args.clone()),
        ("game", vec!["30".to_owned()])
    );
    assert_eq!(game.asked.cwd, Some(PathBuf::from("/")));
    assert!(!game.running);
}

#[test]
fn removing_a_plugin_ends_its_programs() {
    let entry = plugin(
        "game",
        r#"
local app = ui.app("game", { command = "sleep", args = { "30" } })
ui.contribute("bar", ui.text { id = "pid", tostring(app.pid) })
"#,
    );
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.column { ui.slot("bar", { id = "bar" }) }"#,
        );
        host.set_plugins(vec![Plugin {
            name: "game".into(),
            entry,
            grants: vec![Capability::Apps],
        }]);
        host
    });
    let pid: i32 = h.text("pid").parse().unwrap();
    assert!(alive(pid));
    h.host.set_plugins(vec![]);
    h.host.reload().unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(Instant::now() < until, "the plugin's program outlived it");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_app_guard_is_asked_with_the_whole_spec() {
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"
local ok, err = pcall(ui.app, "x", { command = "sleep", args = { "30" }, env = { B = "2", A = "1" } })
return ui.text { id = "x", ok and "started" or tostring(err) }
"#,
        );
        host.guard(|kind, name| match (kind, name) {
            (GuardKind::App, r#"{"args":["30"],"command":"sleep","env":{"A":"1","B":"2"}}"#) => {
                Err("a preview starts nothing".into())
            }
            (GuardKind::App, other) => panic!("asked {other}"),
            _ => Ok(()),
        });
        host
    });
    let x = h.text("x");
    assert!(x.contains("a preview starts nothing"), "{x}");
}

#[test]
fn a_surface_needs_an_app() {
    let mut h = Harness::inline(
        r#"
local ok, err = pcall(ui.surface, { app = 3 })
return ui.text { id = "x", tostring(err) }
"#,
    );
    let error = h.text("x");
    assert!(
        error.contains("ui.surface needs `app = ui.app(id, …)`, not integer"),
        "{error}"
    );
}

/// What the element forwards, as the program reads it: keys with their
/// physical codes, text apart, the pointer in its pixels from its corner,
/// focus, the wheel, and a pointer lock the program asks for just after a
/// press, its raw motion, and Escape ending it.
#[test]
fn the_surface_forwards_input_and_locks_the_pointer_on_request() {
    use zgx::{
        compose::prelude::*,
        input::{InputEvent, Key, Modifiers, PointerButton},
    };
    use zgx_surface::{ClientMessage, HostMessage, host, transport};

    let ((host_tx, host_rx), (mut client_tx, mut client_rx)) = transport::pair().unwrap();
    let (heard_tx, heard) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        while let Ok(Some(message)) = client_rx.recv_host() {
            let _ = heard_tx.send(format!("{message:?}"));
        }
    });
    let app: Rc<RefCell<Option<host::App>>> = Rc::default();
    let made = app.clone();
    let mut h = Harness::wrapped(
        move |runtime| {
            *made.borrow_mut() = Some(host::App::over(runtime, host_tx, host_rx));
            ScriptHost::inline(runtime, "app", r#"return ui.text { "beside" }"#)
        },
        {
            let app = app.clone();
            move |view| {
                let app = app.borrow().clone().unwrap();
                // In a row: a column this short would shrink it.
                row()
                    .items_start()
                    .child(host::surface(&app).w(200.).h(100.))
                    .child(view)
            }
        },
    );
    h.frames(2);
    let drain = || {
        let mut got = Vec::new();
        while let Ok(message) = heard.recv_timeout(Duration::from_millis(200)) {
            got.push(message);
        }
        got
    };
    // Shown, as a client takes it until told otherwise: nothing to say.
    assert_eq!(drain(), Vec::<String>::new());

    // A press focuses it; positions are its own (scale 1 without a GPU).
    let button = PointerButton::Primary;
    h.ui.dispatch(InputEvent::PointerMove { x: 50., y: 60. });

    h.ui.input.note_gesture();
    h.ui.dispatch(InputEvent::PointerDown {
        x: 50.,
        y: 60.,
        button,
    });
    h.ui.dispatch(InputEvent::PointerUp {
        x: 50.,
        y: 60.,
        button,
    });
    h.ui.input.set_key_code(Some("KeyW".into()));
    h.ui.dispatch(InputEvent::KeyDown {
        key: Key::Character("w".into()),
        modifiers: Modifiers {
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
    h.ui.dispatch(InputEvent::Text("W".into()));
    h.ui.dispatch(InputEvent::KeyUp {
        key: Key::Character("w".into()),
        modifiers: Modifiers::default(),
    });
    h.ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 60.,
        delta_x: 0.,
        delta_y: 30.,
    });
    h.frames(2);
    let got = drain();
    let expected = [
        "PointerMove { x: 50.0, y: 60.0 }",
        "Focus(true)",
        "Button { button: Left, down: true, x: 50.0, y: 60.0 }",
        "Button { button: Left, down: false, x: 50.0, y: 60.0 }",
        r#"Key(KeyEvent { code: "KeyW", down: true, repeat: false, modifiers: Modifiers { shift: true, control: false, alt: false, meta: false } })"#,
        r#"Text("W")"#,
        r#"Key(KeyEvent { code: "KeyW", down: false, repeat: false, modifiers: Modifiers { shift: false, control: false, alt: false, meta: false } })"#,
        "Wheel { dx: 0.0, dy: 30.0, unit: Pixels }",
    ];
    for want in expected {
        assert!(got.iter().any(|m| m == want), "{want} not in {got:#?}");
    }
    let at = |m: &str| got.iter().position(|g| g == m).unwrap();
    assert!(at(expected[2]) < at(expected[3]) && at(expected[4]) < at(expected[6]));

    // The program asks for the lock after a press: granted.
    h.ui.input.note_gesture();
    h.ui.dispatch(InputEvent::PointerDown {
        x: 80.,
        y: 20.,
        button,
    });
    client_tx
        .send_client(&ClientMessage::LockPointer(true))
        .unwrap();
    h.ui.dispatch(InputEvent::PointerUp {
        x: 80.,
        y: 20.,
        button,
    });
    // The lock request arrives from the connection's thread.
    let until = Instant::now() + Duration::from_secs(5);
    let mut got = Vec::new();
    while !got.iter().any(|m: &String| m == "PointerLock(true)") {
        assert!(Instant::now() < until, "no lock: {got:#?}");
        h.settle();
        h.frames(1);
        got.extend(heard.try_iter());
    }
    h.ui.dispatch(InputEvent::PointerMotion { dx: 7., dy: -2. });
    h.ui.input.set_key_code(Some("Escape".into()));
    h.ui.input.note_gesture();
    h.ui.dispatch(InputEvent::KeyDown {
        key: Key::Escape,
        modifiers: Modifiers::default(),
        repeat: false,
    });
    h.frames(2);
    let got = drain();
    assert_eq!(
        got.iter()
            .filter(|m| m.starts_with("PointerMotion")
                || m.starts_with("PointerLock")
                || m.contains("Escape"))
            .collect::<Vec<_>>(),
        vec!["PointerMotion { dx: 7.0, dy: -2.0 }", "PointerLock(false)"],
        "Escape ends the lock and reaches no one"
    );
    // Revoked, it forwards nothing more; its program is told to close.
    app.borrow().as_ref().unwrap().revoke();
    h.ui.dispatch(InputEvent::PointerMove { x: 20., y: 20. });
    h.frames(2);
    let got = drain();
    assert_eq!(got, vec![format!("{:?}", HostMessage::Close)]);
}

/// The host's hook says what runs: the spec asked for (args, cwd, env and
/// all) goes to it, and what it returns starts, exactly.
#[test]
fn the_hook_decides_exactly_what_starts() {
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"
local app = ui.app("game", { command = "horizon", args = { "--world", "x" }, cwd = "/tmp", env = { LANG = "C" } })
return ui.text { id = "pid", tostring(app.pid) }
"#,
        );
        host.vet_apps(|launch| {
            assert_eq!(launch.asked.command, "horizon");
            assert_eq!(launch.asked.args, ["--world", "x"]);
            assert_eq!(launch.asked.cwd, Some(PathBuf::from("/tmp")));
            assert_eq!(launch.asked.env, [("LANG".to_owned(), "C".to_owned())]);
            // Resolved to a file, its arguments pinned, its environment kept.
            Ok(AppSpec::new("/bin/sleep", ["29"]).cwd("/"))
        });
        host
    });
    let pid: i32 = h.text("pid").parse().expect("started what the hook said");
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap();
    assert_eq!(cmdline, b"/bin/sleep\x0029\x00");
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).unwrap();
    assert_eq!(cwd, PathBuf::from("/"));
}

/// Reaching a running session again (a reload, a lookup) asks the hook
/// again: a permission taken back holds from then on.
#[test]
fn a_session_reached_again_is_vetted_again() {
    let allowed = Rc::new(std::cell::Cell::new(true));
    let checks: Rc<RefCell<Vec<bool>>> = Rc::default();
    let mut h = Harness::new({
        let (allowed, checks) = (allowed.clone(), checks.clone());
        move |runtime| {
            let host = ScriptHost::inline(
                runtime,
                "app",
                r#"
local ok, err = pcall(ui.app, "s", { command = "sleep", args = { "30" } })
local found, again = pcall(ui.app, "s")
return ui.column {
    ui.text { id = "x", ok and tostring(err.pid) or tostring(err) },
    ui.text { id = "y", found and tostring(again and again.pid) or tostring(again) },
}
"#,
            );
            host.vet_apps(move |launch| {
                checks.borrow_mut().push(launch.running);
                match allowed.get() {
                    true => Ok(launch.asked.clone()),
                    false => Err("taken back".into()),
                }
            });
            host
        }
    });
    let pid: i32 = h.text("x").parse().unwrap();
    assert_eq!(h.text("y"), pid.to_string(), "a lookup reaches it");
    assert_eq!(
        *checks.borrow(),
        [false, true],
        "started, then reached again"
    );
    allowed.set(false);
    h.host.reload().unwrap();
    h.settle();
    assert!(
        h.text("x").contains("ui.app `s`: taken back"),
        "{}",
        h.text("x")
    );
    // Refused, it ended: a lookup after finds nothing, and it was killed.
    assert_eq!(h.text("y"), "nil");
    let until = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(
            Instant::now() < until,
            "a refused session's program kept running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A plugin reaches only the sessions it started: not the app's, not
/// another plugin's of the same id.
#[test]
fn a_plugin_reaches_only_its_own_sessions() {
    let lookup = |name: &str| {
        plugin(
            name,
            r#"
local found = ui.app("shared")
local mine = ui.app("shared", { command = "sleep", args = { "30" } })
ui.contribute("bar", ui.text { id = `PLUGIN`, `{found and found.pid or "none"} {mine.pid}` })
"#
            .replace("PLUGIN", name)
            .as_str(),
        )
    };
    let (a, b) = (lookup("a"), lookup("b"));
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"
local app = ui.app("shared", { command = "sleep", args = { "30" } })
return ui.column { ui.text { id = "app", tostring(app.pid) }, ui.slot("bar", { id = "bar" }) }
"#,
        );
        host.set_plugins(vec![
            Plugin {
                name: "a".into(),
                entry: a,
                grants: vec![Capability::Apps],
            },
            Plugin {
                name: "b".into(),
                entry: b,
                grants: vec![Capability::Apps],
            },
        ]);
        host
    });
    let app = h.text("app");
    let (a, b) = (h.text("a"), h.text("b"));
    let (a_found, a_pid) = a.split_once(' ').unwrap();
    let (b_found, b_pid) = b.split_once(' ').unwrap();
    assert_eq!(
        (a_found, b_found),
        ("none", "none"),
        "neither found another's"
    );
    let pids = [app.as_str(), a_pid, b_pid];
    assert!(
        pids[0] != pids[1] && pids[1] != pids[2] && pids[0] != pids[2],
        "{pids:?}"
    );
}

/// A refusal of a session reached again ends it: its program is killed and
/// the handles scripts kept to it are dead (reads, `ui.surface`).
#[test]
fn a_refused_session_ends_and_its_handles_die() {
    let allowed = Rc::new(std::cell::Cell::new(true));
    let mut h = Harness::new({
        let allowed = allowed.clone();
        move |runtime| {
            let host = ScriptHost::inline(
                runtime,
                "app",
                r#"
local app = ui.app("s", { command = "sleep", args = { "30" } })
local result = ui.state("result", "")
return ui.column {
    ui.text { id = "pid", tostring(app.pid) },
    ui.text { id = "result", function() return result:get() end },
    ui.button { id = "lookup", "Look", on_press = function()
        local _, looked = pcall(ui.app, "s")
        local _, read = pcall(function() return app.status end)
        local _, shown = pcall(ui.surface, { app = app })
        result:set(`{looked} | {read} | {shown}`)
    end },
}
"#,
            );
            host.vet_apps(move |launch| match allowed.get() {
                true => Ok(launch.asked.clone()),
                false => Err("taken back".into()),
            });
            host
        }
    });
    let pid: i32 = h.text("pid").parse().unwrap();
    assert!(alive(pid));
    allowed.set(false);
    h.click("lookup");
    h.settle();
    let result = h.text("result");
    let parts: Vec<&str> = result.split(" | ").collect();
    assert!(parts[0].contains("ui.app `s`: taken back"), "{result}");
    assert!(parts[1].contains("ui.app `s`: revoked"), "{result}");
    assert!(parts[2].contains("ui.app `s`: revoked"), "{result}");
    let until = Instant::now() + Duration::from_secs(5);
    while alive(pid) {
        assert!(
            Instant::now() < until,
            "a refused session's program kept running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
