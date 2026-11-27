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
//! lent to each script's VM) calls [`compiled`] for each of its files and keeps
//! what it returns while it loads them.
//!
//! The bytecode is what loading the text makes, compiled here from the text,
//! never read from a file (Luau loads bytecode unchecked). A hit compares the
//! stored source with the code byte for byte: the hash finds the entry, it
//! doesn't vouch for it, so no source can stand for another. It hashes a
//! large source's ends alone ([`KEY_ENDS`]): sources that differ only in
//! their middles share a key and are told apart by that compare.
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
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Key {
    options: u32,
    len: usize,
    hash: u64,
}
/// At most this much of a source's start, and as much of its end, is
/// hashed for its key: a data module of megabytes (a feed in a string) is
/// hashed in microseconds, not a millisecond a megabyte, and a file of up
/// to twice this whole.
const KEY_ENDS: usize = 64 << 10;
impl Key {
    fn of(code: &[u8]) -> Self {
        let mut hasher = std::hash::DefaultHasher::new();
        if code.len() <= 2 * KEY_ENDS {
            hasher.write(code);
        } else {
            hasher.write(&code[..KEY_ENDS]);
            hasher.write(&code[code.len() - KEY_ENDS..]);
        }
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
    /// Compiled ahead: the worker that made it, which frees it ([`Home`]).
    home: Option<Home>,
}
/// Where what [`prefetch`]'s worker made goes back to be freed: on the
/// thread that allocated it. A data module's source and bytecode are
/// megabytes each, and an allocator keeps what another thread frees for the
/// thread that made it until that thread collects: freed on the main
/// thread as its `require` lets it go and never collected, it stayed out of
/// reach, and a window's peak footprint was 15 MB higher in a third of the
/// heavy list's starts. The worker frees what comes back, then rests
/// (`zgx::image::rest_thread`).
type Home = std::sync::mpsc::Sender<(Box<[u8]>, Option<Box<[u8]>>)>;
impl Drop for Compiled {
    fn drop(&mut self) {
        if let Some(home) = self.home.take() {
            // A worker gone frees nothing: what it sends back drops here.
            let _ = home.send((std::mem::take(&mut self.source), self.bytecode.take()));
        }
    }
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
            home: None,
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
                home: None,
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
                home: None,
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

/// A source, its key (made on the worker), its bytecode (none when it
/// doesn't compile), and the worker that frees them.
type Ready = (Key, Box<[u8]>, Option<Box<[u8]>>, Home);

/// What [`prefetch`] compiled ahead and the loading thread hasn't taken
/// yet: one session at a time, open from the first `prefetch` until a load
/// ends; another `prefetch` meanwhile (a second script) joins it.
struct Ahead {
    generation: u64,
    /// Sources and their bytecode, waiting for the loading thread's table.
    ready: Vec<Ready>,
    /// What is left to compile, while the session is open.
    work: Option<Work>,
    /// Files compiled in this session, and its workers still compiling.
    compiled: usize,
    workers: usize,
    /// Until the first load after it ends ([`release_prefetched`]).
    open: bool,
}
static AHEAD: std::sync::Mutex<Ahead> = std::sync::Mutex::new(Ahead {
    generation: 0,
    ready: Vec::new(),
    work: None,
    compiled: 0,
    workers: 0,
    open: false,
});
/// Told as a file's requires are put in `Work::todo`, or the last worker on
/// one leaves it with none.
static MORE: std::sync::Condvar = std::sync::Condvar::new();

/// What a session's workers have left to do.
#[derive(Default)]
struct Work {
    /// Files to compile, the next one last: what a file names goes over what
    /// was there, its first on top, so files are taken about in the order a
    /// load requires them (a file, its first require and all below that,
    /// then its second).
    todo: Vec<Todo>,
    /// Whose names led to what: a file is looked at once a script.
    seen: HashSet<(usize, std::path::PathBuf)>,
    /// Files compiled or being: one two scripts reach compiles once.
    taken: HashSet<std::path::PathBuf>,
    /// Files and bytes read.
    files: usize,
    bytes: usize,
    /// Workers on a file, which may put more on `todo`.
    busy: usize,
}
enum Todo {
    /// A script [`prefetch`] was asked for, and the packages its host lends.
    Script(std::path::PathBuf, Vec<(String, std::path::PathBuf)>),
    /// A file it leads to.
    File(std::path::PathBuf, std::sync::Arc<Roots>),
}
/// Where a script's modules are.
struct Roots {
    /// This script's, of all asked for: no other's, gone or not.
    id: usize,
    /// The script's folder, and `.luaurc`'s aliases and the packages lent,
    /// by name.
    root: std::path::PathBuf,
    aliases: HashMap<String, std::path::PathBuf>,
    /// The folders `require` takes modules from (the script's, each
    /// package's) where they really are.
    real: Vec<std::path::PathBuf>,
}
impl Roots {
    fn of(entry: &Path, packages: Vec<(String, std::path::PathBuf)>) -> Self {
        let root = entry.parent().unwrap_or(Path::new(".")).to_owned();
        let mut aliases = luaurc_aliases(&root);
        let real = std::iter::once(&root)
            .chain(packages.iter().map(|(_, dir)| dir))
            .filter_map(|dir| dir.canonicalize().ok())
            .collect();
        for (name, dir) in packages {
            aliases.entry(name.to_lowercase()).or_insert(dir);
        }
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        Self {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            root,
            aliases,
            real,
        }
    }
    /// Whether `path` is a file a `require` may read: really in one of the
    /// folders it takes modules from (not through a link out of it).
    fn hold(&self, path: &Path) -> bool {
        path.canonicalize()
            .is_ok_and(|path| self.real.iter().any(|root| path.starts_with(root)))
    }
}

/// At most this many workers compile ahead, and at most half the machine's
/// processors (one, where it has two): the app starts much else as it
/// launches, its window, the GPU, perhaps a process of its own.
const PREFETCH_WORKERS: usize = 4;
fn most_workers() -> usize {
    static MOST: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *MOST.get_or_init(|| {
        let processors = std::thread::available_parallelism().map_or(1, |n| n.get());
        (processors / 2).clamp(1, PREFETCH_WORKERS)
    })
}

/// Whether anything was ever compiled ahead: until then a `require` pays
/// one atomic load for it, a host that never asks nothing more.
static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `require`s that found their file compiled ahead.
static HITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Holds the workers before each file (for tests: a `require` meanwhile
/// compiles its own, never waiting for one).
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
thread_local! {
    /// The entries this thread took from a session and no `require` has
    /// loaded yet, held until its first load ends.
    static HELD: RefCell<Vec<Rc<Compiled>>> = RefCell::default();
}
/// At most this many files and this many bytes in all; a file may be as
/// large as all of them (a data module of megabytes, compiled in tens of
/// milliseconds, is the file worth compiling ahead most).
const PREFETCH_FILES: usize = 4096;
const PREFETCH_BYTES: usize = 16 << 20;
const PREFETCH_FILE: u64 = PREFETCH_BYTES as u64;

fn ahead() -> std::sync::MutexGuard<'static, Ahead> {
    AHEAD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The names `code` requires with a string literal (`require("./x")`,
/// `require "@ds/button"`), and where each is: what the workers follow. One
/// on a line after `--` is a comment's; a name built at run time isn't seen
/// (it compiles at `require`).
fn required_names(code: &[u8]) -> Vec<(usize, String)> {
    let mut names = Vec::new();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    // A data module of megabytes is searched at memory speed, not a
    // comparison of seven bytes at every one.
    let finder = memchr::memmem::Finder::new(b"require");
    let mut i = 0;
    while let Some(at) = finder.find(&code[i..]) {
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
            names.push((start, name.to_owned()));
        }
        i = j + 1 + end;
    }
    names
}

/// The strings of `code` that name a module under an alias (`"@app/page"`),
/// and where each is: what a script requires by another way than
/// `require("…")` (`pcall(require, "@app/theme")`, a function of its own that
/// requires what it is given, a list of its screens). `known` says whether
/// an alias is one; a string that names no file costs finding that out.
fn module_strings(code: &[u8], known: impl Fn(&str) -> bool) -> Vec<(usize, String)> {
    let mut names = Vec::new();
    for at in memchr::memchr_iter(b'@', code) {
        let Some(&quote) = at.checked_sub(1).and_then(|before| code.get(before)) else {
            continue;
        };
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        let part = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/');
        let len = code[at + 1..].iter().take_while(|b| part(b)).count();
        let rest = &code[at + 1..at + 1 + len];
        if code.get(at + 1 + len) != Some(&quote) {
            continue;
        }
        let Ok(name) = std::str::from_utf8(rest) else {
            continue;
        };
        let Some((alias, inside)) = name.split_once('/') else {
            continue;
        };
        if inside.is_empty()
            || inside.split('/').any(|part| part == "..")
            || !known(&alias.to_lowercase())
        {
            continue;
        }
        // In a comment (a line of a data module is megabytes: not looked
        // through for one).
        let near = at.saturating_sub(1024);
        let line = code[near..at]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(near, |n| near + n + 1);
        if code[line..at].windows(2).any(|w| w == b"--") {
            continue;
        }
        names.push((at, format!("@{name}")));
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

/// Compile `entry` and what it requires, and what those require, on worker
/// threads while the app's window and GPU are set up, so that the first
/// load's `require`s find their bytecode compiled ([`prefetched`]). Only
/// what the entry reaches by a string that names a module: a test, a type
/// file or a module no one names isn't read. Compiled from the text read, in
/// memory: the same as compiling it at `require`, which still reads the file
/// and uses the bytecode only for the same bytes. A file not ready in time
/// compiles at `require` as it always has; nothing waits for the workers.
pub(crate) fn prefetch(entry: &Path, packages: Vec<(String, std::path::PathBuf)>) {
    let (generation, start) = {
        let mut ahead = ahead();
        if !ahead.open {
            ahead.generation += 1;
            ahead.ready = Vec::new();
            ahead.work = Some(Work::default());
            ahead.compiled = 0;
            // An earlier session's workers stop on their own.
            ahead.workers = 0;
            ahead.open = true;
        }
        // After what an earlier script of the session leads to.
        let work = ahead.work.get_or_insert_default();
        work.todo
            .insert(0, Todo::Script(crate::normalize(entry), packages));
        let start = ahead.workers < most_workers();
        ahead.workers += usize::from(start);
        (ahead.generation, start)
    };
    STARTED.store(true, std::sync::atomic::Ordering::Relaxed);
    MORE.notify_one();
    if start {
        start_worker(generation);
    }
}

/// One more worker of the session `generation`, already counted.
fn start_worker(generation: u64) {
    if std::thread::Builder::new()
        .name("zgx-luau-ahead".into())
        .spawn(move || compile_files(generation))
        .is_err()
    {
        let mut ahead = ahead();
        if ahead.generation == generation {
            ahead.workers -= 1;
        }
    }
}

/// A worker, while it is counted in `Ahead::workers`: one that panics is
/// taken off, as one with nothing left takes itself off.
struct Counted(u64);
impl Drop for Counted {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let mut ahead = ahead();
            if ahead.generation == self.0 {
                ahead.workers = ahead.workers.saturating_sub(1);
            }
        }
    }
}
/// A worker on a file, while it may put more on `todo` (`Work::busy`); gone
/// once it has, or as it panics, so no other waits for it.
struct OnFile(u64);
impl Drop for OnFile {
    fn drop(&mut self) {
        {
            let mut ahead = ahead();
            if ahead.generation == self.0
                && let Some(work) = ahead.work.as_mut()
            {
                work.busy = work.busy.saturating_sub(1);
            }
        }
        MORE.notify_all();
    }
}

/// The next file for a worker of the session `generation`: waits while
/// another worker is on one that may name more. None once there is nothing
/// left, or the session ended, and the worker isn't counted any more.
fn next_file(generation: u64) -> Option<Todo> {
    let mut ahead = ahead();
    loop {
        if ahead.generation != generation {
            return None;
        }
        let open = ahead.open;
        if let Some(work) = ahead.work.as_mut().filter(|_| open) {
            if let Some(todo) = work.todo.pop() {
                work.busy += 1;
                return Some(todo);
            }
            if work.busy > 0 {
                ahead = MORE
                    .wait(ahead)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                continue;
            }
        }
        ahead.workers -= 1;
        return None;
    }
}

/// A worker: compiles files until the session has none left.
fn compile_files(generation: u64) {
    let (home, returned) = std::sync::mpsc::channel();
    let counted = Counted(generation);
    while let Some(todo) = next_file(generation) {
        let on_file = OnFile(generation);
        let (path, roots) = match todo {
            Todo::File(path, roots) => (path, roots),
            Todo::Script(entry, packages) => {
                let roots = Roots::of(&entry, packages);
                (entry, std::sync::Arc::new(roots))
            }
        };
        let read = read_file(generation, &path, &roots);
        let mut found = Vec::new();
        if let Some((code, _)) = &read {
            for name in names_in(code, &roots) {
                if let Some(next) = required_path(&name, &path, &roots.root, &roots.aliases) {
                    found.push(next);
                }
            }
        }
        // What it names, its first on top, before it compiles: the other
        // workers take them meanwhile.
        let start = put_found(generation, found, &roots);
        drop(on_file);
        for _ in 0..start {
            start_worker(generation);
        }
        // (A file another script's names led to has compiled, or will.)
        let Some((code, true)) = read else {
            continue;
        };
        let bytecode = mlua::chunk::Compiler::new()
            .compile(&code)
            .ok()
            .map(Into::into);
        let mut ahead = ahead();
        if ahead.generation == generation && ahead.open {
            ahead
                .ready
                .push((Key::of(&code), code.into(), bytecode, home.clone()));
            ahead.compiled += 1;
        }
    }
    drop(counted);
    // What it made comes back as each entry goes (most as their `require`s
    // load them, the rest as the load ends), freed here. Every entry gone,
    // the channel closes; one kept past the bound frees where it drops.
    drop(home);
    // What comes back, freed here and given back to the system
    // (`zgx::image::rest_thread`): the allocator keeps a thread's freed
    // pages until that thread collects them.
    while let Ok(back) = returned.recv_timeout(std::time::Duration::from_secs(30)) {
        drop(back);
        while let Ok(more) = returned.try_recv() {
            drop(more);
        }
        zgx::image::rest_thread();
    }
}

/// Puts what a worker's file names on the session's `todo`; how many more
/// workers there are files for, counted.
fn put_found(
    generation: u64,
    found: Vec<std::path::PathBuf>,
    roots: &std::sync::Arc<Roots>,
) -> usize {
    let mut ahead = ahead();
    if ahead.generation != generation {
        return 0;
    }
    let most = most_workers().saturating_sub(ahead.workers);
    let mut start = 0;
    if let Some(work) = ahead.work.as_mut() {
        for next in found.into_iter().rev() {
            if work.seen.insert((roots.id, next.clone())) {
                work.todo.push(Todo::File(next, roots.clone()));
            }
        }
        // (This worker compiles its file first.)
        start = work.todo.len().min(most);
    }
    ahead.workers += start;
    start
}

/// The file at `path`, and whether it is this session's first time there.
/// None where it isn't one `require` would take, or is past what a session
/// reads.
fn read_file(generation: u64, path: &Path, roots: &Roots) -> Option<(Vec<u8>, bool)> {
    let current = || {
        let ahead = ahead();
        ahead.generation == generation && ahead.open
    };
    // (A string for no file ends here.)
    let file = std::fs::metadata(path).ok()?;
    if !file.is_file() || file.len() > PREFETCH_FILE || !roots.hold(path) {
        return None;
    }
    while PAUSED.load(std::sync::atomic::Ordering::Relaxed) && current() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    #[cfg(test)]
    assert!(
        !path.ends_with("panics.luau"),
        "a test's worker panics here"
    );
    let code = std::fs::read(path).ok()?;
    let mut ahead = ahead();
    if ahead.generation != generation || !ahead.open {
        return None;
    }
    let work = ahead.work.as_mut()?;
    work.files += 1;
    work.bytes += code.len();
    if work.files > PREFETCH_FILES || work.bytes > PREFETCH_BYTES {
        work.todo.clear();
        return None;
    }
    let first = work.taken.insert(path.to_owned());
    Some((code, first))
}

/// The module names in `code`, in order: what it requires by literal, and
/// the strings that name one.
fn names_in(code: &[u8], roots: &Roots) -> Vec<String> {
    let mut names = required_names(code);
    names.extend(module_strings(code, |alias| {
        matches!(alias, "app" | "self") || roots.aliases.contains_key(alias)
    }));
    names.sort();
    names.into_iter().map(|(_, name)| name).collect()
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
        for (key, source, bytecode, home) in arrived {
            let entry = Rc::new(Compiled {
                source,
                bytecode,
                home: Some(home),
            });
            shared.entry(key).or_default().push(Rc::downgrade(&entry));
            HELD.with_borrow_mut(|held| held.push(entry));
        }
        shared
            .get(&Key::of(code))
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .find(|entry| entry.is(code))
    });
    if let Some(found) = &found {
        HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Its `require` holds it while it loads: not held to the load's end
        // too (a data module's source and bytecode, megabytes each, would
        // last through what the app does with its value).
        HELD.with_borrow_mut(|held| held.retain(|entry| !Rc::ptr_eq(entry, found)));
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
        ahead.work = None;
    }
    // Workers waiting for more leave.
    MORE.notify_all();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two sources that differ only between the ends their key hashes share
    /// a key, and are still told apart by their bytes: each compiles to its
    /// own bytecode, and each is found again by its own.
    #[test]
    fn sources_alike_at_their_ends_each_compile_their_own() {
        let pad = "-".repeat(KEY_ENDS);
        let source = |label: &str| format!("--{pad}\nreturn \"{label}\"\n--{pad}\n").into_bytes();
        let (one, two) = (source("one"), source("two"));
        assert_eq!(one.len(), two.len());
        assert!(
            one.len() > 2 * KEY_ENDS,
            "large enough that only its ends are hashed"
        );
        assert_eq!(
            Key::of(&one),
            Key::of(&two),
            "one key: only their middles differ"
        );
        let (a, fresh_a) = compiled(&one);
        let (b, fresh_b) = compiled(&two);
        assert!(
            fresh_a && fresh_b,
            "each compiled, neither taken for the other"
        );
        assert!(!Rc::ptr_eq(&a, &b));
        let (again, fresh) = compiled(&two);
        assert!(!fresh && Rc::ptr_eq(&again, &b), "found again by its bytes");
        let lua = mlua::Lua::new();
        for (entry, label) in [(&a, "one"), (&b, "two")] {
            let value: String = lua
                .load(entry.bytecode().expect("compiles"))
                .set_mode(mlua::chunk::ChunkMode::Binary)
                .eval()
                .unwrap();
            assert_eq!(value, label);
        }
    }
}

#[cfg(test)]
mod ahead_tests {
    use super::*;

