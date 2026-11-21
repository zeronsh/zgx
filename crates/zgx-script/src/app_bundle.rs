//! An app as a bundle (`zgx build`), and a bundle opened to run
//! (`zgx-run <bundle>`): its scripts as bytecode, its other files as they
//! are, and its native modules' libraries for each platform, with a
//! manifest of hashes.
//!
//! ```text
//! dist/<app_id>/
//!   manifest.json
//!   ui/                     the app's folder: .luau as bytecode, the rest as is
//!   modules/<platform>/     each module's library, `x86_64-linux` and so on
//! ```
//!
//! Luau checks no bytecode it loads, so a bundle is trusted as native code
//! is: only one a person or an app's install names is run, never one a
//! script, a plugin or a download names. It is checked against its
//! manifest: each module's library by its SHA-256 as it is opened (it is
//! native code, loaded then), and each file of `ui/` by its own SHA-256 as
//! it is read, the bytes checked being the bytes run. So a launch hashes
//! what it loads, when it loads it, and nothing else: no walk of `ui/` up
//! front, and nothing a file changed after its check could slip past.
use crate::module_build::{self, Profile, Source, file_hash};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zgx_module::abi;

/// The bundle format's number (`"zgx_bundle"` in its manifest).
pub const FORMAT: u32 = 2;

/// The platform a library is for, as a bundle's manifest names it: this
/// build's (`x86_64-linux`, `aarch64-macos`).
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

/// A cargo target's platform, as [`platform`] names one.
fn platform_of(triple: &str) -> String {
    let arch = triple.split('-').next().unwrap_or(triple);
    let os = if triple.contains("linux") {
        "linux"
    } else if triple.contains("darwin") || triple.contains("apple") {
        "macos"
    } else if triple.contains("windows") {
        "windows"
    } else {
        triple.rsplit('-').next().unwrap_or(triple)
    };
    format!("{arch}-{os}")
}

/// Whether `dir` is a bundle: it has a manifest saying so.
pub fn is_bundle(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|manifest| manifest.get("zgx_bundle").is_some())
}

/// What `zgx build` is asked for.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Where the bundle goes: `dist/<app_id>` if none.
    pub out: Option<PathBuf>,
    /// The cargo targets its modules are built for: the host's if none.
    pub targets: Vec<String>,
    /// Its version, over zgx.json's `version`.
    pub version: Option<String>,
}

/// A bundle written.
#[derive(Debug)]
pub struct Written {
    /// Where it is.
    pub dir: PathBuf,
    /// How many files its `ui/` holds, and how many of them are scripts.
    pub files: usize,
    /// The scripts among them.
    pub scripts: usize,
    /// Its modules, each for each target: built now, or the cache's.
    pub modules: Vec<module_build::Built>,
    /// How long compiling and copying its `ui/` took.
    pub ui_in: Duration,
}

/// `bytes`' SHA-256, in hex.
fn hash_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A SHA-256 written in hex, as its 32 bytes.
fn sum_of(hex: &str) -> Option<[u8; 32]> {
    let mut sum = [0; 32];
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    for (at, byte) in sum.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[at * 2..at * 2 + 2], 16).ok()?;
    }
    Some(sum)
}

