//! The Luau types of an app's native modules, written from their manifests
//! (`zgx_module::abi::MANIFEST`): what `zgx types` adds to an app's
//! definitions so an editor completes `ui.module("words").count(…)` and a
//! wrong argument is a type error before it is a run-time one. Only a
//! module's table is typed: a call by name (`ui.call_sync`) isn't.
//!
//! A manifest is read from a module's library file, by its magic, without
//! running the module. An app names its modules in `zgx.json`:
//!
//! ```json
//! { "modules": { "words": "modules/words" } }
//! ```
//!
//! each by the folder of its crate (its newest built library under
//! `target/` is read) or by the library's own path.
use serde_json::Value;
use std::path::{Path, PathBuf};
use zgx_module::abi;

/// Where `zgx.d.luau` declares `ui`: an app's modules' types go before it.
const UI: &str = "declare ui: {\n";
/// `ui.module`'s type in `zgx.d.luau`, which an app's modules replace.
const MODULE: &str = "\tmodule: (name: string) -> any,\n";

/// The manifest in the library file at `path`, as JSON.
pub fn manifest_of_library(path: &Path) -> Result<String, String> {
    let file = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    abi::manifest_in(&file).map(str::to_owned).ok_or_else(|| {
        format!(
            "{}: not a zgx module (its file has no manifest)",
            path.display()
        )
    })
}

/// The library of the module `name` that `zgx.json` gives as `path`: the
/// file itself, or the newest library under a crate folder's `target/`
/// whose manifest names the module.
fn library_of(name: &str, path: &Path) -> Result<(PathBuf, String), String> {
    if path.is_file() {
        return manifest_of_library(path).map(|manifest| (path.to_owned(), manifest));
    }
    let mut found: Vec<(std::time::SystemTime, PathBuf, String)> = Vec::new();
    let profiles = std::fs::read_dir(path.join("target")).into_iter().flatten();
    for profile in profiles.flatten() {
        for entry in std::fs::read_dir(profile.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let file = entry.path();
            let is_library = file
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(std::env::consts::DLL_SUFFIX));
            if !is_library {
                continue;
            }
            let Ok(manifest) = manifest_of_library(&file) else {
                continue;
            };
            let named = serde_json::from_str::<Value>(&manifest)
                .ok()
                .is_some_and(|m| m["name"] == name);
            if let (true, Ok(modified)) = (named, entry.metadata().and_then(|m| m.modified())) {
                found.push((modified, file, manifest));
            }
        }
    }
    found.sort_by_key(|(modified, ..)| *modified);
    found.pop().map(|(_, file, manifest)| (file, manifest)).ok_or_else(|| {
        format!(
            "module `{name}`: no built library of it under {} (build it, or name its library's path)",
            path.join("target").display()
        )
    })
}

/// The names of the native modules `dir`'s `zgx.json` lists (none without
/// one), in its order of names; what is wrong with its `modules` if it
/// isn't an object of names and paths.
pub fn named_in(dir: &Path) -> Result<Vec<String>, String> {
    let path = dir.join("zgx.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let json: Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    match json.get("modules") {
        None => Ok(Vec::new()),
        Some(Value::Object(modules)) => modules
            .iter()
            .map(|(name, at)| match at.as_str() {
                Some(_) if abi::is_name(name) => Ok(name.clone()),
                Some(_) => Err(format!(
                    "{}: module `{name}`: a module's name is letters, digits and `_`",
                    path.display()
                )),
                None => Err(format!(
                    "{}: module `{name}`: its path is not a string",
                    path.display()
                )),
            })
            .collect(),
        Some(_) => Err(format!(
            "{}: `modules` is not an object of names and paths",
            path.display()
        )),
    }
}

