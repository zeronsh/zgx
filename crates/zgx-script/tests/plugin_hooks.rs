//! Scripts hooking into the root script's interface (`ui.contribute {
//! target, before | after | replace }`): any element by its id, a declared
//! hook or its role and name, matched as the interface is built. Any script
//! adds before or after, or replaces, any element.
mod common;
use common::Harness;
use std::path::PathBuf;
use zgx_script::{Script, ScriptHost};

fn script_file(name: &str, code: &str) -> PathBuf {
    // A directory of each call's own: tests run in parallel in one process,
    // and two writing one name's file would load each other's code.
    static MADE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = common::temp_dir(&format!("hooks-{n}-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    entry
}
fn with(app: &'static str, name: &str, code: &str) -> Harness {
    let entry = script_file(name, code);
    let name = name.to_string();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", app);
        host.set_scripts(vec![Script::file(name, entry)]);
        host
    })
}

const APP: &str = r##"
local shown = ui.state("shown", true)
return ui.column {
    ui.row {
        id = "toolbar",
        ui.button { id = "open", w = 60, h = 24, "Open" },
        ui.button { id = "save", hook = "toolbar.save", w = 60, h = 24, "Save" },
        ui.button { id = "delete", w = 60, h = 24, "Delete" },
        ui.button { id = "secret", w = 60, h = 24, "Secret" },
    },
    ui.show(shown, ui.column { ui.text { id = "later", "Later" } }),
    ui.button { id = "toggle", w = 60, h = 24, on_click = ui.toggle(shown), "Toggle" },
}
"##;

#[test]
fn a_script_adds_before_and_after_an_element_by_id() {
    let mut h = with(
        APP,
        "extra",
        r##"
ui.contribute { target = "#save", before = ui.text { id = "before", "Before" } }
ui.contribute { target = "#save", after = ui.text { id = "after", "After" } }
"##,
    );
    assert_eq!(h.error(), None);
    let texts = h.texts();
    let at = |t: &str| {
        texts
            .iter()
            .position(|x| x == t)
            .unwrap_or_else(|| panic!("{t} in {texts:?}"))
    };
    assert!(at("Open") < at("Before") && at("Before") < at("Save") && at("Save") < at("After"));
    assert!(at("After") < at("Delete"), "{texts:?}");
}

#[test]
fn a_selector_names_a_target_by_role() {
    let mut h = with(
        APP,
        "selector",
        r##"local target = ui.selector('role:button "Open"')
assert(target == 'role:button "Open"', "the text itself")
ui.contribute { target = target, after = ui.text { "After open" } }"##,
    );
    assert_eq!(h.error(), None);
    let texts = h.texts();
    let at = |t: &str| texts.iter().position(|x| x == t);
    assert_eq!(
        at("After open"),
        at("Open").map(|at| at + 1),
        "the selector found the button: {texts:?}"
    );
}

#[test]
fn a_script_naming_no_selector_makes_none() {
    let h = with(
        APP,
        "plain",
        r##"assert(rawget(ui, "selector") == nil, "made only for a script that names it")
ui.contribute { target = "#open", after = ui.text { "After open" } }"##,
    );
    assert_eq!(h.error(), None);
}

#[test]
fn a_declared_hook_and_a_role_and_name_find_the_element() {
    let mut h = with(
        APP,
        "finder",
        r##"
ui.contribute { target = "hook:toolbar.save", after = ui.text { "By hook" } }
ui.contribute { target = 'role:button "Open"', after = ui.text { "By name" } }
"##,
    );
    assert_eq!(h.error(), None);
    let texts = h.texts();
    let at = |t: &str| texts.iter().position(|x| x == t).unwrap();
    assert_eq!(at("By name"), at("Open") + 1, "{texts:?}");
    assert_eq!(at("By hook"), at("Save") + 1, "{texts:?}");
}

