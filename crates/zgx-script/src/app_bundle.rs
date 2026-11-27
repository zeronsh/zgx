//! An app as a bundle (`zgx build`), and a bundle opened to run
//! (`zgx-run <bundle>`): its scripts as bytecode, its other files as they
//! are, and its native modules' libraries for each platform, with a
//! manifest of hashes.
//!
//! ```text
//! dist/<app_id>/
//!   manifest.json
//!   ui/                     the app's folder: .luau as bytecode (or, asked
//!                           for, as source), the rest as is
//!   modules/<platform>/     each module's library, `x86_64-linux` and so on
//! ```
//!
//! Luau checks no bytecode it loads, so a bundle is trusted as native code
//! is: only one a person or an app's install names is run, never one a
//! script, a script or a download names. It is checked against its
//! manifest: each module's library by its SHA-256 as it is opened (it is
//! native code, loaded then), and each file of `ui/` by its own SHA-256 as
//! it is read, the bytes checked being the bytes run. So a launch hashes
//! what it loads, when it loads it, and nothing else: no walk of `ui/` up
//! front, and nothing a file changed after its check could slip past.
//!
//! The manifest is no signature: it sits beside what it names, so its
//! hashes catch damage and mistakes, not whoever can write both.
//!
//! A bundle whose `ui/` is source ([`Ui::Source`]) has no hashes of it: it
//! is the folder an app runs from, to be read, copied and laid over as any
//! folder is, and run as source, which Luau compiles and checks. Its
//! modules are checked all the same.
use crate::module_build::{self, Profile, Source, file_hash};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use zgx_module::abi;

/// The bundle format's number (`"zgx_bundle"` in its manifest), of a bundle
/// whose `ui/` is checked bytecode.
pub const FORMAT: u32 = 2;
/// The format of a bundle whose `ui/` is source: a number of its own, so a
/// runtime that knows only the other refuses it, rather than finding no
/// files in it.
pub const SOURCE_FORMAT: u32 = 3;

/// What a bundle's `ui/` is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ui {
    /// Its scripts compiled, and every file checked against the manifest as
    /// it is read.
    #[default]
    Bytecode,
    /// Its files as they are, every one (but `.git`, `.zgx` and
    /// `.DS_Store`), read as any app's folder is.
    Source,
}

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

