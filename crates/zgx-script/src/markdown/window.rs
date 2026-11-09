//! A long text held in runs (`ui.markdown_view { window = … }`): its
//! top-level blocks in runs of [`RUN`], each built while the look says it is
//! near what shows (`Look::near`), and away from it held as a box of the
//! height it last had (or a guess at it), so a reply of hundreds of blocks
//! lays out and holds what shows of it, not all of it. The look is handed
//! each run's box as it mounts, to judge by, and told as a run's height
//! changes, to keep the reader's place.
use crate::value::Value;
use std::{cell::RefCell, collections::HashMap};

/// How many top-level blocks a run holds.
pub(crate) const RUN: usize = 16;

/// What a run's blocks will likely stand as, laid out, before they have
/// been: its words at a reply's width (about 85 characters a line), a code
/// block by its lines, a list and a quote by what they hold, a table by
/// its rows; each with the room after a block.
pub(crate) fn guess(blocks: &[Value]) -> f32 {
    blocks.iter().map(|b| estimate(b) + 16.).sum()
}

fn chars(runs: Option<&Value>) -> usize {
    runs.map(|r| {
        r.list()
            .iter()
            .map(|run| match run.field("x") {
                Some(Value::Str(x)) => x.len(),
                _ => 0,
            })
            .sum()
    })
    .unwrap_or(0)
}

fn estimate(b: &Value) -> f32 {
    let kind = match b.field("t") {
        Some(Value::Str(t)) => t.clone(),
        _ => return 24.,
    };
    let list = |name: &str| b.field(name).map(|l| l.list().to_vec()).unwrap_or_default();
    match &*kind {
        "p" => (chars(b.field("s")).div_ceil(85).max(1) * 22) as f32,
        "h" => 34.,
        "code" => {
            let text = match b.field("text") {
                Some(Value::Str(t)) => t.matches('\n').count() + 1,
                _ => 1,
            };
            (text * 19 + 56) as f32
        }
        "ul" | "ol" => list("items")
            .iter()
            .map(|item| item.list().iter().map(estimate).sum::<f32>())
            .sum::<f32>()
            .max(22.),
        "quote" => list("b").iter().map(estimate).sum::<f32>().max(22.),
        "table" => {
            let mut h = 37.;
            for row in list("rows") {
                let longest = row
                    .list()
                    .iter()
                    .map(|cell| chars(Some(cell)))
                    .max()
                    .unwrap_or(0);
                h += 17. + 20. * longest.div_ceil(64).max(1) as f32;
            }
            h
        }
        "img" => 412.,
        "pics" => 240. * list("list").len().div_ceil(3) as f32,
        _ => 24.,
    }
}

thread_local! {
    /// Each held run's height as last laid out, by its text's key and its
    /// place: a run scrolled back to is held at the height it had, not a
    /// guess. Forgotten in a lump as it fills.
    static HEIGHTS: RefCell<HashMap<(std::rc::Rc<str>, usize), f32>> = RefCell::default();
}
const HEIGHTS_MOST: usize = 8192;

/// The height run `at` of the text `key` was last laid out at.
pub(crate) fn height_of(key: &std::rc::Rc<str>, at: usize) -> Option<f32> {
    HEIGHTS.with_borrow(|h| h.get(&(key.clone(), at)).copied())
}
/// Keep run `at`'s laid-out height.
pub(crate) fn keep_height(key: &std::rc::Rc<str>, at: usize, height: f32) {
    HEIGHTS.with_borrow_mut(|h| {
        if h.len() >= HEIGHTS_MOST {
            h.clear();
        }
        h.insert((key.clone(), at), height);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{Options, parse};

    #[test]
    fn a_runs_guess_is_its_blocks_estimates_and_their_room() {
        let blocks = parse(
            "A paragraph.\n\n# Head\n\n```\na\nb\n```\n\n- one\n- two",
            &Options::default(),
        );
        // 22 + 34 + (2·19 + 56) + (22 + 22), and 16 after each.
        assert_eq!(guess(&blocks), 22. + 34. + 94. + 44. + 4. * 16.);
    }
}
