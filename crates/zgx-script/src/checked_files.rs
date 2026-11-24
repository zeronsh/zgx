//! A bundle's `ui/` as `app_bundle::open` found it: each file's SHA-256,
//! from its manifest, against which each file is checked as it is read
//! (`Files::Folder`). Apart from `app_bundle` because a host names it on
//! every target, the browser too, where no bundle opens.
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// A bundle's `ui/` and its files' sums. Made by `app_bundle::open` alone,
/// so a host reading a folder as a bundle's (`ScriptFiles::Folder`) reads
/// only files that check out. One pointer: every host's `Files` holds one,
/// and is no larger for it.
#[derive(Clone, Debug)]
pub struct Checked(Arc<Folder>);

#[derive(Debug)]
struct Folder {
    dir: PathBuf,
    sums: HashMap<String, [u8; 32]>,
    ahead: Ahead,
}

/// Scripts read and checked ahead ([`Checked::read_ahead`]), each kept as
/// the bytes that were checked until a host reads it.
#[derive(Debug, Default)]
struct Ahead {
    /// What reads cost the threads that asked (`ZGX_BUNDLE_TIMING`): files
    /// taken ready, files read and hashed there, their bytes, and the time.
    taken: std::sync::atomic::AtomicU64,
    read_here: std::sync::atomic::AtomicU64,
    bytes_here: std::sync::atomic::AtomicU64,
    nanos_here: std::sync::atomic::AtomicU64,
    /// Not started, reading, or let go (`NOT_YET`, `READING`, `DONE`).
    state: AtomicU8,
    /// Workers still reading.
    workers: AtomicU8,
    ready: Mutex<HashMap<String, Vec<u8>>>,
}
const NOT_YET: u8 = 0;
const READING: u8 = 1;
const DONE: u8 = 2;
/// How many workers read ahead: the first load's scripts are ready before
/// it starts. (Zeron's 487, 6.6 MB, take one core of an M-series Mac 7.5
/// ms to read and hash with the disk's cache warm and 39 cold, while its
/// window's GPU and fonts take some 60.)
const WORKERS: usize = 2;

impl Checked {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn new(dir: PathBuf, sums: HashMap<String, [u8; 32]>) -> Self {
        Self(Arc::new(Folder {
            dir,
            sums,
            ahead: Ahead::default(),
        }))
    }

    /// Its `ui/` (canonical).
    pub(crate) fn dir(&self) -> &Path {
        &self.0.dir
    }

