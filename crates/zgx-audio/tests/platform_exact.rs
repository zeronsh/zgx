//! Every reference in tests/references is rendered on one machine and must
//! hold on all of them, to -90 dB. The platform's maths library (`exp`,
//! `ln`, `powf`, `sin` and the like) differs in its last bits from one
//! system to another, and a coefficient a bit off compounds: the limiter's
//! release, made with the library's `exp`, rendered 84 dB under the signal
//! from its reference on Linux, at 0 on the Mac it was blessed on. So the
//! crate's own code calls none of them: `src/math.rs` has the ones it
//! needs, made of arithmetic, whose bits are the same everywhere. This
//! reads the sources, so it fails on any machine, not only another one.
use std::path::{Path, PathBuf};

/// The library's functions whose results can differ by platform. (Square
/// roots, rounding and remainders are exact by IEEE 754: the same
/// everywhere.)
const PLATFORM_DEPENDENT: &[&str] = &[
    "exp", "exp2", "exp_m1", "ln", "ln_1p", "log", "log2", "log10", "powf", "powi", "sin", "cos",
    "tan", "sin_cos", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh",
    "atanh", "cbrt", "hypot",
];

fn sources(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, into);
        } else if path.extension().is_some_and(|e| e == "rs") {
            into.push(path);
        }
    }
}

/// Where in `source` (its code, not its comments or its tests) a method of
/// the maths library is called: (line, name).
fn calls(source: &str) -> Vec<(usize, &'static str)> {
    let mut found = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        // A file's unit tests are its last module: they may check against
        // the library, and render nothing a reference holds.
        if trimmed.starts_with("#[cfg(test)]")
            && source
                .lines()
                .nth(i + 1)
                .is_some_and(|next| next.trim_start().starts_with("mod tests"))
        {
            break;
        }
        let code = match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        };
        for &name in PLATFORM_DEPENDENT {
            if code.contains(&format!(".{name}(")) {
                found.push((i + 1, name));
            }
        }
    }
    found
}

#[test]
fn the_crate_calls_none_of_the_platform_s_maths_library() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&src, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in &files {
        if file.ends_with("math.rs") {
            continue;
        }
        let source = std::fs::read_to_string(file).unwrap();
        for (line, name) in calls(&source) {
            let file = file.strip_prefix(&src).unwrap().display();
            found.push(format!("src/{file}:{line}: .{name}()"));
        }
    }
    assert!(
        found.is_empty(),
        "calls whose bits differ by platform, for src/math.rs's own:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_check_sees_a_call_and_not_a_comment_or_a_test() {
    let source = "let a = x.exp();\n// y.ln()\nlet b = 10f32.powf(c); // d.sin()\n#[cfg(test)]\nmod tests {\n    let e = f.cos();\n}\n";
    assert_eq!(calls(source), [(1, "exp"), (3, "powf")]);
}
