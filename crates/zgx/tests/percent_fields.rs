//! `Style::width_percent` and `height_percent` hold fractions (`1.0` is 100%),
//! while the fluent `w_percent` and `h_percent` take percents. A style written
//! by hand with a percent is a box 100 times too big, which a clip or a
//! `max_w` can hide: it let a plugin's page draw past its box. Neither a type
//! (the fields are public) nor an assert (a script may ask for `2000%`) fits,
//! so the source is read for it.
use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_none_or(|n| n != "target") {
                sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The literals over `limit` given to a percent field in `line`.
fn percents_in(line: &str, limit: f32) -> Vec<f32> {
    let mut found = Vec::new();
    for field in ["width_percent", "height_percent"] {
        for (at, _) in line.match_indices(field) {
            let rest = line[at + field.len()..].trim_start();
            let Some(rest) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('=')) else {
                continue;
            };
            let rest = rest.trim_start();
            let rest = rest.strip_prefix("Some(").unwrap_or(rest);
            let number: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(value) = number.parse::<f32>()
                && value >= limit
            {
                found.push(value);
            }
        }
    }
    found
}

#[test]
fn it_tells_a_percent_from_a_fraction() {
    assert_eq!(percents_in("width_percent: Some(100.),", 10.), [100.]);
    assert_eq!(percents_in("style.height_percent = Some(50.);", 10.), [50.]);
    assert!(percents_in("width_percent: Some(1.),", 10.).is_empty());
    assert!(percents_in("height_percent: fitted.then_some(1.),", 10.).is_empty());
    assert!(percents_in("let width_percent = 100.;", 200.).is_empty());
}

#[test]
fn no_style_holds_a_percent_where_a_fraction_goes() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    sources(crates, &mut files);
    assert!(files.len() > 100, "the crates' sources: {}", files.len());
    let mut wrong = Vec::new();
    for file in files {
        // (This file's own examples.)
        if file.ends_with("percent_fields.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for (at, line) in text.lines().enumerate() {
            if !line.trim_start().starts_with("//") && !percents_in(line, 10.).is_empty() {
                wrong.push(format!("{}:{}: {}", file.display(), at + 1, line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a fraction goes here (1.0 is 100%):\n{}",
        wrong.join("\n")
    );
}
