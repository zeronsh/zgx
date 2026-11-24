//! `zgx.json`'s JSON Schema (`zgx.schema.json` beside this crate), made from
//! the tables its reader takes keys from: the window's keys, the look's
//! (`zgx_script::window_look::LOOK_KEYS`), the platforms' blocks, the
//! app's keys and its host's (`zgx_script::app_config::KEYS`). Editors
//! complete and check a `zgx.json` that names it (`"$schema"`); a test keeps
//! the file what the tables make, so it can't drift from what zgx reads.

use serde_json::{Map, Value, json};
use zgx_script::window_look::{Kind, LOOK_KEYS, Material, PLATFORMS};

/// The schema of `zgx.json`'s window key `key` (one of `WINDOW_KEYS`).
fn window_key(key: &str) -> Value {
    let number = |what: &str| json!({ "type": "number", "description": what });
    match key {
        "title" => {
            json!({ "type": "string", "description": "The window's title (the app's folder's name by default)." })
        }
        "width" => number("Its width in points (960 by default)."),
        "height" => number("Its height in points (640 by default)."),
        "min_width" => number("Its smallest width in points, given with min_height."),
        "min_height" => number("Its smallest height in points, given with min_width."),
        "max_width" => number("Its largest width in points, given with max_height."),
        "max_height" => number("Its largest height in points, given with max_width."),
        "titlebar" => json!({
            "enum": ["standard", "inset", "hidden"],
            "description": "standard, inset (content under a transparent title bar) or hidden."
        }),
        "caption_height" => json!({
            "type": "number",
            "exclusiveMinimum": 0,
            "description": "How tall inset's caption buttons are on Windows, in the content's points."
        }),
        "traffic_lights" => json!({
            "type": "object",
            "properties": { "x": { "type": "number" }, "y": { "type": "number" } },
            "required": ["x", "y"],
            "additionalProperties": false,
            "description": "Where inset's traffic lights go on macOS: the close button's top left in the content's points."
        }),
        "frame_autosave" => json!({
            "type": ["boolean", "string"],
            "description": "true: the window reopens where it was left, its frame saved under the app_id; or a name to save it under."
        }),
        "visible" => json!({
            "type": "boolean",
            "description": "false: it opens hidden, laid out and drawn, until the script shows it."
        }),
        other => panic!("zgx.json's window key `{other}` has no schema"),
    }
}

/// The schema of a look's key.
fn look_key(kind: Kind, doc: &str) -> Value {
    match kind {
        Kind::Bool => json!({ "type": "boolean", "description": doc }),
        Kind::Level => json!({ "enum": ["normal", "always_on_top"], "description": doc }),
        Kind::Glass => {
            let materials: Vec<&str> = Material::NAMES.iter().map(|(name, _)| *name).collect();
            json!({
                "description": doc,
                "oneOf": [
                    { "type": "boolean" },
                    {
                        "type": "object",
                        "properties": {
                            "material": { "enum": materials },
                            "plain": { "type": "boolean", "description": "The blur alone: no tint or saturation of the platform's." }
                        },
                        "additionalProperties": false
                    }
                ]
            })
        }
    }
}

/// Every window key's schema, once (`$defs`): its own keys and its look's.
fn window_key_defs() -> Map<String, Value> {
    let mut defs = Map::new();
    for key in crate::WINDOW_KEYS {
        defs.insert(key.into(), window_key(key));
    }
    for key in LOOK_KEYS {
        defs.insert(key.name.into(), look_key(key.kind, key.doc));
    }
    defs
}

/// A window's keys, each a reference to its schema in `$defs`.
fn window_keys() -> Map<String, Value> {
    crate::WINDOW_KEYS
        .iter()
        .copied()
        .chain(LOOK_KEYS.iter().map(|key| key.name))
        .map(|key| (key.to_owned(), json!({ "$ref": format!("#/$defs/{key}") })))
        .collect()
}

/// A window's keys with its platforms' blocks (`$defs/platform_block`).
fn window_with_blocks() -> Map<String, Value> {
    let mut keys = window_keys();
    for platform in PLATFORMS {
        keys.insert(
            platform.into(),
            json!({
                "$ref": "#/$defs/platform_block",
                "description": format!("Window keys on {platform}: each replaces the window's there, whole.")
            }),
        );
    }
    keys
}

