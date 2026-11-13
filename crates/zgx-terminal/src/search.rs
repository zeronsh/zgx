//! Searching what a terminal holds, its history too: every match of a
//! needle counted, the ones in view highlighted, one selected and scrolled
//! to (Ghostty's search bindings, `TerminalState::search`).
//!
//! Lines are numbered as they scroll (`Grid::pushed`): a line in the
//! history keeps its matches while more output scrolls in, so a search held
//! open searches output only as it arrives, the screen's lines again (they
//! change in place) and the history's new ones once, and drops the matches
//! of lines that fall off the history's top.
use crate::session::Term;
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line},
    term::{TermMode, cell::Flags},
};
use std::collections::VecDeque;

/// A match: its first and last cells, by line number (`Grid::pushed` plus
/// the line) and column, the last a wide character's second cell when it
/// ends on one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Hit {
    pub start: (u64, usize),
    pub end: (u64, usize),
}

/// Columns of a row in view to highlight, first to last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Highlight {
    pub row: usize,
    pub first: usize,
    pub last: usize,
    /// Part of the selected match.
    pub selected: bool,
}

/// The screen a search's matches were found on: one of another size
/// (reflowed) or the other screen is searched again.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Shape {
    columns: usize,
    rows: usize,
    alternate: bool,
}

pub(crate) struct Search {
    /// What it looks for, in lower case when `fold`.
    needle: Vec<char>,
    /// Case doesn't count: the needle has no capital (smart case).
    fold: bool,
    /// Matches in lines wholly in the history, oldest first.
    history: VecDeque<Hit>,
    /// Matches in lines on the screen (or wrapping onto it), found again
    /// each update: they change in place.
    screen: Vec<Hit>,
    /// The first line `history` hasn't been searched to.
    scanned: u64,
    shape: Option<Shape>,
    selected: Option<Hit>,
}

impl Search {
    /// A search for `needle`, case-insensitive unless it has a capital.
    pub(crate) fn new(needle: &str) -> Self {
        let fold = !needle.chars().any(char::is_uppercase);
        let needle = needle.chars().map(|c| fold_char(c, fold)).collect();
        Self {
            needle,
            fold,
            history: VecDeque::new(),
            screen: Vec::new(),
            scanned: 0,
            shape: None,
            selected: None,
        }
    }

    /// Search what `term` holds now: the history's lines not searched yet,
    /// and the screen again. Everything again on another screen or size.
    pub(crate) fn update(&mut self, term: &Term) {
        let grid = term.grid();
        let rows = grid.screen_lines();
        let shape = Shape {
            columns: grid.columns(),
            rows,
            alternate: term.mode().contains(TermMode::ALT_SCREEN),
        };
        let pushed = grid.pushed();
        let oldest = pushed.saturating_sub(grid.history_size() as u64);
        if self.shape != Some(shape) {
            self.shape = Some(shape);
            self.history.clear();
            self.scanned = oldest;
        }
        // Lines that fell off the history's top, their matches with them.
        while self.history.front().is_some_and(|hit| hit.start.0 < oldest) {
            self.history.pop_front();
        }
        self.scanned = self.scanned.max(oldest);
        if self.needle.is_empty() {
            self.screen.clear();
            return;
        }
        let bottom = pushed + rows as u64;
        // The history's new lines, each logical line once it is wholly in
        // the history: it won't change.
        let mut line = self.scanned;
        let mut found = Vec::new();
        while line < pushed {
            let end = logical_end(term, line, bottom);
            if end >= pushed {
                break;
            }
            self.find(term, line, end, &mut found);
            self.history.extend(found.drain(..));
            line = end + 1;
        }
        self.scanned = line;
        // The screen's, again.
        let mut screen = std::mem::take(&mut self.screen);
        screen.clear();
        while line < bottom {
            let end = logical_end(term, line, bottom);
            self.find(term, line, end, &mut screen);
            line = end + 1;
        }
        self.screen = screen;
        if self
            .selected
            .is_some_and(|hit| self.position(hit).is_none())
        {
            self.selected = None;
        }
    }

