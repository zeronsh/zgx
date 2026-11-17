//! The commit a crate's sources were built from, read the way `build.rs`
//! does it (`BUILD_REV`): from the files git keeps, with no git process.
//!
//! A packaged crate carries its commit in `.cargo_vcs_info.json`, and no
//! repository around it is looked for (one around a registry or vendored
//! copy is someone else's). Otherwise the nearest `.git` above the crate
//! is read: a directory, or a worktree's (or submodule's) file naming its
//! git directory, whose `commondir` names where the shared refs live.
//! `HEAD` holds a commit (detached, as CI checks out) or names a ref,
//! found as a file of its own or a line of `packed-refs`.
use std::path::{Path, PathBuf};

/// What was found, and the files that say it: the build reruns when one
/// of them changes (a commit, a checkout, a `git pack-refs`).
#[derive(Debug, Default, PartialEq)]
pub struct Found {
    /// The full 40-hex commit, if one was found.
    pub rev: Option<String>,
    /// The files read, and for a ref with no file of its own the nearest
    /// directory one would be written into.
    pub watched: Vec<PathBuf>,
}

/// The commit the crate in `manifest_dir` was built from.
pub fn find(manifest_dir: &Path) -> Found {
    let mut found = Found::default();
    let vcs_info = manifest_dir.join(".cargo_vcs_info.json");
    if let Ok(text) = std::fs::read_to_string(&vcs_info) {
        found.watched.push(vcs_info);
        found.rev = packaged_sha(&text);
        return found;
    }
    let Some(dot_git) = manifest_dir
        .ancestors()
        .map(|dir| dir.join(".git"))
        .find(|p| p.exists())
    else {
        return found;
    };
    // A directory isn't watched itself: cargo would scan all of it.
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        found.watched.push(dot_git.clone());
        let text = read(&dot_git).unwrap_or_default();
        let Some(dir) = text.strip_prefix("gitdir:").map(str::trim) else {
            return found;
        };
        dot_git.parent().unwrap_or(Path::new("")).join(dir)
    };
    let common = git_dir.join("commondir");
    let common_dir = match read(&common) {
        Some(dir) => {
            found.watched.push(common);
            git_dir.join(dir.trim())
        }
        None => git_dir.clone(),
    };
    let head = git_dir.join("HEAD");
    let Some(mut text) = read(&head) else {
        return found;
    };
    found.watched.push(head);
    // A ref may name another ref; a few steps is plenty, and a loop stops.
    for _ in 0..5 {
        let Some(name) = text.strip_prefix("ref:").map(|n| n.trim().to_string()) else {
            found.rev = sha(&text);
            return found;
        };
        match resolve(&name, &git_dir, &common_dir, &mut found.watched) {
            Some(next) => text = next,
            None => return found,
        }
    }
    found
}

/// What `name` holds: its file in the worktree's git directory or the
/// shared one, or else its line in `packed-refs`.
fn resolve(
    name: &str,
    git_dir: &Path,
    common_dir: &Path,
    watched: &mut Vec<PathBuf>,
) -> Option<String> {
    if name
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    for dir in [git_dir, common_dir] {
        let file = dir.join(name);
        if let Some(text) = read(&file) {
            watched.push(file);
            return Some(text);
        }
    }
    // Packed: a commit makes it a file of its own, in a directory that may
    // not exist yet. The nearest one that does is watched for it.
    let file = common_dir.join(name);
    if let Some(dir) = file
        .ancestors()
        .skip(1)
        .find(|d| d.is_dir() && d.starts_with(common_dir))
    {
        watched.push(dir.to_path_buf());
    }
    let packed = common_dir.join("packed-refs");
    let text = read(&packed)?;
    watched.push(packed);
    text.lines()
        .filter(|line| !line.starts_with(['#', '^']))
        .find_map(|line| {
            line.split_once(' ')
                .filter(|(_, n)| *n == name)
                .map(|(rev, _)| rev.to_string())
        })
}

/// `"sha1": "<rev>"` in `.cargo_vcs_info.json`.
fn packaged_sha(text: &str) -> Option<String> {
    let after = &text[text.find("\"sha1\"")? + "\"sha1\"".len()..];
    let value = after
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    sha(value.split('"').next()?)
}

/// A full commit id, lowercased, or nothing.
fn sha(text: &str) -> Option<String> {
    let text = text.trim();
    (text.len() == 40 && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .filter(|_| path.is_file())
}
