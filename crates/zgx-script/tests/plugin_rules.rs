//! What a plugin may name and take beyond its grants: an element or a module
//! to replace, a slot to offer, a command's id and shortcut. And what a host
//! asks before its own scripts reach it (`ScriptHost::guard`).
mod common;

use common::Harness;
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU32, Ordering},
};
use zgx_script::{Capability, GuardKind, Plugin, ScriptHost, Value};

fn plugin_dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    // One of its own: tests run at once, some with plugins of one name.
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "zgx-plugin-rules-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (file, code) in files {
        std::fs::write(dir.join(file), code).unwrap();
    }
    dir.join("init.luau")
}
fn entry(name: &str, code: &str) -> PathBuf {
    plugin_dir(name, &[("init.luau", code)])
}
fn inline(app: &'static str, name: &str, code: &str, grants: Vec<Capability>) -> Harness {
    let (entry, name) = (entry(name, code), name.to_string());
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", app);
        host.set_plugins(vec![Plugin {
            name,
            entry,
            grants,
        }]);
        host
    })
}
fn error_of(h: &Harness) -> String {
    h.error().unwrap_or_default()
}
fn key_of(h: &Harness, id: &str) -> Option<String> {
    h.host
        .commands()
        .with_untracked(|list| list.iter().find(|c| c.id == id).and_then(|c| c.key.clone()))
}
fn has_command(h: &Harness, id: &str) -> bool {
    h.host
        .commands()
        .with_untracked(|list| list.iter().any(|c| c.id == id))
}

const APP: &str = r##"return ui.column {
    ui.text { id = "save", "Save" },
    ui.slot("bar", { id = "bar" }),
}"##;
const REPLACE: &str =
    r##"ui.contribute { target = "#save", replace = ui.text { id = "mine", "Mine" } }"##;
const FILTER: &str = r##"ui.filter { target = "#save", text = function(t) return t .. "!" end }"##;

// R1: replacing an element and replacing a module are different trusts.

#[test]
fn an_element_grant_replaces_and_filters_elements() {
    for grant in [
        Capability::OverrideElement("#save".into()),
        Capability::OverrideElement("*".into()),
        // The older spelling, for elements.
        Capability::Override("#save".into()),
        Capability::Override("*".into()),
    ] {
        let mut h = inline(APP, "swap", REPLACE, vec![grant.clone()]);
        assert_eq!(h.error(), None, "{grant:?}");
        assert_eq!(h.text("mine"), "Mine", "{grant:?}");
        let mut h = inline(APP, "tweak", FILTER, vec![grant.clone()]);
        assert_eq!(h.error(), None, "{grant:?}");
        assert_eq!(h.text("save"), "Save!", "{grant:?}");
    }
    // A module grant reaches no element.
    let mut h = inline(
        APP,
        "swap",
        REPLACE,
        vec![Capability::OverrideModule("*".into())],
    );
    assert_eq!(h.text("save"), "Save");
    assert!(h.find("mine").is_none());
}

const MODULE_APP: &str = r#"local card = require("@app/card")
return ui.column { card("Inbox") }"#;
const CARD: &str = r#"return function(title) return ui.text { id = "card", title } end"#;

/// `@app/card`, replaced by a plugin holding `grants`: whether it was, and
/// the error if not.
fn replaced_with(name: &str, grants: Vec<Capability>) -> (bool, String) {
    let app = plugin_dir(
        &format!("{name}-app"),
        &[("init.luau", MODULE_APP), ("card.luau", CARD)],
    );
    let entry = plugin_dir(
        name,
        &[
            (
                "init.luau",
                r#"ui.override { module = "@app/card", with = "./mine" }"#,
            ),
            (
                "mine.luau",
                r#"return function(title) return ui.text { id = "card", "mine" } end"#,
            ),
        ],
    );
    let name = name.to_string();
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, app);
        host.set_plugins(vec![Plugin {
            name,
            entry,
            grants,
        }]);
        host
    });
    (h.text("card") == "mine", error_of(&h))
}

#[test]
fn replacing_a_module_takes_a_grant_naming_modules() {
    for (name, grant) in [
        ("named", Capability::OverrideModule("@app/card".into())),
        ("any", Capability::OverrideModule("*".into())),
        // The older spelling, of the module by name.
        ("older", Capability::Override("@app/card".into())),
    ] {
        let (replaced, error) = replaced_with(name, vec![grant.clone()]);
        assert!(replaced, "{grant:?}: {error}");
    }
    // An element's grant, `*` included, is no trust with the app's code.
    for (name, grant) in [
        ("star", Capability::Override("*".into())),
        ("element", Capability::OverrideElement("*".into())),
        ("other", Capability::OverrideModule("@app/other".into())),
    ] {
        let (replaced, error) = replaced_with(name, vec![grant.clone()]);
        assert!(!replaced, "{grant:?}");
        assert!(
            error.contains("needs OverrideModule(\"@app/card\")"),
            "{error}"
        );
    }
}

