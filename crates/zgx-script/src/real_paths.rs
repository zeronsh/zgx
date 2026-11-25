//! Where a load's modules really are: what `require` asks of each path, that
//! it is in its folder and not through a link out of it ([`crate::within`]),
//! without following the whole path at each one.
//!
//! While a host loads ([`Load`]), a folder's real path is found once and kept
//! for the files in it. It is believed only while the folder there is the one
//! it was found for (its device and inode, one `lstat`), and a file itself is
//! looked at each time (another): a link, or a folder a link was put in place
//! of, is followed to where it goes, as ever. One thing is taken that following
//! the whole path wouldn't: a folder above, moved out while the app loads and a
//! link to it put in its place, which leads to the very folder that was there.
//! Kept for the load alone, and on Windows, where a folder has no such number
//! to ask for, not at all.
use std::path::{Path, PathBuf};

/// Where a path really is, as far as a folder it must be in asks.
enum Place {
    /// In it.
    Inside,
    /// Here, which isn't in the folder by its name.
    At(PathBuf),
}

#[cfg(unix)]
mod kept {
    use super::{Place, starts_with};
    use rustc_hash::FxHashMap;
    use std::{
        cell::RefCell,
        ffi::OsStr,
        os::unix::{ffi::OsStrExt, fs::MetadataExt},
        path::{Path, PathBuf},
    };
    /// A folder: the path it is asked by, its device and inode, and where it
    /// really is.
    struct Folder {
        asked: Box<[u8]>,
        at: (u64, u64),
        real: PathBuf,
    }
    /// The folders a load has asked about, and the last of them: requires
    /// come in runs from one.
    #[derive(Default)]
    pub(super) struct Folders {
        all: Vec<Folder>,
        by_path: FxHashMap<Box<[u8]>, usize>,
        last: usize,
    }
    impl Folders {
        /// The folder asked by `path`, if it is the one `at` says.
        fn find(&mut self, path: &[u8], at: (u64, u64)) -> Option<&Folder> {
            if self
                .all
                .get(self.last)
                .is_none_or(|last| *last.asked != *path)
            {
                self.last = *self.by_path.get(path)?;
            }
            self.all.get(self.last).filter(|folder| folder.at == at)
        }
        fn keep(&mut self, path: &[u8], at: (u64, u64), real: PathBuf) {
            let folder = Folder {
                asked: path.into(),
                at,
                real,
            };
            self.last = match self.by_path.get(path) {
                Some(&index) => {
                    self.all[index] = folder;
                    index
                }
                None => {
                    self.by_path.insert(path.into(), self.all.len());
                    self.all.push(folder);
                    self.all.len() - 1
                }
            };
        }
    }
    thread_local! {
        /// None while nothing loads.
        static FOLDERS: RefCell<Option<Folders>> = const { RefCell::new(None) };
    }

    pub(super) fn begin() -> bool {
        FOLDERS.with_borrow_mut(|folders| {
            let outermost = folders.is_none();
            folders.get_or_insert_default();
            outermost
        })
    }
    pub(super) fn end() {
        FOLDERS.with_borrow_mut(|folders| *folders = None);
    }
    /// The folders kept (for tests), or none outside a load.
    #[cfg(test)]
    pub(super) fn len() -> Option<usize> {
        FOLDERS.with_borrow(|folders| folders.as_ref().map(|folders| folders.all.len()))
    }

