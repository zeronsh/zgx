//! `BUILD_REV`: the commit zgx-script was built from, read from git's files
//! by its build script (`src/build_rev.rs`), which these tests also run on
//! made-up trees.
#[path = "../src/build_rev.rs"]
#[allow(dead_code)]
mod resolver;

use std::path::{Path, PathBuf};

const A: &str = "0123456789abcdef0123456789abcdef01234567";
const B: &str = "89abcdef0123456789abcdef0123456789abcdef";

/// A directory of its own under the temp directory, removed when dropped.
struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("zgx-build-rev-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("crates/zgx-script")).unwrap();
        Tree(dir)
    }
    fn put(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    fn found(&self) -> resolver::Found {
        resolver::find(&self.0.join("crates/zgx-script"))
    }
    fn path(&self, path: &str) -> PathBuf {
        self.0.join(path)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A path build says the commit its worktree is at, as git says it.
#[test]
fn a_path_build_reports_the_worktrees_head() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(manifest)
        .args(["rev-parse", "HEAD"])
        .output()
    else {
        eprintln!("no git to compare with");
        return;
    };
    if !out.status.success() {
        // Built from no checkout: then nothing says a commit.
        assert_eq!(zgx_script::BUILD_REV, "unknown");
        return;
    }
    let head = String::from_utf8(out.stdout).unwrap();
    assert_eq!(zgx_script::BUILD_REV, head.trim());
    assert_eq!(resolver::find(manifest).rev.as_deref(), Some(head.trim()));
}

#[test]
fn a_tree_without_git_is_unknown() {
    let tree = Tree::new("none");
    // Whatever is above the temp directory, a tree in it may sit in no
    // repository; only one that does would be found.
    let found = tree.found();
    if !std::env::temp_dir()
        .ancestors()
        .any(|d| d.join(".git").exists())
    {
        assert_eq!(found, resolver::Found::default());
    }
}

#[test]
fn a_branch_is_read_from_its_ref_file() {
    let tree = Tree::new("loose");
    tree.put(".git/HEAD", "ref: refs/heads/main\n");
    tree.put(".git/refs/heads/main", &format!("{A}\n"));
    tree.put(
        ".git/packed-refs",
        &format!("# pack-refs with: peeled\n{B} refs/heads/main\n"),
    );
    let found = tree.found();
    assert_eq!(
        found.rev.as_deref(),
        Some(A),
        "a loose ref is newer than its packed line"
    );
    assert_eq!(
        found.watched,
        [tree.path(".git/HEAD"), tree.path(".git/refs/heads/main")]
    );
}

#[test]
fn a_packed_branch_is_read_from_packed_refs() {
    let tree = Tree::new("packed");
    tree.put(".git/HEAD", "ref: refs/heads/lane/x\n");
    std::fs::create_dir_all(tree.path(".git/refs/heads")).unwrap();
    tree.put(
        ".git/packed-refs",
        &format!("# pack-refs with: peeled\n{B} refs/heads/lane/xy\n{A} refs/heads/lane/x\n^{B}\n"),
    );
    let found = tree.found();
    assert_eq!(found.rev.as_deref(), Some(A));
    // A commit writes the ref as a file of its own: the directory it would
    // go into is watched for it.
    assert_eq!(
        found.watched,
        [
            tree.path(".git/HEAD"),
            tree.path(".git/refs/heads"),
            tree.path(".git/packed-refs")
        ]
    );
}

#[test]
fn a_detached_head_is_the_commit() {
    let tree = Tree::new("detached");
    tree.put(".git/HEAD", &format!("{}\n", A.to_uppercase()));
    assert_eq!(tree.found().rev.as_deref(), Some(A));
}

#[test]
fn a_worktree_reads_its_head_and_the_shared_refs() {
    let tree = Tree::new("worktree");
    tree.put("main/.git/refs/heads/feature", &format!("{B}\n"));
    tree.put("main/.git/worktrees/wt/HEAD", "ref: refs/heads/feature\n");
    tree.put("main/.git/worktrees/wt/commondir", "../..\n");
    tree.put(".git", "gitdir: main/.git/worktrees/wt\n");
    let found = tree.found();
    assert_eq!(found.rev.as_deref(), Some(B));
    assert!(found.watched.contains(&tree.path(".git")));
    assert!(
        found
            .watched
            .contains(&tree.path("main/.git/worktrees/wt/HEAD"))
    );
    assert!(
        found
            .watched
            .contains(&tree.path("main/.git/worktrees/wt/commondir"))
    );
}

#[test]
fn a_packaged_crate_says_its_own_commit() {
    let tree = Tree::new("packaged");
    tree.put(".git/HEAD", &format!("{B}\n"));
    tree.put(
        "crates/zgx-script/.cargo_vcs_info.json",
        &format!("{{\n  \"git\": {{\n    \"sha1\": \"{A}\"\n  }},\n  \"path_in_vcs\": \"crates/zgx-script\"\n}}"),
    );
    // The repository around it isn't the one it was packaged from.
    assert_eq!(tree.found().rev.as_deref(), Some(A));
}

#[test]
fn a_broken_head_is_unknown() {
    let tree = Tree::new("broken");
    tree.put(".git/HEAD", "ref: refs/heads/../../../etc\n");
    assert_eq!(tree.found().rev, None);
    tree.put(".git/HEAD", "ref: refs/heads/gone\n");
    assert_eq!(tree.found().rev, None);
    tree.put(".git/HEAD", "abc\n");
    assert_eq!(tree.found().rev, None);
}
