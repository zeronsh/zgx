//! A bundle's `ui/` as `app_bundle::open` found it: each file's SHA-256,
//! from its manifest, against which each file is checked as it is read
//! (`Files::Folder`). Apart from `app_bundle` because a host names it on
//! every target, the browser too, where no bundle opens.
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Component, Path};
use std::sync::Arc;

/// A bundle's `ui/` and its files' sums. Made by `app_bundle::open` alone,
/// so a host reading a folder as a bundle's (`ScriptFiles::Folder`) reads
/// only files that check out.
#[derive(Clone, Debug)]
pub struct Checked {
    pub(crate) dir: Arc<Path>,
    sums: Arc<HashMap<String, [u8; 32]>>,
}

impl Checked {
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn new(dir: Arc<Path>, sums: Arc<HashMap<String, [u8; 32]>>) -> Self {
        Self { dir, sums }
    }

    /// The file at `path`, under its `ui/`, if it is the one its manifest
    /// names: a file the manifest doesn't name isn't there; one changed
    /// since the build is refused.
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
