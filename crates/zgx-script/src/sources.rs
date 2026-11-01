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
