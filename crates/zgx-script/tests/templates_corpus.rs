//! Templates change nothing a script makes. Every app in the repository
//! (its modules through its requires), constructors made at random, and
//! scripts that fail (in a hole, in a constructor, in a module) run with
//! templates (`ZGX_TEMPLATES=force`) and without, each way in a process of
//! its own (the compile caches are the process's), and what each made is
//! compared line for line: its tree, its error (file, line and traceback),
//! its holes' order. The run with templates proves it stamped.
//!
//! `cargo test -p zgx-script --test templates_corpus -- --nocapture` also
//! prints what the rewrite left as written, file by file.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
use zgx_script::{ScriptHost, testing::TestApp};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.luau` under `crates/`, by path from the repository's root (no
/// build output, generated types or packages fetched).
fn luau_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !(name.starts_with("target") || name.starts_with('.') || name == "node_modules")
                {
                    walk(&path, out);
                }
            } else if name.ends_with(".luau") && !name.ends_with(".d.luau") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&repo().join("crates"), &mut out);
    out.sort();
    out
}

/// A run's record, by name: what it made or how it failed.
type Record = BTreeMap<String, String>;

/// A random source of constructors (a fixed seed: both runs make the same).
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A random node of `kind`, `depth` levels at most below it: literals,
/// holes that log their order, nested templates, nils, spreads.
fn node(r: &mut Random, depth: u32, n: &mut u32) -> String {
    *n += 1;
    let me = *n;
    let kinds = ["column", "row", "div", "overlay", "grid", "button", "text"];
    let kind = kinds[r.below(kinds.len() as u64) as usize];
    let mut fields: Vec<String> = Vec::new();
    match kind {
        "text" => fields.push(match r.below(4) {
            0 => format!("\"t{me}\""),
            1 => format!("f(\"t{me}\", \"x{me}\")"),
            2 => format!("`i{me} {{f(\"i{me}\", 1)}}`"),
            _ => format!("function() return \"fn{me}\" end"),
        }),
        "button" => {
            fields.push(format!("\"b{me}\""));
            fields.push("on_click = function() end".into());
        }
        "grid" => fields.push("columns = 2".into()),
        _ => {}
    }
    fields.push(format!("id = \"n{me}\""));
    for _ in 0..r.below(3) {
        fields.push(match r.below(5) {
            0 => format!("gap = {}", r.below(9)),
            1 => format!("w = f(\"w{me}\", {})", 10 + r.below(90)),
            2 => format!("h = -{}", r.below(3)),
            3 => format!("opacity = {}.5", r.below(1)),
            _ => format!("padding = f(\"p{me}\", nil)"),
        });
    }
    if kind != "text" && kind != "button" && depth > 0 {
        for _ in 0..r.below(4) {
            fields.push(match r.below(6) {
                0..=2 => node(r, depth - 1, n),
                3 => "nil".into(),
                4 => format!("f(\"c{me}\", {})", node(r, depth - 1, n)),
                _ => format!("ui.text {{ f(\"h{me}\", \"held\") }}"),
            });
        }
        // Last in the list: a spread, whose values all go in.
        match r.below(4) {
            0 => fields.push(format!(
                "f(\"s{me}\", {}, nil, {})",
                node(r, depth - 1, n),
                node(r, depth - 1, n)
            )),
            1 => fields.push(format!(
                "table.unpack({{ {}, {} }})",
                node(r, depth - 1, n),
                node(r, depth - 1, n)
            )),
            _ => {}
        }
    }
    // Fields in a shuffled order, separators mixed, over lines.
    for i in (1..fields.len()).rev() {
        let j = r.below(i as u64 + 1) as usize;
        fields.swap(i, j);
    }
    // A spread stays last (moved back if shuffled away).
    if let Some(at) = fields
        .iter()
        .position(|f| f.starts_with("f(\"s") || f.starts_with("table.unpack"))
    {
        let spread = fields.remove(at);
        fields.push(spread);
    }
    let mut out = format!("ui.{kind} {{");
    for (i, field) in fields.iter().enumerate() {
        out.push_str(if r.below(3) == 0 { "\n" } else { " " });
        out.push_str(field);
        if i + 1 < fields.len() {
            out.push(if r.below(4) == 0 { ';' } else { ',' });
        }
    }
    out.push_str(if r.below(2) == 0 { "\n}" } else { " }" });
    out
}

