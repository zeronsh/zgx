//! Compiled components kept on disk, so a session starting again doesn't
//! compile its module again.
//!
//! An entry is named by the module's SHA-256 and the engine's compatibility
//! hash (Wasmtime's version, its configuration, the target): another
//! Wasmtime or configuration misses rather than loads what it can't run.
//! Entries are written whole (a temporary file renamed over), used ones
//! touched, and the oldest removed past the cache's size. Loading compiled
//! code runs it as it is, so it is loaded only from this folder, the
//! host's, which only this process writes, and never from bytes a plugin
//! gives: a plugin gives a module, which is compiled here first.
use sha2::{Digest, Sha256};
use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Instant, SystemTime},
};
use wasmtime::{Engine, component::Component};

/// The cache's size before the oldest entries go (512 MiB).
pub const DEFAULT_LIMIT: u64 = 512 << 20;

struct Settings {
    /// `None`: no cache.
    dir: Option<PathBuf>,
    limit: u64,
}
static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);

/// Where compiled components are kept, and how much of them: from now on.
/// `None` keeps none (every start compiles). The default is the platform's
/// cache folder's `zgx/wasm` (`$XDG_CACHE_HOME`, `~/.cache`,
/// `~/Library/Caches`, `%LOCALAPPDATA%`), 512 MiB.
pub fn set_cache(dir: Option<PathBuf>, limit: u64) {
    *SETTINGS.lock().unwrap() = Some(Settings { dir, limit });
}

fn settings() -> (Option<PathBuf>, u64) {
    let mut settings = SETTINGS.lock().unwrap();
    let settings = settings.get_or_insert_with(|| Settings {
        dir: default_dir(),
        limit: DEFAULT_LIMIT,
    });
    (settings.dir.clone(), settings.limit)
}

fn default_dir() -> Option<PathBuf> {
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_vendor = "apple") {
        env("HOME").map(|home| home.join("Library/Caches"))
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|home| home.join(".cache")))
    }?;
    Some(base.join("zgx").join("wasm"))
}

/// Where a component came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Compiled,
    Cached,
}

/// `bytes` as a component on `engine`: from the cache when it holds it,
/// else compiled (and kept). How, and how long it took.
pub(crate) fn component(
    engine: &Engine,
    bytes: &[u8],
) -> wasmtime::Result<(Component, Origin, std::time::Duration)> {
    let started = Instant::now();
    let (dir, limit) = settings();
    let Some(dir) = dir else {
        return Ok((
            Component::new(engine, bytes)?,
            Origin::Compiled,
            started.elapsed(),
        ));
    };
    let path = dir.join(name(engine, bytes));
    if path.is_file() {
        // Safety: the file is one this process (or an earlier run of it)
        // wrote in the host's cache folder from `Component::serialize`;
        // its name says the engine it was made for, and Wasmtime checks
        // that again.
        match unsafe { Component::deserialize_file(engine, &path) } {
            Ok(component) => {
                touch(&path);
                return Ok((component, Origin::Cached, started.elapsed()));
            }
            // Broken or another engine's after all: compiled anew.
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    let component = Component::new(engine, bytes)?;
    let compiled = started.elapsed();
    if let Ok(serialized) = component.serialize()
        && write(&dir, &path, &serialized).is_ok()
    {
        evict(&dir, limit, &path);
    }
    Ok((component, Origin::Compiled, compiled))
}

/// `<sha256 of the module>-<the engine's compatibility hash>.cwasm`.
fn name(engine: &Engine, bytes: &[u8]) -> String {
    let module: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut engine_hash = Fnv(0xcbf2_9ce4_8422_2325);
    engine
        .precompile_compatibility_hash()
        .hash(&mut engine_hash);
    format!("{module}-{:016x}.cwasm", engine_hash.finish())
}

/// FNV-1a: the same hash in every run of this build, as `DefaultHasher`
/// needn't be.
struct Fnv(u64);
impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// Written whole or not at all: a temporary file in the folder, renamed.
fn write(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join(format!(
        ".{}.{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("entry"),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written =
        std::fs::write(&temporary, bytes).and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// Used now: the last to go.
fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().append(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// The least recently used entries removed until the folder holds `limit`
/// (`kept`, just written, stays).
fn evict(dir: &Path, limit: u64, kept: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(SystemTime, u64, PathBuf)> = read
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "cwasm") && e.path() != kept)
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let kept = std::fs::metadata(kept).map_or(0, |m| m.len());
    let mut total: u64 = kept + entries.iter().map(|(_, len, _)| len).sum::<u64>();
    entries.sort();
    for (_, len, path) in entries {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        let mut config = wasmtime::Config::new();
        config.wasm_component_model(true).epoch_interruption(true);
        Engine::new(&config).unwrap()
    }
    fn guest(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/guest/built/{name}.wasm",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zgx-wasm-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// One test: the cache's settings are the process's.
    #[test]
    fn a_second_start_loads_from_the_cache_and_the_oldest_go() {
        let engine = engine();
        let dir = scratch("start");
        set_cache(Some(dir.clone()), DEFAULT_LIMIT);
        let cube = guest("cube");
        let (_, first, compiled) = component(&engine, &cube).unwrap();
        let (_, second, cached) = component(&engine, &cube).unwrap();
        eprintln!(
            "cube.wasm ({} KB): compiled in {:.1} ms, from the cache in {:.1} ms",
            cube.len() / 1024,
            compiled.as_secs_f64() * 1000.,
            cached.as_secs_f64() * 1000.
        );
        assert_eq!((first, second), (Origin::Compiled, Origin::Cached));
        assert!(cached < compiled, "{cached:?} against {compiled:?}");

        // A broken entry is compiled anew, not loaded.
        let entry = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .find(|e| e.path().extension().is_some_and(|x| x == "cwasm"))
            .unwrap()
            .path();
        std::fs::write(&entry, b"not compiled code").unwrap();
        assert_eq!(component(&engine, &cube).unwrap().1, Origin::Compiled);
        assert_eq!(component(&engine, &cube).unwrap().1, Origin::Cached);

        // Past its size, the least recently used goes: room for one (the
        // newest stays, whatever its size).
        set_cache(Some(dir.clone()), 1);
        let pixels = guest("pixels");
        component(&engine, &pixels).unwrap();
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 1, "{left:?}");
        assert_eq!(component(&engine, &pixels).unwrap().1, Origin::Cached);
        assert_eq!(component(&engine, &cube).unwrap().1, Origin::Compiled);

        // None: no cache.
        set_cache(None, DEFAULT_LIMIT);
        assert_eq!(component(&engine, &cube).unwrap().1, Origin::Compiled);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
