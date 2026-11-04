//! What a host asks of a plugin it runs: where what it added shows
//! (`ScriptHost::contribution_bounds`), and forgetting its state as it is
//! uninstalled (`ScriptHost::forget_state`).
mod common;

use common::Harness;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
};
use zgx_script::{Capability, Plugin, ScriptHost};

fn temp(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "zgx-plugin-bf-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn plugin(name: &str, code: &str, grants: Vec<Capability>) -> Plugin {
    let entry = temp(name).join("init.luau");
    std::fs::write(&entry, code).unwrap();
    Plugin {
        name: name.into(),
        entry,
        grants,
    }
}

const APP: &str = r##"return ui.column {
    ui.text { id = "save", "Save" },
    ui.text { id = "title", "Title" },
    ui.text { id = "other", "Other" },
    ui.slot("bar", { id = "bar" }),
}"##;
const ADDS: &str = r##"
ui.contribute("bar", ui.div { id = "in-slot", w = 30, h = 10 })
ui.contribute { target = "#save", after = ui.div { id = "beside", w = 40, h = 12 } }
ui.contribute { target = "#other", replace = ui.div { id = "instead", w = 50, h = 14 } }
ui.filter { target = "#title", text = function(t) return t .. "!" end }
"##;

fn sorted(mut rects: Vec<zgx::scene::Rect>) -> Vec<(f32, f32, f32, f32)> {
    rects.sort_by(|a, b| a.width.total_cmp(&b.width));
    rects
        .into_iter()
        .map(|r| (r.x, r.y, r.width, r.height))
        .collect()
}

#[test]
fn a_plugins_contributions_say_where_they_show() {
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_plugins(vec![
            plugin("adds", ADDS, vec![Capability::Override("*".into())]),
            plugin("quiet", "", vec![]),
        ]);
        host
    });
    assert_eq!(h.error(), None);
    h.ui.prepare_frame();
    // Each box it added, where the scene has it. A filter's is the app's.
    let want: Vec<_> = sorted(vec![
        h.bounds("in-slot"),
        h.bounds("beside"),
        h.bounds("instead"),
    ]);
    assert_eq!(sorted(h.host.contribution_bounds("adds")), want);
    assert!(h.host.contribution_bounds("quiet").is_empty());
    assert!(h.host.contribution_bounds("nobody").is_empty());
    // Gone with the plugin.
    h.host.set_plugins(vec![]);
    h.host.reload().unwrap();
    h.settle();
    assert!(h.host.contribution_bounds("adds").is_empty());
}

#[test]
fn what_a_plugin_gives_its_page_says_where_it_shows_too() {
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(
            runtime,
            "app",
            r##"return ui.sealed { id = "settings", w = 300, h = 200,
                ui.plugin_page { plugin = "mine", w = 300, h = 100 },
            }"##,
        );
        host.set_plugins(vec![plugin(
            "mine",
            r##"ui.contribute("plugin_page", ui.div { id = "my-page", w = 100, h = 30 })"##,
            vec![],
        )]);
        host
    });
    assert_eq!(h.error(), None);
    h.ui.prepare_frame();
    let want = sorted(vec![h.bounds("my-page")]);
    assert_eq!(sorted(h.host.contribution_bounds("mine")), want);
}

#[test]
fn a_contribution_that_goes_takes_its_box_with_it() {
    let mut h = Harness::new(|runtime| {
        let host = ScriptHost::inline(runtime, "app", APP);
        host.set_plugins(vec![plugin(
            "shows",
            r##"
local on = ui.state("on", true)
ui.contribute("bar", ui.show(on, ui.div { id = "maybe", w = 30, h = 10 }))
ui.contribute("bar", ui.button { id = "off", w = 20, h = 10, on_click = ui.set(on, false) })
"##,
            vec![],
        )]);
        host
    });
    assert_eq!(h.error(), None);
    h.ui.prepare_frame();
    let before = h.host.contribution_bounds("shows").len();
    h.click("off");
    h.settle();
    h.ui.prepare_frame();
    assert!(h.find("maybe").is_none());
    // The `ui.show` stays mounted, empty: a box of no area is left out.
    let after = h.host.contribution_bounds("shows");
    assert!(after.len() < before, "{before} then {after:?}");
    assert!(after.iter().all(|r| r.width > 0. && r.height > 0.));
}