    /// Where the file at `path` really is, as `canonicalize` has it; for
    /// anything but a plain file there, or outside a load, `canonicalize`'s.
    pub(super) fn place(path: &Path, root: &Path) -> std::io::Result<Place> {
        let plain = || path.canonicalize().map(Place::At);
        let bytes = path.as_os_str().as_bytes();
        let Some(slash) = memchr::memrchr(b'/', bytes).filter(|&slash| slash > 0) else {
            return plain();
        };
        let (folder, name) = (&bytes[..slash], &bytes[slash + 1..]);
        if matches!(name, b"" | b"." | b"..") || FOLDERS.with_borrow(Option::is_none) {
            return plain();
        }
        if !path.symlink_metadata()?.is_file() {
            return plain();
        }
        // The folder itself, not where a link put in its place goes.
        let at_folder = Path::new(OsStr::from_bytes(folder));
        let at = at_folder.symlink_metadata()?;
        if !at.is_dir() {
            return plain();
        }
        let at = (at.dev(), at.ino());
        // In `root` if its folder is; the file's own path is made where it's
        // the only answer.
        let answer = |real: &Path| match starts_with(real, root) {
            true => Place::Inside,
            false => Place::At(real.join(OsStr::from_bytes(name))),
        };
        let kept = FOLDERS.with_borrow_mut(|folders| {
            let found = folders.as_mut()?.find(folder, at)?;
            Some(answer(&found.real))
        });
        if let Some(place) = kept {
            return Ok(place);
        }
        let real = at_folder.canonicalize()?;
        let place = answer(&real);
        // (The folder as it was when asked where it is: one swapped
        // meanwhile isn't kept.)
        if real
            .symlink_metadata()
            .is_ok_and(|now| (now.dev(), now.ino()) == at)
        {
            FOLDERS.with_borrow_mut(|folders| {
                if let Some(folders) = folders {
                    folders.keep(folder, at, real);
                }
            });
        }
        Ok(place)
    }
}

/// A host's load, on this thread: what is found of its folders is kept until
/// the outermost ends.
pub(crate) struct Load(#[allow(dead_code)] bool);
impl Load {
    pub(crate) fn begin() -> Self {
        #[cfg(unix)]
        return Self(kept::begin());
        #[cfg(not(unix))]
        Self(false)
    }
}
impl Drop for Load {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.0 {
            kept::end();
        }
    }
}

fn place(path: &Path, root: &Path) -> std::io::Result<Place> {
    #[cfg(unix)]
    return kept::place(path, root);
    #[cfg(not(unix))]
    path.canonicalize()
        .map(|real| match starts_with(&real, root) {
            true => Place::Inside,
            false => Place::At(real),
        })
}

/// `path.starts_with(root)`, the two compared byte by byte where that says
/// yes, and part by part where it doesn't.
fn starts_with(path: &Path, root: &Path) -> bool {
    let (whole, start) = (
        path.as_os_str().as_encoded_bytes(),
        root.as_os_str().as_encoded_bytes(),
    );
    let plainly = whole.starts_with(start)
        && whole
            .get(start.len())
            .is_none_or(|&next| next == std::path::MAIN_SEPARATOR as u8);
    plainly || path.starts_with(root)
}

