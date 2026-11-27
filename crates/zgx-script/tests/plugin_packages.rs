//! Packages from scripts beside the root: each requires any package's
//! modules into its own VM, with state of its own; a package's module
//! requires the other packages and the root script's `@app/…` modules, and
//! follows the package's files as the root's do.
mod common;
use common::Harness;
use std::path::{Path, PathBuf};
use zgx_script::{Script, ScriptHost};

/// A directory of files, made fresh for one test.
fn files(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = common::temp_dir_named(&format!("zgx-script-pkg-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (path, code) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    }
    dir
}

/// A design system: a badge, a counter module with state, and modules that
/// reach another package and the root script's modules.
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
            ("to_app.luau", r#"return require("@app/secret")"#),
            ("to_kit.luau", r#"return require("@kit/button")"#),
        ],
    )
}

fn script(dir: &Path, name: &str, code: &str) -> Script {
    let entry = dir.join(format!("{name}.luau"));
    std::fs::write(&entry, code).unwrap();
    Script::file(name, entry)
}

const APP: &str = r#"
local count = require("@ds/count")
count.n += 1
return ui.column { id = "app", ui.text { id = "mine", "app " .. count.n } }
"#;

/// An app with `@ds` and `@kit` and a module of its own, `@app/secret`, and
/// the scripts `make` gives beside it.
fn host(name: &str, make: impl FnOnce(&Path) -> Vec<Script> + 'static) -> Harness {
    let ds = ds(name);
    let kit = files(
        &format!("{name}-kit"),
        &[("button.luau", r#"return "button""#)],
    );
    let app = files(
        &format!("{name}-app"),
        &[("app.luau", APP), ("secret.luau", r#"return "secret""#)],
    )
    .join("app.luau");
    let dir = files(&format!("{name}-scripts"), &[]);
    Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, &app)
            .package("ds", &ds)
            .package("kit", &kit);
        host.set_scripts(make(&dir));
        host
    })
}

#[test]
fn a_package_is_required_into_each_scripts_own_vm() {
    let mut h = host("vms", |dir| {
        let code = |n| {
            format!(
                r##"local badge, count = require("@ds/badge"), require("@ds/count")
count.n += 1
ui.contribute {{ target = "#mine", after = ui.column {{ badge("p{n} " .. count.n) }} }}"##
            )
        };
        vec![script(dir, "one", &code(1)), script(dir, "two", &code(2))]
    });
    h.frames(1);
    assert_eq!(h.error(), None);
    // Each VM ran the module itself: its own table, each count 1.
    assert_eq!(h.texts(), ["app 1", "p1 1 calm", "p2 1 calm"]);
}

/// A script requires every package, and a package's module requires the
/// others and the root script's modules: nothing is lent.
#[test]
fn a_script_requires_every_package_and_the_roots_modules() {
    let mut h = host("reach", |dir| {
        vec![script(
            dir,
            "p",
            r##"local said = require("@kit/button") .. " " .. require("@ds/to_kit") .. " "
    .. require("@ds/to_app") .. " " .. require("@app/secret")
ui.contribute { target = "#mine", after = ui.text { id = "said", said } }"##,
        )]
    });
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("said"), "button button secret secret");
    assert_eq!(h.text("mine"), "app 1", "the root runs");
}

#[test]
fn an_edit_to_a_package_reaches_the_scripts_using_it() {
    let mut h = host("edit", |dir| {
        vec![script(
            dir,
            "p",
            r##"local badge = require("@ds/badge")
ui.contribute { target = "#mine", after = ui.column { badge("p") } }"##,
        )]
    });
    h.frames(1);
    assert_eq!(h.texts(), ["app 1", "p calm"]);
    let tone = std::env::temp_dir().join(format!(
        "zgx-script-pkg-{}-edit-ds/tone.luau",
        std::process::id()
    ));
    // A later modification time, as an editor's save gives.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&tone, r#"return { name = "bright" }"#).unwrap();
    assert!(h.host.reload_if_changed(), "the package's file is watched");
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.texts(), ["app 1", "p bright"]);
}