    /// Reads and checks its scripts on worker threads, as the app does
    /// whatever comes before its first load (its window, the GPU, fonts):
    /// a `require` of one finds the bytes the worker checked, and takes
    /// them, so what runs is what was checked, with no reading or hashing
    /// on the thread that loads. One not ready yet, or that didn't check out
    /// (whose read says why), is read and checked as it is asked for. What
    /// no load took goes as the first load ends. Once: later calls do
    /// nothing.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn read_ahead(&self) {
        if self
            .0
            .ahead
            .state
            .compare_exchange(NOT_YET, READING, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        for worker in 0..WORKERS {
            let me = self.clone();
            self.0.ahead.workers.fetch_add(1, Ordering::AcqRel);
            let spawned = std::thread::Builder::new()
                .name("zgx-read-ahead".into())
                .spawn(move || {
                    // (Listed here, not on the thread that asked.)
                    let mut scripts: Vec<&String> =
                        me.0.sums
                            .keys()
                            .filter(|rel| rel.ends_with(".luau"))
                            .collect();
                    scripts.sort();
                    for rel in scripts.into_iter().skip(worker).step_by(WORKERS) {
                        if me.0.ahead.state.load(Ordering::Acquire) != READING {
                            break;
                        }
                        let Ok(bytes) = std::fs::read(me.0.dir.join(rel)) else {
                            continue;
                        };
                        if Sha256::digest(&bytes)[..] != me.0.sums[rel][..] {
                            continue;
                        }
                        let mut ready = me.0.ahead.ready.lock().unwrap_or_else(|e| e.into_inner());
                        if me.0.ahead.state.load(Ordering::Acquire) != READING {
                            break;
                        }
                        ready.insert(rel.clone(), bytes);
                    }
                    me.0.ahead.workers.fetch_sub(1, Ordering::AcqRel);
                });
            if spawned.is_err() {
                self.0.ahead.workers.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    /// Lets go of what was read ahead and no load took, and stops reading.
    pub(crate) fn release_ahead(&self) {
        if zgx::env::var_os("ZGX_BUNDLE_TIMING").is_some() {
            let count = |n: &std::sync::atomic::AtomicU64| n.swap(0, Ordering::Relaxed);
            eprintln!(
                "zgx: bundle files as a load ended: {} taken ready, {} read and checked by the loading thread ({:.2} MB, {:.2} ms), {} ready untaken",
                count(&self.0.ahead.taken),
                count(&self.0.ahead.read_here),
                count(&self.0.ahead.bytes_here) as f64 / 1e6,
                count(&self.0.ahead.nanos_here) as f64 / 1e6,
                self.0
                    .ahead
                    .ready
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .len(),
            );
        }
        if self.0.ahead.state.swap(DONE, Ordering::AcqRel) == READING {
            self.0
                .ahead
                .ready
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        }
    }

    /// How many scripts wait read and checked, and whether the workers
    /// are done. For tests.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn ahead_counts(&self) -> (usize, bool) {
        let ready = self
            .0
            .ahead
            .ready
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        (ready, self.0.ahead.workers.load(Ordering::Acquire) == 0)
    }

    /// Whether its manifest names the file `rel` (its path under `ui/`, with
    /// `/`).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn names(&self, rel: &str) -> bool {
        self.0.sums.contains_key(rel)
    }

    /// The file at `path`, under its `ui/`, if it is the one its manifest
    /// names: a file the manifest doesn't name isn't there; one changed
    /// since the build is refused. A script read ahead is the bytes that
    /// were checked, handed over.
    pub(crate) fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        use std::io::{Error, ErrorKind};
        let absent = || Error::new(ErrorKind::NotFound, "not in the app's bundle");
        let rel = path.strip_prefix(&self.0.dir).map_err(|_| absent())?;
        let mut key = String::new();
        for part in rel.components() {
            match part {
                Component::Normal(name) => {
                    if !key.is_empty() {
                        key.push('/');
                    }
                    key.push_str(name.to_str().ok_or_else(absent)?);
                }
                Component::CurDir => {}
                _ => return Err(absent()),
            }
        }
        let sum = self.0.sums.get(&key).ok_or_else(absent)?;
        if self.0.ahead.state.load(Ordering::Acquire) == READING {
            let taken = self
                .0
                .ahead
                .ready
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&key);
            if let Some(bytes) = taken {
                self.0.ahead.taken.fetch_add(1, Ordering::Relaxed);
                return Ok(bytes);
            }
        }
        let started = std::time::Instant::now();
        let bytes = std::fs::read(path)?;
        let matches = Sha256::digest(&bytes)[..] == sum[..];
        self.0.ahead.read_here.fetch_add(1, Ordering::Relaxed);
        self.0
            .ahead
            .bytes_here
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        self.0
            .ahead
            .nanos_here
            .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
        if !matches {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "isn't the file its bundle's manifest names (changed since it was built)",
            ));
        }
        Ok(bytes)
    }
}

/// Lets go of what a bundle's files read ahead as it drops: held for the
/// length of a host's load, as `sources::ReleaseAhead` is.
pub(crate) struct ReleaseRead(pub(crate) Option<Checked>);
impl Drop for ReleaseRead {
    fn drop(&mut self) {
        if let Some(checked) = &self.0 {
            checked.release_ahead();
        }
    }
}
