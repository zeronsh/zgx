//! Compiled scripts, shared by every host and VM on a thread, keyed by what
//! they are compiled from: the source's bytes and the compile options. Never
//! by path, host or VM, so the same text compiles once however many hosts,
//! VMs or paths load it.
//!
//! [`compiled`] is the lookup. What holds the `Rc<Compiled>` it returns keeps
//! the entry; the shared table holds it weakly, so it ends with its last user
//! and bounds itself. A hot-reloading host holds the compiled files of its
//! last load, by path, in its [`Sources`]: a reload compiles only the files
//! whose contents changed (every module still runs again), and a watcher
//! woken for files whose bytes are what is loaded (an editor saving the same
//! text, a checkout touching times) reloads nothing. Another user (a package
//! lent to each plugin's VM) calls [`compiled`] for each of its files and keeps
//! what it returns while it loads them.
//!
//! The bytecode is what loading the text makes, compiled here from the text,
//! never read from a file (Luau loads bytecode unchecked). A hit compares the
//! stored source with the code byte for byte: the hash finds the entry, it
//! doesn't vouch for it, so no source can stand for another.
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    hash::Hasher,
    path::Path,
    rc::{Rc, Weak},
};

/// The options [`compiled`] compiles with, part of every key: Luau's defaults
/// (`Compiler::new()`, from luacode.h), which `luaL_loadbuffer` compiles text
/// with, so the bytecode is what loading the text would make. A change of
/// options changes this tag. Luau's version can't differ within a process.
const OPTIONS: u32 = 1;

/// What finds an entry: the options, the source's length and a hash of it.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    options: u32,
    len: usize,
    hash: u64,
}
impl Key {
    fn of(code: &[u8]) -> Self {
        let mut hasher = std::hash::DefaultHasher::new();
        hasher.write(code);
        Self {
            options: OPTIONS,
            len: code.len(),
            hash: hasher.finish(),
        }
    }
}

/// A source and what compiling it made.
pub(crate) struct Compiled {
    source: Box<[u8]>,
    /// None when it doesn't compile: its load reads the text, so the error
    /// shows as it always has.
    bytecode: Option<Box<[u8]>>,
}
impl Compiled {
    pub(crate) fn bytecode(&self) -> Option<&[u8]> {
        self.bytecode.as_deref()
    }
    fn is(&self, code: &[u8]) -> bool {
        *self.source == *code
    }
}

thread_local! {
    /// Every entry in use on this thread, held weakly.
    static SHARED: RefCell<HashMap<Key, Vec<Weak<Compiled>>>> = RefCell::default();
}

/// `code` compiled, with the options of [`OPTIONS`]: the entry another host
/// or VM on this thread compiled from the same bytes, or compiled now. The
/// second value is whether it was compiled now. Keep the `Rc` as long as
/// the entry should be found.
pub(crate) fn compiled(code: &[u8]) -> (Rc<Compiled>, bool) {
    let key = Key::of(code);
    SHARED.with_borrow_mut(|shared| {
        if let Some(found) = shared
            .get(&key)
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .find(|entry| entry.is(code))
        {
            return (found, false);
        }
        let entry = Rc::new(Compiled {
            source: code.into(),
            bytecode: mlua::chunk::Compiler::new()
                .compile(code)
                .ok()
                .map(Into::into),
        });
        // Compiling is rare (a file changed): drop the entries no one holds.
        shared.retain(|_, entries| {
            entries.retain(|e| e.strong_count() > 0);
            !entries.is_empty()
        });
        shared.entry(key).or_default().push(Rc::downgrade(&entry));
        (entry, true)
    })
}

/// Packages' modules compiled (`ScriptHost::package`), shared by every host
/// on every thread: a package's files are the same for each host that
/// requires them (a video's stages one after another, render workers, a
/// player building its stage again), so each compiles once until a host goes
/// idle: [`release_packages`], run then, drops them all. No host holds them
/// (its VM keeps what it loaded), so a process at rest keeps none, released
/// at that point and never as a collector runs. A hot-reloading host keeps
/// its own by path, in its [`Sources`].
static PACKAGES: std::sync::Mutex<Vec<std::sync::Arc<Compiled>>> =
    std::sync::Mutex::new(Vec::new());

fn packages() -> std::sync::MutexGuard<'static, Vec<std::sync::Arc<Compiled>>> {
    PACKAGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A package module's `code` compiled: the entry another host compiled from
