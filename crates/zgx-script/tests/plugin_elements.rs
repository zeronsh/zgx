//! `ScriptHost::elements`: the app's named elements by the context they
//! read, for a plugin's author (or an agent) to see where it can put
//! something and what it can read there.
mod common;
use common::Harness;
use zgx_script::{ListedElement, Plugin, ScriptHost};

fn with(test: &str, app: &'static str, code: &str) -> Harness {
    let dir = std::env::temp_dir().join(format!("zgx-elements-{}-{test}", std::process::id()));
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

const APP: &str = r##"
local profile = ui.state("profile", { name = "Ada" })
local sessions = ui.state("sessions", { "one", "two", "three" })
local settings = ui.state("settings", false)
return ui.row {
    id = "app",
    provide = { profile = profile },
    ui.column {
        id = "sidebar",
        ui.each(sessions, function(name)
            return ui.row {
                class = "session-row",
                provide = { session = name },
                ui.button { class = "open", name },
            }
        end),
    },
    ui.column { id = "vault", private = true, ui.text { id = "secret", "x" } },
    ui.show(settings, ui.column { id = "settings", ui.text { class = "pref", "Prefs" } }),
}
"##;

fn named(id: Option<&str>, classes: &[&str], mounted: bool, count: usize) -> ListedElement {
    ListedElement {
        id: id.map(str::to_owned),
        classes: classes.iter().map(|c| c.to_string()).collect(),
        hook: None,
        private: false,
        mounted,
        count,
    }
}

#[test]
fn elements_are_grouped_by_the_context_they_read() {
    let h = with(
        "groups",
        APP,
        r##"ui.contribute { target = "#sidebar", before = ui.text { "x" } }"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None);
    let report = h.host.elements(&h.ui, &h.handle);
    assert!(report.live);
    let root = &report.root;
    assert_eq!(root.selector, "#app");
    assert_eq!(root.provides, ["profile"]);
    assert!(
        root.elements
            .contains(&named(Some("sidebar"), &[], true, 1))
    );
    assert!(
        root.elements.contains(&ListedElement {
            private: true,
            ..named(Some("vault"), &[], true, 1)
        }),
        "private, and nothing inside it: {:?}",
        root.elements
    );
    assert!(
        !root
            .elements
            .iter()
            .any(|e| e.id.as_deref() == Some("secret"))
    );
    assert!(
        root.elements
            .contains(&named(Some("settings"), &[], false, 1))
            && root.elements.contains(&named(None, &["pref"], false, 1)),
        "a page not open, from the source: {:?}",
        root.elements
    );
    assert_eq!(
        root.groups.len(),
        1,
        "three alike rows are one: {:?}",
        root.groups
    );
    let rows = &root.groups[0];
    assert_eq!(rows.count, 3);
    assert_eq!(rows.selector, "#sidebar >> .session-row");
    assert_eq!(rows.provides, ["session"]);
    assert_eq!(rows.inherited, ["profile"]);
    assert_eq!(
        rows.elements,
        [
            named(None, &["session-row"], true, 1),
            named(None, &["open"], true, 1)
        ]
    );
    let json = report.to_value().to_json();
    assert!(
        json.contains(r##""selector":"#sidebar >> .session-row""##),
        "{json}"
    );
}

#[test]
fn without_plugins_the_source_is_listed() {
    let h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", APP));
    let report = h.host.elements(&h.ui, &h.handle);
    assert!(!report.live, "nothing mounted is tracked");
    assert!(
        report
            .root
            .elements
            .iter()
            .any(|e| e.id.as_deref() == Some("sidebar"))
    );
}
