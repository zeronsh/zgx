//! The Luau definitions a build embeds: `types/zgx.d.luau` with the parts of
//! features it was built without left out, so an app pays for no API it
//! can't call (in bytes too). Not a module of the library: build.rs and the
//! tests include it by path.
//!
//! In the definitions, a feature's part is marked two ways:
//! - a block: a line `--@if <feature>`, the part, a line `--@end`;
//! - a line: ending `--@<feature>` (lines Merge writes out into every
//!   element's options, where a block can't follow them).
//!
//! What a build keeps is the file without the marks, byte for byte, so a
//! build with no such feature embeds what it did before they were added.

/// `full` with the parts of features `on` says are off left out, and every
/// mark taken away.
pub fn filtered(full: &str, on: impl Fn(&str) -> bool) -> String {
    lines(full, on)
        .into_iter()
        .filter(|(_, kept)| *kept)
        .map(|(line, _)| line)
        .collect()
}

/// Each line of `full` (its line break kept) without its mark, and whether
/// a build with the features `on` says are on keeps it. The marks' own
/// lines are left out.
pub fn lines(full: &str, on: impl Fn(&str) -> bool) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    // Blocks open now, each whether its feature is on.
    let mut open: Vec<bool> = Vec::new();
    for line in full.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if let Some(feature) = text.trim().strip_prefix("--@if ") {
            open.push(on(feature.trim()));
            continue;
        }
        if text.trim() == "--@end" {
            open.pop().expect("an --@end with no --@if open");
            continue;
        }
        let in_block = open.iter().all(|kept| *kept);
        match text.rsplit_once(" --@") {
            Some((kept, feature)) if !feature.contains(' ') && !feature.is_empty() => {
                let line = format!("{kept}{}", &line[text.len()..]);
                out.push((line, in_block && on(feature)));
            }
            _ => out.push((line.to_owned(), in_block)),
        }
    }
    assert!(open.is_empty(), "an --@if with no --@end");
    out
}
