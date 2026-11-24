use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use bitflags::bitflags;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::grid::{self, GridCell};
use crate::index::Column;
use crate::vte::ansi::{Color, Hyperlink as VteHyperlink, NamedColor};

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
    pub struct Flags: u16 {
        const INVERSE                   = 0b0000_0000_0000_0001;
        const BOLD                      = 0b0000_0000_0000_0010;
        const ITALIC                    = 0b0000_0000_0000_0100;
        const BOLD_ITALIC               = 0b0000_0000_0000_0110;
        const UNDERLINE                 = 0b0000_0000_0000_1000;
        const WRAPLINE                  = 0b0000_0000_0001_0000;
        const WIDE_CHAR                 = 0b0000_0000_0010_0000;
        const WIDE_CHAR_SPACER          = 0b0000_0000_0100_0000;
        const DIM                       = 0b0000_0000_1000_0000;
        const DIM_BOLD                  = 0b0000_0000_1000_0010;
        const HIDDEN                    = 0b0000_0001_0000_0000;
        const STRIKEOUT                 = 0b0000_0010_0000_0000;
        const LEADING_WIDE_CHAR_SPACER  = 0b0000_0100_0000_0000;
        const DOUBLE_UNDERLINE          = 0b0000_1000_0000_0000;
        const UNDERCURL                 = 0b0001_0000_0000_0000;
        const DOTTED_UNDERLINE          = 0b0010_0000_0000_0000;
        const DASHED_UNDERLINE          = 0b0100_0000_0000_0000;
        const ALL_UNDERLINES            = Self::UNDERLINE.bits() | Self::DOUBLE_UNDERLINE.bits()
                                        | Self::UNDERCURL.bits() | Self::DOTTED_UNDERLINE.bits()
                                        | Self::DASHED_UNDERLINE.bits();
    }
}

/// Counter for hyperlinks without explicit ID.
static HYPERLINK_ID_SUFFIX: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Hyperlink {
    inner: Arc<HyperlinkInner>,
}

impl Hyperlink {
    pub fn new<T: ToString>(id: Option<T>, uri: String) -> Self {
        let inner = Arc::new(HyperlinkInner::new(id, uri));
        Self { inner }
    }

    pub fn id(&self) -> &str {
        &self.inner.id
    }

    pub fn uri(&self) -> &str {
        &self.inner.uri
    }
}

impl From<VteHyperlink> for Hyperlink {
    fn from(value: VteHyperlink) -> Self {
        Self::new(value.id, value.uri)
    }
}