/// The schema of the app's key `key`: one of `APP_KEYS` or
/// `zgx_script::app_config::KEYS`.
fn app_key(key: &str) -> Value {
    let names_and_paths = |what: &str| json!({ "type": "object", "additionalProperties": { "type": "string" }, "description": what });
    match key {
        "$schema" => json!({ "type": "string", "description": "This schema, for editors." }),
        "app_id" => {
            json!({ "type": "string", "description": "Where ui.persist keeps its data, and on Linux its windows' desktop file ID." })
        }
        "application_id" => {
            json!({ "type": "string", "description": "A reverse-DNS application ID: the system then hands the app the files and URLs it opens with it." })
        }
        "first" => {
            json!({ "type": "string", "description": "The window of named_windows that opens (zgx run --window opens another)." })
        }
        "named_windows" => json!({
            "type": "object",
            "description": "Windows by name, each with the window keys of the top level, which it starts from.",
            "additionalProperties": { "$ref": "#/$defs/named_window" }
        }),
        "packages" => names_and_paths(
            "Module folders outside the app it may require, by name: { \"kit\": \"../shared/kit\" }.",
        ),
        "modules" => json!({
            "type": "object",
            "propertyNames": { "pattern": "^[A-Za-z0-9_]+$" },
            "additionalProperties": { "type": "string" },
            "description": "The app's native modules, by name: each a crate's folder or a library's path."
        }),
        "version" => {
            json!({ "type": "string", "description": "The app's version, as zgx build gives its bundle." })
        }
        "expose" => json!({
            "type": "object",
            "description": "What plugins reach of the app.",
            "properties": {
                "shared": { "type": "object", "additionalProperties": { "enum": ["read", "write"] } },
                "actions": { "type": "array", "items": { "type": "string" } },
                "hooks": { "type": "object", "additionalProperties": { "enum": ["open", "override"] } }
            },
            "additionalProperties": false
        }),
        "plugins" => json!({
            "type": "object",
            "description": "The app's plugins, by name, each with what it is allowed.",
            "additionalProperties": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "allow": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["path"],
                "additionalProperties": false
            }
        }),
        other => panic!("zgx.json's key `{other}` has no schema"),
    }
}

/// `zgx.json`'s schema, as `zgx.schema.json` holds it.
pub fn schema() -> Value {
    let mut keys = window_with_blocks();
    for key in crate::APP_KEYS.iter().chain(zgx_script::app_config::KEYS) {
        keys.insert((*key).into(), app_key(key));
    }
    let mut defs = window_key_defs();
    defs.insert(
        "platform_block".into(),
        json!({ "type": "object", "properties": window_keys(), "additionalProperties": false }),
    );
    let mut named = window_with_blocks();
    named.insert(
        "script".into(),
        json!({ "type": "string", "description": "A script of its own to open, beside the app's." }),
    );
    defs.insert(
        "named_window".into(),
        json!({ "type": "object", "properties": named, "additionalProperties": false }),
    );
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "zgx.json",
        "description": "An app's window and host, as zgx-run reads them (docs/scripting.md).",
        "type": "object",
        "properties": keys,
        "additionalProperties": false,
        "$defs": defs
    })
}

/// [`schema`] as its file is written: pretty, with a newline at the end.
pub fn schema_text() -> String {
    let mut text = serde_json::to_string_pretty(&schema()).expect("a schema is JSON");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    /// `zgx.schema.json` is what the key tables make; `ZGX_BLESS=1` writes it.
    #[test]
    fn the_schema_file_is_what_the_key_tables_make() {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("zgx.schema.json");
        let made = super::schema_text();
        if std::env::var_os("ZGX_BLESS").is_some() {
            std::fs::write(&file, &made).unwrap();
        }
        let held = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(
            held == made,
            "zgx.schema.json is not what the key tables make: run \
             `ZGX_BLESS=1 cargo test -p zgx-run --lib schema`"
        );
    }

    /// A `zgx.json` may name the schema, for its editor.
    #[test]
    fn a_zgx_json_may_name_its_schema() {
        let dir = std::env::temp_dir().join(format!("zgx-run-schema-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("app.luau");
        std::fs::write(&script, "return ui.text 'x'").unwrap();
        std::fs::write(
            dir.join("zgx.json"),
            r#"{ "$schema": "../zgx.schema.json", "title": "Named" }"#,
        )
        .unwrap();
        assert_eq!(
            crate::AppConfig::for_script(&script).unwrap().title,
            "Named"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// The schema's keys are those the reader takes, no more and no fewer.
    #[test]
    fn the_schema_has_the_keys_zgx_json_takes() {
        let schema = super::schema();
        let mut listed: Vec<&str> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        listed.sort_unstable();
        let mut taken: Vec<&str> = crate::WINDOW_KEYS
            .iter()
            .copied()
            .chain(
                zgx_script::window_look::LOOK_KEYS
                    .iter()
                    .map(|key| key.name),
            )
            .chain(zgx_script::window_look::PLATFORMS)
            .chain(crate::APP_KEYS)
            .chain(zgx_script::app_config::KEYS.iter().copied())
            .collect();
        taken.sort_unstable();
        assert_eq!(listed, taken);
    }
}
