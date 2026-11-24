//! Building an app's native modules only when they change: what `zgx run`
//! and `zgx build` do with each crate `zgx.json`'s `modules` names
//! (docs/modules.md).
//!
//! A module's **key** is a SHA-256 over everything its build depends on
//! that this can see: every file in its crate's folder (but `target/` and
//! hidden folders), the compiler (`rustc -vV`), the target, the profile and
//! the module interface's number. A library built for a key is kept in the
//! cache, `<cache>/modules/<name>/<key>/`, and taken from there the next
//! time that key comes up: a module whose files didn't change is never
//! built again, by any app or checkout on the machine. A miss runs cargo
//! with a target directory of the module's own in the cache, so even a
//! rebuild only compiles what changed.
//!
//! What the key can't see: a path dependency outside the crate's folder.
//! Keep those inside it, or depend by version or git (which `Cargo.lock`
//! pins).
use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use zgx_module::abi;

/// How a module is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// `cargo build`: what `zgx run` builds by default, fastest to rebuild.
    Dev,
    /// `cargo build --release`: what `zgx build` ships.
    Release,
}
impl Profile {
    fn name(self) -> &'static str {
        match self {
            Profile::Dev => "dev",
            Profile::Release => "release",
        }
    }
}

/// Where zgx keeps what it builds: `ZGX_CACHE`, else `$XDG_CACHE_HOME/zgx`,
/// else `~/.cache/zgx` (`%LOCALAPPDATA%\zgx` on Windows, `~/Library/Caches/zgx`
/// on macOS).
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ZGX_CACHE").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("zgx");
    }
    if cfg!(windows)
        && let Some(dir) = std::env::var_os("LOCALAPPDATA")
    {
        return PathBuf::from(dir).join("zgx");
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    if cfg!(target_os = "macos") {
        home.join("Library/Caches/zgx")
    } else {
        home.join(".cache/zgx")
    }
}

/// A module `zgx.json` names: a crate to build, or a library as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A crate's folder, built as its sources change.
    Crate(PathBuf),
    /// A library, loaded as it is.
    Library(PathBuf),
}

/// The modules `dir`'s `zgx.json` names, in its order of names: each a
/// crate's folder (one with a `Cargo.toml`) or a library's path.
pub fn modules_of(dir: &Path) -> Result<Vec<(String, Source)>, String> {
    let names = crate::module_types::named_in(dir)?;
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let path = dir.join("zgx.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(names
        .into_iter()
        .map(|name| {
            let at = dir.join(json["modules"][&name].as_str().unwrap_or_default());
            let source = if at.join("Cargo.toml").is_file() {
                Source::Crate(at)
            } else {
                Source::Library(at)
            };
            (name, source)
        })
        .collect())
}

