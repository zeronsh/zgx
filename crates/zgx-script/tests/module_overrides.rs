//! A plugin replacing one of the app's modules (`ui.override`): only the
//! modules the app requires as `@app/…`, with `Capability::OverrideModule` (a trust
//! grant: the replacement runs as the app's own code), reaching the
//! original as `@app-original/…` from the replacement alone.
mod common;
use common::Harness;
use std::path::{Path, PathBuf};
use zgx_script::{Capability, Plugin, ScriptHost};

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

/// A plugin replacing `@app/card` with one that wraps the original.
fn plugin(name: &str, plugin_code: &str) -> PathBuf {
    files(
        &format!("{name}-plugin"),
        &[
            ("init.luau", plugin_code),
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

fn host(app: &Path, entry: PathBuf, grants: Vec<Capability>) -> Harness {
    let app = app.to_path_buf();
    Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, &app);
        host.set_plugins(vec![Plugin {
            name: "badges".into(),
            entry,
            grants,
            app_root: None,
        }]);
        host
    })
}

const OVERRIDE: &str = r#"ui.override { module = "@app/card", with = "./card" }"#;

#[test]
fn a_plugin_replaces_an_app_module_it_is_trusted_with() {
    let app = app("trusted");
    let mut h = host(
        &app,
        plugin("trusted", OVERRIDE),
        vec![Capability::OverrideModule("@app/card".into())],
    );
    h.ui.prepare_frame();
    assert_eq!(h.error(), None);
    // The replacement wraps the original, required as `@app-original/card`.
    assert!(h.find("wrapped").is_some());
    assert_eq!(h.text("badge"), "New");
    assert_eq!(h.text("card"), "Inbox");
    // `OverrideModule("*")` trusts the plugin with every module.
    let mut any = host(
        &app,
        plugin("any", OVERRIDE),
        vec![Capability::OverrideModule("*".into())],
    );
    any.ui.prepare_frame();
    assert!(any.find("wrapped").is_some());
}

#[test]
fn without_the_grant_the_app_keeps_its_module() {
    let mut h = host(&app("ungranted"), plugin("ungranted", OVERRIDE), Vec::new());
    h.ui.prepare_frame();
    let error = h.error().unwrap_or_default();
    assert!(
        error.contains("needs OverrideModule(\"@app/card\")") && error.contains("trust grant"),
        "{error}"
    );
    assert!(h.find("wrapped").is_none());
    assert_eq!(h.text("card"), "Inbox");
}