    // (`cfg(test)`'s panic is this binary's alone, so the test is here and not
    // with the others in tests/compile_ahead.rs.)
    /// A worker that panics on a file leaves the others to what is left:
    /// they take it, and go, rather than waiting for a file that puts up no
    /// more.
    #[test]
    fn a_worker_that_panics_leaves_no_other_waiting() {
        // In a process of its own: the session is the process's, and any
        // other test's load ends it.
        const ALONE: &str = "ZGX_TEST_AHEAD_PANIC_ALONE";
        if std::env::var_os(ALONE).is_none() {
            let run = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "--test-threads=1"])
                .arg("sources::ahead_tests::a_worker_that_panics_leaves_no_other_waiting")
                .env(ALONE, "1")
                .output()
                .unwrap();
            let said = String::from_utf8_lossy(&run.stdout);
            assert!(
                run.status.success() && said.contains("1 passed"),
                "{said}\n{}",
                String::from_utf8_lossy(&run.stderr)
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!("zgx-ahead-panics-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = String::from("local a = require(\"./panics\")\n");
        std::fs::write(dir.join("panics.luau"), "return 1\n").unwrap();
        for i in 0..40 {
            std::fs::write(dir.join(format!("m{i}.luau")), "return 1\n").unwrap();
            app += &format!("local m{i} = require(\"./m{i}\")\n");
        }
        std::fs::write(dir.join("app.luau"), app).unwrap();
        prefetch(&dir.join("app.luau"), Vec::new());
        let started = std::time::Instant::now();
        while ahead_counts().0 < 41 && most_workers() > 1 || !ahead_counts().1 {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(20),
                "workers wait for one that panicked: {:?}",
                ahead_counts()
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // (One worker alone compiles the script and no more.)
        let compiled = ahead_counts().0;
        assert_eq!(compiled, if most_workers() > 1 { 41 } else { 1 });
        release_prefetched();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