/// [`crate::within`], of a path a load asks about.
pub(crate) fn within(root: &Path, path: &Path) -> bool {
    starts_with(path, root)
        && match place(path, root) {
            Err(_) | Ok(Place::Inside) => true,
            // Where it really is, against where the root really is: the root
            // itself may lie under a link (macOS's temporary directory does).
            Ok(Place::At(real)) => {
                real.starts_with(root)
                    || root.canonicalize().is_ok_and(|root| real.starts_with(root))
            }
        }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("zgx-real-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// In a load and out of one, the answers are the same: files in a
    /// folder, links of a file and of a folder, out and in, what isn't
    /// there, a root under a link, a way out by name.
    #[test]
    fn a_load_answers_as_within_does() {
        let base = dir("same");
        let (real, outside) = (base.join("real"), base.join("outside"));
        std::fs::create_dir_all(real.join("sub/deep")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        for file in ["in.luau", "sub/in.luau", "sub/deep/in.luau"] {
            std::fs::write(real.join(file), "").unwrap();
        }
        std::fs::write(outside.join("secret.luau"), "").unwrap();
        symlink(&outside, real.join("out")).unwrap();
        symlink(outside.join("secret.luau"), real.join("file")).unwrap();
        symlink(real.join("sub"), real.join("back")).unwrap();
        symlink(real.join("in.luau"), real.join("also")).unwrap();
        symlink(base.join("nowhere"), real.join("broken")).unwrap();
        symlink(&real, base.join("root")).unwrap();
        let roots = [real.clone(), base.join("root")];
        let names = [
            "in.luau",
            "sub/in.luau",
            "sub/deep/in.luau",
            "out/secret.luau",
            "file",
            "back/in.luau",
            "also",
            "broken",
            "sub",
            "sub/deep",
            "missing.luau",
            "gone/missing.luau",
            "in.luau/under",
            "../outside/secret.luau",
        ];
        for _ in 0..2 {
            let load = Load::begin();
            for root in &roots {
                for name in names {
                    let path = crate::normalize(&root.join(name));
                    assert_eq!(
                        within(root, &path),
                        crate::within(root, &path),
                        "{root:?} {name}"
                    );
                    // (As kept, too.)
                    assert_eq!(within(root, &path), crate::within(root, &path), "{name}");
                }
            }
            assert!(kept::len().unwrap() > 0, "folders were kept");
            drop(load);
            assert_eq!(kept::len(), None, "nothing kept past the load");
        }
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A folder, or one above it, swapped for a link out while a load has
    /// its place kept: the next file there is refused, as `within` refuses
    /// it, and one put back is taken again.
    #[test]
    fn a_folder_swapped_for_a_link_mid_load_is_followed() {
        let base = dir("swapped");
        let (root, outside) = (base.join("root"), base.join("outside"));
        for at in [root.join("a/b"), outside.join("b")] {
            std::fs::create_dir_all(&at).unwrap();
            std::fs::write(at.join("x.luau"), "").unwrap();
            std::fs::write(at.join("y.luau"), "").unwrap();
        }
        let load = Load::begin();
        assert!(within(&root, &root.join("a/b/x.luau")));
        // The folder itself.
        std::fs::rename(root.join("a/b"), base.join("kept")).unwrap();
        symlink(outside.join("b"), root.join("a/b")).unwrap();
        assert!(!within(&root, &root.join("a/b/x.luau")));
        assert!(!within(&root, &root.join("a/b/y.luau")));
        std::fs::remove_file(root.join("a/b")).unwrap();
        std::fs::rename(base.join("kept"), root.join("a/b")).unwrap();
        assert!(within(&root, &root.join("a/b/y.luau")));
        // One above it.
        std::fs::rename(root.join("a"), base.join("kept")).unwrap();
        symlink(&outside, root.join("a")).unwrap();
        assert!(!within(&root, &root.join("a/b/x.luau")));
        assert!(!within(&root, &root.join("a/b/y.luau")));
        std::fs::remove_file(root.join("a")).unwrap();
        std::fs::rename(base.join("kept"), root.join("a")).unwrap();
        // Moved out, and a link to it put in its place.
        std::fs::rename(root.join("a/b"), outside.join("moved")).unwrap();
        symlink(outside.join("moved"), root.join("a/b")).unwrap();
        assert!(!within(&root, &root.join("a/b/y.luau")));
        std::fs::remove_file(root.join("a/b")).unwrap();
        std::fs::rename(outside.join("moved"), root.join("a/b")).unwrap();
        assert!(within(&root, &root.join("a/b/y.luau")));
        // (One above it so is taken: it is the folder that was there.)
        std::fs::rename(root.join("a"), outside.join("moved")).unwrap();
        symlink(outside.join("moved"), root.join("a")).unwrap();
        assert!(within(&root, &root.join("a/b/y.luau")));
        assert!(!crate::within(&root, &root.join("a/b/y.luau")));
        std::fs::remove_file(root.join("a")).unwrap();
        std::fs::rename(outside.join("moved"), root.join("a")).unwrap();
        // A file in it.
        std::fs::remove_file(root.join("a/b/x.luau")).unwrap();
        symlink(outside.join("b/x.luau"), root.join("a/b/x.luau")).unwrap();
        assert!(!within(&root, &root.join("a/b/x.luau")));
        assert!(within(&root, &root.join("a/b/y.luau")));
        drop(load);
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// A load in a load (a plugin's, as the app loads) keeps what is kept
    /// until the outer ends.
    #[test]
    fn a_load_in_a_load_ends_with_the_outer() {
        assert_eq!(kept::len(), None);
        let outer = Load::begin();
        drop(Load::begin());
        assert_eq!(kept::len(), Some(0));
        drop(outer);
        assert_eq!(kept::len(), None);
    }
}
