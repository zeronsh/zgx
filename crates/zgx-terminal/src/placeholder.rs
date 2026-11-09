//! Unicode placeholders (the kitty graphics protocol's virtual placements,
//! `U=1`): a picture shown where the text puts it. Each cell holding the
//! placeholder U+10EEEE shows one cell of a picture fitted into its virtual
//! placement: its foreground colour names the picture (the low 24 bits, or
//! a 256-colour index), its underline colour the placement, and combining
//! diacritics after it the row, the column and the picture's top byte, any
//! of which a cell may leave to the one at its left. As tmux passes them on
//! and programs inside it (yazi, image.nvim) draw pictures.

use alacritty_terminal::{
    term::cell::Cell,
    vte::ansi::{Color, Rgb},
};

/// The placeholder character.
pub(crate) const PLACEHOLDER: char = '\u{10EEEE}';

/// The diacritics that number rows and columns, in order: those of
/// Unicode 6.0.0's `UnicodeData.txt` that are non-spacing marks of
/// combining class 230 without a decomposition (`Mn;230;NSM;;`), but for
/// 0300-0304, 0306-030C, 030F, 0311, 0313, 0314, 0342, 0653 and 0654, as the
/// protocol has it. The `n`th numbers row or column `n`.
const DIACRITICS: [u32; 297] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B, 0x034C,
    0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369,
    0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592,
    0x0593, 0x0594, 0x0595, 0x0597, 0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1,
    0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
    0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6, 0x06D7, 0x06D8,
    0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2, 0x06E4, 0x06E7, 0x06E8, 0x06EB,
    0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743,
    0x0745, 0x0747, 0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3,
    0x0816, 0x0817, 0x0818, 0x0819, 0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
    0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C, 0x082D, 0x0951, 0x0953, 0x0954,
    0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
    0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71,
    0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5,
    0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
    0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7,
    0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2,
    0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE,
    0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1,
    0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED,
    0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1,
    0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186,
    0x1D187, 0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];

/// The number a diacritic stands for, if it is one.
fn number(c: char) -> Option<u32> {
    DIACRITICS.binary_search(&(c as u32)).ok().map(|n| n as u32)
}

/// A colour as a number: 24-bit, or a 256-colour index.
fn id(color: Color) -> Option<u32> {
    match color {
        Color::Spec(Rgb { r, g, b }) => Some(u32::from_be_bytes([0, r, g, b])),
        Color::Indexed(index) => Some(u32::from(index)),
        Color::Named(named) => Some(named as u32).filter(|n| *n < 256),
    }
}

/// What a placeholder cell shows: which picture and placement, and which of
/// its cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shows {
    pub image: u32,
    pub placement: u32,
    pub row: u32,
    pub column: u32,
}

/// A placeholder cell as read, with what the next may take from it.
#[derive(Clone, Copy)]
pub(crate) struct Read {
    pub shows: Shows,
    /// The picture's low 24 bits, the underline colour and the top byte,
    /// as the next cell compares and takes them.
    low: u32,
    underline: Option<Color>,
    top: u32,
}

