//! An app's scripts from files held in memory (`ScriptHost::from_files`),
//! swapped for others in place (`retarget`) and tried first in a host of
//! their own (`validate`): state kept across a swap, and nothing left of a
//! load that fails.
mod common;
use common::Harness;
use std::{sync::Arc, time::Duration};
use zgx_script::{Code, Limits, MemoryFiles, ScriptFiles, ScriptHost, Value, build};

fn memory(name: &str, files: &[(&str, &[u8])]) -> Arc<MemoryFiles> {
    Arc::new(MemoryFiles::new(
        name,
        files
            .iter()
            .map(|(path, bytes)| (path.to_string(), bytes.to_vec())),
    ))
}
fn files(name: &str, files: &[(&str, &str)]) -> ScriptFiles {
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, t)| (*p, t.as_bytes())).collect();
    ScriptFiles::Memory(memory(name, &files))
}
fn state(h: &Harness, key: &str) -> Option<Value> {
    h.host
        .states()
        .into_iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}
const ONE: &str = r#"local count = ui.state("count", 1)
ui.command { id = "bump", run = function() count:add(1) end }
return ui.text { "one {count}", id = "label" }"#;
fn one(runtime: &zgx::reactive::Runtime) -> ScriptHost {
    ScriptHost::from_files(
        runtime,
        files("one", &[("app.luau", ONE)]),
        "app.luau",
        Code::Source,
    )
}

#[test]
fn scripts_from_memory_run_and_a_retarget_keeps_their_state() {
    let mut h = Harness::new(one);
    assert_eq!(h.text("label"), "one 1");
    assert!(h.host.run_command("bump"));
    h.settle();
    assert_eq!(h.text("label"), "one 2");
    h.host
        .retarget(
            files(
                "two",
                &[
                    (
                        "app.luau",
                        r#"local greet = require("./greet")
local count = ui.state("count", 1)
return ui.text { greet("two") .. " {count}", id = "label" }"#,
                    ),
                    ("greet.luau", "return function(name) return name end"),
                ],
            ),
            "app.luau",
        )
        .unwrap();
    h.settle();
    assert_eq!(h.text("label"), "two 2", "its state kept");
}

#[test]
fn a_retarget_that_fails_changes_nothing() {
    let mut h = Harness::new(one);
    assert!(h.host.run_command("bump"));
    h.settle();
    let error = h
        .host
        .retarget(
            files(
                "broken",
                &[(
                    "app.luau",
                    r#"local count = ui.state("count", 1)
count:set(99)
ui.state("made", 5)
ui.shared("made.shared", 7)
error("boom")"#,
                )],
            ),
            "app.luau",
        )
        .unwrap_err();
    assert!(error.message.contains("boom"), "{error}");
    assert!(!error.timed_out);
    h.settle();
    assert_eq!(h.text("label"), "one 2", "the scripts it had, as they were");
    assert_eq!(state(&h, "count"), Some(Value::Num(2.)), "its write undone");
    assert_eq!(state(&h, "made"), None, "a key it made gone");
    assert_eq!(state(&h, "made.shared"), None, "a shared key too");
    // Its own files still: a reload loads them, and it runs on.
    h.host.reload().unwrap();
    assert!(h.host.run_command("bump"));
    h.settle();
    assert_eq!(h.text("label"), "one 3");
}

#[test]
fn a_reload_that_fails_undoes_what_its_scripts_wrote() {
    let mut h = Harness::new(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r#"local fail = ui.shared("fail", false)
local count = ui.state("count", 0)
count:add(1)
local seen = ui.persist("seen", 0)
if fail:get() then
    seen:set(5)
    ui.state("made", 1)
    ui.persist("fresh", 3)
    error("boom")
end
return ui.text { "{count}", id = "label" }"#,
        )
    });
    assert_eq!(h.text("label"), "1");
    h.host.shared("fail", false).set(Value::Bool(true));
    assert!(h.host.reload().is_err());
    h.settle();
    assert_eq!(h.text("label"), "1", "the interface it had");
    assert_eq!(
        state(&h, "count"),
        Some(Value::Num(1.)),
        "a changed key back"
    );
    assert_eq!(
        state(&h, "seen"),
        Some(Value::Num(0.)),
        "a persisted one too"
    );
    assert_eq!(state(&h, "made"), None, "a key it made gone");
    assert_eq!(state(&h, "fresh"), None, "a persisted key it made gone");
    // Once it loads again, it writes as before.
    h.host.shared("fail", false).set(Value::Bool(false));
    h.host.reload().unwrap();
    h.settle();
    assert_eq!(h.text("label"), "2");
}