// R22: a plugin offers its own names; what a host reserves is the app's.

#[test]
fn a_plugin_offers_only_slots_of_its_own() {
    let offers = |slot: &str| format!(r#"ui.contribute("bar", ui.slot("{slot}"))"#);
    let h = inline(APP, "mine", &offers("mine.panel"), vec![]);
    assert_eq!(h.error(), None);
    for slot in ["bar", "mine", "mine.", "minesweeper.panel", "other.panel"] {
        let h = inline(APP, "mine", &offers(slot), vec![]);
        let error = error_of(&h);
        assert!(
            error.contains("offers only slots of its own"),
            "{slot}: {error}"
        );
    }
    // The app's own scripts name theirs as they like.
    let h = Harness::inline(r#"return ui.slot("anything.at.all")"#);
    assert_eq!(h.error(), None);
}

const CONTRIBUTES: &str = r#"ui.contribute("zeron.sidebar", ui.text { id = "seen", "seen" })"#;
const POSING: &str = r#"
ui.contribute("zeron.x", ui.slot("zeron.x.panel"))
"#;

#[test]
fn a_reserved_prefix_is_the_apps_alone() {
    // Even a plugin named under it, which would otherwise own the name.
    let entry = entry("zeron.x", POSING);
    let h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", r#"return ui.slot("zeron.sidebar")"#);
        host.reserve_prefix("zeron.");
        host.set_plugins(vec![Plugin {
            name: "zeron.x".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    let error = error_of(&h);
    assert!(
        error.contains("`zeron.` names are the app's alone"),
        "{error}"
    );
    // The app's VM offers it, and plugins still contribute to it.
    let entry = self::entry("adds", CONTRIBUTES);
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", r#"return ui.slot("zeron.sidebar")"#);
        host.reserve_prefix("zeron");
        host.set_plugins(vec![Plugin {
            name: "adds".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    assert_eq!(h.error(), None);
    assert_eq!(h.text("seen"), "seen");
}

#[test]
fn a_reserved_prefix_holds_past_the_hosts_later_configuring() {
    let entry = entry(
        "later",
        r#"ui.command { id = "zeron.new", run = function() end }"#,
    );
    let h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", "return ui.text { \"app\" }");
        host.reserve_prefix("zeron");
        // A builder makes the host anew: what was asked stays asked.
        let host = host.error_banner(true);
        host.set_plugins(vec![Plugin {
            name: "later".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    let error = error_of(&h);
    assert!(
        error.contains("`zeron.` names are the app's alone"),
        "{error}"
    );
    assert!(!has_command(&h, "later/zeron.new"));
}

#[test]
fn a_command_id_under_a_reserved_prefix_is_the_apps_alone() {
    let h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"ui.command { id = "zeron.new", run = function() end }
            return ui.text { "app" }"#,
        );
        host.reserve_prefix("zeron");
        host
    });
    assert_eq!(h.error(), None);
    assert!(has_command(&h, "zeron.new"));
    // A plugin's ids are its own otherwise.
    let h = inline(
        APP,
        "mine",
        r#"ui.command { id = "zeron.new", run = function() end }"#,
        vec![],
    );
    assert_eq!(h.error(), None);
    assert!(has_command(&h, "mine/zeron.new"));
}

// R27: a key alone is taken from every field, page and terminal.

fn bare_grant() -> Capability {
    Capability::Named("zgx.command.bare_keys".into())
}

#[test]
fn a_plugins_shortcut_has_a_modifier_unless_granted_a_key_alone() {
    let command =
        |key: &str| format!(r#"ui.command {{ id = "go", key = "{key}", run = function() end }}"#);
    for key in ["k", "enter", "f5", "shift+k"] {
        let h = inline(APP, "keys", &command(key), vec![]);
        let error = error_of(&h);
        assert!(error.contains("zgx.command.bare_keys"), "{key}: {error}");
        assert!(!has_command(&h, "keys/go"), "{key}");
        // Granted, it holds it.
        let h = inline(APP, "keys", &command(key), vec![bare_grant()]);
        assert_eq!(h.error(), None, "{key}");
        assert_eq!(key_of(&h, "keys/go").as_deref(), Some(key));
    }
    for key in ["cmd+k", "ctrl+shift+k", "shift+f5"] {
        let h = inline(APP, "keys", &command(key), vec![]);
        assert_eq!(h.error(), None, "{key}");
        assert_eq!(key_of(&h, "keys/go").as_deref(), Some(key));
    }
    // The app's own are the app's to choose.
    let h = Harness::inline(
        r#"ui.command { id = "go", key = "k", run = function() end }
        return ui.text { "app" }"#,
    );
    assert_eq!(h.error(), None);
    assert_eq!(key_of(&h, "go").as_deref(), Some("k"));
}

#[test]
fn a_plugins_shortcut_that_follows_a_state_is_refused_as_it_reads_a_key_alone() {
    const DYNAMIC: &str = r#"
local key = ui.state("key", "cmd+j")
ui.command { id = "go", key = key, run = function() end }
ui.contribute("bar", ui.button { id = "bare", w = 10, h = 10, on_click = function() key:set("j") end })
ui.contribute("bar", ui.button { id = "held", w = 10, h = 10, on_click = function() key:set("cmd+k") end })
"#;
    let mut h = inline(APP, "keys", DYNAMIC, vec![]);
    h.settle();
    assert_eq!(key_of(&h, "keys/go").as_deref(), Some("cmd+j"));
    h.click("bare");
    h.settle();
    assert_eq!(key_of(&h, "keys/go"), None);
    let dropped = h.host.dropped_shortcuts();
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].plugin, "keys");
    assert_eq!(dropped[0].key, "j");
    assert!(dropped[0].reason.contains("zgx.command.bare_keys"));
    h.click("held");
    h.settle();
    assert_eq!(key_of(&h, "keys/go").as_deref(), Some("cmd+k"));
    assert!(h.host.dropped_shortcuts().is_empty());
    // Granted: a key alone stands.
    let mut h = inline(APP, "keys", DYNAMIC, vec![bare_grant()]);
    h.settle();
    h.click("bare");
    h.settle();
    assert_eq!(key_of(&h, "keys/go").as_deref(), Some("j"));
}

// The guard: what the app's own scripts reach of the host and the network.

const GUARDED: &str = r##"
local out = ui.state("out", "")
local function try(name, f)
    local ok, why = pcall(f)
    out:set(out:peek() .. name .. "=" .. (ok and "ok" or tostring(why)) .. ";")
end
return ui.column {
    ui.button { id = "invoke", w = 10, h = 10, on_click = function() try("invoke", function() ui.invoke("act") end) end },
    ui.button { id = "sync", w = 10, h = 10, on_click = function() try("sync", function() ui.call_sync("fn") end) end },
    ui.button { id = "call", w = 10, h = 10, on_click = function() try("call", function() ui.call("later") end) end },
    ui.button { id = "plain", w = 10, h = 10, on_click = function() try("plain", function() ui.call("act") end) end },
    ui.button { id = "fetch", w = 10, h = 10, on_click = function() try("fetch", function() ui.fetch("https://x.test/a") end) end },
    ui.button { id = "action", w = 10, h = 10, on_click = ui.action("act") },
    ui.text { id = "out", function() return out:get() end },
}"##;

type Asked = Rc<RefCell<Vec<(GuardKind, String)>>>;

fn guarded(host_ran: &Rc<RefCell<Vec<String>>>, allow: bool) -> (Harness, Asked) {
    let asked: Asked = Rc::default();
    let (ran, told) = (host_ran.clone(), asked.clone());
    let h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", GUARDED);
        let log = ran.clone();
        host.action("act", move |_| log.borrow_mut().push("act".into()));
        let log = ran.clone();
        host.function("fn", move |_| {
            log.borrow_mut().push("fn".into());
            Value::Nil
        });
        let log = ran.clone();
        host.async_action("later", move |_| {
            log.borrow_mut().push("later".into());
            async { Ok(Value::Nil) }
        });
        let log = ran.clone();
        host.fetcher(move |request: zgx_script::Request| {
            log.borrow_mut().push(request.url.clone());
            async { Err("no network here".to_string()) }
        });
        host.guard(move |kind, name| {
            told.borrow_mut().push((kind, name.to_owned()));
            match allow {
                true => Ok(()),
                false => Err("not in a preview".into()),
            }
        });
        host
    });
    (h, asked)
}

#[test]
fn the_guard_is_asked_before_the_app_reaches_the_host_or_the_network() {
    let ran = Rc::default();
    let (mut h, asked) = guarded(&ran, true);
    for id in ["invoke", "sync", "call", "plain", "fetch", "action"] {
        h.click(id);
        h.settle();
    }
    assert_eq!(
        *asked.borrow(),
        vec![
            (GuardKind::Action, "act".to_owned()),
            (GuardKind::Function, "fn".to_owned()),
            (GuardKind::AsyncAction, "later".to_owned()),
            (GuardKind::Action, "act".to_owned()),
            (GuardKind::Fetch, "https://x.test/a".to_owned()),
            (GuardKind::Action, "act".to_owned()),
        ]
    );
    assert_eq!(
        *ran.borrow(),
        ["act", "fn", "later", "act", "https://x.test/a", "act"]
    );
}

#[test]
fn what_the_guard_refuses_never_reaches_the_host_and_says_why() {
    let ran: Rc<RefCell<Vec<String>>> = Rc::default();
    let (mut h, asked) = guarded(&ran, false);
    for id in ["invoke", "sync", "call", "plain", "fetch"] {
        h.click(id);
        h.settle();
    }
    assert!(ran.borrow().is_empty(), "{:?}", ran.borrow());
    let out = h.text("out");
    for name in ["invoke", "sync", "call", "plain"] {
        assert!(
            out.contains(&format!("{name}=")) && !out.contains(&format!("{name}=ok")),
            "{out}"
        );
    }
    assert_eq!(out.matches("not in a preview").count(), 4, "{out}");
    // A fetch doesn't raise: its promise rejects.
    assert!(out.contains("fetch=ok"), "{out}");
    // A button's `ui.action` reports it, as an error in a handler is.
    h.click("action");
    h.settle();
    assert!(ran.borrow().is_empty());
    assert!(error_of(&h).contains("not in a preview"), "{:?}", h.error());
    assert_eq!(asked.borrow().len(), 6);
}

#[test]
fn a_guard_set_after_the_app_loaded_applies_at_once() {
    let ran: Rc<RefCell<Vec<String>>> = Rc::default();
    let log = ran.clone();
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", GUARDED);
        host.action("act", move |_| log.borrow_mut().push("act".into()));
        host
    });
    h.click("invoke");
    h.settle();
    assert_eq!(*ran.borrow(), ["act"]);
    h.host.guard(|_, _| Err("no".into()));
    h.click("invoke");
    h.settle();
    assert_eq!(*ran.borrow(), ["act"], "refused now");
}

#[test]
fn a_plugin_reaches_what_its_grants_say_unasked() {
    let asked: Asked = Rc::default();
    let ran: Rc<RefCell<Vec<String>>> = Rc::default();
    let (told, log) = (asked.clone(), ran.clone());
    let entry = entry(
        "granted",
        r#"ui.contribute("bar", ui.button { id = "go", w = 10, h = 10, on_click = function() ui.invoke("act") end })"#,
    );
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.action("act", move |_| log.borrow_mut().push("act".into()));
        host.expose_action("act", None);
        host.guard(move |kind, name| {
            told.borrow_mut().push((kind, name.to_owned()));
            Err("no".into())
        });
        host.set_plugins(vec![Plugin {
            name: "granted".into(),
            entry,
            grants: vec![],
        }]);
        host
    });
    h.click("go");
    h.settle();
    assert_eq!(*ran.borrow(), ["act"]);
    assert!(asked.borrow().is_empty(), "{:?}", asked.borrow());
}

#[test]
fn the_guard_is_asked_of_a_remote_picture_and_a_socket() {
    let asked: Asked = Rc::default();
    let ran: Rc<RefCell<Vec<String>>> = Rc::default();
    let (told, fetched, connected) = (asked.clone(), ran.clone(), ran.clone());
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"local socket = ui.socket("wss://x.test/live")
            return ui.column {
                ui.image { src = "https://x.test/a.png", id = "a", w = 20, h = 20 },
                ui.text { id = "error", function()
                    local error = socket.error
                    return error and error.message or ""
                end },
            }"#,
        );
        host.fetcher(move |request: zgx_script::Request| {
            fetched.borrow_mut().push(request.url.clone());
            async { Err("no network here".to_string()) }
        });
        host.socket_connector(move |request, _socket| {
            connected.borrow_mut().push(request.url.clone());
            async { Ok(()) }
        });
        host.guard(move |kind, name| {
            told.borrow_mut().push((kind, name.to_owned()));
            Err("not in a preview".into())
        });
        host
    });
    h.settle();
    h.frames(2);
    assert!(ran.borrow().is_empty(), "{:?}", ran.borrow());
    let asked = asked.borrow();
    assert!(
        asked.contains(&(GuardKind::Picture, "https://x.test/a.png".to_owned())),
        "{asked:?}"
    );
    assert!(
        asked.contains(&(GuardKind::Socket, "wss://x.test/live".to_owned())),
        "{asked:?}"
    );
    assert!(
        h.text("error").contains("not in a preview"),
        "{}",
        h.text("error")
    );
    assert!(error_of(&h).contains("not in a preview"), "{:?}", h.error());
}
