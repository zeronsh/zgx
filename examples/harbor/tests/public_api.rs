//! The demo uses the zgx crates' public API alone: this test fails if its
//! code reaches past it.
//!
//! - `pub(crate)` and private items: the compiler already refuses them.
//! - Items marked `#[doc(hidden)]` in zgx, zgx-3d, zgx-desktop or zgx-gpu
//!   (test and bench hooks, `World::frames_drawn` and the like): read from
//!   those crates' sources here, and refused wherever the demo names them.
//!   A hidden method that shares its name with a public one (`new`) can't
//!   be told apart by name; those are printed, not refused.
//! - Reaching into another crate's files: `#[path]`, or `include!` /
//!   `include_str!` / `include_bytes!` of a path out of this crate.
//! - Dependency features commented as internal, test, bench, debug or
//!   experimental in the crate that declares them.
//!
//! One exception, by name: `harbor_bench` reads GPU time from the perf
//! lane's patched wgpu-hal (`wgpu::hal::metal::gpu_time`) behind its
//! `gpu-time` feature. That is the measuring instrument, not the API under
//! test, and it is allowed in that one file.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const CRATES: [&str; 4] = ["zgx", "zgx-3d", "zgx-desktop", "zgx-gpu"];
/// The instrument's names, and the one file allowed them.
const INSTRUMENT: (&[&str], &str) = (&["hal", "gpu_time"], "harbor_bench.rs");

fn here() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// A function inside an `impl` or a trait (indented): called as
    /// `.name(` or `Type::name(`.
    Method,
    /// A module-level item: named in paths.
    Item,
}

/// Every `pub` item of `source`, hidden or not: (kind, name, hidden).
fn items(source: &str) -> Vec<(Kind, String, bool)> {
    let mut out = Vec::new();
    let mut hidden = false;
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[doc(hidden)]") {
            hidden = true;
            continue;
        }
        if trimmed.starts_with("#[") || trimmed.starts_with("//") || trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let mut words = trimmed.split(|c: char| !(c.is_alphanumeric() || c == '_'));
        if words.next() == Some("pub") {
            // `pub fn name`, `pub const fn name`, `pub async fn name`,
            // `pub unsafe fn name`, `pub struct Name`, ...
            let rest: Vec<&str> = words.filter(|w| !w.is_empty()).collect();
            let keyword = rest.iter().position(|w| {
                matches!(
                    *w,
                    "fn" | "struct" | "enum" | "mod" | "const" | "static" | "trait" | "type"
                )
            });
            if let Some(k) = keyword
                && let Some(name) = rest.get(k + 1)
                && !trimmed.starts_with("pub(")
            {
                let kind = if rest[k] == "fn" && indent > 0 {
                    Kind::Method
                } else {
                    Kind::Item
                };
                out.push((kind, (*name).to_string(), hidden));
            }
        }
        hidden = false;
    }
    out
}

/// `source` with comments and string literals blanked, so names in prose
/// don't count.
fn code_only(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '"' => {
                let mut escaped = false;
                for c in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        break;
                    }
                }
                out.push_str("\"\"");
            }
            _ => out.push(c),
        }
    }
    out
}

/// Where `code` names `name` as `kind` is named.
fn names(code: &str, kind: Kind, name: &str) -> bool {
    let bytes = code.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut from = 0;
    while let Some(at) = code[from..].find(name) {
        let start = from + at;
        let end = start + name.len();
        from = end;
        if (start > 0 && ident(bytes[start - 1])) || (end < bytes.len() && ident(bytes[end])) {
            continue;
        }
        let before = code[..start].trim_end();
        let after = code[end..].trim_start();
        let found = match kind {
            Kind::Method => {
                (before.ends_with('.') || before.ends_with("::")) && after.starts_with('(')
            }
            Kind::Item => {
                before.ends_with("::")
                    || after.starts_with("::")
                    || before.ends_with('{')
                    || before.ends_with(',')
            }
        };
        if found {
            return true;
        }
    }
    false
}

#[test]
fn the_demo_names_no_hidden_item_of_the_zgx_crates() {
    let crates = here().join("../../crates");
    let mut hidden: BTreeMap<(Kind, String), Vec<String>> = BTreeMap::new();
    let mut visible: BTreeSet<(Kind, String)> = BTreeSet::new();
    for name in CRATES {
        let mut files = Vec::new();
        rust_files(&crates.join(name).join("src"), &mut files);
        assert!(!files.is_empty(), "no sources for {name}");
        for file in files {
            let source = fs::read_to_string(&file).unwrap();
            for (kind, item, is_hidden) in items(&source) {
                if is_hidden {
                    let at = file.strip_prefix(&crates).unwrap().display().to_string();
                    hidden.entry((kind, item)).or_default().push(at);
                } else {
                    visible.insert((kind, item));
                }
            }
        }
    }
    // The hooks this test exists for are among those it reads.
    for known in ["frames_drawn", "viewport", "set_culling", "culled"] {
        assert!(
            hidden.contains_key(&(Kind::Method, known.to_string())),
            "World::{known} is no longer read as hidden: the parser or the engine changed"
        );
    }
    let mut files = Vec::new();
    rust_files(&here().join("src"), &mut files);
    rust_files(&here().join("tests"), &mut files);
    let mut refused = Vec::new();
    let mut unsure = BTreeSet::new();
    for file in &files {
        if file.ends_with("public_api.rs") {
            continue;
        }
        let code = code_only(&fs::read_to_string(file).unwrap());
        let instrument_allowed = file.ends_with(INSTRUMENT.1);
        for ((kind, name), places) in &hidden {
            if instrument_allowed && INSTRUMENT.0.contains(&name.as_str()) {
                continue;
            }
            if !names(&code, *kind, name) {
                continue;
            }
            if visible.contains(&(*kind, name.clone())) {
                unsure.insert(format!(
                    "{name} ({kind:?}; hidden in {})",
                    places.join(", ")
                ));
            } else {
                refused.push(format!(
                    "{} names {name} ({kind:?}), hidden in {}",
                    file.strip_prefix(here()).unwrap().display(),
                    places.join(", ")
                ));
            }
        }
    }
    if !unsure.is_empty() {
        println!("names both hidden and public somewhere, used here (not refused):");
        for u in &unsure {
            println!("  {u}");
        }
    }
    assert!(
        refused.is_empty(),
        "hidden items used:\n{}",
        refused.join("\n")
    );
}

