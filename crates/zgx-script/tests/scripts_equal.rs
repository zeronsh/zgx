//! Every script is equal: one the host loads beside its root script
//! (`set_scripts`) reaches all the root's does and targets any of its
//! elements, goes without a trace when the host drops it, runs within the
//! limits its host sets and no others, and the host knows which script calls
//! it.
mod common;
use common::Harness;
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use zgx_script::{Limits, Script, ScriptHost, testing::TestApp};

/// `code` as a script's own file, in a folder of its own.
fn script_file(name: &str, code: &str) -> PathBuf {
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = common::temp_dir(&format!("scripts-equal-{n}-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    entry
}

const APP: &str = r#"
local secret = ui.state("secret", "hunter2")
local ticks = ui.shared("ticks", 0)
return ui.column {
    ui.text { id = "ticks", "{ticks}" },
    ui.column { id = "settings",
        ui.input { id = "password", value = secret },
        ui.button { id = "delete", "Delete everything", on_click = function() end },
    },
    ui.slot("bar", { id = "bar" }),
}
"#;

fn app_with(scripts: Vec<Script>) -> Harness {
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_scripts(scripts);
        host
    })
}

/// A script replaces and filters any of the root's elements, with nothing
/// granted or exposed: a password field and a destructive button alike.
#[test]
fn a_script_targets_any_element_of_the_roots() {
    let mut h = app_with(vec![Script::code(
        "mine",
        r##"ui.contribute { target = "#password", replace = ui.text { id = "mine", "replaced" } }
        ui.filter { target = "#delete", text = function() return "Mine now" end }"##,
    )]);
    assert_eq!(h.text("mine"), "replaced");
    assert!(
        h.find("password").is_none(),
        "the field it replaced is gone"
    );
    assert_eq!(h.text("delete"), "Mine now");
    assert!(h.error().is_none(), "{:?}", h.error());
}