/// `zgx build`: the app in `app` (a folder with its `app.luau`) as a
/// bundle, its modules built for release where their key is new.
pub fn write(app: &Path, options: &Options) -> Result<Written, String> {
    let app = app
        .canonicalize()
        .map_err(|e| format!("{}: {e}", app.display()))?;
    if !app.join("app.luau").is_file() {
        return Err(format!("{}: no app.luau", app.display()));
    }
    let config: serde_json::Map<String, serde_json::Value> =
        match std::fs::read_to_string(app.join("zgx.json")) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("zgx.json: {e}"))?,
            Err(_) => Default::default(),
        };
    let app_id = config
        .get("app_id")
        .and_then(|id| id.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            app.file_name()
                .map_or_else(|| "app".into(), |n| n.to_string_lossy().into_owned())
        });
    let version = options
        .version
        .clone()
        .or_else(|| {
            config
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "0.0.0".into());
    let out = options
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from("dist").join(&app_id));
    // What was there must be a bundle (or nothing): it is replaced whole.
    if out.exists()
        && !is_bundle(&out)
        && std::fs::read_dir(&out).map_or(true, |mut d| d.next().is_some())
    {
        return Err(format!(
            "{}: not a bundle, and not empty: name another place (--out)",
            out.display()
        ));
    }
    let partial = out.with_file_name(format!(
        ".{}-partial-{}",
        out.file_name()
            .map_or_else(|| "bundle".into(), |n| n.to_string_lossy().into_owned()),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&partial);
    let made =
        |path: &Path| std::fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()));
    made(&partial.join("ui"))?;

    // Its modules, for each target.
    let modules = module_build::modules_of(&app)?;
    let targets = if options.targets.is_empty() {
        vec![module_build::target_of(None)?]
    } else {
        options.targets.clone()
    };
    let mut built = Vec::new();
    let mut listed = serde_json::Map::new();
    for (name, source) in &modules {
        let mut by_platform = serde_json::Map::new();
        for target in &targets {
            let library = match source {
                Source::Crate(dir) => {
                    let module = module_build::ensure(name, dir, Profile::Release, Some(target))?;
                    let library = module.library.clone();
                    built.push(module);
                    library
                }
                // A library named as it is: the host's alone.
                Source::Library(library) => library.clone(),
            };
            let platform = platform_of(target);
            let file = library
                .file_name()
                .ok_or_else(|| format!("module `{name}`: {}", library.display()))?;
            let rel = format!("modules/{platform}/{}", file.to_string_lossy());
            made(&partial.join(format!("modules/{platform}")))?;
            std::fs::copy(&library, partial.join(&rel))
                .map_err(|e| format!("{}: {e}", library.display()))?;
            let hash = file_hash(&partial.join(&rel)).map_err(|e| format!("{rel}: {e}"))?;
            by_platform.insert(
                platform,
                serde_json::json!({ "path": rel, "sha256": hash, "target": target }),
            );
        }
        listed.insert(name.clone(), by_platform.into());
    }

    // Its folder: scripts compiled, the rest as is; zgx.json without what
    // the bundle carries itself (its packages are in it, its modules are
    // the manifest's).
    let started = Instant::now();
    let mut skip: Vec<PathBuf> = modules
        .iter()
        .filter_map(|(_, source)| match source {
            Source::Crate(dir) => dir.canonicalize().ok(),
            Source::Library(_) => None,
        })
        .collect();
    if let Ok(out) = out.canonicalize() {
        skip.push(out);
    }
    skip.push(partial.clone());
    let sources =
        crate::bundle::build::sources_skipping(&app, &skip, false).map_err(|e| e.to_string())?;
    let mut sums = serde_json::Map::new();
    let mut scripts = 0;
    for (rel, _, bytes) in sources {
        let bytes = if rel.ends_with(".luau") {
            scripts += 1;
            crate::build::compile(&bytes).map_err(|e| format!("{rel}: {e}"))?
        } else if rel == "zgx.json" {
            let mut config = config.clone();
            config.remove("packages");
            config.remove("modules");
            serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?
        } else {
            bytes
        };
        let path = partial.join("ui").join(&rel);
        if let Some(parent) = path.parent() {
            made(parent)?;
        }
        std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        sums.insert(rel, hash_of(&bytes).into());
    }
    let ui_in = started.elapsed();
    let manifest = serde_json::json!({
        "zgx_bundle": FORMAT,
        "app_id": app_id,
        "version": version,
        "runtime": env!("CARGO_PKG_VERSION"),
        "abi": abi::ABI,
        "entry": "app.luau",
        "ui": { "files": sums.len(), "sha256": sums },
        "modules": listed,
    });
    std::fs::write(
        partial.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("manifest.json: {e}"))?;
    // In place whole: the old one goes as the new one comes.
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    if let Some(parent) = out.parent() {
        made(parent)?;
    }
    std::fs::rename(&partial, &out).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(Written {
        dir: out,
        files: sums.len(),
        scripts,
        modules: built,
        ui_in,
    })
}

