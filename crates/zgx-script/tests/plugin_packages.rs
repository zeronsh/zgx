//! Packages lent to plugins (`Capability::Package`, `grant_all_plugins`):
//! a plugin requires a lent package's modules into its own VM, with state
//! of its own; they require only inside lent packages, lend no authority,
//! and follow the package's files as the app does.
mod common;
use common::Harness;
use std::path::{Path, PathBuf};
use zgx_script::{Capability, Plugin, ScriptHost};

/// A directory of files, made fresh for one test.
fn files(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zgx-lent-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (path, code) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    }
    dir
}

/// A design system: a badge, a counter module with state, and modules that
/// reach out of it.
fn ds(name: &str) -> PathBuf {
    files(
        &format!("{name}-ds"),
        &[
            (
                "badge.luau",
                r#"local tone = require("./tone")
return function(label) return ui.text { label .. " " .. tone.name } end"#,
            ),
            ("tone.luau", r#"return { name = "calm" }"#),
            ("count.luau", r#"return { n = 0 }"#),
            (
                "copy.luau",
                r#"return function() return pcall(ui.clipboard.write, "x") end"#,
            ),
            ("to_app.luau", r#"return require("@app/secret")"#),
            ("to_kit.luau", r#"return require("@kit/button")"#),
            ("to_plugin.luau", r#"return require("../plugin/own")"#),
        ],
    )
}

fn plugin(dir: &Path, name: &str, code: &str) -> Plugin {
    let entry = dir.join(format!("{name}.luau"));
    std::fs::write(&entry, code).unwrap();
    Plugin {
        name: name.into(),
        entry,
        grants: Vec::new(),
    }
}

const APP: &str = r#"
local count = require("@ds/count")
count.n += 1
return ui.column { id = "app", ui.text { id = "mine", "app " .. count.n } }
"#;

/// An app with `@ds` and `@kit`, and the plugins `make` gives, `@ds` lent
/// to every plugin.
fn host(name: &str, make: impl FnOnce(&Path) -> Vec<Plugin> + 'static) -> Harness {
    let ds = ds(name);
    let kit = files(
        &format!("{name}-kit"),
        &[("button.luau", r#"return "button""#)],
    );
    let app = files(&format!("{name}-app"), &[("app.luau", APP)]).join("app.luau");
    let dir = files(&format!("{name}-plugins"), &[]);
    Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, &app)
            .package("ds", &ds)
            .package("kit", &kit)
            .grant_all_plugins(Capability::Package("ds".into()));
        host.set_plugins(make(&dir));
        host
    })
}

#[test]
fn a_lent_package_is_required_into_each_plugins_own_vm() {
    let mut h = host("lent", |dir| {
        let code = |n| {
            format!(
                r##"local badge, count = require("@ds/badge"), require("@ds/count")
count.n += 1
ui.contribute {{ target = "#mine", after = ui.column {{ badge("p{n} " .. count.n) }} }}"##
            )
        };
        vec![plugin(dir, "one", &code(1)), plugin(dir, "two", &code(2))]
    });
    h.frames(1);
    assert_eq!(h.error(), None);
    // Each VM ran the module itself: its own table, each count 1.
    assert_eq!(h.texts(), ["app 1", "p1 1 calm", "p2 1 calm"]);
}

#[test]
fn an_unlent_package_or_the_apps_modules_say_what_is_missing() {
    for (name, code, says) in [
        (
            "unlent",
            r#"require("@kit/button")"#,
            "package `@kit` isn't lent to plugin `p`: the host lends it with \
             Capability::Package(\"kit\")",
        ),
        (
            "from-ds",
            r#"require("@ds/to_kit")"#,
            "package `@kit` isn't lent to plugin `p`",
        ),
        (
            "to-app",
            r#"require("@ds/to_app")"#,
            "`@app/…` names the app's modules",
        ),
        (
            "to-plugin",
            r#"require("@ds/to_plugin")"#,
            "a module of package `@ds` requires only inside it",
        ),
    ] {
        let mut h = host(name, move |dir| {
            std::fs::create_dir_all(dir.join("plugin")).unwrap();
            std::fs::write(dir.join("plugin/own.luau"), "return 1").unwrap();
            vec![plugin(dir, "p", code)]
        });
        h.frames(1);
        let error = h.error().unwrap_or_default();
        assert!(error.contains(says), "{name}: {error}");
        assert_eq!(h.text("mine"), "app 1", "{name}: the app runs");
    }
}

#[test]
fn a_lent_module_has_only_the_plugins_authority() {
    let mut h = host("authority", |dir| {
        vec![plugin(
            dir,
            "p",
            r##"local copy = require("@ds/copy")
local ok, err = copy()
ui.contribute { target = "#mine", after = ui.text { id = "said", ok and "copied" or tostring(err) } }"##,
        )]
    });
    h.frames(1);
    assert_eq!(h.error(), None);
    let said = h.text("said");
    assert!(
        said.contains("Clipboard"),
        "denied by the plugin's grants: {said}"
    );
}

#[test]
fn an_edit_to_a_lent_package_reaches_its_plugins() {
    let mut h = host("edit", |dir| {
        vec![plugin(
            dir,
            "p",
            r##"local badge = require("@ds/badge")
ui.contribute { target = "#mine", after = ui.column { badge("p") } }"##,
        )]
    });
    h.frames(1);
    assert_eq!(h.texts(), ["app 1", "p calm"]);
    let tone =
        std::env::temp_dir().join(format!("zgx-lent-{}-edit-ds/tone.luau", std::process::id()));
    // A later modification time, as an editor's save gives.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&tone, r#"return { name = "bright" }"#).unwrap();
    assert!(h.host.reload_if_changed(), "the package's file is watched");
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.texts(), ["app 1", "p bright"]);
}

#[test]
fn contributions_list_what_each_plugin_was_lent() {
    let h = host("listed", |dir| {
        vec![plugin(dir, "p", r#"local badge = require("@ds/badge")"#)]
    });
    h.frames(1);
    let lent: Vec<_> = h
        .host
        .contributions()
        .into_iter()
        .filter(|r| r.place == "package")
        .map(|r| (r.plugin, r.target, r.matches))
        .collect();
    // `matches`: the modules it required from it (badge and tone).
    assert_eq!(lent, [("p".to_string(), "@ds".to_string(), 2)]);
}
