//! Pictures placed on the grid (the kitty graphics protocol's placements):
//! the cells each covers, kept in grid lines (negative in the scrollback),
//! moved with the lines they sit on as those scroll, and dropped once their
//! lines leave the scrollback or their screen is cleared. Which picture, and
//! its pixels, are the embedder's: the screen model keeps where.
//!
//! Each screen keeps its own. A screen with none allocates nothing, and pays
//! one comparison where its lines move.

use std::ops::Range;

use crate::index::{Line, Point};

/// A picture over a rectangle of cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The embedder's picture.
    pub image: u32,
    /// Its number among the picture's placements: placing the picture again
    /// with the same nonzero number moves this one. Zero places another.
    pub id: u32,
    /// The top-left cell.
    pub point: Point,
    /// The lines it covers.
    pub lines: usize,
    /// The columns it covers.
    pub columns: usize,
    /// Under the text when negative, over it otherwise; higher over lower.
    pub z: i32,
    /// What of the picture shows, in its pixels: x, y, width and height (a
    /// zero width or height reaches the picture's edge).
    pub source: [u32; 4],
    /// From the top-left cell's corner to the picture's, in pixels.
    pub offset: [u32; 2],
}

impl Placement {
    /// The lines it covers.
    pub fn line_range(&self) -> Range<Line> {
        self.point.line..self.point.line + self.lines
    }
}

/// A screen's placements, in the order they were placed.
#[derive(Debug, Default)]
pub(crate) struct Placements(Vec<Placement>);

impl Placements {
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    pub fn as_slice(&self) -> &[Placement] {
        &self.0
    }

    pub fn place(&mut self, placement: Placement) {
        if placement.id != 0 {
            if let Some(old) = self
                .0
                .iter_mut()
                .find(|old| old.image == placement.image && old.id == placement.id)
            {
                *old = placement;
                return;
            }
        }
        self.0.push(placement);
    }

    /// Keep the placements `keep` says to; whether any went.
    pub fn retain(&mut self, keep: impl FnMut(&Placement) -> bool) -> bool {
        let before = self.0.len();
        self.0.retain(keep);
        self.0.len() != before
    }

    /// Drop them all, and the room they took.
    pub fn clear(&mut self) {
        self.0 = Vec::new();
    }

    /// The lines of `region` moved up `lines`, as the grid does: from a
    /// region starting at the top, into the scrollback (which holds
    /// `history` lines afterwards), with the scrollback; otherwise out of the
    /// region, which drops those starting above it.
    #[cold]
    pub fn scroll_up(&mut self, region: &Range<Line>, lines: usize, history: usize) {
        let into_history = region.start == 0;
        self.0.retain_mut(|placement| {
            let top = placement.point.line;
            if top >= region.end || (!into_history && top < region.start) {
                return true;
            }
            placement.point.line = top - lines;
            if into_history {
                placement.point.line + placement.lines > -(history as i32)
            } else {
                placement.point.line >= region.start
            }
        });
    }

    /// The lines of `region` moved down `lines`: those starting below the
    /// region afterwards are dropped.
    #[cold]
    pub fn scroll_down(&mut self, region: &Range<Line>, lines: usize) {
        self.0.retain_mut(|placement| {
            let top = placement.point.line;
            if top < region.start || top >= region.end {
                return true;
            }
            placement.point.line = top + lines;
            placement.point.line < region.end
        });
    }

