//! A text area's caret at the edge of an inline span (`EditorHandle::
//! set_caret_edges`): an inline code span's start or end is one offset but
//! two places, just inside the span or just outside it, as a note editor's
//! stored marks tell apart (what is typed there is code or not). The first
//! ← or → at an edge moves between the two without moving over a
//! character; the caret shows a hair inside or outside. A text area whose
//! app sets no edges carries one empty pointer.
use crate::widgets::CaretSide;
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};

/// The span whose edges the caret tells apart, at an offset (one the
/// offset is in or at an end of), if any.
pub(crate) type Edges = Rc<dyn Fn(usize) -> Option<Range<usize>>>;

/// How far inside or outside an edge the caret shows, in points.
pub(crate) const HAIR: f32 = 1.;

/// What a text area's edges need: made when the app first sets them.
#[derive(Default)]
pub(crate) struct EdgeHint {
    pub edges: RefCell<Option<Edges>>,
    pub side: Cell<CaretSide>,
}
impl EdgeHint {
    /// The span at `at` with an edge there, if any.
    fn edge_at(&self, at: usize) -> Option<Range<usize>> {
        let edges = self.edges.borrow().clone()?;
        edges(at).filter(|span| span.start < span.end && (at == span.start || at == span.end))
    }
    /// An arrow (`right`, else left) with a caret at `focus`: at a span's
    /// edge, on the side it leaves, it goes to the other side and is taken
    /// (true); otherwise it moves the caret as ever (false).
    pub fn step(&self, focus: usize, right: bool) -> bool {
        let Some(span) = self.edge_at(focus) else {
            return false;
        };
        let inside = self.side.get() == CaretSide::Inside;
        // Into the span is rightward at its start, leftward at its end.
        let inward = (focus == span.start) == right;
        if inward == inside {
            return false;
        }
        self.side.set(if inside {
            CaretSide::Outside
        } else {
            CaretSide::Inside
        });
        true
    }
    /// An arrow moved the caret from `from` to `to`: reaching a span's edge
    /// from within it, it is inside; anywhere else, outside.
    pub fn moved(&self, from: usize, to: usize) {
        let inside = self
            .edge_at(to)
            .is_some_and(|span| span.start < from && from < span.end);
        self.side.set(if inside {
            CaretSide::Inside
        } else {
            CaretSide::Outside
        });
    }
    /// How far right of where it is laid out the caret at `focus` shows: a
    /// hair inside or outside an edge, nothing elsewhere.
    pub fn nudge(&self, focus: usize) -> f32 {
        let Some(span) = self.edge_at(focus) else {
            return 0.;
        };
        let inward = if focus == span.start { HAIR } else { -HAIR };
        match self.side.get() {
            CaretSide::Inside => inward,
            CaretSide::Outside => -inward,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `a` then code `bc` then `d`: edges at 1 and 3.
    fn hint() -> EdgeHint {
        let hint = EdgeHint::default();
        *hint.edges.borrow_mut() = Some(Rc::new(|at| (1..=3).contains(&at).then_some(1..3)));
        hint
    }

    #[test]
    fn the_first_arrow_at_an_edge_crosses_it_without_moving() {
        let hint = hint();
        // At its start, outside: → goes in, then moves; ← comes out.
        assert!(hint.step(1, true));
        assert_eq!(hint.side.get(), CaretSide::Inside);
        assert!(!hint.step(1, true));
        assert!(hint.step(1, false));
        assert_eq!(hint.side.get(), CaretSide::Outside);
        assert!(!hint.step(1, false));
        // At its end, reached from within: inside; → goes out.
        hint.moved(2, 3);
        assert_eq!(hint.side.get(), CaretSide::Inside);
        assert!(hint.step(3, true));
        assert_eq!(hint.side.get(), CaretSide::Outside);
        assert!(!hint.step(3, true));
        // Reached from outside, it is outside.
        hint.moved(4, 3);
        assert_eq!(hint.side.get(), CaretSide::Outside);
        assert!(!hint.step(2, true), "not at an edge");
    }

    #[test]
    fn it_shows_a_hair_inside_or_outside() {
        let hint = hint();
        assert_eq!(hint.nudge(1), -HAIR);
        assert_eq!(hint.nudge(3), HAIR);
        hint.side.set(CaretSide::Inside);
        assert_eq!(hint.nudge(1), HAIR);
        assert_eq!(hint.nudge(3), -HAIR);
        assert_eq!(hint.nudge(2), 0.);
    }
}
