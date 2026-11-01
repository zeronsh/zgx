//! What plugins add beside the root of a branch: a `ui.show`'s or
//! `ui.switch`'s, or a component's root. The additions are siblings in the
//! parent's flow, come and go with the branch, and read what the root
//! provides, live. Where a root can't have siblings (a presence's, the
//! app's), the contribution reports an error.
mod common;
use common::Harness;
use zgx_script::{Plugin, ScriptHost};

fn with(test: &str, app: &'static str, code: &str) -> Harness {
    let dir = std::env::temp_dir().join(format!("zgx-branches-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", app);
        host.set_plugins(vec![Plugin {
            name: "marks".into(),
            entry,
            grants: vec![],
        }]);
        host
    })
}
fn shown(h: &mut Harness) -> Vec<String> {
    h.frames(1);
    h.texts()
}

const BADGE: &str = r##"
local Badge = ui.component(function()
    local session = ui.use_context("session")
    return ui.text { function() return `[{session:get()}]` end }
end)
ui.contribute { target = "#header", after = Badge {} }
"##;

#[test]
fn beside_a_shown_branch_root_it_comes_goes_and_reads_the_roots_context() {
    let mut h = with(
        "show",
        r##"
local open = ui.state("open", true)
local session = ui.state("session", "one")
ui.command { id = "toggle", run = function() open:set(not open:get()) end }
ui.command { id = "rename", run = function() session:set("two") end }
return ui.row {
    ui.text { "before" },
    ui.show(open, ui.row { id = "header", provide = { session = session }, ui.text { "Header" } }),
    ui.text { "after" },
}
"##,
        BADGE,
    );
    assert_eq!(h.error(), None);
    assert_eq!(shown(&mut h), ["before", "Header", "[one]", "after"]);
    assert_eq!(h.host.contributions()[0].matches, 1);
    // A sibling in the row's flow, not inside the header.
    let (header, badge) = (h.find("header").unwrap(), h.find("\"[one]\"").unwrap());
    let (header, badge) = {
        let scene = h.ui.scene.borrow();
        (scene.bounds(header), scene.bounds(badge))
    };
    assert!(
        badge.x >= header.x + header.width,
        "{badge:?} after {header:?}"
    );
    assert!(h.host.run_command("rename"));
    assert_eq!(
        shown(&mut h),
        ["before", "Header", "[two]", "after"],
        "live"
    );
    assert!(h.host.run_command("toggle"));
    assert_eq!(shown(&mut h), ["before", "after"], "gone with the branch");
    assert_eq!(h.host.contributions()[0].matches, 0);
    assert!(h.host.run_command("toggle"));
    assert_eq!(
        shown(&mut h),
        ["before", "Header", "[two]", "after"],
        "back with it"
    );
}

#[test]
fn beside_a_switchs_case_root_it_follows_the_case() {
    let mut h = with(
        "switch",
        r##"
local page = ui.state("page", "chat")
local session = ui.state("session", "one")
ui.command { id = "files", run = function() page:set("files") end }
ui.command { id = "chat", run = function() page:set("chat") end }
return ui.row {
    ui.switch(page, {
        chat = ui.row { id = "header", provide = { session = session }, ui.text { "Chat" } },
        files = ui.text { "Files" },
    }),
}
"##,
        BADGE,
    );
    assert_eq!(h.error(), None);
    assert_eq!(shown(&mut h), ["Chat", "[one]"]);
    assert!(h.host.run_command("files"));
    assert_eq!(shown(&mut h), ["Files"], "gone with its case");
    assert!(h.host.run_command("chat"));
    assert_eq!(shown(&mut h), ["Chat", "[one]"]);
}

#[test]
fn beside_a_components_root_it_reads_the_roots_context() {
    let mut h = with(
        "component",
        r##"
local session = ui.state("session", "one")
local Header = ui.component(function()
    return ui.row { id = "header", provide = { session = session }, ui.text { "Header" } }
end)
return ui.row { ui.text { "before" }, Header {}, ui.text { "after" } }
"##,
        BADGE,
    );
    assert_eq!(h.error(), None);
    assert_eq!(shown(&mut h), ["before", "Header", "[one]", "after"]);
    assert_eq!(h.host.contributions()[0].matches, 1);
}

#[test]
fn beside_a_presences_content_or_the_apps_root_is_an_error() {
    let mut h = with(
        "presence",
        r##"
local page = ui.state("page", "a")
return ui.row {
    id = "app",
    ui.presence(page, function(p) return ui.row { id = "header", ui.text { p } } end),
}
"##,
        r##"
ui.contribute { target = "#header", after = ui.text { "x" } }
ui.contribute { target = "#app", before = ui.text { "y" } }
"##,
    );
    h.frames(1);
    let report = h.host.contributions();
    assert!(
        report[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("the content of a ui.presence")),
        "{report:?}"
    );
    assert!(
        report[1]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("the app's root")),
        "{report:?}"
    );
    assert!(!h.texts().iter().any(|t| t == "x" || t == "y"));
}

#[test]
fn beside_a_windows_root_is_an_error() {
    let h = with(
        "window",
        r##"
ui.window { id = "settings", title = "Settings", width = 300, height = 200, open = ui.state("open", true),
    ui.column { id = "settings-root", ui.text { "Settings" } } }
return ui.text { "main" }
"##,
        r##"ui.contribute { target = "#settings-root", after = ui.text { "x" } }"##,
    );
    h.frames(1);
    let mut window = zgx::widgets::Ui::with_runtime(h.ui.runtime.clone(), 300., 200.);
    window.mount(h.host.window_view("settings"));
    window.prepare_frame();
    let report = h.host.contributions();
    assert!(
        report[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("a window's root")),
        "{report:?}"
    );
}