/// A relative require is the app's own: no plugin replaces it.
#[test]
fn a_relative_require_stays_the_apps() {
    let mut h = host(
        &app("relative"),
        plugin("relative", OVERRIDE),
        vec![Capability::OverrideModule("*".into())],
    );
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

/// `@app-original/…` is the replacement's alone: not the app's, not a
/// plugin's.
#[test]
fn the_original_is_reached_from_its_replacement_alone() {
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
    let runtime = zgx::reactive::Runtime::new();
    let host = ScriptHost::new(&runtime, &app);
    let error = host.reload().unwrap_err();
    assert!(
        error.contains("for the module that overrides it alone"),
        "{error}"
    );
    // A plugin requiring the app's modules is refused.
    let h = host_with(
        "plugin-requires",
        r#"local card = require("@app/card")"#,
        vec![Capability::OverrideModule("*".into())],
    );
    h.ui.prepare_frame();
    let error = h.error().unwrap_or_default();
    assert!(
        error.contains("a plugin replaces with ui.override"),
        "{error}"
    );
}

fn host_with(name: &str, plugin_code: &str, grants: Vec<Capability>) -> Harness {
    host(&app(name), plugin(name, plugin_code), grants)
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

/// A replacement is one of the plugin's own files; `module` is one of the
/// app's `@app/…` modules.
#[test]
fn an_override_names_an_app_module_and_a_plugin_file() {
    for (code, says) in [
        (
            r#"ui.override { module = "./card", with = "./card" }"#,
            "only the app's `@app/…` modules",
        ),
        (
            r#"ui.override { module = "@app/card", with = "../../elsewhere" }"#,
            "one of the plugin's own files",
        ),
    ] {
        let h = host_with("names", code, vec![Capability::OverrideModule("*".into())]);
        h.ui.prepare_frame();
        let error = h.error().unwrap_or_default();
        assert!(error.contains(says), "{code}: {error}");
    }
}

/// An app whose `ds/map` moved into a plugin (`ds/` keeps a module of its
/// own), and the plugin: its `app/` laid out as the app's root.
fn moved(name: &str, plugin_init: &str) -> (PathBuf, PathBuf) {
    let app = files(
        &format!("{name}-app"),
        &[
            (
                "main.luau",
                r#"
local map = require("@app/ds/map")
local own = require("./ds/own")
return ui.column { map(), own() }
"#,
            ),
            (
                "ds/own.luau",
                r#"
local open = ui.state("open", "app")
return function() return ui.text { id = "own", "{open}" } end
"#,
            ),
        ],
    );
    let plugin = files(
        &format!("{name}-plugin"),
        &[
            ("init.luau", plugin_init),
            (
                "app/ds/map.luau",
                r#"
local parts = require("./parts")
local tone = require("@self/../shared/tone")
local open = ui.state("open", "map")
local saved = ui.persist("saved", "kept")
return function()
    return ui.row { ui.text { id = "map", "{open} {saved}" }, parts(), tone() }
end
"#,
            ),
            (
                "app/ds/parts.luau",
                r#"
local own = require("./own")
local count = ui.state("count", 3)
return function() return ui.column { ui.text { id = "parts", "{count}" }, own() } end
"#,
            ),
            // A name the app's own file has: the app's stays its own.
            (
                "app/ds/own.luau",
                r#"
local open = ui.state("open", "plugin")
return function() return ui.text { id = "plugin-own", "{open}" } end
"#,
            ),
            (
                "app/shared/tone.luau",
                r#"
local tone = ui.state("tone", "calm")
return function() return ui.text { id = "tone", "{tone}" } end
"#,
            ),
        ],
    );
    (app.join("main.luau"), plugin)
}

const MOVED: &str = r#"ui.override { module = "@app/ds/map", with = "./app/ds/map" }"#;

fn moved_host(name: &str, grants: Vec<Capability>, app_root: bool) -> (Harness, PathBuf) {
    let (app, plugin) = moved(name, MOVED);
    let entry = plugin.join("init.luau");
    let root = app_root.then(|| plugin.join("app"));
    let h = Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, &app);
        host.set_plugins(vec![Plugin {
            name: "moved".into(),
            entry,
            grants,
            app_root: root,
        }]);
        host
    });
    h.ui.prepare_frame();
    (h, plugin.canonicalize().unwrap())
}

fn names(h: &Harness) -> Vec<String> {
    h.host.states().into_iter().map(|(name, _)| name).collect()
}

/// A plugin's `app_root`: the modules the app runs from it are named by
/// their path in it, as the app's are, and so are their states; their
/// requires still resolve from where their files are.
#[test]
fn modules_from_a_plugins_app_root_are_named_as_the_apps() {
    let (mut h, plugin) = moved_host(
        "app-root",
        vec![Capability::OverrideModule("@app/ds/map".into())],
        true,
    );
    assert_eq!(h.error(), None);
    assert_eq!(h.text("map"), "map kept");
    assert_eq!(h.text("parts"), "3");
    assert_eq!(h.text("tone"), "calm");
    // The plugin's `ds/own` keeps its full path: the app has its own.
    assert_eq!(h.text("own"), "app");
    assert_eq!(h.text("plugin-own"), "plugin");
    let own = format!("{}::open", plugin.join("app/ds/own.luau").display());
    let mut want = vec![
        "ds/map.luau::open".to_owned(),
        "ds/map.luau::saved".to_owned(),
        "ds/own.luau::open".to_owned(),
        "ds/parts.luau::count".to_owned(),
        "shared/tone.luau::tone".to_owned(),
        own,
    ];
    want.sort();
    assert_eq!(names(&h), want);
}