/// A random script: a column of random nodes, the holes' order shown last.
fn fuzzed(seed: u64) -> String {
    let mut r = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut n = 0;
    let mut children = Vec::new();
    for _ in 0..1 + r.below(3) {
        children.push(node(&mut r, 3, &mut n));
    }
    format!(
        "local order = {{}}\n\
         local function f(tag, ...)\n\
         \ttable.insert(order, tag)\n\
         \treturn ...\n\
         end\n\
         local made = ui.column {{\n{}\n}}\n\
         return ui.column {{ made, ui.text {{ table.concat(order, \" \") }} }}\n",
        children.join(",\n")
    )
}

/// Scripts that fail: in a hole, in a constructor, in a nested one, in a
/// module (its file and line).
const FAILING: &[(&str, &str)] = &[
    (
        "hole",
        "local function boom()\n\terror(\"boom\")\nend\nreturn ui.column {\n\tgap = 4,\n\tui.text { boom() },\n}\n",
    ),
    (
        "constructor",
        "return ui.column {\n\tgap = 4,\n\tui.row { true },\n}\n",
    ),
    (
        "nested",
        "return ui.column {\n\tui.row {\n\t\tid = \"r\",\n\t\tui.div { gap = 2, 7 },\n\t},\n}\n",
    ),
    (
        "one line",
        "return ui.column { gap = 2, ui.row { true } }\n",
    ),
    (
        "text template before its state",
        "local function make()\n\tui.state(\"n\", 1)\n\treturn ui.text { \"x\" }\nend\nreturn ui.column {\n\tui.text { \"{n}\" },\n\tmake(),\n}\n",
    ),
    (
        "in a function",
        "local function row(i)\n\treturn ui.row {\n\t\tid = `r{i}`,\n\t\ti,\n\t}\nend\nreturn ui.column {\n\trow(1),\n}\n",
    ),
    (
        "handler",
        "return ui.column {\n\tui.button { \"go\", on_click = 5 },\n}\n",
    ),
];

/// What `make` mounts, run 30 frames: its tree, or its error.
fn record(make: impl FnOnce(&zgx::reactive::Runtime) -> ScriptHost) -> String {
    match TestApp::try_new(make) {
        Err(error) => format!("load error: {error}"),
        Ok(mut app) => {
            app.frames(30);
            let tree = app.tree();
            let (lines, hidden) = zgx_script::driver::tree_lines(
                &app.ui,
                &app.handle,
                None,
                &zgx_script::driver::TreeOptions {
                    budget: usize::MAX,
                    all: true,
                    ..Default::default()
                },
            );
            let all = zgx_script::driver::tree_lines_text(&lines, hidden);
            format!(
                "error: {:?}\n{tree}\n--- all\n{all}",
                app.host.error().with_untracked(|e| e.clone())
            )
        }
    }
}