/// Read `cell`, a placeholder, given the placeholder at its left (`left`),
/// if there is one: what a cell leaves out it takes from the cell at its
/// left, when that one names the same picture and placement by its colours
/// (the row, unless given; the column after that cell's, unless given; the
/// top byte, unless given); else what's left out is 0. None when its
/// foreground names no picture.
pub(crate) fn read(cell: &Cell, left: Option<&Read>) -> Option<Read> {
    let low = id(cell.fg)?;
    let underline = cell.underline_color();
    let marks = cell.zerowidth().unwrap_or(&[]);
    let mut given = marks.iter().map_while(|&c| number(c));
    let (row, column, top) = (given.next(), given.next(), given.next());
    let left = left.filter(|left| left.low == low && left.underline == underline);
    let (row, column, top) = match (row, column, top, left) {
        (row, column, Some(top), _) => (row.unwrap_or(0), column.unwrap_or(0), top),
        (None, _, _, Some(left)) => (left.shows.row, left.shows.column + 1, left.top),
        (Some(row), None, _, Some(left)) if row == left.shows.row => {
            (row, left.shows.column + 1, left.top)
        }
        (Some(row), Some(column), _, Some(left))
            if row == left.shows.row && column == left.shows.column + 1 =>
        {
            (row, column, left.top)
        }
        (row, column, _, _) => (row.unwrap_or(0), column.unwrap_or(0), 0),
    };
    Some(Read {
        shows: Shows {
            image: low | top << 24,
            placement: underline.and_then(id).unwrap_or(0),
            row,
            column,
        },
        low,
        underline,
        top,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A placeholder cell in `fg`, underlined in `underline`, with `marks`.
    fn cell(fg: Color, underline: Option<Color>, marks: &[char]) -> Cell {
        let mut cell = Cell {
            c: PLACEHOLDER,
            fg,
            ..Cell::default()
        };
        cell.set_underline_color(underline);
        for &mark in marks {
            cell.push_zerowidth(mark);
        }
        cell
    }

    /// The `n`th diacritic.
    fn mark(n: usize) -> char {
        char::from_u32(DIACRITICS[n]).unwrap()
    }

    fn shows(cells: &[Cell]) -> Vec<(u32, u32, u32, u32)> {
        let mut left = None;
        cells
            .iter()
            .map(|cell| {
                let read = read(cell, left.as_ref()).unwrap();
                left = Some(read);
                let s = read.shows;
                (s.image, s.placement, s.row, s.column)
            })
            .collect()
    }

    /// The table is the protocol's: 297 diacritics, in order, from U+0305
    /// to U+1D244, each numbering its place.
    #[test]
    fn the_diacritics_number_rows_and_columns() {
        assert_eq!(DIACRITICS.len(), 297);
        assert!(DIACRITICS.is_sorted());
        assert_eq!((DIACRITICS[0], DIACRITICS[296]), (0x0305, 0x1D244));
        assert_eq!(number('\u{0305}'), Some(0));
        assert_eq!(number('\u{030D}'), Some(1));
        assert_eq!(number('\u{0300}'), None, "left out, as the protocol has it");
        assert_eq!(number('a'), None);
    }

    /// The picture is the foreground colour (24 bits, or a 256-colour
    /// index) with a third diacritic as its top byte; the placement the
    /// underline colour.
    #[test]
    fn colours_name_the_picture_and_placement() {
        let rgb = Color::Spec(Rgb { r: 1, g: 2, b: 3 });
        let both = [
            cell(Color::Indexed(42), None, &[mark(2), mark(3)]),
            cell(rgb, Some(Color::Indexed(9)), &[mark(0), mark(1), mark(5)]),
        ];
        assert_eq!(
            read(&both[0], None).unwrap().shows,
            Shows {
                image: 42,
                placement: 0,
                row: 2,
                column: 3,
            }
        );
        assert_eq!(
            read(&both[1], None).unwrap().shows,
            Shows {
                image: 5 << 24 | 0x010203,
                placement: 9,
                row: 0,
                column: 1,
            }
        );
    }

    /// What a cell leaves out it takes from the one at its left, if that
    /// names the same picture and placement: nothing given, the next column
    /// on its row; only the row, the next column if the row is the same;
    /// row and column, the top byte if it follows on. Else what's left out
    /// is 0.
    #[test]
    fn a_cell_takes_what_it_leaves_out_from_its_left() {
        let fg = Color::Indexed(7);
        let id = |marks: &[char]| cell(fg, None, marks);
        // Row 1, column 2, top byte 3; then nothing; then the row alone;
        // then row and column following on.
        let cells = [
            id(&[mark(1), mark(2), mark(3)]),
            id(&[]),
            id(&[mark(1)]),
            id(&[mark(1), mark(5)]),
        ];
        let image = 3 << 24 | 7;
        assert_eq!(
            shows(&cells),
            [
                (image, 0, 1, 2),
                (image, 0, 1, 3),
                (image, 0, 1, 4),
                (image, 0, 1, 5)
            ]
        );
        // A different row given: the column isn't taken.
        assert_eq!(
            shows(&[id(&[mark(1), mark(2)]), id(&[mark(4)])])[1],
            (7, 0, 4, 0)
        );
        // A column that doesn't follow on: the top byte isn't taken.
        let cells = [id(&[mark(0), mark(0), mark(9)]), id(&[mark(0), mark(5)])];
        assert_eq!(shows(&cells)[1], (7, 0, 0, 5));
        // Another picture at the left: nothing is taken.
        let cells = [cell(Color::Indexed(8), None, &[mark(1), mark(1)]), id(&[])];
        assert_eq!(shows(&cells)[1], (7, 0, 0, 0));
        // A default foreground names no picture.
        assert!(
            read(
                &cell(
                    Color::Named(alacritty_terminal::vte::ansi::NamedColor::Foreground),
                    None,
                    &[]
                ),
                None
            )
            .is_none()
        );
    }
}
