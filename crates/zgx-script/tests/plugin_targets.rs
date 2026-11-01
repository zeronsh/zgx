//! Plugin targets by class and inside other elements (`.class`, `a >> b`),
//! matched as the interface is built: a target matching several elements
//! contributes at each, and a chain only inside its ancestors.
mod common;
use common::Harness;
use zgx_script::{Capability, Plugin, ScriptHost};

fn with(test: &str, app: &'static str, code: &str) -> Harness {
    let dir = std::env::temp_dir().join(format!("zgx-targets-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", app);
        host.set_plugins(vec![Plugin {
            name: "marks".into(),
            entry,
            grants: vec![Capability::Override("*".into())],
        }]);
        host
    })
}
fn count(h: &mut Harness, text: &str) -> usize {
    h.frames(1);
    h.texts().iter().filter(|t| t.as_str() == text).count()
}

const APP: &str = r##"
local sessions = ui.state("sessions", { "one", "two" })
return ui.row {
    ui.column {
        id = "sidebar",
        ui.button { class = "tool", "New" },
        ui.each(sessions, function(name)
            return ui.row { ui.text { class = "session-row", name } }
        end),
    },
    ui.column {
        id = "main",
        ui.button { class = "tool", "Send" },
        ui.text { class = "session-row", "Elsewhere" },
    },
}
"##;

#[test]
fn a_class_matches_every_element_that_has_it() {
    let mut h = with(
        "class",
        APP,
        r##"ui.contribute { target = ".tool", after = ui.text { "Mark" } }"##,
    );
    assert_eq!(count(&mut h, "Mark"), 2, "one beside each .tool");
    assert_eq!(h.host.contributions()[0].matches, 2);
    assert_eq!(h.error(), None);
}

#[test]
fn a_chain_matches_only_inside_its_ancestors() {
    let mut h = with(
        "chain",
        APP,
        r##"ui.contribute { target = "#sidebar >> .session-row", after = ui.text { "Mark" } }"##,
    );
    assert_eq!(
        count(&mut h, "Mark"),
        2,
        "beside the sidebar's two rows' texts, not main's"
    );
    let report = &h.host.contributions()[0];
    assert_eq!(report.target, "#sidebar >> .session-row");
    assert_eq!(report.matches, 2);
    assert_eq!(h.error(), None);
}

#[test]
fn a_chain_that_holds_nowhere_matches_nothing() {
    let mut h = with(
        "nowhere",
        APP,
        r##"ui.contribute { target = "#main >> #sidebar >> .tool", after = ui.text { "Mark" } }"##,
    );
    assert_eq!(count(&mut h, "Mark"), 0);
    assert_eq!(h.host.contributions()[0].matches, 0);
}

#[test]
fn a_chain_replaces_and_filters_only_inside_its_ancestors() {
    let mut h = with(
        "replace",
        APP,
        r##"
ui.contribute { target = "#main >> .tool", replace = ui.text { "Replaced" } }
ui.filter { target = "#sidebar >> .tool", text = function(t) return t .. "!" end }
"##,
    );
    assert_eq!(
        count(&mut h, "Replaced"),
        1,
        "main's tool, not the sidebar's"
    );
    assert_eq!(count(&mut h, "New!"), 1, "the sidebar's tool, filtered");
    assert_eq!(count(&mut h, "Send"), 0);
    assert_eq!(h.error(), None);
}

#[test]
fn a_test_target_says_classes_are_for_plugins() {
    let mut h = Harness::inline(r#"return ui.button { id = "b", class = "tool", "B" }"#);
    let error = h.target(".tool").unwrap_err();
    assert!(error.contains("plugins"), "{error}");
}