/// One way's records (this process's `ZGX_TEMPLATES`), printed for the
/// test that compares them.
#[test]
#[ignore = "run by `templates_lower_as_written`, once each way"]
fn corpus_records() {
    let root = repo();
    let mut out = Record::new();
    for path in luau_files() {
        if path.file_name().is_none_or(|n| n != "app.luau") {
            continue;
        }
        let name = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let plugins = path.with_file_name("plugins");
        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            record(|runtime| {
                let host = ScriptHost::new(runtime, path.clone());
                // What hot_reload.rs publishes for its plugins.
                host.shared("window", "400 × 300");
                host.action("quit", |_| {});
                host.expose_shared("window", None);
                host.expose_shared("count", None);
                host.expose_action("quit", None);
                if plugins.is_dir() {
                    host.plugins(plugins.clone())
                } else {
                    host
                }
            })
        }))
        .unwrap_or_else(|_| "panicked".into());
        out.insert(name, made);
    }
    for seed in 0..400 {
        let code = fuzzed(seed);
        out.insert(
            format!("fuzz {seed:03}"),
            record(|runtime| ScriptHost::inline(runtime, "app", &code)),
        );
    }
    for &(name, code) in FAILING {
        let made = record(|runtime| ScriptHost::inline(runtime, "app", code));
        out.insert(format!("fails {name}"), made);
    }
    // A module's error names its file and line.
    let dir = std::env::temp_dir().join(format!("zgx-templates-corpus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("app.luau"),
        "local row = require(\"./row\")\nreturn ui.column { row(1), row(2) }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("row.luau"),
        "return function(i)\n\treturn ui.row {\n\t\tid = `r{i}`,\n\t\tui.text { if i == 2 then nil + i else \"one\" },\n\t}\nend\n",
    )
    .unwrap();
    let app = dir.join("app.luau");
    out.insert(
        "fails in a module".into(),
        record(|runtime| ScriptHost::new(runtime, app.clone())),
    );
    std::fs::remove_dir_all(&dir).ok();
    println!("\n<<<STAMPS {}>>>", zgx_script::__templates_stamped());
    for (name, made) in &out {
        println!("<<<RECORD {name}>>>\n{made}\n<<<END>>>");
    }
}

/// `corpus_records` run in a process of its own, `ZGX_TEMPLATES` as given.
fn records(templates: bool) -> (u64, Record) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "corpus_records",
        "--nocapture",
        "--test-threads=1",
    ]);
    if templates {
        command.env("ZGX_TEMPLATES", "force");
    } else {
        command.env_remove("ZGX_TEMPLATES");
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "corpus_records failed (templates {templates}):\n{}\n{stdout}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut stamps = 0;
    let mut out = Record::new();
    let mut lines = stdout.lines();
    while let Some(line) = lines.next() {
        // libtest's `test corpus_records ... ` may lead the line.
        if let Some(n) = line
            .find("<<<STAMPS ")
            .and_then(|at| line[at + "<<<STAMPS ".len()..].strip_suffix(">>>"))
        {
            stamps = n.parse().unwrap();
        } else if let Some(name) = line
            .strip_prefix("<<<RECORD ")
            .and_then(|s| s.strip_suffix(">>>"))
        {
            let made: Vec<&str> = lines.by_ref().take_while(|l| *l != "<<<END>>>").collect();
            out.insert(name.to_owned(), made.join("\n"));
        }
    }
    (stamps, out)
}

#[test]
fn templates_lower_as_written() {
    let (off_stamps, off) = records(false);
    let (on_stamps, on) = records(true);
    assert_eq!(off_stamps, 0, "no stamps with templates off");
    assert!(on_stamps > 1000, "templates on stamped: {on_stamps}");
    assert_eq!(
        off.keys().collect::<Vec<_>>(),
        on.keys().collect::<Vec<_>>()
    );
    let apps = off.keys().filter(|k| k.ends_with("app.luau")).count();
    assert!(apps >= 20, "the corpus found its apps: {apps}");
    // The failing scripts fail, each with its line.
    for (name, made) in &off {
        if name.starts_with("fails") {
            assert!(
                made.starts_with("load error") || !made.starts_with("error: None"),
                "{name} fails:\n{made}"
            );
        }
    }
    let differ: Vec<_> = off
        .iter()
        .filter(|(name, made)| on[*name] != **made)
        .map(|(name, made)| {
            let other = &on[name];
            let at = made
                .lines()
                .zip(other.lines())
                .position(|(a, b)| a != b)
                .unwrap_or(made.lines().count().min(other.lines().count()));
            let show = |s: &str| s.lines().skip(at).take(4).collect::<Vec<_>>().join("\n");
            format!(
                "{name}, from line {at}:\n  off: {}\n  on:  {}",
                show(made),
                show(other)
            )
        })
        .collect();
    assert!(
        differ.is_empty(),
        "{} of {} differ:\n{}",
        differ.len(),
        off.len(),
        differ.join("\n")
    );
    println!(
        "{} records alike ({apps} apps, 400 fuzzed, {} failing); {on_stamps} stamps",
        off.len(),
        FAILING.len() + 1
    );
}

