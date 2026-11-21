//! A bundle's `ui/` as `app_bundle::open` found it: each file's SHA-256,
//! from its manifest, against which each file is checked as it is read
//! (`Files::Folder`). Apart from `app_bundle` because a host names it on
//! every target, the browser too, where no bundle opens.
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// A bundle's `ui/` and its files' sums. Made by `app_bundle::open` alone,
/// so a host reading a folder as a bundle's (`ScriptFiles::Folder`) reads
/// only files that check out.
#[derive(Clone, Debug)]
pub struct Checked {
    pub(crate) dir: Arc<Path>,
    sums: Arc<HashMap<String, [u8; 32]>>,
    ahead: Arc<Ahead>,
}

/// Scripts read and checked ahead ([`Checked::read_ahead`]), each kept as
/// the bytes that were checked until a host reads it.
#[derive(Debug, Default)]
struct Ahead {
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
    pub(crate) fn new(dir: Arc<Path>, sums: Arc<HashMap<String, [u8; 32]>>) -> Self {
        Self {
            dir,
            sums,
            ahead: Arc::default(),
        }
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
            .ahead
            .state
            .compare_exchange(NOT_YET, READING, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        let mut scripts: Vec<&String> = self
            .sums
            .keys()
            .filter(|rel| rel.ends_with(".luau"))
            .collect();
        scripts.sort();
        let scripts: Vec<String> = scripts.into_iter().cloned().collect();
        let scripts = Arc::new(scripts);
        for worker in 0..WORKERS {
            let (me, scripts) = (self.clone(), scripts.clone());
            self.ahead.workers.fetch_add(1, Ordering::AcqRel);
            let spawned = std::thread::Builder::new()
                .name("zgx-read-ahead".into())
                .spawn(move || {
                    for rel in scripts.iter().skip(worker).step_by(WORKERS) {
                        if me.ahead.state.load(Ordering::Acquire) != READING {
                            break;
                        }
                        let Ok(bytes) = std::fs::read(me.dir.join(rel)) else {
                            continue;
                        };
                        if Sha256::digest(&bytes)[..] != me.sums[rel][..] {
                            continue;
                        }
                        let mut ready = me.ahead.ready.lock().unwrap_or_else(|e| e.into_inner());
                        if me.ahead.state.load(Ordering::Acquire) != READING {
                            break;
                        }
                        ready.insert(rel.clone(), bytes);
                    }
                    me.ahead.workers.fetch_sub(1, Ordering::AcqRel);
                });
            if spawned.is_err() {
                self.ahead.workers.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    /// Lets go of what was read ahead and no load took, and stops reading.
    pub(crate) fn release_ahead(&self) {
        if self.ahead.state.swap(DONE, Ordering::AcqRel) == READING {
            self.ahead
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
            .ahead
            .ready
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        (ready, self.ahead.workers.load(Ordering::Acquire) == 0)
    }

    /// The file at `path`, under its `ui/`, if it is the one its manifest
    /// names: a file the manifest doesn't name isn't there; one changed
    /// since the build is refused. A script read ahead is the bytes that
    /// were checked, handed over.
    pub(crate) fn read(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        use std::io::{Error, ErrorKind};
        let absent = || Error::new(ErrorKind::NotFound, "not in the app's bundle");
        let rel = path.strip_prefix(&*self.dir).map_err(|_| absent())?;
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
        let sum = self.sums.get(&key).ok_or_else(absent)?;
        if self.ahead.state.load(Ordering::Acquire) == READING {
            let taken = self
                .ahead
                .ready
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&key);
            if let Some(bytes) = taken {
                return Ok(bytes);
            }
        }
        let bytes = std::fs::read(path)?;
        if Sha256::digest(&bytes)[..] != sum[..] {
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
