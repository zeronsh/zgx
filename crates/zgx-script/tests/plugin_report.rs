//! `ScriptHost::contributions`: each plugin contribution's target, place,
//! the elements it applies at now and its latest error, for a host or an
//! agent to tell a target that matches nothing from one that works.
mod common;
use common::Harness;
use zgx_script::{Plugin, ScriptHost};

fn with(test: &str, app: &'static str, code: &str) -> Harness {
    let dir = std::env::temp_dir().join(format!("zgx-report-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", app);
        host.set_plugins(vec![Plugin {
            name: "marks".into(),
            entry,
            grants: vec![zgx_script::Capability::Override("*".into())],
        }]);
        host
    })
}
/// (target, place, matches, has an error), in the order made.
fn report(h: &Harness) -> Vec<(String, &'static str, usize, bool)> {
    h.host
        .contributions()
        .into_iter()
        .map(|r| {
            assert_eq!(r.plugin, "marks");
            (r.target, r.place, r.matches, r.error.is_some())
        })
        .collect()
}

const APP: &str = r##"
local shown = ui.state("shown", true)
local names = ui.state("names", { "a", "b" })
ui.command { id = "toggle", run = function() shown:set(not shown:get()) end }
return ui.column {
    ui.button { id = "save", w = 60, h = 24, "Save" },
    ui.show(shown, ui.text { id = "later", "Later" }),
    ui.each(names, function(name)
        return ui.row { id = "row", ui.text { id = "name", name } }
    end),
}
"##;

#[test]
fn each_contribution_says_where_it_applies_now() {
    let h = with(
        "now",
        APP,
        r##"
ui.contribute { target = "#save", after = ui.text { "a" } }
ui.contribute { target = "#nowhere", before = ui.text { "b" } }
ui.contribute { target = "#later", replace = ui.text { "c" } }
ui.contribute { target = "#name", after = ui.text { "d" } }
ui.filter { target = "#save", text = function(t) return t end }
ui.contribute { target = "#row", after = ui.text { "e" } }
"##,
    );
    h.frames(1);
    assert_eq!(
        report(&h),
        vec![
            ("#save".to_string(), "after", 1, false),
            ("#nowhere".to_string(), "before", 0, false),
            ("#later".to_string(), "replace", 1, false),
            ("#name".to_string(), "after", 2, false),
            ("#save".to_string(), "filter", 1, false),
            ("#row".to_string(), "after", 0, true),
        ],
        "a target matching nothing reads 0; one in each row, a match a row"
    );
    // Hidden, the element takes nothing: its count follows.
    assert!(h.host.run_command("toggle"));
    h.frames(1);
    assert_eq!(report(&h)[2], ("#later".to_string(), "replace", 0, false));
    assert!(h.host.run_command("toggle"));
    h.frames(1);
    assert_eq!(report(&h)[2], ("#later".to_string(), "replace", 1, false));
}

#[test]
fn without_plugins_there_is_nothing_to_report() {
    let h = Harness::inline(r#"return ui.text { "plain" }"#);
    assert!(h.host.contributions().is_empty());
}