/// the same bytes, what was compiled ahead, or compiled now (outside the
/// lock: another thread's requires don't wait for it). The second value is
/// whether it was compiled now.
pub(crate) fn package_compiled(code: &[u8]) -> (std::sync::Arc<Compiled>, bool) {
    let found =
        |entries: &[std::sync::Arc<Compiled>]| entries.iter().find(|entry| entry.is(code)).cloned();
    if let Some(entry) = found(&packages()) {
        return (entry, false);
    }
    // Compiled ahead as the app's window was made (`prefetch`), or now.
    let (entry, now) = match prefetched(code) {
        Some(ahead) => (
            Compiled {
                source: ahead.source.clone(),
                bytecode: ahead.bytecode.clone(),
            },
            false,
        ),
        None => (
            Compiled {
                source: code.into(),
                bytecode: mlua::chunk::Compiler::new()
                    .compile(code)
                    .ok()
                    .map(Into::into),
            },
            true,
        ),
    };
    let entry = std::sync::Arc::new(entry);
    let mut entries = packages();
    // Another thread may have compiled the same meanwhile: one entry each.
    if let Some(entry) = found(&entries) {
        return (entry, false);
    }
    entries.push(entry.clone());
    (entry, now)
}

/// Drops every package module compiled (a host went idle).
pub(crate) fn release_packages() {
    // Taken out under the lock, dropped outside it.
    let entries = std::mem::take(&mut *packages());
    drop(entries);
}

/// How many packages' modules are compiled and kept (for tests).
pub(crate) fn packages_kept() -> usize {
    packages().len()
}

/// A host's compiled files, by path, once it hot-reloads.
#[derive(Default)]
pub(crate) struct Sources {
    /// Whether the host hot-reloads: before it does, it holds nothing.
    hot: Cell<bool>,
    /// By the path's bytes, not `Path`: hashing a `Path` here as well as in
    /// `require`'s module table took `Path::hash` out of line there, a cost
    /// every load paid.
    by_path: RefCell<HashMap<OsString, Rc<Compiled>>>,
    /// Files compiled for it since it began hot-reloading.
    compiles: Cell<u64>,
}
impl Sources {
    /// The host hot-reloads: hold what its loads compile from now on.
    pub(crate) fn hot_reload(&self) {
        self.hot.set(true);
    }
    pub(crate) fn hot(&self) -> bool {
        self.hot.get()
    }
    /// `code`, read from the disk at `path`, compiled ([`compiled`]) and held
    /// for `path`. None while the host doesn't hot-reload.
    pub(crate) fn compiled(&self, path: &Path, code: &[u8]) -> Option<Rc<Compiled>> {
        if !self.hot.get() {
            return None;
        }
        if let Some(held) = self.by_path.borrow().get(path.as_os_str())
            && held.is(code)
        {
            return Some(held.clone());
        }
        let (entry, fresh) = compiled(code);
        if fresh {
            self.compiles.set(self.compiles.get() + 1);
        }
        self.by_path
            .borrow_mut()
            .insert(path.as_os_str().to_owned(), entry.clone());
        Some(entry)
    }
    /// Whether each of `paths` holds what its last load read: false for one
    /// not loaded since the host began hot-reloading, or unreadable.
    pub(crate) fn as_loaded(&self, paths: &[&Path]) -> bool {
        let by_path = self.by_path.borrow();
        paths.iter().all(
            |path| match (by_path.get(path.as_os_str()), std::fs::read(path)) {
                (Some(held), Ok(code)) => held.is(&code),
                _ => false,
            },
        )
    }
    /// Let go of every file but `paths`, the last load's: one renamed or
    /// no longer required goes (and from the shared table, once no one else
    /// holds it).
    pub(crate) fn keep_only<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        let keep: HashSet<&OsStr> = paths.into_iter().map(Path::as_os_str).collect();
        self.by_path
            .borrow_mut()
            .retain(|path, _| keep.contains(path.as_os_str()));
    }
    /// The files held, and how many were compiled for this host since it
    /// began hot-reloading.
    pub(crate) fn counts(&self) -> (usize, u64) {
        (self.by_path.borrow().len(), self.compiles.get())
    }
}

/// A source and its bytecode (none when it doesn't compile).
type Ready = (Box<[u8]>, Option<Box<[u8]>>);