/// What the rewrite leaves as written, by why, over every `.luau` file.
#[test]
fn what_is_left_as_written_is_reported() {
    let root = repo();
    let (mut stamped, mut by_why, mut files) = (0, BTreeMap::<String, usize>::new(), 0);
    let mut per_file = Vec::new();
    for path in luau_files() {
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        files += 1;
        let (count, left) = zgx_script::__templates_report(&source);
        stamped += count;
        for why in &left {
            *by_why.entry(why.clone()).or_default() += 1;
        }
        if !left.is_empty() {
            per_file.push(format!(
                "{} ({count} stamped, {} left)",
                path.strip_prefix(&root).unwrap_or(&path).display(),
                left.len()
            ));
        }
    }
    assert!(files > 100, "the corpus: {files} files");
    assert!(stamped > 0);
    let left: usize = by_why.values().sum();
    println!("{files} files: {stamped} constructors stamped, {left} left as written");
    let mut by_why: Vec<_> = by_why.into_iter().collect();
    by_why.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (why, n) in by_why {
        println!("{n:6}  {why}");
    }
    for line in per_file {
        println!("  {line}");
    }
}

/// A file of `dir` written, its time later than any before (a watcher
/// sees each save).
fn save(dir: &Path, file: &str, code: &str, n: u64) {
    let path = dir.join(file);
    std::fs::write(&path, code).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5 + n))
        .unwrap();
}

/// Templates go with the code that stamps them: a module edited and run
/// again (a partial reload, in the same VM) or everything loaded anew
/// leaves as many alive as the first load made, never one per edit.
#[test]
#[ignore = "run by `templates_hold_no_memory_across_reloads`, with templates"]
fn templates_after_reloads() {
    let dir = std::env::temp_dir().join(format!("zgx-templates-reloads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let row = |n: u64| {
        format!(
            "return function(i)\n\treturn ui.row {{\n\t\tid = `r{{i}}`,\n\t\tgap = 4,\n\t\tui.text {{ \"v{n}\" }},\n\t\tui.text {{ `#{{i}}` }},\n\t}}\nend\n"
        )
    };
    save(
        &dir,
        "app.luau",
        "local row = require(\"./row\")\nreturn ui.column { gap = 2, row(1), row(2) }\n",
        0,
    );
    save(&dir, "row.luau", &row(0), 0);
    let path = dir.join("app.luau");
    let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, path.clone()));
    drop(app.host.watch(std::time::Duration::from_secs(3600)));
    app.host.reload_partially(true);
    app.frames(2);
    app.host.collect_garbage();
    // The module's: its function stamps on. The app chunk's went with it
    // once it ran (it keeps no closure).
    let first = zgx_script::__templates_live();
    assert!(first >= 1, "the module's templates live: {first}");
    let stamped = zgx_script::__templates_stamped();
    for n in 1..=30 {
        save(&dir, "row.luau", &row(n), n);
        assert!(app.host.reload_if_changed(), "edit {n} reloaded");
        app.frames(2);
        assert_eq!(app.host.error().with_untracked(|e| e.clone()), None);
        assert!(app.tree().contains(&format!("v{n}")), "edit {n} shows");
        app.host.collect_garbage();
        assert_eq!(
            zgx_script::__templates_live(),
            first,
            "alive after edit {n} (partial reloads)"
        );
    }
    for n in 0..10 {
        app.host.reload().unwrap();
        app.frames(2);
        app.host.collect_garbage();
        assert_eq!(
            zgx_script::__templates_live(),
            first,
            "alive after full reload {n}"
        );
    }
    assert!(
        zgx_script::__templates_stamped() > stamped + 80,
        "the reloads stamped"
    );
    drop(app);
    assert_eq!(zgx_script::__templates_live(), 0, "none once the host goes");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn templates_hold_no_memory_across_reloads() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "templates_after_reloads",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ZGX_TEMPLATES", "force")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