#[test]
fn every_element_is_in_reach() {
    let mut h = with(
        APP,
        "anywhere",
        r##"
ui.contribute { target = "#delete", after = ui.text { "Next to delete" } }
ui.contribute { target = '"Secret"', before = ui.text { "Next to secret" } }
"##,
    );
    assert_eq!(h.error(), None);
    let texts = h.texts();
    let at = |t: &str| texts.iter().position(|x| x == t).unwrap();
    assert_eq!(at("Next to delete"), at("Delete") + 1, "{texts:?}");
    assert_eq!(at("Next to secret") + 1, at("Secret"), "{texts:?}");
}

#[test]
fn any_script_replaces_an_element() {
    let mut h = with(
        APP,
        "swap",
        r##"ui.contribute { target = "#save", replace = ui.button { id = "mine", w = 60, h = 24, "Mine" } }"##,
    );
    assert_eq!(h.error(), None);
    assert!(h.find("save").is_none());
    assert!(h.find("mine").is_some());
}

/// What a script adds is as the root script's own would be: zgx describes
/// it as nobody's.
#[test]
fn what_a_script_adds_carries_no_description_of_zgxs() {
    let mut h = with(
        APP,
        "lookalike",
        r##"ui.contribute { target = "#save", after = ui.button { id = "confirm", w = 60, h = 24, "Confirm" } }"##,
    );
    let node = h.find("confirm").expect("contributed");
    let semantics = h.ui.semantics.borrow();
    let described = semantics.get(node).and_then(|s| s.description.clone());
    assert_eq!(described, None);
}

#[test]
fn a_contribution_goes_and_comes_back_with_its_element() {
    let mut h = with(
        APP,
        "follower",
        r##"ui.contribute { target = "#later", after = ui.text { id = "tag", "Tag" } }"##,
    );
    assert!(h.find("tag").is_some());
    h.click("toggle");
    assert!(h.find("later").is_none());
    assert!(h.find("tag").is_none(), "gone with its element");
    h.click("toggle");
    assert!(h.find("tag").is_some(), "back with it");
}

#[test]
fn a_target_known_only_once_shown_is_refused() {
    let h = with(
        APP,
        "late",
        r##"ui.contribute { target = "#toolbar row 2", after = ui.text { "x" } }"##,
    );
    let error = h.error().unwrap_or_default();
    assert!(
        error.contains("matched as the interface is built"),
        "{error}"
    );
}

/// `private` and `hook = false` are gone: either is an error saying so.
#[test]
fn private_and_hook_false_are_errors_saying_they_are_gone() {
    let runtime = zgx::reactive::Runtime::new();
    for (prop, says) in [
        (
            "private = true",
            "`private` is gone, with no replacement: every script reaches every element; \
             leave it out",
        ),
        (
            "hook = false",
            "`hook = false` is gone, with no replacement: every script reaches every \
             element; leave it out",
        ),
    ] {
        let code = format!(r#"return ui.column {{ {prop}, ui.text {{ "Hi" }} }}"#);
        let error = ScriptHost::inline(&runtime, "app", code)
            .reload()
            .unwrap_err();
        assert!(error.contains(says), "{prop}: {error}");
    }
}

#[test]
fn what_a_show_or_a_switch_shows_is_replaced_or_filtered() {
    const SHOWS: &str = r##"
local empty = ui.state("empty", true)
local mode = ui.state("mode", "a")
return ui.column {
    ui.show(empty, ui.text { id = "none", "Nothing yet" }),
    ui.switch(mode, { a = ui.text { id = "mode-a", "Mode A" } }),
}
"##;
    let mut h = with(
        SHOWS,
        "shown",
        r##"
ui.contribute { target = "#none", replace = ui.text { id = "mine", "Mine instead" } }
ui.filter { target = "#mode-a", text = function(t) return t .. "!" end }
"##,
    );
    assert_eq!(h.error(), None);
    assert!(h.find("none").is_none() && h.find("mine").is_some());
    assert!(
        h.texts().contains(&"Mode A!".to_string()),
        "{:?}",
        h.texts()
    );
}