impl From<Hyperlink> for VteHyperlink {
    fn from(val: Hyperlink) -> Self {
        VteHyperlink {
            id: Some(val.id().to_owned()),
            uri: val.uri().to_owned(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
struct HyperlinkInner {
    /// Identifier for the given hyperlink.
    id: String,

    /// Resource identifier of the hyperlink.
    uri: String,
}

impl HyperlinkInner {
    pub fn new<T: ToString>(id: Option<T>, uri: String) -> Self {
        let id = match id {
            Some(id) => id.to_string(),
            None => {
                let mut id = HYPERLINK_ID_SUFFIX
                    .fetch_add(1, Ordering::Relaxed)
                    .to_string();
                id.push_str("_alacritty");
                id
            }
        };

        Self { id, uri }
    }
}

/// Trait for determining if a reset should be performed.
pub trait ResetDiscriminant<T> {
    /// Value based on which equality for the reset will be determined.
    fn discriminant(&self) -> T;
}

// zgx: for the cell types the grid's tests use, not every `Copy` type: that
// would take in `Cell`, `Copy` now, whose discriminant is its background.
impl ResetDiscriminant<usize> for usize {
    fn discriminant(&self) -> usize {
        *self
    }
}

impl ResetDiscriminant<char> for char {
    fn discriminant(&self) -> char {
        *self
    }
}

impl ResetDiscriminant<Color> for Cell {
    fn discriminant(&self) -> Color {
        self.bg
    }
}

/// Dynamically allocated cell content.
///
/// This storage is reserved for cell attributes which are rarely set. This allows reducing the
/// allocation required ahead of time for every cell, with some additional overhead when the extra
/// storage is actually required.
///
/// zgx: a cell names its extra by a handle into its row's table of them
/// (`Row::extras`), which cells with the same extra share (a hyperlink's run
/// is one slot), so a cell is 16 bytes and `Copy`. A slot is never changed
/// once in the table: a cell's extra changed is a new slot.
#[derive(Default, Debug, Clone, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CellExtra {
    /// zgx: the characters joined to the cell's, inline for as many as most
    /// clusters have (a selector, a ZWJ and an emoji, a skin tone), so a
    /// cell with them costs one allocation, not two.
    zerowidth: SmallVec<[char; 4]>,
    underline_color: Option<Color>,
    hyperlink: Option<Hyperlink>,
}

impl CellExtra {
    /// Zerowidth characters joined to the cell's.
    #[inline]
    pub fn zerowidth(&self) -> &[char] {
        &self.zerowidth
    }

    /// The cell's underline colour, if one of its own.
    #[inline]
    pub fn underline_color(&self) -> Option<Color> {
        self.underline_color
    }

    /// The cell's hyperlink (OSC 8).
    #[inline]
    pub fn hyperlink(&self) -> Option<&Hyperlink> {
        self.hyperlink.as_ref()
    }

    /// zgx: `extra` (a cell's or the cursor's pen) with `character` joined,
    /// copied where it's shared.
    pub(crate) fn push_zerowidth(extra: &mut Option<Arc<Self>>, character: char) {
        let extra = extra.get_or_insert(Default::default());
        Arc::make_mut(extra).zerowidth.push(character);
    }

    /// zgx: `extra` without joined characters, copied where it's shared (and
    /// kept, as a cell's was, where it holds nothing else).
    pub(crate) fn clear_zerowidth(extra: &mut Option<Arc<Self>>) {
        if let Some(extra) = extra.as_mut() {
            Arc::make_mut(extra).zerowidth = SmallVec::new();
        }
    }

    /// zgx: `extra` with the underline colour `color`, copied where it's
    /// shared; none left where it holds nothing else.
    pub(crate) fn set_underline_color(extra: &mut Option<Arc<Self>>, color: Option<Color>) {
        // If we reset color and we don't have zerowidth we should drop extra storage.
        if color.is_none()
            && extra
                .as_ref()
                .is_none_or(|extra| extra.zerowidth.is_empty() && extra.hyperlink.is_none())
        {
            *extra = None;
        } else {
            let extra = extra.get_or_insert(Default::default());
            Arc::make_mut(extra).underline_color = color;
        }
    }

    /// zgx: `extra` with the hyperlink `hyperlink`, copied where it's
    /// shared; none left where it holds nothing else.
    pub(crate) fn set_hyperlink(extra: &mut Option<Arc<Self>>, hyperlink: Option<Hyperlink>) {
        let should_drop = hyperlink.is_none()
            && extra
                .as_ref()
                .is_none_or(|extra| extra.zerowidth.is_empty() && extra.underline_color.is_none());

        if should_drop {
            *extra = None;
        } else {
            let extra = extra.get_or_insert(Default::default());
            Arc::make_mut(extra).hyperlink = hyperlink;
        }
    }
}

/// Content and attributes of a single cell in the terminal grid.
///
/// zgx: what's rarely set (joined characters, an underline colour, a
/// hyperlink) is in its row: `Row::zerowidth`, `Row::underline_color`,
/// `Row::hyperlink` read it by column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Cell {
    pub c: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
    /// zgx: the cell's extra, slot `extra - 1` of its row's table, or none
    /// for 0. Only meaningful in its row.
    pub(crate) extra: u16,
}

impl Default for Cell {
    #[inline]
    fn default() -> Cell {
        BLANK
    }
}

/// zgx: a blank cell on the default background (`Cell::default`).
const BLANK: Cell = Cell {
    c: ' ',
    bg: Color::Named(NamedColor::Background),
    fg: Color::Named(NamedColor::Foreground),
    flags: Flags::empty(),
    extra: 0,
};

/// zgx: blanks to copy a row's cells from in one go, as bytes: a `Cell`
/// stored a field at a time is four stores (`Color`'s padding).
static BLANKS: [Cell; 512] = [BLANK; 512];

impl Cell {
    /// zgx: `cells` made blank on the default background (`Cell::default`),
    /// copied from `BLANKS` as bytes.
    #[inline]
    pub(crate) fn blank_all(cells: &mut [Cell]) {
        for chunk in cells.chunks_mut(BLANKS.len()) {
            chunk.copy_from_slice(&BLANKS[..chunk.len()]);
        }
    }
}

impl Cell {
    /// zgx: whether the cell has an extra in its row's table (joined
    /// characters, an underline colour or a hyperlink; `Row::extra`).
    #[inline]
    pub fn has_extra(&self) -> bool {
        self.extra != 0
    }
}

impl GridCell for Cell {
    /// zgx: what the cell holds itself; whether its row holds zero-width
    /// characters for it is `Row::is_cell_empty`'s to add.
    #[inline]
    fn is_empty(&self) -> bool {
        (self.c == ' ' || self.c == '\t')
            && self.bg == Color::Named(NamedColor::Background)
            && self.fg == Color::Named(NamedColor::Foreground)
            && !self.flags.intersects(
                Flags::INVERSE
                    | Flags::ALL_UNDERLINES
                    | Flags::STRIKEOUT
                    | Flags::WRAPLINE
                    | Flags::WIDE_CHAR_SPACER
                    | Flags::LEADING_WIDE_CHAR_SPACER,
            )
    }

    #[inline]
    fn flags(&self) -> &Flags {
        &self.flags
    }

    #[inline]
    fn flags_mut(&mut self) -> &mut Flags {
        &mut self.flags
    }

    #[inline]
    fn reset(&mut self, template: &Self) {
        *self = Cell {
            bg: template.bg,
            ..Cell::default()
        };
    }

    /// zgx: a cell holds nothing to drop: the template's blank stored over
    /// each, the old ones unread (the row lets go of its extras). Copied as
    /// bytes, not field by field (`Color`'s padding keeps a blank stored a
    /// field at a time from being stored whole): from `BLANKS` on the default
    /// background, otherwise doubling from the first.
    #[inline]
    fn reset_all(cells: &mut [Self], template: &Self) {
        // On the default background (the usual): copied from `BLANKS`.
        if template.bg == BLANK.bg {
            Cell::blank_all(cells);
            return;
        }
        let Some(first) = cells.first_mut() else {
            return;
        };
        *first = Cell {
            bg: template.bg,
            ..Cell::default()
        };
        let mut filled = 1;
        while filled < cells.len() {
            let count = filled.min(cells.len() - filled);
            cells.copy_within(0..count, filled);
            filled += count;
        }
    }

    #[inline]
    fn extra_handle(&self) -> u16 {
        self.extra
    }

    #[inline]
    fn set_extra_handle(&mut self, handle: u16) {
        self.extra = handle;
    }
}

impl From<Color> for Cell {
    #[inline]
    fn from(color: Color) -> Self {
        Self {
            bg: color,
            ..Cell::default()
        }
    }
}

/// Get the length of occupied cells in a line.
pub trait LineLength {
    /// Calculate the occupied line length.
    fn line_length(&self) -> Column;
}

impl LineLength for grid::Row<Cell> {
    fn line_length(&self) -> Column {
        let mut length = Column(0);

        if self[Column(self.len() - 1)].flags.contains(Flags::WRAPLINE) {
            return Column(self.len());
        }

        for (index, cell) in self[..].iter().rev().enumerate() {
            if cell.c != ' ' || self.has_zerowidth(cell) {
                length = Column(self.len() - index);
                break;
            }
        }

        length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::mem;

    use crate::grid::Row;
    use crate::index::Column;

    /// Something written to a row's cell at a column, and to a row.
    type WriteAt<'a> = dyn Fn(&mut Row<Cell>, Column) + 'a;
    type Write<'a> = dyn Fn(&mut Row<Cell>) + 'a;

    /// zgx: 16 bytes, its extra in its row.
    #[test]
    fn cell_size_is_16_bytes() {
        assert_eq!(mem::size_of::<Cell>(), 16);
    }

    /// zgx: a row is four words (its cells boxed, `occ`, its table boxed):
    /// seven took a multiply to index the storage, and a search over the
    /// history paid it on every row.
    #[test]
    fn row_size_is_four_words() {
        assert_eq!(mem::size_of::<Row<Cell>>(), 4 * mem::size_of::<usize>());
    }

    /// zgx: a row resets to blanks on the template's background whatever its
    /// cells hold (a combining mark, an underline colour, a hyperlink, one
    /// shared with others, or nothing extra) and wherever, and lets go of
    /// what they held.
    /// zgx: a row wider than `BLANKS` resets whole, on the default
    /// background (copied from `BLANKS` in chunks) and on another (doubling).
    #[test]
    fn a_row_wider_than_the_blanks_resets_whole() {
        for bg in [Color::Named(NamedColor::Background), Color::Indexed(21)] {
            let template = Cell {
                bg,
                ..Cell::default()
            };
            let mut row = Row::<Cell>::new(600);
            for column in 0..600 {
                let cell = &mut row[Column(column)];
                cell.c = 'x';
                cell.fg = Color::Indexed(1);
                cell.bg = Color::Indexed(2);
                cell.flags = Flags::BOLD;
            }
            row.reset(&template);
            for column in 0..600 {
                assert_eq!(row[Column(column)], template, "{bg:?}, column {column}");
            }
        }
    }

    #[test]
    fn a_row_resets_whatever_its_cells_hold() {
        let shared = Arc::new(CellExtra {
            hyperlink: Some(Hyperlink::new(Some("a"), "https://example.com".into())),
            ..CellExtra::default()
        });
        let extras: [&WriteAt<'_>; 5] = [
            &|_, _| {},
            &|row, at| row.push_zerowidth(at, '\u{301}'),
            &|row, at| row.set_underline_color(at, Some(Color::Indexed(3))),
            &|row, at| {
                row.set_hyperlink(at, Some(Hyperlink::new(Some("b"), "https://b.com".into())))
            },
            &|row, at| row.set_extra(at, Some(shared.clone())),
        ];
        for bg in [Color::Named(NamedColor::Background), Color::Indexed(21)] {
            let template = Cell {
                bg,
                ..Cell::default()
            };
            for (kind, extra) in extras.iter().enumerate() {
                for at in [0, 4, 9] {
                    let mut row = Row::<Cell>::new(10);
                    for column in 0..10 {
                        let cell = &mut row[Column(column)];
                        cell.c = 'x';
                        cell.fg = Color::Indexed(1);
                        cell.bg = bg;
                        cell.flags = Flags::BOLD | Flags::UNDERLINE;
                    }
                    extra(&mut row, Column(at));
                    extra(&mut row, Column(9 - at));
                    row.reset(&template);
                    for column in 0..10 {
                        assert_eq!(row[Column(column)], template, "{kind} at {at}, {column}");
                    }
                    assert!(row.extras.is_empty(), "{kind} at {at}");
                    assert_eq!(Arc::strong_count(&shared), 1, "{kind} at {at}");
                }
            }
        }
    }

    /// zgx: a blank cell with a hyperlink or an underline colour alone is
    /// empty (its row clear, its line's length short of it), as it was
    /// before its extra moved to its row; one with a joined character isn't.
    #[test]
    fn only_joined_characters_make_a_blank_cell_content() {
        let link = Hyperlink::new(Some("a"), "https://example.com".into());
        let extras: [(&Write<'_>, bool); 3] = [
            (
                &|row| row.set_hyperlink(Column(5), Some(link.clone())),
                true,
            ),
            (
                &|row| row.set_underline_color(Column(5), Some(Color::Indexed(3))),
                true,
            ),
            (&|row| row.push_zerowidth(Column(5), '\u{301}'), false),
        ];
        for (extra, empty) in extras {
            let mut row = Row::<Cell>::new(10);
            extra(&mut row);
            assert!(row[Column(5)].has_extra());
            assert_eq!(row.is_cell_empty(&row[Column(5)]), empty);
            assert_eq!(row.is_clear(), empty);
            assert_eq!(row.line_length(), Column(if empty { 0 } else { 6 }));
        }
    }

    /// zgx: rows compare by what their cells hold, extras by value, not by
    /// their handles' numbers.
    #[test]
    fn rows_compare_by_their_cells_extras() {
        let link = |id: &str| Some(Hyperlink::new(Some(id), "https://example.com".into()));
        let mut one = Row::<Cell>::new(4);
        one.set_hyperlink(Column(0), link("a"));
        one.set_underline_color(Column(2), Some(Color::Indexed(3)));
        let mut other = Row::<Cell>::new(4);
        other.set_underline_color(Column(2), Some(Color::Indexed(3)));
        other.set_hyperlink(Column(0), link("a"));
        assert_ne!(one[Column(0)].extra, other[Column(0)].extra);
        assert_eq!(one, other);
        other.set_hyperlink(Column(0), link("b"));
        assert_ne!(one, other);
        other.set_hyperlink(Column(0), link("a"));
        other.push_zerowidth(Column(1), '\u{301}');
        assert_ne!(one, other);
    }

    #[test]
    fn line_length_works() {
        let mut row = Row::<Cell>::new(10);
        row[Column(5)].c = 'a';

        assert_eq!(row.line_length(), Column(6));
    }

    #[test]
    fn line_length_works_with_wrapline() {
        let mut row = Row::<Cell>::new(10);
        row[Column(9)].flags.insert(super::Flags::WRAPLINE);

        assert_eq!(row.line_length(), Column(10));
    }
}