/// The files a module's key covers: every file in its crate's folder but
/// `target/` and hidden folders, sorted.
pub fn sources(crate_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut dirs = vec![crate_dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !(name.starts_with('.') || (dir == crate_dir && name == "target")) {
                    dirs.push(path);
                }
            } else if kind.is_file() && !name.starts_with('.') {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The compiler as it says it is (`rustc -vV`), and the target it builds
/// for unless told another: once a process.
fn rustc() -> Result<&'static (String, String), String> {
    static RUSTC: std::sync::OnceLock<Result<(String, String), String>> =
        std::sync::OnceLock::new();
    RUSTC
        .get_or_init(|| {
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let out = Command::new(rustc)
                .arg("-vV")
                .output()
                .map_err(|e| format!("rustc -vV: {e}"))?;
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let host = text
                .lines()
                .find_map(|line| line.strip_prefix("host: "))
                .ok_or("rustc -vV names no host")?
                .to_owned();
            Ok((text, host))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The target a build for `target` (or the host's) is for.
pub fn target_of(target: Option<&str>) -> Result<String, String> {
    match target {
        Some(target) => Ok(target.to_owned()),
        None => rustc().map(|(_, host)| host.clone()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A file's SHA-256, as hex.
pub fn file_hash(path: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut file = std::fs::File::open(path)?;
    io::copy(&mut file, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

/// The key of the module in `crate_dir`, built with `profile` for `target`.
pub fn key(crate_dir: &Path, profile: Profile, target: &str) -> Result<String, String> {
    let (rustc, _) = rustc()?;
    let mut hasher = Sha256::new();
    for part in [
        "zgx module key 1",
        &abi::ABI.to_string(),
        profile.name(),
        target,
        rustc,
    ] {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    for file in sources(crate_dir) {
        let relative = file.strip_prefix(crate_dir).unwrap_or(&file);
        let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    // (32 hex digits name a cache entry well enough.)
    Ok(hex(&hasher.finalize()[..16]))
}

/// A module's library, ready to load.
#[derive(Clone, Debug)]
pub struct Built {
    /// The module's name.
    pub name: String,
    /// Its library, in the cache.
    pub library: PathBuf,
    /// Its key (see the module's docs).
    pub key: String,
    /// Whether this call built it (else the cache had it).
    pub built: bool,
    /// How long finding its key took, and building it if it did.
    pub keyed_in: std::time::Duration,
    /// How long building it took (none if the cache had it).
    pub built_in: std::time::Duration,
}

/// The library file in a cache entry, if it holds one.
fn library_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|path| {
            path.extension()
                .is_some_and(|ext| ext == std::env::consts::DLL_EXTENSION)
        })
}

/// The module `name` from the crate in `crate_dir`, built with `profile`
/// for `target` (the host's if none): the cache's if it has this key, else
/// built now (cargo's output on stderr, as it goes) and kept there.
pub fn ensure(
    name: &str,
    crate_dir: &Path,
    profile: Profile,
    target: Option<&str>,
) -> Result<Built, String> {
    let started = Instant::now();
    let target = target_of(target)?;
    let key = key(crate_dir, profile, &target)?;
    let keyed_in = started.elapsed();
    let cache = cache_dir();
    let entry = cache.join("modules").join(name).join(&key);
    if let Some(library) = library_in(&entry) {
        return Ok(Built {
            name: name.to_owned(),
            library,
            key,
            built: false,
            keyed_in,
            built_in: Default::default(),
        });
    }
    let building = Instant::now();
    let host = rustc()?.1 == target;
    let target_dir = cache.join("target").join(if host {
        name.to_owned()
    } else {
        format!("{name}-{target}")
    });
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo
        .arg("build")
        .arg("--manifest-path")
        .arg(crate_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir)
        .args(["--message-format", "json-render-diagnostics"])
        .stderr(std::process::Stdio::inherit());
    if profile == Profile::Release {
        cargo.arg("--release");
    }
    if !host {
        cargo.args(["--target", &target]);
    }
    let out = cargo
        .output()
        .map_err(|e| format!("module `{name}`: cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!("module `{name}` didn't build"));
    }
    // The library cargo says it made.
    let made = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|message| message["reason"] == "compiler-artifact")
        .filter(|message| {
            message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|k| k == "cdylib"))
        })
        .flat_map(|message| {
            message["filenames"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|f| f.as_str().map(PathBuf::from))
                .collect::<Vec<_>>()
        })
        .find(|file| {
            file.extension()
                .is_some_and(|ext| ext == std::env::consts::DLL_EXTENSION)
        })
        .ok_or_else(|| {
            format!("module `{name}`: its crate builds no library (`crate-type = [\"cdylib\"]`)")
        })?;
    // Its manifest names it as zgx.json does.
    let bytes = std::fs::read(&made).map_err(|e| format!("{}: {e}", made.display()))?;
    let manifest = abi::manifest_in(&bytes)
        .ok_or_else(|| format!("module `{name}`: {} has no manifest", made.display()))?;
    let named = crate::module_types::name_of(manifest);
    if named.as_deref() != Some(name) {
        return Err(format!(
            "module `{name}`: its manifest names it `{}`",
            named.unwrap_or_default()
        ));
    }
    // An entry left without its library (a build cut short) is made again.
    if entry.exists() {
        let _ = std::fs::remove_dir_all(&entry);
    }
    // Into the cache whole or not at all: copied beside, then renamed.
    let partial = cache
        .join("modules")
        .join(name)
        .join(format!(".{key}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    let file = made.file_name().expect("a library's file name");
    std::fs::write(partial.join(file), &bytes)
        .map_err(|e| format!("{}: {e}", partial.display()))?;
    match std::fs::rename(&partial, &entry) {
        Ok(()) => {}
        // Another build of the same key got there first: its library is
        // this one.
        Err(_) if library_in(&entry).is_some() => {
            let _ = std::fs::remove_dir_all(&partial);
        }
        Err(e) => return Err(format!("{}: {e}", entry.display())),
    }
    Ok(Built {
        name: name.to_owned(),
        library: entry.join(file),
        key,
        built: true,
        keyed_in,
        built_in: building.elapsed(),
    })
}