/// A bundle checked, ready to run.
#[derive(Debug)]
pub struct Opened {
    /// Its `ui/` folder, where its scripts are (canonical).
    pub ui: PathBuf,
    /// Its first script, in `ui/`.
    pub entry: String,
    /// Its app's id.
    pub app_id: String,
    /// Its version, as `zgx build` was told.
    pub version: String,
    /// Its modules' libraries for this platform, by name, each checked.
    pub modules: Vec<(String, PathBuf)>,
    /// How long checking it took (its manifest and its modules' libraries:
    /// its files are checked as they are read).
    pub checked_in: Duration,
    files: Checked,
}

impl Opened {
    /// Its `ui/` as a host reads it (`ScriptHost::from_files`): each file
    /// checked against its manifest as it is read.
    pub fn files(&self) -> crate::ScriptFiles {
        crate::ScriptFiles::Folder(self.files.clone())
    }
}

pub use crate::checked_files::Checked;

/// `zgx-run <bundle>`: the bundle in `dir`, checked against its manifest
/// (see the module's docs), with its modules for this platform.
pub fn open(dir: &Path) -> Result<Opened, String> {
    let started = Instant::now();
    let fail = |why: String| format!("{}: {why}", dir.display());
    let text = std::fs::read(dir.join("manifest.json"))
        .map_err(|e| fail(format!("manifest.json: {e}")))?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&text).map_err(|e| fail(format!("manifest.json: {e}")))?;
    if manifest["zgx_bundle"] != FORMAT {
        return Err(fail(format!(
            "a bundle of another format than this runtime reads ({FORMAT})"
        )));
    }
    if manifest["abi"] != abi::ABI {
        return Err(fail(format!(
            "its modules are for another module interface than this runtime's ({})",
            abi::ABI
        )));
    }
    let ui = dir
        .join("ui")
        .canonicalize()
        .map_err(|e| fail(format!("ui: {e}")))?;
    let string = |key: &str| manifest[key].as_str().unwrap_or_default().to_owned();
    // Each module's library, now: native code, loaded as it is checked.
    let here = platform();
    let mut modules = Vec::new();
    for (name, by_platform) in manifest["modules"].as_object().into_iter().flatten() {
        let library = &by_platform[&here];
        let (Some(rel), Some(hash)) = (library["path"].as_str(), library["sha256"].as_str()) else {
            return Err(fail(format!("module `{name}` isn't built for {here}")));
        };
        let path = dir.join(rel);
        let found = file_hash(&path).map_err(|e| fail(format!("{rel}: {e}")))?;
        if found != hash {
            return Err(fail(format!(
                "module `{name}`: {rel} isn't the library its manifest names"
            )));
        }
        modules.push((name.clone(), path));
    }
    // Its files' sums, each checked as its file is read.
    let mut sums = HashMap::new();
    for (rel, hex) in manifest["ui"]["sha256"].as_object().into_iter().flatten() {
        let sum = hex
            .as_str()
            .and_then(sum_of)
            .ok_or_else(|| fail(format!("manifest.json: ui/{rel}'s sha256")))?;
        sums.insert(rel.clone(), sum);
    }
    Ok(Opened {
        files: Checked::new(ui.as_path().into(), Arc::new(sums)),
        ui,
        entry: manifest["entry"].as_str().unwrap_or("app.luau").to_owned(),
        app_id: string("app_id"),
        version: string("version"),
        modules,
        checked_in: started.elapsed(),
    })
}
