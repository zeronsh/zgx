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
//! script, a plugin or a download names. Opening one checks it against its
//! manifest: each module's library by its SHA-256 every time (it is native
//! code), and the scripts by one SHA-256 over the whole `ui/` tree once,
//! after which the size and time of each file are enough, kept in the cache
//! (`<cache>/bundles/`), until one of them changes.
use crate::module_build::{self, Profile, Source, cache_dir, file_hash};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};
use zgx_module::abi;

/// The bundle format's number (`"zgx_bundle"` in its manifest).
pub const FORMAT: u32 = 1;

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

/// One SHA-256 over a tree of files: each file's path, length and bytes,
/// in order of path.
fn tree_hash(files: &[(String, Vec<u8>)]) -> String {
    let mut hasher = Sha256::new();
    for (rel, bytes) in files {
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The files under `dir`, by `/`-separated path from it, sorted.
fn files_under(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(at) = dirs.pop() {
        for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => dirs.push(path),
                Ok(kind) if kind.is_file() => {
                    let rel = path.strip_prefix(dir).unwrap_or(&path);
                    found.push((rel.to_string_lossy().replace('\\', "/"), path));
                }
                _ => {}
            }
        }
    }
    found.sort();
    found
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
    let mut files = Vec::with_capacity(sources.len());
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
        files.push((rel, bytes));
    }
    let ui_in = started.elapsed();
    let manifest = serde_json::json!({
        "zgx_bundle": FORMAT,
        "app_id": app_id,
        "version": version,
        "runtime": env!("CARGO_PKG_VERSION"),
        "abi": abi::ABI,
        "entry": "app.luau",
        "ui": { "sha256": tree_hash(&files), "files": files.len() },
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
        files: files.len(),
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
    /// Its app's id and version.
    pub app_id: String,
    /// Its version, as `zgx build` was told.
    pub version: String,
    /// Its modules' libraries for this platform, by name, each checked.
    pub modules: Vec<(String, PathBuf)>,
    /// Whether its scripts were hashed now (the first time this bundle was
    /// opened, or a file of it changed since), not taken from the cache.
    pub hashed: bool,
    /// How long checking it took.
    pub checked_in: Duration,
}

/// Each file's path, length and time of change: what says a checked tree
/// is still the one checked.
fn stamps(files: &[(String, PathBuf)]) -> String {
    let mut out = String::new();
    for (rel, path) in files {
        let meta = std::fs::metadata(path).ok();
        let len = meta.as_ref().map_or(0, |m| m.len());
        let at = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |t| t.as_nanos());
        out.push_str(&format!("{len} {at} {rel}\n"));
    }
    out
}

/// `zgx-run <bundle>`: the bundle in `dir`, checked against its manifest
/// (see the module's docs), with its modules for this platform.
pub fn open(dir: &Path) -> Result<Opened, String> {
    open_with(dir, &cache_dir())
}

/// As [`open`], remembering what it checked in `cache` (`cache_dir()`'s).
#[doc(hidden)]
pub fn open_with(dir: &Path, cache: &Path) -> Result<Opened, String> {
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
    // Each module's library, every time: native code.
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
    // Its scripts: hashed once, then known by their stamps.
    let files = files_under(&ui);
    let now = stamps(&files);
    let mut key = Sha256::new();
    key.update(ui.to_string_lossy().as_bytes());
    key.update([0]);
    key.update(&text);
    let key: String = key.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let record = cache.join("bundles").join(key);
    let hashed = std::fs::read_to_string(&record).ok().as_deref() != Some(&*now);
    if hashed {
        let read = files
            .iter()
            .map(|(rel, path)| {
                std::fs::read(path)
                    .map(|bytes| (rel.clone(), bytes))
                    .map_err(|e| fail(format!("ui/{rel}: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if manifest["ui"]["sha256"].as_str() != Some(&*tree_hash(&read)) {
            return Err(fail(
                "its scripts aren't those its manifest names (ui/ changed since it was built)"
                    .into(),
            ));
        }
        // (A cache that can't be written only costs the next launch a hash.)
        if let Some(parent) = record.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&record, &now);
    }
    Ok(Opened {
        ui,
        entry: manifest["entry"].as_str().unwrap_or("app.luau").to_owned(),
        app_id: string("app_id"),
        version: string("version"),
        modules,
        hashed,
        checked_in: started.elapsed(),
    })
}