    /// The matches in lines `first` to `last`, a logical line, onto `found`.
    /// Each cell's character is compared with the needle's first (in either
    /// case, when case doesn't count) and nothing more is asked of it; the
    /// rest of the needle only where that one matches.
    fn find(&self, term: &Term, first: u64, last: u64, found: &mut Vec<Hit>) {
        let Some(&lead) = self.needle.first() else {
            return;
        };
        let other = match self.fold {
            true => other_case(lead),
            false => lead,
        };
        let grid = term.grid();
        let columns = grid.columns();
        let mut number = first;
        let mut from = 0;
        while number <= last {
            let cells = &row(term, number)[..Column(columns)];
            let mut at = from;
            from = 0;
            let mut next = number + 1;
            while let Some(skipped) = cells[at..]
                .iter()
                .position(|cell| cell.c == lead || cell.c == other)
            {
                let start = at + skipped;
                match self.matches_at(term, number, start, last) {
                    Some(end) => {
                        found.push(Hit {
                            start: (number, start),
                            end,
                        });
                        // Past it: on this row, or on the one it ends on.
                        if end.0 > number {
                            next = end.0;
                            from = end.1 + 1;
                            break;
                        }
                        at = end.1 + 1;
                    }
                    None => at = start + 1,
                }
                if at >= columns {
                    break;
                }
            }
            if from >= columns {
                next += 1;
                from = 0;
            }
            number = next;
        }
    }

    /// Where the needle ends if it starts at line `number`, `column` (a
    /// character's first cell), read on through the logical line to `last`:
    /// its last cell, a wide character's second.
    fn matches_at(
        &self,
        term: &Term,
        number: u64,
        column: usize,
        last: u64,
    ) -> Option<(u64, usize)> {
        let columns = term.grid().columns();
        let skip = Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER;
        let (mut number, mut column) = (number, column);
        let mut cells = &row(term, number)[..Column(columns)];
        if cells[column].flags.intersects(skip) {
            return None;
        }
        let mut end = (number, column);
        for &wanted in &self.needle {
            // The next character's cell, past spacers, onto the next row.
            loop {
                if column == columns {
                    if number == last {
                        return None;
                    }
                    number += 1;
                    column = 0;
                    cells = &row(term, number)[..Column(columns)];
                }
                if !cells[column].flags.intersects(skip) {
                    break;
                }
                column += 1;
            }
            let cell = &cells[column];
            if fold_char(cell.c, self.fold) != wanted {
                return None;
            }
            end = match cell.flags.contains(Flags::WIDE_CHAR) {
                true => (number, (column + 1).min(columns - 1)),
                false => (number, column),
            };
            column += 1;
        }
        Some(end)
    }

    /// Every match found.
    pub(crate) fn total(&self) -> usize {
        self.history.len() + self.screen.len()
    }

    /// The selected match, counted from the newest (0: nearest the
    /// bottom), as Ghostty counts it.
    pub(crate) fn selected(&self) -> Option<usize> {
        let position = self.position(self.selected?)?;
        Some(self.total() - 1 - position)
    }

    /// The selected match's first line, to scroll to.
    pub(crate) fn selected_line(&self) -> Option<u64> {
        self.selected.map(|hit| hit.start.0)
    }

    /// Select the next older match (`older`), or the next newer, around the
    /// ends; the newest first going older, the oldest going newer.
    pub(crate) fn navigate(&mut self, older: bool) {
        let total = self.total();
        if total == 0 {
            self.selected = None;
            return;
        }
        let position = match (self.selected.and_then(|hit| self.position(hit)), older) {
            (None, true) => total - 1,
            (None, false) => 0,
            (Some(0), true) => total - 1,
            (Some(position), true) => position - 1,
            (Some(position), false) => (position + 1) % total,
        };
        self.selected = Some(self.at(position));
    }

    /// Where `hit` is among the matches, oldest first.
    fn position(&self, hit: Hit) -> Option<usize> {
        if let Ok(position) = self.history.binary_search(&hit) {
            return Some(position);
        }
        let position = self.screen.binary_search(&hit).ok()?;
        Some(self.history.len() + position)
    }

    fn at(&self, position: usize) -> Hit {
        match self.history.get(position) {
            Some(hit) => *hit,
            None => self.screen[position - self.history.len()],
        }
    }