#[test]
fn the_demo_reaches_into_no_other_crates_files() {
    let mut files = Vec::new();
    rust_files(&here().join("src"), &mut files);
    rust_files(&here().join("tests"), &mut files);
    for file in files {
        if file.ends_with("public_api.rs") {
            continue;
        }
        let source = fs::read_to_string(&file).unwrap();
        for (n, line) in source.lines().enumerate() {
            let at = format!("{}:{}", file.display(), n + 1);
            assert!(!line.contains("#[path"), "{at}: #[path]");
            for mac in ["include!(", "include_str!(", "include_bytes!("] {
                if let Some(i) = line.find(mac) {
                    assert!(
                        !line[i..].contains(".."),
                        "{at}: {mac} of a path outside the crate"
                    );
                }
            }
            assert!(!line.contains("extern crate"), "{at}: extern crate");
        }
    }
}

/// Features of `crate_dir`'s Cargo.toml whose comment says they are not
/// for apps.
fn internal_features(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_features = false;
    let mut comment = String::new();
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_features = t == "[features]";
            comment.clear();
            continue;
        }
        if !in_features {
            continue;
        }
        if let Some(c) = t.strip_prefix('#') {
            comment.push_str(&c.to_lowercase());
            comment.push(' ');
            continue;
        }
        if let Some((name, _)) = t.split_once('=') {
            let marked = [
                "internal",
                "test hook",
                "for tests",
                "bench",
                "debug",
                "experimental",
            ]
            .iter()
            .any(|w| comment.contains(w));
            if marked {
                out.push(name.trim().to_string());
            }
        }
        comment.clear();
    }
    out
}

#[test]
fn the_demo_turns_on_no_internal_feature() {
    let manifest = fs::read_to_string(here().join("Cargo.toml")).unwrap();
    for name in CRATES {
        let theirs =
            fs::read_to_string(here().join("../../crates").join(name).join("Cargo.toml")).unwrap();
        let internal = internal_features(&theirs);
        // This crate's line for `name`, and the features it lists.
        let ours = manifest
            .lines()
            .find(|l| l.trim_start().starts_with(&format!("{name} =")))
            .unwrap_or_else(|| panic!("{name} is a dependency"));
        assert!(
            ours.contains(&format!("path = \"../../crates/{name}\"")),
            "{name} by path"
        );
        if let Some(i) = ours.find("features") {
            for feature in internal.iter() {
                assert!(
                    !ours[i..].contains(&format!("\"{feature}\"")),
                    "{name}'s feature {feature} is marked as not for apps"
                );
            }
        }
        // With default features on, the crate's defaults must be clean too.
        if !ours.contains("default-features = false") {
            let defaults = theirs
                .lines()
                .find(|l| l.trim_start().starts_with("default ="))
                .unwrap_or("");
            for feature in &internal {
                assert!(
                    !defaults.contains(&format!("\"{feature}\"")),
                    "{name}'s default feature {feature} is marked as not for apps"
                );
            }
        }
    }
}

#[test]
fn the_guard_sees_what_it_guards() {
    // A hidden method named as a call is caught; a public module of the
    // same name is not mistaken for it.
    assert!(names("world.frames_drawn()", Kind::Method, "frames_drawn"));
    assert!(names("World::viewport(&w)", Kind::Method, "viewport"));
    assert!(!names(
        "use zgx_desktop::viewport;",
        Kind::Method,
        "viewport"
    ));
    assert!(names("use zgx_gpu::secret::x;", Kind::Item, "secret"));
    assert!(!names(
        "let frames_drawn = 3;",
        Kind::Method,
        "frames_drawn"
    ));
    assert!(!names(
        &code_only("// world.culled()"),
        Kind::Method,
        "culled"
    ));
    let found = items(
        "impl World {\n    #[doc(hidden)]\n    #[inline]\n    pub fn culled(&self) {}\n    pub fn view(&self) {}\n}\n",
    );
    assert_eq!(
        found,
        vec![
            (Kind::Method, "culled".into(), true),
            (Kind::Method, "view".into(), false)
        ]
    );
}