/// Without one, a plugin's modules are named by their full path, as before.
#[test]
fn without_an_app_root_a_plugins_modules_keep_their_paths() {
    let (h, plugin) = moved_host(
        "no-app-root",
        vec![Capability::OverrideModule("@app/ds/map".into())],
        false,
    );
    assert_eq!(h.error(), None);
    let full = |file: &str, key: &str| format!("{}::{key}", plugin.join(file).display());
    let mut want = vec![
        full("app/ds/map.luau", "open"),
        full("app/ds/map.luau", "saved"),
        full("app/ds/own.luau", "open"),
        full("app/ds/parts.luau", "count"),
        full("app/shared/tone.luau", "tone"),
        "ds/own.luau::open".to_owned(),
    ];
    want.sort();
    assert_eq!(names(&h), want);
}

/// A host of the moved app and its plugin, loaded once: its error, if any.
fn moved_load(
    name: &str,
    grants: Vec<Capability>,
    edit: impl FnOnce(&Path),
) -> (ScriptHost, Result<(), String>) {
    let (app, plugin) = moved(name, MOVED);
    edit(&plugin);
    let runtime = zgx::reactive::Runtime::new();
    let host = ScriptHost::new(&runtime, &app);
    host.set_plugins(vec![Plugin {
        name: "moved".into(),
        entry: plugin.join("init.luau"),
        grants,
        app_root: Some(plugin.join("app")),
    }]);
    let loaded = host.reload();
    (host, loaded)
}

/// An error in such a module names it by its path in the `app_root`, not
/// by a folder of the machine's.
#[test]
fn an_app_root_modules_error_names_it_as_the_apps() {
    let (_, loaded) = moved_load(
        "app-root-error",
        vec![Capability::OverrideModule("*".into())],
        |plugin| std::fs::write(plugin.join("app/ds/parts.luau"), r#"error("broken")"#).unwrap(),
    );
    let error = loaded.unwrap_err();
    assert!(error.contains("ds/parts.luau:1: broken"), "{error}");
    assert!(!error.contains("app-root-error-plugin"), "{error}");
}

/// An `app_root` names only what the app runs as its own, which takes the
/// trust grant: a plugin without it replaces nothing, and none of its
/// files runs as the app's, by that name or another.
#[test]
fn an_app_root_is_nothing_without_the_trust_grant() {
    let (host, loaded) = moved_load("app-root-untrusted", Vec::new(), |_| {});
    let error = loaded.unwrap_err();
    assert!(
        error.contains("require(\"@app/ds/map\")") && error.contains("app-root-untrusted-app"),
        "the app's own module, missing: {error}"
    );
    let names: Vec<String> = host.states().into_iter().map(|(name, _)| name).collect();
    assert!(
        names.iter().all(|name| !name.contains("ds/map")),
        "{names:?}"
    );
}

/// The plugin's own VM names its files by their path in its folder, as
/// before: an `app_root` is for what the app runs.
#[test]
fn a_plugins_own_requires_keep_their_names() {
    let (app, plugin) = moved(
        "app-root-own",
        &format!(
            "{MOVED}\nlocal tone = require(\"./app/shared/tone\")\n\
             ui.contribute {{ target = \"#own\", after = tone() }}"
        ),
    );
    let entry = plugin.join("init.luau");
    let mut h = Harness::new(move |runtime| {
        let host = ScriptHost::new(runtime, &app);
        host.set_plugins(vec![Plugin {
            name: "moved".into(),
            entry,
            grants: vec![Capability::OverrideModule("*".into())],
            app_root: Some(plugin.join("app")),
        }]);
        host
    });
    h.ui.prepare_frame();
    assert_eq!(h.error(), None);
    assert_eq!(h.text("map"), "map kept");
    let names = names(&h);
    assert!(
        names.iter().any(|n| n == "app/shared/tone.luau::tone")
            && names.iter().any(|n| n == "shared/tone.luau::tone"),
        "{names:?}"
    );
}