    /// The columns to highlight in the rows `term` shows, top first.
    pub(crate) fn in_view(&self, term: &Term) -> Vec<Highlight> {
        let grid = term.grid();
        let rows = grid.screen_lines() as u64;
        let columns = grid.columns();
        let top = grid.pushed() - grid.display_offset() as u64;
        let bottom = top + rows;
        let mut highlights = Vec::new();
        let from = self.history.partition_point(|hit| hit.end.0 < top);
        let older = self.history.range(from..);
        let from = self.screen.partition_point(|hit| hit.end.0 < top);
        for hit in older.chain(&self.screen[from..]) {
            if hit.start.0 >= bottom {
                break;
            }
            let selected = self.selected == Some(*hit);
            for line in hit.start.0.max(top)..=hit.end.0.min(bottom - 1) {
                highlights.push(Highlight {
                    row: (line - top) as usize,
                    first: if line == hit.start.0 { hit.start.1 } else { 0 },
                    last: if line == hit.end.0 {
                        hit.end.1
                    } else {
                        columns - 1
                    },
                    selected,
                });
            }
        }
        highlights
    }
}

/// The last line of the logical line starting at `line`: the lines after it
/// it wraps onto, before `bottom`.
fn logical_end(term: &Term, line: u64, bottom: u64) -> u64 {
    let grid = term.grid();
    let pushed = grid.pushed() as i64;
    let last = Column(grid.columns() - 1);
    let mut end = line;
    while end + 1 < bottom
        && grid[Line((end as i64 - pushed) as i32)][last]
            .flags
            .contains(Flags::WRAPLINE)
    {
        end += 1;
    }
    end
}

/// Line `number`'s cells.
#[inline]
fn row(
    term: &Term,
    number: u64,
) -> &alacritty_terminal::grid::Row<alacritty_terminal::term::cell::Cell> {
    let grid = term.grid();
    &grid[Line((number as i64 - grid.pushed() as i64) as i32)]
}

/// `c` in the other case, if it has one (itself, if not).
fn other_case(c: char) -> char {
    match (c.to_uppercase().next(), c.to_lowercase().next()) {
        (Some(upper), _) if upper != c => upper,
        (_, Some(lower)) => lower,
        _ => c,
    }
}