    /// Every line moved down `delta` (up when negative), as a resize moves
    /// the screen over the scrollback; those left wholly above the
    /// scrollback's `history` lines or below the screen's `screen_lines` are
    /// dropped.
    #[cold]
    pub fn shift(&mut self, delta: i32, history: usize, screen_lines: usize) {
        self.0.retain_mut(|placement| {
            placement.point.line += delta;
            let lines = placement.line_range();
            lines.end > -(history as i32) && lines.start < screen_lines as i32
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::VoidListener;
    use crate::index::Column;
    use crate::term::test::TermSize;
    use crate::term::{Config, Term};
    use crate::vte::ansi::{ClearMode, Handler};

    fn at(line: i32, lines: usize) -> Placement {
        Placement {
            image: 1,
            id: 0,
            point: Point::new(Line(line), Column(2)),
            lines,
            columns: 3,
            z: 0,
            source: [0; 4],
            offset: [0; 2],
        }
    }

    fn tops<T>(term: &Term<T>) -> Vec<i32> {
        term.placements().iter().map(|p| p.point.line.0).collect()
    }

    fn term(history: usize) -> Term<VoidListener> {
        let config = Config {
            scrolling_history: history,
            ..Config::default()
        };
        Term::new(config, &TermSize::new(10, 5), VoidListener)
    }

    /// Output scrolling the screen carries a placement up into the
    /// scrollback with its lines, until its last line leaves the scrollback.
    #[test]
    fn a_placement_scrolls_with_its_lines_into_the_scrollback_and_out() {
        let mut term = term(3);
        term.place(at(2, 2));
        term.goto(4, 0);
        term.linefeed();
        assert_eq!(tops(&term), [1]);
        for _ in 0..4 {
            term.linefeed();
        }
        // Line -3, the scrollback's first: its second line is still in it.
        assert_eq!(tops(&term), [-3]);
        term.linefeed();
        assert_eq!(tops(&term), [-4]);
        term.linefeed();
        assert_eq!(tops(&term), Vec::<i32>::new());
    }

    /// In a scrolling region below the top, a placement moves with the
    /// region's lines, and goes once it starts outside the region; those
    /// outside the region stay.
    #[test]
    fn a_scrolling_region_moves_the_placements_in_it_alone() {
        let mut term = term(10);
        // Lines 2 and 3.
        term.set_scrolling_region(3, Some(4));
        term.place(at(0, 1));
        term.place(at(2, 1));
        term.place(at(4, 1));
        term.scroll_down(1);
        assert_eq!(tops(&term), [0, 3, 4]);
        term.scroll_up(1);
        assert_eq!(tops(&term), [0, 2, 4]);
        term.scroll_up(1);
        assert_eq!(tops(&term), [0, 4]);
        term.place(at(3, 1));
        term.scroll_down(1);
        assert_eq!(tops(&term), [0, 4]);
    }

    /// Lines inserted or deleted at the cursor move the placements below it.
    #[test]
    fn inserted_and_deleted_lines_move_the_placements_below() {
        let mut term = term(10);
        term.place(at(1, 1));
        term.place(at(3, 1));
        term.goto(2, 0);
        term.insert_blank_lines(1);
        assert_eq!(tops(&term), [1, 4]);
        term.delete_lines(2);
        assert_eq!(tops(&term), [1, 2]);
        // Its own line deleted, it goes.
        term.delete_lines(1);
        assert_eq!(tops(&term), [1]);
    }

    /// Clearing the screen drops what it shows, and moves what the
    /// scrollback shows with the lines the clear pushes into it; clearing
    /// the scrollback drops what starts there.
    #[test]
    fn clearing_drops_what_the_cleared_lines_show() {
        let mut term = term(10);
        for line in 0..5 {
            term.goto(line, 0);
            term.input('x');
        }
        term.goto(4, 0);
        term.linefeed();
        term.linefeed();
        term.place(at(-2, 1));
        term.place(at(1, 1));
        term.clear_screen(ClearMode::All);
        // The clear pushed the screen's three written lines into the
        // scrollback.
        assert_eq!(tops(&term), [-5]);
        term.clear_screen(ClearMode::Saved);
        assert_eq!(tops(&term), Vec::<i32>::new());
    }

    /// Each screen keeps its own: the alternate screen starts with none and
    /// leaves the main screen's where they were.
    #[test]
    fn each_screen_keeps_its_own() {
        let mut term = term(10);
        term.place(at(1, 1));
        term.swap_alt();
        assert_eq!(tops(&term), Vec::<i32>::new());
        term.place(at(2, 1));
        assert_eq!(tops(&term), [2]);
        term.swap_alt();
        assert_eq!(tops(&term), [1]);
        // Entered again, the alternate screen is blank.
        term.swap_alt();
        assert_eq!(tops(&term), Vec::<i32>::new());
    }

    /// Placing a picture again under the same nonzero number moves it; under
    /// zero it places another.
    #[test]
    fn a_numbered_placement_moves_where_placed_again() {
        let mut term = term(10);
        term.place(Placement { id: 7, ..at(1, 1) });
        term.place(Placement { id: 7, ..at(3, 1) });
        term.place(at(0, 1));
        term.place(at(0, 1));
        assert_eq!(tops(&term), [3, 0, 0]);
    }

    /// Fewer lines move the screen down over the scrollback (the cursor at
    /// its last line), and the placements with it; more pull it back.
    #[test]
    fn a_resize_moves_placements_with_the_screen() {
        let mut term = term(10);
        term.goto(4, 0);
        term.input('x');
        term.place(at(3, 1));
        term.resize(TermSize::new(10, 3));
        assert_eq!(tops(&term), [1]);
        term.resize(TermSize::new(10, 5));
        assert_eq!(tops(&term), [3]);
    }

    /// A screen that never shows a picture holds no room for one, and a
    /// reset gives back what one held.
    #[test]
    fn no_placements_hold_no_room() {
        let mut term = term(10);
        for _ in 0..20 {
            term.linefeed();
        }
        term.swap_alt();
        term.swap_alt();
        assert_eq!(term.placements.0.capacity(), 0);
        assert_eq!(term.inactive_placements.0.capacity(), 0);
        term.place(at(1, 1));
        term.reset_state();
        assert_eq!(term.placements.0.capacity(), 0);
    }
}