/// Whether `dir` is a bundle: it has a manifest naming the bundle format
/// (not parsed here: [`open`] reads it, and refuses one that isn't).
pub fn is_bundle(dir: &Path) -> bool {
    std::fs::read(dir.join("manifest.json"))
        .is_ok_and(|text| text.windows(12).any(|at| at == b"\"zgx_bundle\""))
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
    /// What its `ui/` is.
    pub ui: Ui,
    /// The files that look like secrets (docs/modules.md lists the names)
    /// and ship all the same where its `ui/` is source, by their path in
    /// `ui/` (a package's are in `.packages/<name>/`). One not named here
    /// fails the build, and so does a name for nothing.
    pub allow: Vec<String>,
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
    /// The app's files it doesn't hold, by their paths in `ui/`: not of a
    /// kind a host reads, or hidden (a hidden folder as one, with a `/`).
    pub left_out: Vec<String>,
    /// Its modules, each for each target: built now, or the cache's.
    pub modules: Vec<module_build::Built>,
    /// How long its `ui/` took: compiled (or, as source, parsed) and copied.
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
    if options.ui == Ui::Bytecode && !options.allow.is_empty() {
        return Err("--allow is for --ui source, where every file ships".into());
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

    // Its folder: scripts compiled (or as source), the rest as is; zgx.json
    // without what the bundle carries itself (its modules are the
    // manifest's; its packages are in it, where source's zgx.json names
    // them).
    let started = Instant::now();
    let mut skip: Vec<PathBuf> = modules
        .iter()
        .filter_map(|(_, source)| match source {
            Source::Crate(from) | Source::Library(from) => from.canonicalize().ok(),
        })
        .collect();
    if let Ok(out) = out.canonicalize() {
        skip.push(out);
    }
    skip.push(partial.clone());
    let keep = match options.ui {
        Ui::Bytecode => crate::bundle::build::Keep::KnownAndLicences,
        Ui::Source => crate::bundle::build::Keep::Every {
            allow: &options.allow,
        },
    };
    let mut left_out = Vec::new();
    let sources = crate::bundle::build::sources_skipping(&app, &skip, false, keep, &mut left_out)
        .map_err(|e| e.to_string())?;
    let mut sums = serde_json::Map::new();
    let mut scripts = 0;
    for (rel, _, bytes) in sources {
        let bytes = if rel.ends_with(".luau") {
            scripts += 1;
            // (As source, compiled to be told what doesn't parse.)
            let compiled = crate::build::compile(&bytes).map_err(|e| format!("{rel}: {e}"))?;
            match options.ui {
                Ui::Bytecode => compiled,
                Ui::Source => bytes,
            }
        } else if rel == "zgx.json" {
            let mut config = config.clone();
            config.remove("modules");
            match options.ui {
                Ui::Bytecode => drop(config.remove("packages")),
                Ui::Source => {
                    let packages = config.get_mut("packages").and_then(|p| p.as_object_mut());
                    for (name, folder) in packages.into_iter().flatten() {
                        let copy = crate::bundle::build::PACKAGES;
                        *folder = format!("{copy}/{}", name.to_lowercase()).into();
                    }
                }
            }
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
    let files = sums.len();
    let (format, ui) = match options.ui {
        Ui::Bytecode => (
            FORMAT,
            serde_json::json!({ "files": files, "sha256": sums }),
        ),
        Ui::Source => (SOURCE_FORMAT, serde_json::json!({ "source": true })),
    };
    let manifest = serde_json::json!({
        "zgx_bundle": format,
        "app_id": app_id,
        "version": version,
        "runtime": env!("CARGO_PKG_VERSION"),
        "abi": abi::ABI,
        "entry": "app.luau",
        "ui": ui,
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
        files,
        scripts,
        left_out,
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
    /// Its `ui/`'s files as the manifest names them; none, as source.
    files: Option<Checked>,
}

impl Opened {
    /// Whether `path` is in its `ui/`: by its name, and, where it exists, by
    /// what it is (a symbolic link out of it isn't in it).
    #[doc(hidden)]
    pub fn holds(&self, path: &Path) -> bool {
        crate::within(&self.ui, &crate::normalize(path))
    }

    /// What its `ui/` is.
    pub fn kind(&self) -> Ui {
        match self.files {
            Some(_) => Ui::Bytecode,
            None => Ui::Source,
        }
    }

    /// Its `ui/` as a host reads it (`ScriptHost::from_files`): each file
    /// checked against its manifest as it is read. `None` where its `ui/` is
    /// source ([`Ui::Source`]): a host runs [`ui`](Self::ui) as it does any
    /// folder (`ScriptHost::new`).
    pub fn files(&self) -> Option<crate::ScriptFiles> {
        self.files.clone().map(crate::ScriptFiles::Folder)
    }

    /// The file `rel` of its `ui/` (its path there, with `/`), checked
    /// against its manifest: `None` if the manifest names none, an error if
    /// it isn't the file the manifest names (changed or removed since it
    /// was built). Of a `ui/` that is source, the file as it is, `None` if
    /// there is none.
    #[doc(hidden)]
    pub fn file(&self, rel: &str) -> Result<Option<Vec<u8>>, String> {
        let path = self.ui.join(rel);
        let read = match &self.files {
            Some(files) if !files.names(rel) => return Ok(None),
            Some(files) => files.read(&path),
            None if !self.holds(&path) => return Err(outside(&path)),
            None => match std::fs::read(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                read => read,
            },
        };
        read.map(Some)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Reads and checks its scripts on worker threads now, for the first
    /// load to find them ready, as `compile_ahead` compiles an app's
    /// sources: call it first thing, before the app's window is made. The
    /// bytes a worker checked are the bytes that run (handed over, not read
    /// again); a script not ready in time is read and checked as it is
    /// asked for, and what no load took goes as the first load ends. A `ui/`
    /// that is source has none to check: `compile_ahead` is for it.
    pub fn read_ahead(&self) {
        if let Some(files) = &self.files {
            files.read_ahead();
        }
    }

    /// How many scripts wait read ahead, and whether its workers are done.
    /// For tests.
    #[doc(hidden)]
    pub fn read_ahead_counts(&self) -> (usize, bool) {
        self.files
            .as_ref()
            .map_or((0, true), |files| files.ahead_counts())
    }
}

/// Why a bundle's file isn't read: it isn't the bundle's.
fn outside(path: &Path) -> String {
    format!("{}: outside its bundle's ui/", path.display())
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
    // Its format says which of the two its `ui/` is, and its `ui` the same:
    // one that says both, or neither, is refused.
    let (source, sums) = (&manifest["ui"]["source"], &manifest["ui"]["sha256"]);
    let kind = if manifest["zgx_bundle"] == FORMAT {
        Ui::Bytecode
    } else if manifest["zgx_bundle"] == SOURCE_FORMAT {
        Ui::Source
    } else {
        return Err(fail(format!(
            "a bundle of another format than this runtime reads ({FORMAT} or {SOURCE_FORMAT})"
        )));
    };
    let says = match kind {
        Ui::Bytecode => source.is_null() && sums.is_object(),
        Ui::Source => *source == true && sums.is_null(),
    };
    if !says {
        return Err(fail(
            "manifest.json: its `ui` isn't what a bundle of its format has".into(),
        ));
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
    let mut checked = HashMap::new();
    for (rel, hex) in sums.as_object().into_iter().flatten() {
        let sum = hex
            .as_str()
            .and_then(sum_of)
            .ok_or_else(|| fail(format!("manifest.json: ui/{rel}'s sha256")))?;
        checked.insert(rel.clone(), sum);
    }
    // Its first script is one of its own files. (Checked files say so as the
    // script is read; a script's modules are its folder's, as any host's.)
    let entry = manifest["entry"].as_str().unwrap_or("app.luau").to_owned();
    let first = ui.join(&entry);
    if kind == Ui::Source && !crate::within(&ui, &crate::normalize(&first)) {
        return Err(outside(&first));
    }
    Ok(Opened {
        files: match kind {
            Ui::Bytecode => Some(Checked::new(ui.clone(), checked)),
            Ui::Source => None,
        },
        ui,
        entry,
        app_id: string("app_id"),
        version: string("version"),
        modules,
        checked_in: started.elapsed(),
    })
}
