//! zgx-gpu's shader check asks a smaller module first, which may only say
//! yes: here its answer, error text and line included, is the whole module's
//! for every shader in the repository (the effects', the examples', the
//! tests' and the docs') and for changes of each, good and bad.
use std::path::{Path, PathBuf};
use zgx_fx::{FxKind, sources};

/// `source` by the check apps get, and by the whole module.
fn same(source: &str, fill: bool) -> Result<bool, String> {
    let (checked, whole) = match fill {
        true => (
            zgx_gpu::check_fill_shader(source),
            zgx_gpu::check_fill_shader_whole(source),
        ),
        false => (
            zgx_gpu::check_layer_shader(source),
            zgx_gpu::check_layer_shader_whole(source),
        ),
    };
    match checked == whole {
        true => Ok(whole.is_ok()),
        false => Err(format!("{checked:?}, the whole module {whole:?}: {source}")),
    }
}

/// The files to read shaders from: scripts, Rust and Markdown.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                files(&path, out);
            }
        } else if name.ends_with(".luau") || name.ends_with(".rs") || name.ends_with(".md") {
            out.push(path);
        }
    }
}

/// The string literals of `text` that hold a shader (a `fn fill(` or a `fn
/// layer(`): Luau's long strings and Rust's raw ones.
fn literals(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (open, close) in [("[[", "]]"), ("[=[", "]=]"), ("r#\"", "\"#")] {
        let mut rest = text;
        while let Some(at) = rest.find(open) {
            let inside = &rest[at + open.len()..];
            let Some(end) = inside.find(close) else {
                break;
            };
            let literal = &inside[..end];
            if literal.contains("fn fill(") || literal.contains("fn layer(") {
                found.push(literal.to_owned());
            }
            rest = &inside[end + close.len()..];
        }
    }
    found
}

/// Every shader there is, and whether it's a fill's.
fn corpus() -> Vec<(String, bool)> {
    let mut all: Vec<(String, bool)> = sources()
        .into_iter()
        .map(|(_, kind, source)| (source.to_owned(), matches!(kind, FxKind::Fill)))
        .collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths = Vec::new();
    for dir in ["crates", "docs", "comparisons"] {
        files(&root.join(dir), &mut paths);
    }
    paths.sort();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for literal in literals(&text) {
            let fill = literal.contains("fn fill(");
            all.push((literal, fill));
        }
    }
    all.sort();
    all.dedup();
    all
}

/// A number that follows from the last (the same every run).
struct Next(u64);
impl Next {
    fn below(&mut self, most: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % most.max(1) as u64) as usize
    }
}

/// `source` changed in one place: some ways leave a good shader good, most
/// don't, and some reach for the renderer's own names.
fn changed(source: &str, next: &mut Next) -> String {
    let words: Vec<(usize, usize)> = {
        let part = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.';
        let mut words = Vec::new();
        let mut start = None;
        for (at, c) in source.char_indices().chain([(source.len(), ' ')]) {
            match (part(c), start) {
                (true, None) => start = Some(at),
                (false, Some(from)) => {
                    words.push((from, at));
                    start = None;
                }
                _ => {}
            }
        }
        words
    };
    if words.is_empty() {
        return source.to_owned();
    }
    let (from, to) = words[next.below(words.len())];
    let with = |new: &str| format!("{}{new}{}", &source[..from], &source[to..]);
    let anywhere = {
        let mut at = next.below(source.len());
        while !source.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let put = |new: &str| format!("{}{new}{}", &source[..anywhere], &source[anywhere..]);
    let names = [
        "shade",
        "rounded",
        "region",
        "regions",
        "atlas",
        "tex_sampler",
        "viewport",
        "vs",
        "Out",
        "Region",
        "zgx_sample",
        "zgx_backdrop",
        "zgx_blend",
        "zgx_filtered",
        "zgx_behind",
        "fill",
        "layer",
        "LayerInput",
        "FillInput",
        "f32",
        "vec4",
        "1.",
        "2",
        "0.5",
        "x",
        "dpdx",
    ];
    let pieces = [
        "@",
        ";",
        "{",
        "}",
        "(",
        ")",
        "//",
        "/*",
        "*/",
        " discard; ",
        "\n",
        "1.",
        "/* a */",
        "// b\n",
        "-",
        "+1.",
        " let q=1.; ",
        " return vec4(1.); ",
        "\nvar<private> z:f32;\n",
        "\nfn rounded()->f32 { return 1.; }\n",
        "\nconst K=1.;\n",
        "\u{3b1}",
        "override ",
        "\nfn more(a:f32)->f32 { return dpdx(a); }\n",
        "\nfn vs()->f32 { return 1.; }\n",
        "\nconst fs=1.;\n",
        "\nstruct fs_basic { a:f32 }\n",
        "\n@group(0) @binding(1) var mine:texture_2d<f32>;\nfn read()->f32 { return textureLoad(mine,vec2(0),0).x; }\n",
        "\n@fragment fn other()->@location(0) vec4<f32> { return vec4(1.); }\n",
    ];
    match next.below(6) {
        0 => with(""),
        1 => with(names[next.below(names.len())]),
        2 => {
            let (a, b) = words[next.below(words.len())];
            with(&source[a..b])
        }
        3 => put(pieces[next.below(pieces.len())]),
        4 => format!("{source}{}", pieces[next.below(pieces.len())]),
        _ => match source[from..to].parse::<f32>() {
            Ok(number) => with(&format!("{:.3}", number * 1.5 + 0.25)),
            Err(_) => put(" "),
        },
    }
}

#[test]
fn the_check_says_what_the_whole_module_says() {
    // (A long run by hand: ZGX_SHADER_CHANGES=400.)
    let each = std::env::var("ZGX_SHADER_CHANGES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(10);
    let corpus = corpus();
    assert!(corpus.len() >= 50, "{} shaders found", corpus.len());
    let (mut good, mut known, mut all_known) = (0, 0, 0);
    let (mut changes, mut changes_good, mut changes_known) = (0, 0, 0);
    let mut next = Next(0x9e37_79b9_7f4a_7c15);
    for (source, fill) in &corpus {
        good += usize::from(same(source, *fill).unwrap_or_else(|e| panic!("{e}")));
        known += usize::from(zgx_gpu::check_is_known(source, *fill));
        for _ in 0..each {
            let source = changed(source, &mut next);
            changes += 1;
            changes_good += usize::from(same(&source, *fill).unwrap_or_else(|e| panic!("{e}")));
            changes_known += usize::from(zgx_gpu::check_is_known(&source, *fill));
        }
    }
    for (name, kind, source) in sources() {
        let answered = zgx_gpu::check_is_known(source, matches!(kind, FxKind::Fill));
        all_known += usize::from(answered);
        if !answered {
            println!("{name}: checked whole");
        }
    }
    println!(
        "{} shaders, {good} good, {known} known by the smaller module; {changes} changes of them, {changes_good} good, {changes_known} known; {all_known} of {} effects known",
        corpus.len(),
        sources().len()
    );
    // The smaller module answers for what it's for, changes too.
    assert!(known * 2 > good, "{known} of {good}");
    assert!(
        changes_known * 5 > changes_good,
        "{changes_known} of {changes_good}"
    );
}