/// The native modules `dir`'s `zgx.json` names, each with its library and
/// its manifest, and what is wrong with those that have none to read (not
/// built yet, say): their types are left out.
pub fn of_app(dir: &Path) -> (Vec<(PathBuf, String)>, Vec<String>) {
    let (mut read, mut wrong) = (Vec::new(), Vec::new());
    let Ok(text) = std::fs::read_to_string(dir.join("zgx.json")) else {
        return (read, wrong);
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        return (read, wrong);
    };
    let Some(modules) = json.get("modules").and_then(Value::as_object) else {
        return (read, wrong);
    };
    for (name, path) in modules {
        let Some(path) = path.as_str() else {
            wrong.push(format!("module `{name}`: its path is not a string"));
            continue;
        };
        match library_of(name, &dir.join(path)) {
            Ok(found) => read.push(found),
            Err(why) => wrong.push(why),
        }
    }
    (read, wrong)
}

/// The name a manifest gives its module.
pub fn name_of(manifest: &str) -> Option<String> {
    let read: Value = serde_json::from_str(manifest).ok()?;
    read["name"]
        .as_str()
        .filter(|name| abi::is_name(name))
        .map(str::to_owned)
}

/// A module's name as a type's: `chat_reply` is `ChatReply`.
fn pascal(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut letters = part.chars();
            let first = letters.next().map(|c| c.to_ascii_uppercase());
            first.into_iter().chain(letters).collect::<String>()
        })
        .collect()
}

/// A manifest's type as Luau writes it; a module's record by its name
/// after `prefix`.
fn luau_type(ty: &Value, prefix: &str) -> String {
    match ty {
        Value::String(name) => match name.as_str() {
            "nil" | "boolean" | "number" | "string" => name.clone(),
            _ => "any".into(),
        },
        Value::Object(of) => {
            if let Some(item) = of.get("list") {
                format!("{{ {} }}", luau_type(item, prefix))
            } else if let Some(value) = of.get("map") {
                format!("{{ [string]: {} }}", luau_type(value, prefix))
            } else if let Some(Value::String(record)) = of.get("record") {
                format!("{prefix}{record}")
            } else if let Some(inner) = of.get("optional") {
                let inner = luau_type(inner, prefix);
                match inner.as_str() {
                    "any" | "nil" => inner,
                    _ if inner.ends_with('?') => inner,
                    _ => format!("{inner}?"),
                }
            } else {
                "any".into()
            }
        }
        _ => "any".into(),
    }
}

/// `(a: string, b: number?)`, of a function's `args`.
fn parameters(function: &Value, prefix: &str) -> Vec<String> {
    let args = function["args"].as_array().map_or(&[][..], |args| args);
    args.iter()
        .map(|arg| {
            let name = arg[0].as_str().unwrap_or("arg");
            format!("{name}: {}", luau_type(&arg[1], prefix))
        })
        .collect()
}

/// A doc comment's lines, each as a comment at `indent`.
fn comment(doc: &Value, indent: &str) -> String {
    doc.as_str()
        .into_iter()
        .flat_map(str::lines)
        .map(|line| format!("{indent}-- {}\n", line.trim_end()))
        .collect()
}

/// One module's types.
struct Written {
    name: String,
    /// Its records' types and its table's, as `export type` lines.
    types: String,
    /// Its table's type's name.
    table: String,
}

