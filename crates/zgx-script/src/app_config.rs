//! What an app's `zgx.json` gives its script host beyond the window: the
//! packages it requires, and the scripts it loads beside its own, each with
//! its limits. One reading for every host that runs a Luau app on its own
//! (`zgx run`, `zgx test`'s `mount()`, `zgx audit`), so an app behaves the
//! same under each.
//!
//! ```json
//! {
//!   "packages": { "kit": "../shared/kit" },
//!   "scripts": {
//!     "focus": "scripts/focus",
//!     "stamps": { "path": "scripts/stamps", "limits": { "memory_mb": 32, "call_ms": 50 } }
//!   }
//! }
//! ```
//!
//! Each script is a folder with an `init.luau`, or a file. It runs in a VM of
//! its own and reaches everything the app's script does. `limits` sets its
//! `memory_mb`, `load_ms`, `call_ms`, `voices` and `count_state`; each left
//! out has none.
use crate::{Limits, Script, ScriptHost};
use serde_json::{Map, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// The keys this reads from `zgx.json`; a host reading the rest (the
/// window's) accepts these. (`plugins` and `expose` are read to say what
/// replaced them.)
pub const KEYS: &[&str] = &[
    "packages", "scripts", "modules", "version", "plugins", "expose",
];

/// What `zgx.json` says of an app's host (`read`), to give it (`apply`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppHost {
    /// Module folders outside the app it may require, by name.
    pub packages: Vec<(String, PathBuf)>,
    /// The scripts it loads beside its own, in order.
    scripts: Vec<Script>,
}

/// `dir`'s `zgx.json` (nothing if it has none), or what is wrong with it.
pub fn read(dir: &Path) -> Result<AppHost, String> {
    let path = dir.join("zgx.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(AppHost::default());
    };
    let fail = |e: String| format!("{}: {e}", path.display());
    let json: Value = serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
    let object = json
        .as_object()
        .ok_or_else(|| fail("not an object".into()))?;
    from_object(dir, object).map_err(fail)
}

/// The app's host as `zgx.json`'s `object` (already read, from `dir`) says,
/// its other keys left to the caller: for a host reading the rest.
pub fn from_object(dir: &Path, object: &Map<String, Value>) -> Result<AppHost, String> {
    if object.contains_key("plugins") {
        return Err(
            "`plugins` is now `scripts`: { \"name\": \"path\" }, or { \"path\", \
                    \"limits\" }; a script reaches everything the app's does, with no `allow`"
                .into(),
        );
    }
    if object.contains_key("expose") {
        return Err(
            "`expose` is gone: every script reaches every shared state, action and \
                    element of the app's; leave it out"
                .into(),
        );
    }
    let mut app = AppHost::default();
    if let Some(listed) = object.get("packages") {
        let listed = listed
            .as_object()
            .ok_or("`packages` is not an object of names and folders")?;
        for (name, folder) in listed {
            let folder = folder
                .as_str()
                .ok_or_else(|| format!("package `{name}` is not a folder's path"))?;
            let folder = dir.join(folder);
            if name.is_empty() || name.contains('/') {
                return Err(format!("package `{name}`: a name, without a `/`"));
            }
            if !folder.is_dir() {
                return Err(format!(
                    "package `{name}`: {} is not a folder",
                    folder.display()
                ));
            }
            app.packages.push((name.clone(), folder));
        }
    }
    if let Some(scripts) = object.get("scripts") {
        let scripts = scripts
            .as_object()
            .ok_or("`scripts` is not an object of names and paths (or { path, limits })")?;
        for (name, script) in scripts {
            let (path, limits) = match script {
                Value::String(path) => (path.as_str(), Limits::default()),
                Value::Object(script) => {
                    for key in script.keys() {
                        if !["path", "limits"].contains(&key.as_str()) {
                            return Err(format!(
                                "unknown key `scripts.{name}.{key}` (path or limits)"
                            ));
                        }
                    }
                    let path = script
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("script `{name}` has no `path`"))?;
                    let limits = match script.get("limits") {
                        None => Limits::default(),
                        Some(limits) => read_limits(name, limits)?,
                    };
                    (path, limits)
                }
                _ => {
                    return Err(format!(
                        "script `{name}` is not a path, nor {{ path, limits }}"
                    ));
                }
            };
            let path = dir.join(path);
            let entry = if path.is_dir() {
                path.join("init.luau")
            } else {
                path.clone()
            };
            if !entry.is_file() {
                return Err(format!(
                    "script `{name}`: {} is neither a script nor a folder with an init.luau",
                    path.display()
                ));
            }
            app.scripts
                .push(Script::file(name.clone(), entry).limits(limits));
        }
    }
    Ok(app)
}