/// `private` and `hook = false`, which kept elements out of reach, are gone:
/// each is an error saying so, with nothing to use instead.
#[test]
fn private_and_hook_false_are_errors_saying_they_are_gone() {
    for prop in ["private = true", "hook = false"] {
        let code = format!(r#"return ui.text {{ id = "x", {prop}, "x" }}"#);
        let error = TestApp::try_new(move |runtime| ScriptHost::inline(runtime, "app", code))
            .err()
            .unwrap_or_default();
        assert!(
            error.contains("is gone, with no replacement"),
            "{prop}: {error}"
        );
    }
}

/// A script the host stops loading leaves nothing running: its timer stops
/// ticking and what it added goes, from the reload that drops it on.
#[test]
fn a_dropped_script_leaves_nothing_running() {
    let ticker = Script::code(
        "ticker",
        r#"local ticks = ui.shared("ticks", 0)
        ui.every(10, function() ticks:set(ticks:peek() + 1) return true end)
        ui.contribute("bar", ui.text { id = "ticking", "ticking" })"#,
    );
    let mut h = app_with(vec![ticker]);
    h.frames(3);
    let ticked: u32 = h.text("ticks").parse().unwrap();
    assert!(ticked > 0, "it ticks while loaded");
    assert_eq!(h.text("ticking"), "ticking");
    h.host.set_scripts(Vec::new());
    h.host.reload().unwrap();
    h.settle();
    let after: u32 = h.text("ticks").parse().unwrap();
    h.frames(5);
    assert_eq!(h.text("ticks"), after.to_string(), "its timer went with it");
    assert!(h.find("ticking").is_none(), "what it added went with it");
}

/// A script has no limits unless its host sets them: one holds 40 MB (past
/// the 32 a plugin had by default), and one given 8 MB fails to load.
#[test]
fn a_script_has_no_limits_unless_set() {
    assert_eq!(
        Limits::default(),
        Limits {
            memory: None,
            load: None,
            call: None,
            voices: None,
            count_state: false,
        }
    );
    let big = r#"local s = string.rep("x", 40 * 1024 * 1024)
        ui.contribute("bar", ui.text { id = "big", tostring(#s) })"#;
    let mut h = app_with(vec![Script::file("big", script_file("big", big))]);
    assert_eq!(h.text("big"), (40 * 1024 * 1024).to_string());
    let limited = Limits {
        memory: Some(8 << 20),
        ..Limits::default()
    };
    let mut h = app_with(vec![
        Script::file("small", script_file("small", big)).limits(limited),
    ]);
    assert!(h.find("big").is_none(), "it fails to load");
    let error = h.error().unwrap_or_default();
    assert!(error.contains("memory"), "{error}");
}

/// A heap limit alone caps the VM's heap and counts nothing a script's
/// states hold in Rust: twenty states of 1 MB fit under 8 MB. Asked to count
/// them too (`count_state`, the meter), they don't: the VM is left what
/// they don't hold, and the script fails for want of memory.
#[test]
fn a_heap_limit_alone_counts_no_state() {
    let many = r#"for i = 1, 20 do ui.state("s" .. i, string.rep("x", 1024 * 1024)) end
        ui.contribute("bar", ui.text { id = "held", "held" })"#;
    let heap = Limits {
        memory: Some(8 << 20),
        ..Limits::default()
    };
    let mut h = app_with(vec![
        Script::file("many", script_file("many", many)).limits(heap),
    ]);
    assert_eq!(h.text("held"), "held", "{:?}", h.error());
    let counted = Limits {
        count_state: true,
        ..heap
    };
    let mut h = app_with(vec![
        Script::file("counted", script_file("counted", many)).limits(counted),
    ]);
    assert!(h.find("held").is_none(), "its states don't fit");
    let error = h.error().unwrap_or_default();
    assert!(error.contains("memory"), "{error}");
}

/// A host action asked which script calls it hears the root's and each
/// other script's name, each call its own; outside a call, none.
#[test]
fn a_host_action_knows_which_script_calls_it() {
    let heard: Rc<RefCell<Vec<Option<String>>>> = Rc::default();
    let log = heard.clone();
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r#"return ui.column {
                ui.button { id = "root", "Root", on_click = ui.action("who") },
                ui.slot("bar", { id = "bar" }),
            }"#,
        );
        let asking = host.clone();
        host.action("who", move |_| {
            log.borrow_mut()
                .push(asking.calling_script().map(|name| name.to_string()));
        });
        host.set_scripts(vec![
            Script::code(
                "one",
                r#"ui.contribute("bar", ui.button { id = "one", "One", on_click = function() ui.invoke("who") end })"#,
            ),
            Script::code(
                "two",
                r#"ui.contribute("bar", ui.button { id = "two", "Two", on_click = ui.action("who") })"#,
            ),
        ]);
        host
    });
    h.click("two");
    h.click("root");
    h.click("one");
    let names: Vec<Option<String>> = heard.borrow().clone();
    assert_eq!(
        names,
        [Some("two".into()), Some("app".into()), Some("one".into())]
    );
    assert_eq!(h.host.calling_script(), None, "no call under way");
}

/// What a script kept as a plugin (`plugin:<name>`) is its own as a script
/// (`script:<name>`): moved there the first time it loads, and read.
#[test]
fn a_scripts_state_kept_as_a_plugins_is_moved_to_its_own() {
    let dir = common::temp_dir("scripts-equal-persist");
    std::fs::create_dir_all(dir.join("plugin%3Anotes")).unwrap();
    std::fs::write(dir.join("plugin%3Anotes/count.json"), "7").unwrap();
    let entry = script_file(
        "notes",
        r#"local count = ui.persist("count", 0)
        ui.contribute("bar", ui.text { id = "count", "{count}" })"#,
    );
    let storage = dir.clone();
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", APP).storage(storage);
        host.set_scripts(vec![Script::file("notes", entry)]);
        host
    });
    assert_eq!(h.text("count"), "7");
    assert!(dir.join("script%3Anotes/count.json").is_file());
    assert!(!dir.join("plugin%3Anotes").exists(), "moved, not copied");
}
