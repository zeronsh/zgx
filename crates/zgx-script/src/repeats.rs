//! What an app's modules ask for again while it loads: most of a load's
//! requires name a module that has loaded, and what each would find by walking
//! its path (a folder or a file, in its folder or not) is the module it found
//! the last time.
//!
//! A name, as written, from a folder is kept with the path it led to once the
//! module there has loaded, and answered from what loaded: nothing is read,
//! and nothing that wasn't looked at then is taken. So a file or a folder
//! changed on the disk after its module loaded, in the same load, changes
//! nothing of what the names already asked by answer (a link put in a module's
//! place, out of its folder, is refused a name that asks for it first). Kept
//! for the load alone, and for the app's VM alone: a plugin's requires are
//! looked at each time, as are `@app/`'s, which `ui.override` changes, and
//! those of a module a plugin replaced one with.
//!
//! Such a module runs in the app's VM under its own path, the plugin's, as a
//! chunk's name is always the path of its file: what it asks is kept, or not,
//! under the plugin's folder, which is no folder of the app's by another name.
//! Where the two are one (a plugin inside the app's folder), a name asked from
//! it leads to one path, whoever asks: only `@app/` is answered by who does.
use rustc_hash::FxHashMap;
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

/// The paths names led to, by [`key`]; none but in a load.
type Paths = Rc<RefCell<Option<FxHashMap<Box<str>, PathBuf>>>>;

thread_local! {
    /// The VMs made in the load this thread is in; None while nothing loads.
    static LOADING: RefCell<Option<Vec<Paths>>> = const { RefCell::new(None) };
}

/// A host's load, on this thread: what its VMs ask for is kept until the
/// outermost ends.
pub(crate) struct Load(bool);
impl Load {
    pub(crate) fn begin() -> Self {
        LOADING.with_borrow_mut(|loading| {
            let outermost = loading.is_none();
            loading.get_or_insert_default();
            Self(outermost)
        })
    }
}
impl Drop for Load {
    fn drop(&mut self) {
        if self.0 {
            for paths in LOADING.take().unwrap_or_default() {
                paths.take();
            }
        }
    }
}

/// One VM's.
#[derive(Default)]
pub(crate) struct Repeats(Paths);
impl Repeats {
    /// Of the app's VM being made: kept to the end of the load it is made in.
    pub(crate) fn of_the_app() -> Self {
        let repeats = Self::default();
        LOADING.with_borrow_mut(|loading| {
            if let Some(vms) = loading {
                *repeats.0.borrow_mut() = Some(FxHashMap::default());
                vms.push(repeats.0.clone());
            }
        });
        repeats
    }
    /// What `name`, asked by the chunk called `chunk` (its path in the root,
    /// as `require` names it), is kept under. None where nothing is kept.
    pub(crate) fn key(&self, chunk: Option<&str>, name: &str) -> Option<Box<str>> {
        if self.0.borrow().is_none()
            || name.starts_with("@app/")
            || name.starts_with("@app-original/")
        {
            return None;
        }
        let (folder, init) = match chunk {
            None => (Path::new(""), false),
            Some(chunk) => {
                let chunk = Path::new(chunk);
                // (An `init.luau`'s relative names start elsewhere.)
                let init = chunk.file_stem().is_some_and(|stem| stem == "init");
                (chunk.parent()?, init)
            }
        };
        let folder = folder.to_str()?;
        let mut key = String::with_capacity(folder.len() + name.len() + 2);
        key.push_str(folder);
        // (No path has a NUL: the folder ends at the first.)
        key.push_str(if init { "\0i" } else { "\0f" });
        key.push_str(name);
        Some(key.into_boxed_str())
    }
    /// The path `key` led to, as `with` takes it.
    pub(crate) fn path<T>(&self, key: &str, with: impl FnOnce(&Path) -> Option<T>) -> Option<T> {
        let paths = self.0.borrow();
        with(paths.as_ref()?.get(key)?)
    }
    /// `key` led to the module at `path`, which has loaded.
    pub(crate) fn keep(&self, key: Box<str>, path: &Path) {
        if let Some(paths) = &mut *self.0.borrow_mut() {
            paths.insert(key, path.to_path_buf());
        }
    }
    /// How many names are kept (for tests), None out of a load.
    #[cfg(test)]
    fn len(&self) -> Option<usize> {
        self.0.borrow().as_ref().map(FxHashMap::len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_kept_by_the_folder_it_is_asked_from() {
        let _load = Load::begin();
        let repeats = Repeats::of_the_app();
        let key = |chunk, name| repeats.key(chunk, name);
        let a = key(Some("ds/a.luau"), "./m");
        assert!(a.is_some());
        assert_eq!(a, key(Some("ds/b.luau"), "./m"), "another file's, there");
        for other in [
            key(Some("ds/init.luau"), "./m"),
            key(Some("ds/sub/a.luau"), "./m"),
            key(Some("a.luau"), "./m"),
            key(Some("ds/a.luau"), "m"),
            key(Some("ds/a.luau"), "./M"),
        ] {
            assert!(other.is_some());
            assert_ne!(a, other);
        }
        assert_eq!(key(None, "m"), key(Some("app.luau"), "m"), "the root's");
        assert!(key(Some("a.luau"), "@apple/m").is_some());
        // What an override changes isn't.
        assert_eq!(key(Some("a.luau"), "@app/m"), None);
        assert_eq!(key(Some("a.luau"), "@app-original/m"), None);
    }

    #[test]
    fn a_load_keeps_to_its_end_and_nothing_is_kept_out_of_one() {
        let before = Repeats::of_the_app();
        assert_eq!(before.len(), None);
        assert_eq!(before.key(Some("a.luau"), "m"), None);
        let outer = Load::begin();
        let repeats = Repeats::of_the_app();
        let key = || repeats.key(Some("a.luau"), "m").expect("in a load");
        assert_eq!(repeats.path(&key(), |_| Some(())), None);
        {
            let _inner = Load::begin();
            repeats.keep(key(), Path::new("/m.luau"));
        }
        let path = repeats.path(&key(), |path| Some(path.to_path_buf()));
        assert_eq!(path, Some("/m.luau".into()), "an inner load's end isn't it");
        // Another VM's, made in the same load, are its own.
        let other = Repeats::of_the_app();
        assert_eq!(other.path(&key(), |_| Some(())), None);
        assert_eq!(other.len(), Some(0));
        let kept = key();
        drop(outer);
        assert_eq!((repeats.len(), other.len()), (None, None));
        repeats.keep(kept.clone(), Path::new("/m.luau"));
        assert_eq!(repeats.path(&kept, |_| Some(())), None);
        // The next load's VMs begin with nothing, and the last one's stay out.
        let _next = Load::begin();
        assert_eq!(repeats.len(), None);
        assert_eq!(Repeats::of_the_app().len(), Some(0));
    }
}