const REMEMBERS: &str = r##"
local kept = ui.persist("kept", 1)
local plain = ui.state("plain", 1)
ui.contribute("bar", ui.column {
    ui.text { id = "NAME-kept", function() return tostring(kept:get()) end },
    ui.text { id = "NAME-plain", function() return tostring(plain:get()) end },
    ui.button { id = "NAME-bump", w = 10, h = 10, on_click = function()
        kept:set(kept:peek() + 1)
        plain:set(plain:peek() + 1)
    end },
})
"##;

fn remembering(dir: PathBuf, entries: [&Plugin; 2]) -> Harness {
    let plugins: Vec<Plugin> = entries.into_iter().cloned().collect();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", APP).storage(dir);
        host.set_plugins(plugins);
        host
    })
}

#[test]
fn forgetting_a_plugins_state_leaves_nothing_of_it_and_others_alone() {
    let dir = temp("storage");
    let gone = plugin("gone", &REMEMBERS.replace("NAME", "gone"), vec![]);
    let stays = plugin("stays", &REMEMBERS.replace("NAME", "stays"), vec![]);
    let mut h = remembering(dir.clone(), [&gone, &stays]);
    assert_eq!(h.error(), None);
    h.click("gone-bump");
    h.click("stays-bump");
    h.settle();
    assert_eq!(h.text("gone-kept"), "2");
    assert_eq!(h.text("gone-plain"), "2");
    h.host.save();
    let folder = dir.join("plugin%3Agone");
    assert!(folder.is_dir(), "{:?}", std::fs::read_dir(&dir));
    // A change not yet saved is forgotten too, and never written after.
    h.click("gone-bump");
    h.settle();
    // Uninstalled: it goes, then its state.
    h.host.set_plugins(vec![stays.clone()]);
    h.host.reload().unwrap();
    h.settle();
    h.host.forget_state("plugin:gone");
    assert!(!folder.exists());
    assert!(dir.join("plugin%3Astays").is_dir());
    h.host.save();
    assert!(
        !folder.exists(),
        "a write queued before it can't bring it back"
    );
    // Installed again: from its `initial`. The other kept its own.
    h.host.set_plugins(vec![gone.clone(), stays.clone()]);
    h.host.reload().unwrap();
    h.settle();
    assert_eq!(h.text("gone-kept"), "1");
    assert_eq!(h.text("gone-plain"), "1");
    assert_eq!(h.text("stays-kept"), "2");
    assert_eq!(h.text("stays-plain"), "2");
    // What stayed goes on saving.
    h.click("stays-bump");
    h.settle();
    h.host.save();
    drop(h);
    // And after a relaunch.
    let mut h = remembering(dir.clone(), [&gone, &stays]);
    assert_eq!(h.text("gone-kept"), "1");
    assert_eq!(h.text("stays-kept"), "3");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_never_reaches_outside_its_own_folder() {
    let dir = temp("names");
    let bystander = dir.join("kept.txt");
    std::fs::write(&bystander, "kept").unwrap();
    let h = Harness::new(|runtime| {
        ScriptHost::inline(runtime, "app", "return ui.text { \"app\" }").storage(dir.clone())
    });
    for name in ["", ".", "..", "../..", "/", "plugin:../.."] {
        h.host.forget_state(name);
    }
    assert!(bystander.exists());
    assert!(dir.is_dir());
    let _ = std::fs::remove_dir_all(&dir);
}
