//! What plugins add beside the root of a branch: a `ui.show`'s or
//! `ui.switch`'s, or a component's root. The additions are siblings in the
//! parent's flow, come and go with the branch, and read what the root
//! provides, live. Beside a presence's content they enter and leave with
//! it. Where a root can't have siblings (the app's, a window's), the
//! contribution reports an error.
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

/// Every text the app shows with where it is, in tree order.
fn placed(h: &mut Harness) -> Vec<(String, zgx::scene::Rect)> {
    h.ui.prepare_frame();
    let scene = h.ui.scene.borrow();
    let mut out = Vec::new();
    let mut stack = vec![scene.root()];
    while let Some(n) = stack.pop() {
        if let zgx::scene::NodeKind::Text { text, .. } = scene.kind(n) {
            out.push((text.to_string(), scene.bounds(n)));
        }
        stack.extend(scene.children(n).iter().rev());
    }
    out
}
fn rects_of(placed: &[(String, zgx::scene::Rect)], text: &str) -> Vec<zgx::scene::Rect> {
    placed
        .iter()
        .filter(|(t, _)| t == text)
        .map(|(_, r)| *r)
        .collect()
}

const PAGES: &str = r##"
local page = ui.state("page", "a")
ui.command { id = "next", run = function() page:set("b") end }
return ui.row {
    id = "app", gap = 10, items = "start",
    ui.text { "before" },
    ui.presence(page, function(p) return ui.row { id = "header", ui.text { p } } end,
        { exit = { opacity = 0, duration = 100 } }),
    ui.text { "after" },
}
"##;

#[test]
fn beside_a_presences_content_it_enters_and_leaves_with_it() {
    let mut h = with(
        "presence",
        PAGES,
        r##"ui.contribute { target = "#header", after = ui.text { "x" } }"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.host.contributions()[0].error, None);
    assert_eq!(h.texts(), ["before", "a", "x", "after"]);
    let at = placed(&mut h);
    let (a, x, after) = (
        rects_of(&at, "a")[0],
        rects_of(&at, "x")[0],
        rects_of(&at, "after")[0],
    );
    // A sibling in the row's flow, with its gap, not stacked under it.
    assert_eq!(x.y, a.y);
    assert_eq!(x.x, a.x + a.width + 10.);
    assert_eq!(after.x, x.x + x.width + 10.);

    assert!(h.host.run_command("next"));
    let mut frames = 0;
    loop {
        h.frame();
        let now = placed(&mut h);
        let (leaving, xs) = (rects_of(&now, "a"), rects_of(&now, "x"));
        // What was added leaves with what it was added beside: in the same
        // frame, not one early or late. Until then it stays where it was.
        assert_eq!(xs.len(), 1 + leaving.len(), "frame {frames}: {now:?}");
        if leaving.is_empty() {
            break;
        }
        assert_eq!(leaving, [a], "frame {frames}");
        assert!(
            xs.contains(&x),
            "frame {frames}: the leaving one in place: {xs:?}"
        );
        frames += 1;
        assert!(frames < 30, "the exit ends");
    }
    assert!(frames >= 5, "it played its exit: {frames} frames");
    // The entering content and its addition in the flow, as before.
    let now = placed(&mut h);
    assert_eq!(h.texts(), ["before", "b", "x", "after"]);
    let (b, x) = (rects_of(&now, "b")[0], rects_of(&now, "x")[0]);
    assert_eq!(x.x, b.x + b.width + 10.);
    assert_eq!(h.host.contributions()[0].matches, 1);
}

/// Without plugins, what leaves stays exactly where it was on every frame
/// of its exit, and what enters takes its place in the flow.
#[test]
fn without_plugins_a_presences_exit_keeps_its_place_every_frame() {
    let mut h = Harness::inline(PAGES);
    h.frames(1);
    let before = placed(&mut h);
    let (a, after) = (rects_of(&before, "a")[0], rects_of(&before, "after")[0]);
    assert!(h.host.run_command("next"));
    let mut frames = 0;
    loop {
        h.frame();
        let now = placed(&mut h);
        assert_eq!(rects_of(&now, "after"), [after], "frame {frames}");
        let leaving = rects_of(&now, "a");
        if leaving.is_empty() {
            break;
        }
        assert_eq!(leaving, [a], "frame {frames}");
        frames += 1;
        assert!(frames < 30, "the exit ends");
    }
    assert!(frames >= 5, "it played its exit: {frames} frames");
}

#[test]
fn beside_the_apps_root_is_an_error() {
    let mut h = with(
        "approot",
        PAGES,
        r##"ui.contribute { target = "#app", before = ui.text { "y" } }"##,
    );
    h.frames(1);
    let report = h.host.contributions();
    assert!(
        report[0]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("the app's root")),
        "{report:?}"
    );
    assert!(!h.texts().iter().any(|t| t == "y"));
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