/// `c` in lower case when case doesn't count (one character for one).
#[inline]
fn fold_char(c: char, fold: bool) -> char {
    match (fold, c.is_ascii()) {
        (false, _) => c,
        (true, true) => c.to_ascii_lowercase(),
        (true, false) => c.to_lowercase().next().unwrap_or(c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GridSize, Session, SessionOptions};
    use alacritty_terminal::grid::Scroll;

    /// A session 20 by 5 keeping `scrollback` lines, fed `output`.
    fn session(scrollback: usize, output: &str) -> Session {
        let options = SessionOptions {
            scrollback,
            ..SessionOptions::default()
        };
        let session = Session::replay(&options, GridSize::new(20, 5), || {});
        session.feed(output.as_bytes());
        session
    }

    /// `needle` searched in what `session` holds.
    fn search(session: &Session, needle: &str) -> Search {
        let mut search = Search::new(needle);
        search.update(&session.term());
        search
    }

    /// Lines `0..n`, every `every`th saying `error`.
    fn lines(n: usize, every: usize) -> String {
        (0..n)
            .map(|i| match i % every {
                0 => format!("{i} error here\r\n"),
                _ => format!("{i} fine\r\n"),
            })
            .collect()
    }

    /// Every match, in the history and on the screen, counted once.
    #[test]
    fn matches_are_counted_in_the_history_and_on_the_screen() {
        let session = session(1000, &lines(100, 10));
        let found = search(&session, "error");
        assert_eq!(found.total(), 10);
        // Lines 95 to 99 and an empty one on the screen: 0 to 90 in the
        // history.
        assert_eq!((found.history.len(), found.screen.len()), (10, 0));
        assert_eq!(found.selected(), None, "none selected until asked");
    }

    /// Case counts only when the needle has a capital.
    #[test]
    fn case_counts_only_with_a_capital() {
        let session = session(100, "Error\r\nerror\r\nERROR\r\n");
        assert_eq!(search(&session, "error").total(), 3);
        assert_eq!(search(&session, "Error").total(), 1);
    }

    /// A match across a line's wrap, and wide characters, each a cell pair.
    #[test]
    fn a_match_wraps_and_wide_characters_cover_two_cells() {
        // 20 columns: "needle" from column 17 wraps after "nee".
        let session = session(100, &format!("{}needle\r\n日本語 日本\r\n", "x".repeat(17)));
        let found = search(&session, "needle");
        assert_eq!(found.total(), 1);
        let hit = found.at(0);
        assert_eq!((hit.start.1, hit.end.1), (17, 2), "{hit:?}");
        assert_eq!(hit.end.0, hit.start.0 + 1, "on the next line");
        let wide = search(&session, "日本");
        assert_eq!(wide.total(), 2);
        let first = wide.at(0);
        assert_eq!((first.start.1, first.end.1), (0, 3), "two wide cells");
    }

    /// Output arriving while a search is open: the history's lines are
    /// searched once, as they arrive; the screen's again.
    #[test]
    fn output_arriving_is_searched_as_it_arrives() {
        let session = session(1000, &lines(50, 10));
        let mut found = search(&session, "error");
        assert_eq!(found.total(), 5);
        let history = found.history.len();
        session.feed(lines(50, 5).as_bytes());
        found.update(&session.term());
        assert_eq!(found.total(), 15);
        assert!(found.history.len() > history);
        // The screen rewritten in place: its match goes.
        session.feed(b"error\r\n\x1b[1A\x1b[2Kfine");
        found.update(&session.term());
        assert_eq!(found.total(), 15, "rewritten");
    }

    /// Lines falling off the history's top take their matches with them.
    #[test]
    fn matches_fall_off_the_history_with_their_lines() {
        let session = session(10, "");
        let mut found = search(&session, "error");
        session.feed(lines(100, 10).as_bytes());
        found.update(&session.term());
        // 10 lines of history and 5 on the screen (the last empty): lines
        // 86 to 99, of which 90 is an error.
        assert_eq!(found.total(), 1);
        assert_eq!(
            found.total(),
            search(&session, "error").total(),
            "as if searched anew"
        );
    }

    /// Navigating: older from the newest, around the ends; newer from the
    /// oldest.
    #[test]
    fn navigating_goes_older_and_newer_around_the_ends() {
        let session = session(1000, &lines(30, 10));
        let mut found = search(&session, "error");
        assert_eq!(found.total(), 3);
        found.navigate(true);
        assert_eq!(found.selected(), Some(0), "the newest first");
        found.navigate(true);
        assert_eq!(found.selected(), Some(1));
        found.navigate(true);
        found.navigate(true);
        assert_eq!(found.selected(), Some(0), "around to the newest");
        found.navigate(false);
        assert_eq!(
            found.selected(),
            Some(2),
            "newer from the newest: the oldest"
        );
        let mut fresh = search(&session, "error");
        fresh.navigate(false);
        assert_eq!(fresh.selected(), Some(2), "newer first: the oldest");
    }

    /// The matches in view, by row and columns, the selected one marked;
    /// scrolled back, the history's.
    #[test]
    fn matches_in_view_are_highlighted() {
        let session = session(1000, "a\r\nb error\r\nc\r\nd\r\nerror e\r\nf");
        let mut found = search(&session, "error");
        found.navigate(true);
        let view = found.in_view(&session.term());
        assert_eq!(
            view,
            [
                Highlight {
                    row: 0,
                    first: 2,
                    last: 6,
                    selected: false
                },
                Highlight {
                    row: 3,
                    first: 0,
                    last: 4,
                    selected: true
                },
            ]
        );
        session.term().scroll_display(Scroll::Delta(1));
        let view = found.in_view(&session.term());
        assert_eq!(view.iter().map(|h| h.row).collect::<Vec<_>>(), [1, 4]);
    }

    /// A screen of another size is searched again.
    #[test]
    fn a_resized_screen_is_searched_again() {
        let session = session(1000, &lines(30, 10));
        let mut found = search(&session, "error");
        session.resize(GridSize::new(10, 4));
        found.update(&session.term());
        assert_eq!(found.total(), search(&session, "error").total());
    }
}
