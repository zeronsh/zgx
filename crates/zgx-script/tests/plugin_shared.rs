//! Shared state a host keeps (`ScriptHost::shared`), as a script beside the
//! root reaches it: it reads and follows it, uses it where a state is only
//! read (a list's items), and writes it (`set`, `update`, `push`,
//! `remove`) as the root script does, each side seeing the other's writes.
mod common;
use common::Harness;
use std::path::PathBuf;
use zgx_script::{Script, ScriptHost, Value};

fn script(name: &str, code: &str) -> PathBuf {
    // A directory of each call's own: tests run in parallel in one process,
    // and two writing one name's file would load each other's code.
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = common::temp_dir(&format!("shared-{n}-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    entry
}

const APP: &str = r##"
local tasks = ui.shared("tasks", {})
return ui.column {
    ui.slot("panel"),
    ui.text { id = "app-count", function() return `{tasks:len()} in the app` end },
    ui.button {
        id = "add",
        w = 60,
        h = 24,
        on_click = function() tasks:push("from the app") end,
        "Add",
    },
}
"##;

/// The app, with `tasks` shared, and one script beside it.
fn with(name: &str, code: &str) -> Harness {
    let entry = script(name, code);
    let name = name.to_string();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.shared("tasks", Value::from_json(r#"["one", "two"]"#).unwrap());
        host.set_scripts(vec![Script::file(name, entry)]);
        host
    })
}

#[test]
fn a_script_reads_and_follows_shared_state() {
    let mut h = with(
        "count",
        r##"
local tasks = ui.shared("tasks", {})
ui.contribute("panel", ui.text { id = "count", function() return `{tasks:len()} tasks` end })
ui.contribute("panel", ui.each(tasks, function(task) return ui.text { task } end))
"##,
    );
    assert_eq!(h.error(), None);
    assert_eq!(h.text("count"), "2 tasks");
    assert!(h.texts().contains(&"two".to_string()));
    h.click("add");
    assert_eq!(
        h.text("count"),
        "3 tasks",
        "the app writes it; the script follows"
    );
    assert!(h.texts().contains(&"from the app".to_string()));
}

#[test]
fn every_write_from_a_script_reaches_the_app() {
    for (write, count) in [
        (r#"tasks:set({})"#, 0),
        (r#"tasks:push("mine")"#, 3),
        (r#"tasks:update(function(list) return list end)"#, 2),
        (r#"tasks:remove(1)"#, 1),
    ] {
        let mut h = with(
            "writer",
            &format!(
                r##"
local tasks = ui.shared("tasks", {{}})
ui.contribute("panel", ui.button {{ id = "write", w = 60, h = 24, on_click = function() {write} end, "Write" }})
"##
            ),
        );
        assert_eq!(h.error(), None);
        assert_eq!(h.text("app-count"), "2 in the app");
        h.click("write");
        assert_eq!(h.error(), None, "{write}");
        assert_eq!(
            h.text("app-count"),
            format!("{count} in the app"),
            "{write}"
        );
    }
}