fn written(manifest: &str) -> Result<Written, String> {
    let read: Value =
        serde_json::from_str(manifest).map_err(|e| format!("a module's manifest: {e}"))?;
    let name = read["name"]
        .as_str()
        .filter(|name| abi::is_name(name))
        .ok_or("a module's manifest doesn't name it")?
        .to_owned();
    let prefix = pascal(&name);
    let table = format!("{prefix}Module");
    let mut types = format!(
        "-- The native module `{name}`{}: `ui.module(\"{name}\")`.\n",
        read["version"]
            .as_str()
            .filter(|v| !v.is_empty())
            .map_or(String::new(), |v| format!(" ({v})"))
    );
    let object = |key: &str| read[key].as_object().into_iter().flatten();
    for (record, fields) in object("records") {
        let fields: Vec<String> = fields
            .as_object()
            .into_iter()
            .flatten()
            .map(|(field, ty)| format!("{field}: {}", luau_type(ty, &prefix)))
            .collect();
        types.push_str(&format!(
            "export type {prefix}{record} = {{ {} }}\n",
            fields.join(", ")
        ));
    }
    types.push_str(&format!("export type {table} = {{\n"));
    for (function, of) in object("functions") {
        let member = of["member"]
            .as_str()
            .unwrap_or_else(|| function.rsplit('.').next().unwrap_or(function));
        let returns = luau_type(&of["returns"], &prefix);
        let returns = if returns == "nil" {
            "()".into()
        } else {
            returns
        };
        types.push_str(&comment(&of["doc"], "\t"));
        types.push_str(&format!(
            "\t{member}: ({}) -> {returns},\n",
            parameters(of, &prefix).join(", ")
        ));
    }
    for (action, of) in object("async") {
        let member = of["member"]
            .as_str()
            .unwrap_or_else(|| action.rsplit('.').next().unwrap_or(action));
        let resolves = luau_type(&of["resolves"], &prefix);
        types.push_str(&comment(&of["doc"], "\t"));
        types.push_str(&format!(
            "\t{member}: ({}) -> Promise<{resolves}>,\n",
            parameters(of, &prefix).join(", ")
        ));
    }
    types.push_str("}\n");
    Ok(Written { name, types, table })
}

/// A module's own types, from its manifest: what `<module>.d.luau` beside
/// its library holds, for reading a module on its own.
pub fn module_definitions(manifest: &str) -> Result<String, String> {
    let written = written(manifest)?;
    Ok(format!(
        "-- The types of the native module `{}`, written from its manifest by\n\
         -- `zgx types`: don't edit it. An app's own definitions\n\
         -- (`.zgx/types.d.luau`) hold these too, with `ui.module` typed by them.\n{}",
        written.name, written.types
    ))
}

/// zgx's definitions (`text`) with the modules of these manifests: each
/// module's types, and `ui.module` giving each its table's, so a module's
/// functions are typed as members of its table, their arguments and what
/// they return. That is the typed way to call them: by name
/// (`ui.call_sync`, `ui.action`, `ui.invoke`) is the untyped one, and stays
/// as it is. (luau-lsp 1.70 also fails a definitions file outright once one
/// function has about ten overloads, which an app's modules would pass.) A
/// manifest that can't be read is left out, and said.
pub fn typed(text: String, manifests: &[String]) -> (String, Vec<String>) {
    let (mut modules, mut wrong) = (Vec::new(), Vec::new());
    for manifest in manifests {
        match written(manifest) {
            Ok(written) => modules.push(written),
            Err(why) => wrong.push(why),
        }
    }
    if modules.is_empty() {
        return (text, wrong);
    }
    for placeholder in [UI, MODULE] {
        assert!(
            text.contains(placeholder),
            "zgx.d.luau declares ui.module as module_types.rs expects: {placeholder}"
        );
    }
    // Each module's name gives its table; any other name, what it did.
    let mut tables: Vec<String> = modules
        .iter()
        .map(|m| format!("((name: {:?}) -> {})", m.name, m.table))
        .collect();
    tables.push("((name: string) -> any)".into());
    let text = text.replacen(MODULE, &format!("\tmodule: {},\n", tables.join(" & ")), 1);
    // Their types go before `ui`, which names them.
    let types: String = modules.iter().map(|m| format!("{}\n", m.types)).collect();
    (text.replacen(UI, &format!("{types}{UI}"), 1), wrong)
}

/// What `zgx types` says of these modules beyond their types: a manifest it
/// can't read.
pub fn notes(manifests: &[String]) -> Vec<String> {
    typed(crate::check::DEFINITIONS.to_owned(), manifests).1
}