/// `scripts.<name>.limits`: `memory_mb`, `load_ms`, `call_ms` and `voices`,
/// each a number, and `count_state`, `true` or `false`.
fn read_limits(name: &str, limits: &Value) -> Result<Limits, String> {
    let limits = limits.as_object().ok_or_else(|| {
        format!("`scripts.{name}.limits` is not an object of memory_mb, load_ms, call_ms, voices")
    })?;
    let mut out = Limits::default();
    for (key, value) in limits {
        if key == "count_state" {
            out.count_state = value.as_bool().ok_or_else(|| {
                format!("`scripts.{name}.limits.count_state` is not true or false")
            })?;
            continue;
        }
        let number = value
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.)
            .ok_or_else(|| format!("`scripts.{name}.limits.{key}` is not a number"))?;
        match key.as_str() {
            "memory_mb" => out.memory = Some((number * 1048576.) as usize),
            "load_ms" => out.load = Some(Duration::from_secs_f64(number / 1000.)),
            "call_ms" => out.call = Some(Duration::from_secs_f64(number / 1000.)),
            "voices" => out.voices = Some(number as usize),
            _ => {
                return Err(format!(
                    "unknown key `scripts.{name}.limits.{key}` (memory_mb, load_ms, call_ms, \
                     voices or count_state)"
                ));
            }
        }
    }
    Ok(out)
}

/// The package aliases `file` may require, from the `zgx.json` nearest
/// above it: every package. For tooling (`zgx check`, `zgx types`) to resolve
/// `@name/…` as zgx does. Empty with no `zgx.json`, or one that doesn't
/// read.
pub fn aliases_for(file: &Path) -> Vec<(String, PathBuf)> {
    let Some(dir) = file
        .ancestors()
        .skip(1)
        .find(|dir| dir.join("zgx.json").is_file())
    else {
        return Vec::new();
    };
    let Ok(app) = read(dir) else {
        return Vec::new();
    };
    canonical(&app.packages)
}

/// The packages a script whose folder is `dir` (one with an `init.luau`)
/// requires from, by the `zgx.json` nearest above it: none if `dir` is no
/// script's folder there.
pub fn packages_for_folder(dir: &Path) -> Option<Vec<(String, PathBuf)>> {
    let dir = dir.canonicalize().ok()?;
    let config = dir
        .ancestors()
        .skip(1)
        .find(|d| d.join("zgx.json").is_file())?;
    let app = read(config).ok()?;
    let entry = dir.join("init.luau");
    app.scripts.iter().find(|script| match &script.source {
        crate::Source::File(path) => path.canonicalize().is_ok_and(|e| e == entry),
        crate::Source::Code(_) => false,
    })?;
    Some(canonical(&app.packages))
}

/// `packages` with their folders canonical.
fn canonical(packages: &[(String, PathBuf)]) -> Vec<(String, PathBuf)> {
    packages
        .iter()
        .map(|(name, folder)| {
            (
                name.clone(),
                folder.canonicalize().unwrap_or_else(|_| folder.clone()),
            )
        })
        .collect()
}

impl AppHost {
    /// Whether it loads scripts beside the app's.
    pub fn has_scripts(&self) -> bool {
        !self.scripts.is_empty()
    }
    /// `host` given its packages and its scripts: call it before the host
    /// first loads.
    pub fn apply(&self, host: ScriptHost) -> ScriptHost {
        let host = self
            .packages
            .iter()
            .fold(host, |host, (name, dir)| host.package(name.clone(), dir));
        if !self.scripts.is_empty() {
            host.set_scripts(self.scripts.clone());
        }
        host
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A script is a path, or a path and its limits; `plugins` and `expose`
    /// say what replaced them.
    #[test]
    fn scripts_read_with_their_limits() {
        let dir = std::env::temp_dir().join(format!("zgx-config-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("init.luau"), "").unwrap();
        let read = |config: &str| {
            let object: Value = serde_json::from_str(config).unwrap();
            from_object(&dir, object.as_object().unwrap())
        };
        let app = read(
            r#"{ "scripts": { "a": ".", "b": { "path": ".", "limits": { "memory_mb": 2, "call_ms": 50, "voices": 3, "count_state": true } } } }"#,
        )
        .unwrap();
        assert_eq!(app.scripts[0].limits, Limits::default());
        let limits = app.scripts[1].limits;
        assert_eq!(limits.memory, Some(2 << 20));
        assert_eq!(limits.call, Some(Duration::from_millis(50)));
        assert_eq!((limits.load, limits.voices), (None, Some(3)));
        assert!(limits.count_state);
        let error = read(r#"{ "plugins": {} }"#).unwrap_err();
        assert!(error.contains("`plugins` is now `scripts`"), "{error}");
        let error = read(r#"{ "expose": {} }"#).unwrap_err();
        assert!(error.contains("`expose` is gone"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
