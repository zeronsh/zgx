//! Where a part of a top-level block (a paragraph in a list item, a heading
//! in a quote) stands among the block's parts, in the order an app draws
//! them: the veil counts its paragraphs by it, and a find numbers its
//! matches through it (paragraphs and headings by their words, code by its
//! text, a table's cells row by row; display math, rules and pictures have
//! no words).
//!
//! A part's place is its path: from the top-level block, each list item's
//! index then the index of the block in it, a quote's block's index.
use crate::value::Value;
use std::rc::Rc;

/// The words of a paragraph's runs, a picture among them a byte no word
/// holds.
pub(crate) fn words(runs: &Value) -> String {
    runs.list()
        .iter()
        .map(|r| match (r.field("img"), r.field("x")) {
            (Some(_), _) => "\0".to_string(),
            (None, Some(Value::Str(x))) => x.to_string(),
            (None, Some(other)) => other.to_string(),
            (None, None) => String::new(),
        })
        .collect()
}

/// What is before the part at `path` (not empty) in `top`: how many parts
/// the veil counts (the part's place is one more), and the words a find
/// counts in, in order.
pub(crate) fn before(top: &Value, path: &[usize]) -> (u32, Vec<String>) {
    let mut out = (0, Vec::new());
    let mut at = Vec::new();
    walk(top, &mut at, path, &mut out);
    out
}

/// Through `b` (at `at`), counting into `out` until `path`: true there.
fn walk(b: &Value, at: &mut Vec<usize>, path: &[usize], out: &mut (u32, Vec<String>)) -> bool {
    if at.as_slice() == path {
        return true;
    }
    let kind = match b.field("t") {
        Some(Value::Str(t)) => t.clone(),
        _ => return false,
    };
    match &*kind {
        "p" | "h" => {
            out.0 += 1;
            out.1.push(b.field("s").map(words).unwrap_or_default());
        }
        "code" => {
            if !matches!(b.field("lang"), Some(Value::Str(l)) if &**l == "math") {
                out.0 += 1;
                out.1.push(match b.field("text") {
                    Some(Value::Str(t)) => t.to_string(),
                    _ => String::new(),
                });
            }
        }
        "ul" | "ol" => {
            let items: Vec<Value> = b
                .field("items")
                .map(|i| i.list().to_vec())
                .unwrap_or_default();
            for (i, item) in items.iter().enumerate() {
                for (j, child) in item.list().iter().enumerate() {
                    at.extend([i, j]);
                    let found = walk(child, at, path, out);
                    at.truncate(at.len() - 2);
                    if found {
                        return true;
                    }
                }
            }
        }
        "quote" => {
            let blocks: Vec<Value> = b.field("b").map(|l| l.list().to_vec()).unwrap_or_default();
            for (j, child) in blocks.iter().enumerate() {
                at.push(j);
                let found = walk(child, at, path, out);
                at.pop();
                if found {
                    return true;
                }
            }
        }
        "table" => {
            let head: Vec<Value> = b
                .field("head")
                .map(|h| h.list().to_vec())
                .unwrap_or_default();
            for cell in &head {
                out.0 += 1;
                out.1.push(words(cell));
            }
            for row in b
                .field("rows")
                .map(|r| r.list().to_vec())
                .unwrap_or_default()
            {
                for c in 0..head.len() {
                    out.0 += 1;
                    out.1.push(row.list().get(c).map(words).unwrap_or_default());
                }
            }
        }
        _ => {}
    }
    false
}

/// A path one step on.
pub(crate) fn on(path: &Rc<[usize]>, step: &[usize]) -> Rc<[usize]> {
    path.iter().chain(step).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{Options, parse};

    #[test]
    fn a_parts_place_counts_what_comes_before_it_in_the_drawn_order() {
        let top = parse(
            "- one **two**\n- three\n\n  four\n\n  > five\n- ```\n  six\n  ```\n- seven",
            &Options::default(),
        )
        .remove(0);
        // The second item's second block ("four"): "one two" and "three" before.
        let (n, words) = before(&top, &[1, 1]);
        assert_eq!(
            (n, words.as_slice()),
            (2, &["one two".to_string(), "three".to_string()][..])
        );
        // In the quote in it: "four" before too.
        assert_eq!(before(&top, &[1, 2, 0]).0, 3);
        // The last item's: the code counted.
        assert_eq!(before(&top, &[3, 0]).0, 5);
    }
}
