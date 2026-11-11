//! zgx patch: FXC's compiled shaders kept on disk between launches.
//!
//! FXC compiles the HLSL naga writes for each pipeline stage at the first
//! frame that draws with it; a zgx window's first frame compiled six, 45 ms
//! of its 280 ms start on an RTX 4090 machine. A shader compiled once is
//! written to the directory `set_shader_cache_dir` names, under a hash of
//! everything FXC was given (its flags, target, entry point and source), and
//! read back the next time: the file holds that key in full, which must
//! match byte for byte, so a hash collision or another version's file is
//! compiled again rather than used. A file is written whole, from a file of
//! its writer's own, and renamed into place, so no other process or thread
//! reads half of one; and only a whole DXBC container is ever read back.
//! DXC's output is kept too, under a key naming its DLL's version and file
//! (size and time), since the DLL is the app's own; a DLL found only by name
//! isn't kept. What the cache did is counted (`shader_cache_counts`), for a
//! start's marks.
use alloc::{string::String, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};
use std::{
    path::{Path, PathBuf},
    sync::RwLock,
};

static DIR: RwLock<Option<PathBuf>> = RwLock::new(None);
static READ: AtomicU32 = AtomicU32::new(0);
static COMPILED: AtomicU32 = AtomicU32::new(0);
static UNWRITTEN: AtomicU32 = AtomicU32::new(0);

/// What FXC's disk cache did in this process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderCacheCounts {
    /// Shaders read back from the cache.
    pub read: u32,
    /// Shaders FXC compiled (none there, or no cache set).
    pub compiled: u32,
    /// Shaders compiled that the cache could not keep: they compile again
    /// next launch.
    pub unwritten: u32,
}

/// What FXC's disk cache has done in this process so far.
pub fn shader_cache_counts() -> ShaderCacheCounts {
    ShaderCacheCounts {
        read: READ.load(Ordering::Relaxed),
        compiled: COMPILED.load(Ordering::Relaxed),
        unwritten: UNWRITTEN.load(Ordering::Relaxed),
    }
}

const MAGIC: &[u8] = b"wgpu-hal dx12 fxc cache 1\n";

/// Keep FXC's compiled shaders in `dir` between launches (`None`: compile
/// every launch, as wgpu-hal does by default). The directory is made when a
/// shader is first written.
pub fn set_shader_cache_dir(dir: Option<PathBuf>) {
    *DIR.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = dir;
}

/// Everything FXC was given for one shader.
pub(super) fn key(flags: u32, target: &str, entry_point: &str, source: &str) -> String {
    format!("{flags:#x}\n{target}\n{entry_point}\n{source}")
}

fn path(dir: &Path, key: &str) -> PathBuf {
    // FNV-1a: stable across Rust versions, as the files must be.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    dir.join(format!("{hash:016x}.fxc"))
}

fn dir() -> Option<PathBuf> {
    DIR.read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// The bytecode compiled for `key` before, if the cache holds it.
pub(super) fn read(key: &str) -> Option<Vec<u8>> {
    let bytes = std::fs::read(path(&dir()?, key)).ok()?;
    let code = decode(&bytes, key)?;
    READ.fetch_add(1, Ordering::Relaxed);
    Some(code)
}

fn decode(bytes: &[u8], key: &str) -> Option<Vec<u8>> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (length, rest) = rest.split_first_chunk::<8>()?;
    let length = usize::try_from(u64::from_le_bytes(*length)).ok()?;
    let (stored, code) = rest.split_at_checked(length)?;
    // A whole DXBC container (FXC's or DXC's): its magic, and the size its
    // header gives (bytes 24 to 28) its length. Part of one would reach the
    // driver as bytecode, which reads past its end.
    let whole = code.starts_with(b"DXBC")
        && code.get(24..28).is_some_and(|size| {
            u32::from_le_bytes(size.try_into().unwrap()) as usize == code.len()
        });
    (stored == key.as_bytes() && whole).then(|| code.to_vec())
}

fn encode(key: &str, code: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MAGIC.len() + 8 + key.len() + code.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(key.len() as u64).to_le_bytes());
    bytes.extend_from_slice(key.as_bytes());
    bytes.extend_from_slice(code);
    bytes
}

/// Keep `code`, just compiled for `key`. A cache that cannot be written is
/// passed over (counted): the shader compiles again next launch.
pub(super) fn write(key: &str, code: &[u8]) {
    COMPILED.fetch_add(1, Ordering::Relaxed);
    let Some(dir) = dir() else {
        return;
    };
    let path = path(&dir, key);
    // A file of this writer's own, not the process's alone: threads making
    // the same pipeline at once (tests do) would write one file, and rename
    // it while another still writes it.
    static WRITES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
    let n = WRITES.fetch_add(1, Ordering::Relaxed);
    let partial = path.with_extension(format!("{}-{n}.partial", std::process::id()));
    let written = std::fs::create_dir_all(&dir).is_ok()
        && std::fs::write(&partial, encode(key, code)).is_ok()
        && std::fs::rename(&partial, &path).is_ok();
    if !written {
        UNWRITTEN.fetch_add(1, Ordering::Relaxed);
        let _ = std::fs::remove_file(&partial);
    }
}