#[test]
fn validate_tries_scripts_in_a_host_of_their_own() {
    let mut h = Harness::new(one);
    assert!(
        h.host
            .validate(files("same", &[("app.luau", ONE)]), "app.luau")
            .is_ok()
    );
    let error = h
        .host
        .validate(
            files("bad", &[("app.luau", r#"error("boom")"#)]),
            "app.luau",
        )
        .unwrap_err();
    assert!(
        error.message.contains("boom") && !error.timed_out,
        "{error}"
    );
    // What it writes as it loads is its own: none of it here.
    h.host
        .validate(
            files(
                "writes",
                &[(
                    "app.luau",
                    r#"ui.shared("from.trial", 1)
ui.state("count", 1):set(50)
return ui.text { "trial" }"#,
                )],
            ),
            "app.luau",
        )
        .unwrap();
    h.settle();
    assert_eq!(state(&h, "from.trial"), None);
    assert_eq!(state(&h, "count"), Some(Value::Num(1.)));
    assert_eq!(h.text("label"), "one 1");
}

#[test]
fn a_load_stopped_for_its_time_is_told_from_one_that_fails() {
    let h = Harness::new(|runtime| {
        one(runtime).limits(Limits {
            load: Duration::from_millis(20),
            ..Limits::default()
        })
    });
    let error = h
        .host
        .validate(
            files("spin", &[("app.luau", "while true do end")]),
            "app.luau",
        )
        .unwrap_err();
    assert!(error.timed_out, "{error}");
    let error = h
        .host
        .retarget(
            files("spin", &[("app.luau", "while true do end")]),
            "app.luau",
        )
        .unwrap_err();
    assert!(error.timed_out, "{error}");
}

#[test]
fn files_swapped_out_are_let_go() {
    let first = memory("first", &[("app.luau", ONE.as_bytes())]);
    let held = Arc::downgrade(&first);
    let mut h = Harness::new(|runtime| {
        ScriptHost::from_files(
            runtime,
            ScriptFiles::Memory(first),
            "app.luau",
            Code::Source,
        )
    });
    assert_eq!(h.text("label"), "one 1");
    h.host
        .retarget(files("second", &[("app.luau", ONE)]), "app.luau")
        .unwrap();
    h.settle();
    assert!(held.upgrade().is_none(), "nothing holds the first files");
}

#[test]
fn under_code_source_bytecode_in_memory_is_refused() {
    let bytecode = build::compile(br#"return ui.text { "hi", id = "label" }"#).unwrap();
    let runtime = zgx::reactive::Runtime::new();
    let host = ScriptHost::from_files(
        &runtime,
        ScriptFiles::Memory(memory("bytecode", &[("app.luau", &bytecode)])),
        "app.luau",
        Code::Source,
    );
    let error = host.reload().unwrap_err();
    assert!(error.contains("binary chunk"), "{error}");
    let host = ScriptHost::from_files(
        &runtime,
        ScriptFiles::Memory(memory("bytecode", &[("app.luau", &bytecode)])),
        "app.luau",
        Code::VerifiedBytecode,
    );
    assert!(host.reload().is_ok(), "trusted, it loads");
}

fn at(root: &str, files: &[(&str, &str)]) -> ScriptFiles {
    ScriptFiles::Memory(Arc::new(MemoryFiles::new_at(
        root,
        files
            .iter()
            .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec())),
    )))
}
const SAYS: &str = r#"ui.i18n {}
return ui.text { ui.t("hello"), id = "label" }"#;

/// A live bundle's new strings: the catalog is read again from the files
/// swapped in, under the same root (`MemoryFiles::new_at`) or another, and
/// the old one is back if the swap fails.
#[test]
fn a_retarget_reads_the_catalog_from_the_new_files() {
    let mut h = Harness::new(|runtime| {
        ScriptHost::from_files(
            runtime,
            at(
                "/app",
                &[
                    ("app.luau", SAYS),
                    ("locales/en.json", r#"{ "hello": "Hello" }"#),
                ],
            ),
            "app.luau",
            Code::Source,
        )
    });
    assert_eq!(h.text("label"), "Hello");
    // The same root, new strings.
    let same = at(
        "/app",
        &[
            ("app.luau", SAYS),
            ("locales/en.json", r#"{ "hello": "Hi" }"#),
        ],
    );
    h.host.retarget(same, "app.luau").unwrap();
    h.settle();
    assert_eq!(h.text("label"), "Hi");
    // Another root.
    let other = at(
        "/next",
        &[
            ("app.luau", SAYS),
            ("locales/en.json", r#"{ "hello": "Hey" }"#),
        ],
    );
    h.host.retarget(other, "app.luau").unwrap();
    h.settle();
    assert_eq!(h.text("label"), "Hey");
    // A swap that fails leaves the strings as they were.
    let broken = at(
        "/next",
        &[
            ("app.luau", "ui.i18n {}\nerror('no')"),
            ("locales/en.json", r#"{ "hello": "Nope" }"#),
        ],
    );
    assert!(h.host.retarget(broken, "app.luau").is_err());
    h.settle();
    assert_eq!(h.text("label"), "Hey");
}

/// Files in memory under the root the app's first ones had: its state and
/// persisted values keyed as before, and its scripts' paths the same.
#[test]
fn files_in_memory_can_keep_the_apps_root() {
    let files = at("/anara-ui/", &[("app.luau", ONE)]);
    let ScriptFiles::Memory(memory) = &files else {
        unreachable!()
    };
    assert_eq!(memory.name(), "anara-ui");
    let mut h =
        Harness::new(|runtime| ScriptHost::from_files(runtime, files, "app.luau", Code::Source));
    assert!(h.host.run_command("bump"));
    h.settle();
    let again = at("/anara-ui", &[("app.luau", ONE)]);
    h.host.retarget(again, "app.luau").unwrap();
    h.settle();
    assert_eq!(h.text("label"), "one 2");
}
