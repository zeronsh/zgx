//! A script replacing one of the root script's modules (`ui.override`):
//! only the modules the root requires as `@app/…`, the replacement running
//! in place of it and reaching the original as `@app-original/…`.
mod common;
use common::Harness;
use std::path::{Path, PathBuf};
use zgx_script::{Script, ScriptHost};

/// A directory of files, made fresh for one test.
fn files(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = common::temp_dir(&format!("overrides-{name}"));
    for (path, code) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, code).unwrap();
    }
    dir
}

const CARD: &str = r#"return function(title) return ui.text { id = "card", title } end"#;

/// The app: a card from `@app/card`, and the same module by a relative path.
fn app(name: &str) -> PathBuf {
    files(
        &format!("{name}-app"),
        &[
            (
                "main.luau",
                r#"
local card = require("@app/card")
local private = require("./card")
return ui.column { card("Inbox"), ui.row { id = "private", private("Sent") } }
"#,
            ),
            ("card.luau", CARD),
        ],
    )
    .join("main.luau")
}

/// A script replacing `@app/card` with one that wraps the original.
fn script(name: &str, script_code: &str) -> PathBuf {
    files(
        &format!("{name}-script"),
        &[
            ("init.luau", script_code),
            (
                "card.luau",
                r#"
local original = require("@app-original/card")
return function(title)
    return ui.row { id = "wrapped", original(title), ui.text { id = "badge", "New" } }
end
"#,
            ),
        ],
    )
    .join("init.luau")
}

fn host(app: &Path, entry: PathBuf) -> Harness {
    let app = app.to_path_buf();
    Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, &app);
        host.set_scripts(vec![Script::file("badges", entry)]);
        host
    })
}

const OVERRIDE: &str = r#"ui.override { module = "@app/card", with = "./card" }"#;

#[test]
fn a_script_replaces_a_root_module() {
    let mut h = host(&app("replaces"), script("replaces", OVERRIDE));
    h.ui.prepare_frame();
    assert_eq!(h.error(), None);
    // The replacement wraps the original, required as `@app-original/card`.
    assert!(h.find("wrapped").is_some());
    assert_eq!(h.text("badge"), "New");
    assert_eq!(h.text("card"), "Inbox");
}

/// A relative require is the root script's own: no script replaces it.
#[test]
fn a_relative_require_stays_the_roots() {
    let mut h = host(&app("relative"), script("relative", OVERRIDE));
    h.ui.prepare_frame();
    let private = h.find("private").unwrap();
    let scene = h.ui.scene.borrow();
    let mut texts = Vec::new();
    let mut pending = vec![private];
    while let Some(id) = pending.pop() {
        pending.extend(scene.children(id).iter().copied());
        if let zgx::scene::NodeKind::Text { text, .. } = scene.kind(id) {
            texts.push(text.to_string());
        }
    }
    assert_eq!(texts, ["Sent"], "the original card alone, unwrapped");
}

/// `@app-original/…` is the root script's module whoever requires it, the
/// root script included; `@app/…` is every script's.
#[test]
fn every_script_requires_the_roots_modules_and_their_originals() {
    let app = files(
        "original-app",
        &[
            (
                "main.luau",
                r#"return require("@app-original/card")("Inbox")"#,
            ),
            ("card.luau", CARD),
        ],
    )
    .join("main.luau");
    let mut h = Harness::new(move |runtime| ScriptHost::new(runtime, &app));
    assert_eq!(h.text("card"), "Inbox");
    // A script beside the root requires its modules.
    let mut h = host_with(
        "script-requires",
        r##"local card = require("@app/card")
ui.contribute { target = "#private", after = card("Beside") }"##,
    );
    h.ui.prepare_frame();
    assert_eq!(h.error(), None);
    assert!(h.texts().contains(&"Beside".to_string()), "{:?}", h.texts());
}

fn host_with(name: &str, script_code: &str) -> Harness {
    host(&app(name), script(name, script_code))
}

/// `app` and `app-original` are zgx's aliases: a `.luaurc` defining one is
/// told to rename it, with the file named.
#[test]
fn a_luaurc_cannot_take_the_apps_alias() {
    let app = files(
        "reserved-app",
        &[
            ("main.luau", r#"return ui.text { "Hi" }"#),
            (".luaurc", r#"{ "aliases": { "app": "./lib" } }"#),
        ],
    )
    .join("main.luau");
    let runtime = zgx::reactive::Runtime::new();
    let error = ScriptHost::new(&runtime, &app).reload().unwrap_err();
    assert!(
        error.contains(".luaurc") && error.contains("`app` is zgx's own alias"),
        "{error}"
    );
}

/// A replacement is one of the script's own files; `module` is one of the
/// root script's `@app/…` modules.
#[test]
fn an_override_names_a_root_module_and_a_file_of_the_script() {
    for (code, says) in [
        (
            r#"ui.override { module = "./card", with = "./card" }"#,
            "only the root script's `@app/…` modules",
        ),
        (
            r#"ui.override { module = "@app/card", with = "../../elsewhere" }"#,
            "the replacement is one of the script's own files",
        ),
    ] {
        let h = host_with("names", code);
        h.ui.prepare_frame();
        let error = h.error().unwrap_or_default();
        assert!(error.contains(says), "{code}: {error}");
    }
}
