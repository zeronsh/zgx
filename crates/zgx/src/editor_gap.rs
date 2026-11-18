//! A text area's gap cursor (`EditorHandle::set_gaps`): a caret the app
//! places between two lines rather than in one, where no text is to put a
//! caret in (between two tables, before a table at the start, after an
//! image), as ProseMirror's gap cursor. It shows as a short bar along the
//! boundary; the arrows go into and out of it, and what is typed at it goes
//! to the app (`EditorHandle::on_gap_input`), which adds a paragraph there.
//! A text area whose app places none carries one empty pointer.
use crate::widgets::{GapDir, GapInput};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Whether a gap may sit before line `n` (`n` the line count: after the
/// last), for the caret coming as `GapDir` says.
pub(crate) type Gaps = Rc<dyn Fn(usize, GapDir) -> bool>;
/// Told the caret went into a gap or out of one.
type GapListener = Box<dyn FnMut(Option<usize>)>;
/// Told what was typed at a gap.
type GapInputListener = Box<dyn FnMut(usize, GapInput)>;

/// How long the bar a gap shows is, and how thick, in points.
pub(crate) const BAR_WIDTH: f32 = 20.;
pub(crate) const BAR_HEIGHT: f32 = 2.;
/// How near a boundary a press puts the caret in its gap, in points.
pub(crate) const PRESS_SLOP: f32 = 4.;

/// What a text area's gaps need: made when the app first sets or listens.
#[derive(Default)]
pub(crate) struct GapHint {
    pub gaps: RefCell<Option<Gaps>>,
    /// The boundary the caret is at (before that line), if at one.
    pub at: Cell<Option<usize>>,
    /// Told the caret went into a gap (`Some`) or out of one (`None`); the
    /// slot is empty while it runs.
    pub on_gap: RefCell<Option<GapListener>>,
    /// Told what was typed at a gap.
    pub on_input: RefCell<Option<GapInputListener>>,
}
impl GapHint {
    /// Whether a gap may sit before line `n`, the caret coming as `dir`.
    pub fn allows(&self, n: usize, dir: GapDir) -> bool {
        let gaps = self.gaps.borrow().clone();
        gaps.is_some_and(|gaps| gaps(n, dir))
    }
    /// The caret goes to boundary `at` (or out of a gap): what listens is
    /// told when it changed. Nothing may be borrowed that it uses.
    pub fn go(&self, at: Option<usize>) {
        if self.at.replace(at) == at {
            return;
        }
        let told = self.on_gap.borrow_mut().take();
        if let Some(mut told) = told {
            told(at);
            let mut slot = self.on_gap.borrow_mut();
            if slot.is_none() {
                *slot = Some(told);
            }
        }
    }
    /// What was typed at gap `at` goes to the app.
    pub fn input(&self, at: usize, input: GapInput) {
        let told = self.on_input.borrow_mut().take();
        if let Some(mut told) = told {
            told(at, input);
            let mut slot = self.on_input.borrow_mut();
            if slot.is_none() {
                *slot = Some(told);
            }
        }
    }
}

/// Where line `n` of `text` starts (lines end at `\n`); `None` past the
/// last.
pub(crate) fn line_start(text: &str, n: usize) -> Option<usize> {
    if n == 0 {
        return Some(0);
    }
    text.match_indices('\n').nth(n - 1).map(|(at, _)| at + 1)
}
/// Where line `n` ends, before its `\n`.
pub(crate) fn line_end(text: &str, n: usize) -> Option<usize> {
    let start = line_start(text, n)?;
    Some(
        text[start..]
            .find('\n')
            .map_or(text.len(), |end| start + end),
    )
}
/// The line `offset` is in.
pub(crate) fn line_of(text: &str, offset: usize) -> usize {
    text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_by_their_breaks() {
        let text = "ab\n\ncd";
        assert_eq!(
            (0..4).map(|n| line_start(text, n)).collect::<Vec<_>>(),
            [Some(0), Some(3), Some(4), None]
        );
        assert_eq!(
            (0..3).map(|n| line_end(text, n)).collect::<Vec<_>>(),
            [Some(2), Some(3), Some(6)]
        );
        assert_eq!([0, 2, 3, 4, 6].map(|at| line_of(text, at)), [0, 0, 1, 2, 2]);
    }
}