/// What [`prefetch`] compiled ahead and the loading thread hasn't taken
/// yet: one session at a time, open from the first `prefetch` until a load
/// ends; another `prefetch` meanwhile (a second directory) joins it.
struct Ahead {
    generation: u64,
    /// Sources and their bytecode, waiting for the loading thread's table.
    ready: Vec<Ready>,
    /// Files compiled in this session, and its workers still walking.
    compiled: usize,
    workers: usize,
    /// Until the first load after it ends ([`release_prefetched`]).
    open: bool,
}
static AHEAD: std::sync::Mutex<Ahead> = std::sync::Mutex::new(Ahead {
    generation: 0,
    ready: Vec::new(),
    compiled: 0,
    workers: 0,
    open: false,
});
/// Whether anything was ever compiled ahead: until then a `require` pays
/// one atomic load for it, a host that never asks nothing more.
static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `require`s that found their file compiled ahead.
static HITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Holds the workers before each file (for tests: a `require` meanwhile
/// compiles its own, never waiting for one).
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
thread_local! {
    /// The entries this thread took from a session, held until its first
    /// load ends.
    static HELD: RefCell<Vec<Rc<Compiled>>> = RefCell::default();
}
/// At most this many files, this many bytes in all, and none larger than
/// `PREFETCH_FILE`.
const PREFETCH_FILES: usize = 4096;
const PREFETCH_BYTES: usize = 16 << 20;
const PREFETCH_FILE: u64 = 1 << 20;

fn ahead() -> std::sync::MutexGuard<'static, Ahead> {
    AHEAD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The names `code` requires with a string literal (`require("./x")`,
/// `require "@ds/button"`), in order: what the worker follows. One on a
/// line after `--` is a comment's; a name built at run time isn't seen (it
/// compiles at `require`).
fn required_names(code: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while let Some(at) = code[i..].windows(7).position(|w| w == b"require") {
        let start = i + at;
        i = start + 7;
        if start > 0 && word(code[start - 1]) || code.get(i).is_some_and(|&b| word(b)) {
            continue;
        }
        let line = code[..start]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |n| n + 1);
        if code[line..start].windows(2).any(|w| w == b"--") {
            continue;
        }
        let mut j = i;
        let skip = |j: &mut usize| {
            while code.get(*j).is_some_and(u8::is_ascii_whitespace) {
                *j += 1;
            }
        };
        skip(&mut j);
        if code.get(j) == Some(&b'(') {
            j += 1;
            skip(&mut j);
        }
        let Some(&quote) = code.get(j).filter(|&&q| q == b'"' || q == b'\'') else {
            continue;
        };
        let rest = &code[j + 1..];
        let Some(end) = rest
            .iter()
            .position(|&b| b == quote || b == b'\\' || b == b'\n')
        else {
            continue;
        };
        if rest[end] == quote
            && let Ok(name) = std::str::from_utf8(&rest[..end])
        {
            names.push(name.to_owned());
        }
        i = j + 1 + end;
    }
    names
}

/// Where `require(name)` from `caller` finds its file, as `require` resolves
/// it (`@app/`, `@self`, `.luaurc` aliases and packages, `./` and `../`, the
/// root otherwise; a directory's `init.luau`; `.luau` added). None where it
/// can't say (`@app-original`, an unknown alias): that one compiles at
/// `require`.
fn required_path(
    name: &str,
    caller: &Path,
    root: &Path,
    aliases: &HashMap<String, std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let here = caller.parent()?;
    let joined = if let Some(rest) = name.strip_prefix("@app/") {
        root.join(rest)
    } else if let Some(rest) = name.strip_prefix("@self") {
        here.join(rest.trim_start_matches('/'))
    } else if let Some(rest) = name.strip_prefix('@') {
        let (alias, rest) = rest.split_once('/').unwrap_or((rest, ""));
        aliases.get(&alias.to_lowercase())?.join(rest)
    } else if name.starts_with("./") || name.starts_with("../") {
        let base = if caller.file_stem().is_some_and(|stem| stem == "init") {
            here.parent().unwrap_or(here)
        } else {
            here
        };
        base.join(name)
    } else {
        root.join(name)
    };
    let mut path = crate::normalize(&joined);
    if path.is_dir() {
        path.push("init.luau");
    } else if path.extension().is_none() {
        path.set_extension("luau");
    }
    Some(path)
}

/// `.luaurc`'s aliases in `root`, lower-cased and resolved against it, as
/// `require` reads them (quietly: `require` reports a broken one).
fn luaurc_aliases(root: &Path) -> HashMap<String, std::path::PathBuf> {
    let Ok(text) = std::fs::read_to_string(root.join(".luaurc")) else {
        return HashMap::new();
    };
    let Ok(config) = serde_json::from_str::<serde_json::Value>(&text) else {
        return HashMap::new();
    };
    config
        .get("aliases")
        .and_then(|a| a.as_object())
        .into_iter()
        .flatten()
        .filter_map(|(name, path)| {
            Some((
                name.to_lowercase(),
                crate::normalize(&root.join(path.as_str()?)),
            ))
        })
        .collect()
}

/// Compile `entry` and what it requires, and what those require, on a worker
/// thread while the app's window and GPU are set up, so that the first
/// load's `require`s find their bytecode compiled ([`prefetched`]). Only
/// what the entry reaches by a string literal: a test, a type file or a
/// module no one requires isn't read. Compiled from the text read, in
/// memory: the same as compiling it at `require`, which still reads the file
/// and uses the bytecode only for the same bytes. A file not ready in time
/// compiles at `require` as it always has; nothing waits for the worker.
pub(crate) fn prefetch(entry: &Path, packages: Vec<(String, std::path::PathBuf)>) {
    let generation = {
        let mut ahead = ahead();
        if !ahead.open {
            ahead.generation += 1;
            ahead.ready = Vec::new();
            ahead.compiled = 0;
            // An earlier session's workers stop on their own.
            ahead.workers = 0;
            ahead.open = true;
        }
        ahead.workers += 1;
        ahead.generation
    };
    STARTED.store(true, std::sync::atomic::Ordering::Relaxed);
    let entry = crate::normalize(entry);
    let worker = move || {
        let current = || {
            let ahead = ahead();
            ahead.generation == generation && ahead.open
        };
        let root = entry.parent().unwrap_or(Path::new(".")).to_owned();
        let mut aliases = luaurc_aliases(&root);
        for (name, dir) in packages {
            aliases.entry(name.to_lowercase()).or_insert(dir);
        }
        let mut queue = std::collections::VecDeque::from([entry]);
        let mut seen = HashSet::new();
        let (mut files, mut bytes) = (0, 0);
        while let Some(path) = queue.pop_front() {
            if !seen.insert(path.clone())
                || std::fs::metadata(&path)
                    .map_or(true, |m| !m.is_file() || m.len() > PREFETCH_FILE)
            {
                continue;
            }
            while PAUSED.load(std::sync::atomic::Ordering::Relaxed) {
                if !current() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let Ok(code) = std::fs::read(&path) else {
                continue;
            };
            files += 1;
            bytes += code.len();
            if files > PREFETCH_FILES || bytes > PREFETCH_BYTES {
                break;
            }
            for name in required_names(&code) {
                if let Some(next) = required_path(&name, &path, &root, &aliases) {
                    queue.push_back(next);
                }
            }
            let bytecode = mlua::chunk::Compiler::new()
                .compile(&code)
                .ok()
                .map(Into::into);
            let mut ahead = ahead();
            if ahead.generation != generation || !ahead.open {
                break;
            }
            ahead.ready.push((code.into(), bytecode));
            ahead.compiled += 1;
        }
        let mut ahead = ahead();
        if ahead.generation == generation {
            ahead.workers -= 1;
        }
    };
    if std::thread::Builder::new()
        .name("zgx-luau-ahead".into())
        .spawn(worker)
        .is_err()
    {
        ahead().workers -= 1;
    }
}

/// Hold the ahead workers before their next file, or let them go (tests).
pub(crate) fn pause_prefetch(paused: bool) {
    PAUSED.store(paused, std::sync::atomic::Ordering::Relaxed);
}

/// `code` compiled ahead ([`prefetch`]), if it was: found in this thread's
/// table (the bytes compared), never compiled here.
pub(crate) fn prefetched(code: &[u8]) -> Option<Rc<Compiled>> {
    if !STARTED.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let arrived = {
        let mut ahead = ahead();
        if !ahead.open {
            return None;
        }
        std::mem::take(&mut ahead.ready)
    };
    let found = SHARED.with_borrow_mut(|shared| {
        for (source, bytecode) in arrived {
            let entry = Rc::new(Compiled { source, bytecode });
            shared
                .entry(Key::of(&entry.source))
                .or_default()
                .push(Rc::downgrade(&entry));
            HELD.with_borrow_mut(|held| held.push(entry));
        }
        shared
            .get(&Key::of(code))
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .find(|entry| entry.is(code))
    });
    if found.is_some() {
        HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    found
}

/// A load is done: let go of what was compiled ahead (a hot-reloading host
/// holds what it required; the rest goes) and end the session.
pub(crate) fn release_prefetched() {
    if !STARTED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    {
        let mut ahead = ahead();
        if !ahead.open {
            return;
        }
        ahead.open = false;
        ahead.ready = Vec::new();
    }
    HELD.with_borrow_mut(|held| *held = Vec::new());
    SHARED.with_borrow_mut(|shared| {
        shared.retain(|_, entries| {
            entries.retain(|e| e.strong_count() > 0);
            !entries.is_empty()
        });
    });
}

/// The current session's files compiled, whether its workers are done,
/// whether it is still open, `require`s that found theirs compiled ahead in
/// all sessions, and the entries this thread holds from it (for tests).
pub(crate) fn ahead_counts() -> (usize, bool, bool, usize, usize) {
    let ahead = ahead();
    (
        ahead.compiled,
        ahead.workers == 0,
        ahead.open,
        HITS.load(std::sync::atomic::Ordering::Relaxed),
        HELD.with_borrow(Vec::len),
    )
}

/// Releases what was compiled ahead as it drops ([`release_prefetched`]):
/// held for the length of a host's load.
pub(crate) struct ReleaseAhead;
impl Drop for ReleaseAhead {
    fn drop(&mut self) {
        release_prefetched();
    }
}
