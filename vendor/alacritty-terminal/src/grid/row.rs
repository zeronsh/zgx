//! Defines the Row type which makes up lines in the grid.

use std::cmp::{max, min};
use std::ops::{
    Deref, DerefMut, Index, IndexMut, Range, RangeFrom, RangeFull, RangeTo, RangeToInclusive,
};
use std::sync::Arc;
use std::{mem, ptr, slice};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::grid::GridCell;
use crate::index::Column;
use crate::term::cell::{Cell, CellExtra, Flags, Hyperlink, ResetDiscriminant};
use crate::vte::ansi::Color;

/// A row in the grid.
#[derive(Default, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Row<T> {
    /// zgx: a boxed slice, not a `Vec`: a row's width changes only with a
    /// resize, and four words a row (with `occ` and `extras`) index the
    /// storage with a shift where seven took a multiply and a register.
    inner: Box<[T]>,

    /// Maximum number of occupied entries.
    ///
    /// This is the upper bound on the number of elements in the row, which have been modified
    /// since the last reset. All cells after this point are guaranteed to be equal.
    pub(crate) occ: usize,

    /// zgx: the extras its cells hold, slot `handle - 1` for a cell's handle
    /// (`GridCell::extra_handle`). Cells written with the same one share its
    /// slot (a hyperlink's run is one); a slot is never changed, a cell's
    /// extra changed is a new one. Slots no cell holds any more stay until
    /// the row is reset, or the table, full (`max(width, 8)` slots), is
    /// compacted to those its cells hold. No cell past `occ` holds one.
    pub(crate) extras: Extras,
}

/// zgx: a row's table of extras, boxed: one word and no allocation for a
/// row without any (most), so a row stays four words.
#[derive(Default, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub(crate) struct Extras(Option<Box<Vec<Arc<CellExtra>>>>);

impl Extras {
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |table| table.len())
    }

    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    #[inline]
    pub(crate) fn clear(&mut self) {
        self.0 = None;
    }

    #[inline]
    pub(crate) fn as_slice(&self) -> &[Arc<CellExtra>] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }

    #[inline]
    pub(crate) fn get(&self, slot: usize) -> Option<&Arc<CellExtra>> {
        self.as_slice().get(slot)
    }

    #[inline]
    pub(crate) fn last(&self) -> Option<&Arc<CellExtra>> {
        self.as_slice().last()
    }

    #[inline]
    pub(crate) fn push(&mut self, extra: Arc<CellExtra>) {
        self.0.get_or_insert_default().push(extra);
    }

    #[inline]
    pub(crate) fn append(&mut self, extras: &mut Vec<Arc<CellExtra>>) {
        if !extras.is_empty() {
            self.0.get_or_insert_default().append(extras);
        }
    }

    #[inline]
    pub(crate) fn extend(&mut self, extras: Vec<Arc<CellExtra>>) {
        if !extras.is_empty() {
            self.0.get_or_insert_default().extend(extras);
        }
    }

    /// The table, taken: empty after.
    #[inline]
    pub(crate) fn take(&mut self) -> Vec<Arc<CellExtra>> {
        self.0.take().map_or_else(Vec::new, |table| *table)
    }
}

impl From<Vec<Arc<CellExtra>>> for Extras {
    #[inline]
    fn from(table: Vec<Arc<CellExtra>>) -> Self {
        match table.is_empty() {
            true => Self(None),
            false => Self(Some(Box::new(table))),
        }
    }
}

/// zgx: rows are equal where their cells are, each with the extra it holds
/// (by value: a hyperlink by its id and uri), whatever its handle's number.
impl<T: GridCell + Clone + PartialEq> PartialEq for Row<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner.len() == other.inner.len()
            && self
                .inner
                .iter()
                .zip(other.inner.iter())
                .all(|(cell, theirs)| {
                    let (handle, their_handle) = (cell.extra_handle(), theirs.extra_handle());
                    if handle == 0 && their_handle == 0 {
                        return cell == theirs;
                    }
                    if self.slot(handle).map(|extra| &**extra)
                        != other.slot(their_handle).map(|extra| &**extra)
                    {
                        return false;
                    }
                    let (mut cell, mut theirs) = (cell.clone(), theirs.clone());
                    cell.set_extra_handle(0);
                    theirs.set_extra_handle(0);
                    cell == theirs
                })
    }
}

impl<T: Default> Row<T> {
    /// Create a new terminal row.
    ///
    /// Ideally the `template` should be `Copy` in all performance sensitive scenarios.
    pub fn new(columns: usize) -> Row<T> {
        debug_assert!(columns >= 1);

        let mut inner: Vec<T> = Vec::with_capacity(columns);

        // This is a slightly optimized version of `std::vec::Vec::resize`.
        unsafe {
            let mut ptr = inner.as_mut_ptr();

            for _ in 1..columns {
                ptr::write(ptr, T::default());
                ptr = ptr.offset(1);
            }
            ptr::write(ptr, T::default());

            inner.set_len(columns);
        }

        Row {
            inner: inner.into_boxed_slice(),
            occ: 0,
            extras: Extras::default(),
        }
    }

    /// Increase the number of columns in the row.
    #[inline]
    pub fn grow(&mut self, columns: usize) {
        if self.inner.len() >= columns {
            return;
        }

        let mut inner = mem::take(&mut self.inner).into_vec();
        inner.resize_with(columns, T::default);
        self.inner = inner.into_boxed_slice();
    }

    /// Reduce the number of columns in the row.
    ///
    /// This will return all non-empty cells that were removed.
    pub fn shrink(&mut self, columns: usize) -> Option<Cells<T>>
    where
        T: GridCell,
    {
        if self.inner.len() <= columns {
            return None;
        }

        // Split off cells for a new row.
        let mut inner = mem::take(&mut self.inner).into_vec();
        let mut new_row = inner.split_off(columns);
        self.inner = inner.into_boxed_slice();
        let index = new_row
            .iter()
            .rposition(|c| !self.is_cell_empty(c))
            .map_or(0, |i| i + 1);
        new_row.truncate(index);

        self.occ = min(self.occ, columns);

        if new_row.is_empty() {
            self.keep_bounded();
            None
        } else {
            let cells = Cells::take(new_row, self.extras.as_slice());
            // zgx: the table's bound is the row's, narrower now.
            self.keep_bounded();
            Some(cells)
        }
    }

    /// Reset all cells in the row to the `template` cell.
    ///
    /// zgx: and lets go of its extras: no cell past `occ` holds one.
    #[inline]
    pub fn reset<D>(&mut self, template: &T)
    where
        T: ResetDiscriminant<D> + GridCell,
        D: PartialEq,
    {
        debug_assert!(!self.inner.is_empty());

        // Mark all cells as dirty if template cell changed.
        let len = self.inner.len();
        if self.inner[len - 1].discriminant() != template.discriminant() {
            self.occ = len;
        }

        // Reset every dirty cell in the row.
        T::reset_all(&mut self.inner[0..self.occ], template);

        self.occ = 0;

        if !self.extras.is_empty() {
            self.extras.clear();
        }
    }
}

#[allow(clippy::len_without_is_empty)]
impl<T> Row<T> {
    /// A row of `vec`'s cells, which hold no extras.
    #[inline]
    pub fn from_vec(vec: Vec<T>, occ: usize) -> Row<T> {
        Row {
            inner: vec.into_boxed_slice(),
            occ,
            extras: Extras::default(),
        }
    }

    /// zgx: a row of `cells`, with the extras they hold.
    #[inline]
    pub fn from_cells(cells: Cells<T>, occ: usize) -> Row<T> {
        Row {
            inner: cells.cells.into_boxed_slice(),
            occ,
            extras: Extras::from(cells.extras),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn last(&self) -> Option<&T> {
        self.inner.last()
    }

    #[inline]
    pub fn last_mut(&mut self) -> Option<&mut T> {
        self.occ = self.inner.len();
        self.inner.last_mut()
    }

    /// zgx: `cells` moved to the row's end, their extras into its table.
    #[inline]
    pub fn append(&mut self, cells: &mut Cells<T>)
    where
        T: GridCell,
    {
        let base = self.make_room(cells.extras.len());
        let mut moved = mem::take(&mut cells.cells);
        renumber(&mut moved, base);
        let mut inner = mem::take(&mut self.inner).into_vec();
        inner.append(&mut moved);
        self.inner = inner.into_boxed_slice();
        self.extras.append(&mut cells.extras);
        // zgx: every cell moved in counts as occupied, wherever `occ` was.
        self.occ = self.inner.len();
        self.keep_bounded();
    }

    /// zgx: `cells` moved to the row's start, their extras into its table.
    #[inline]
    pub fn append_front(&mut self, cells: Cells<T>)
    where
        T: GridCell,
    {
        let Cells {
            cells: mut vec,
            extras,
        } = cells;
        self.occ += vec.len();

        let base = self.make_room(extras.len());
        renumber(&mut vec, base);
        self.extras.extend(extras);
        vec.append(&mut mem::take(&mut self.inner).into_vec());
        self.inner = vec.into_boxed_slice();
        self.keep_bounded();
    }

    /// Check if all cells in the row are empty.
    #[inline]
    pub fn is_clear(&self) -> bool
    where
        T: GridCell,
    {
        self.inner.iter().all(|cell| self.is_cell_empty(cell))
    }

    /// zgx: the cells before `at` moved out, with the extras they hold.
    #[inline]
    pub fn front_split_off(&mut self, at: usize) -> Cells<T>
    where
        T: GridCell,
    {
        self.occ = self.occ.saturating_sub(at);

        let mut inner = mem::take(&mut self.inner).into_vec();
        let mut split = inner.split_off(at);
        std::mem::swap(&mut split, &mut inner);
        self.inner = inner.into_boxed_slice();
        let cells = Cells::take(split, self.extras.as_slice());
        // zgx: the table's bound is the row's, narrower now.
        self.keep_bounded();
        cells
    }

    /// zgx: whether `cell` (one of this row's) holds no content: nothing of
    /// its own (`GridCell::is_empty`) and no zero-width characters in the
    /// row's table. A hyperlink or an underline colour alone isn't content.
    #[inline]
    pub fn is_cell_empty(&self, cell: &T) -> bool
    where
        T: GridCell,
    {
        cell.is_empty() && !self.has_zerowidth(cell)
    }

    /// zgx: whether `cell` (one of this row's) has zero-width characters
    /// joined to it.
    #[inline]
    pub(crate) fn has_zerowidth(&self, cell: &T) -> bool
    where
        T: GridCell,
    {
        self.slot(cell.extra_handle())
            .is_some_and(|extra| !extra.zerowidth().is_empty())
    }

    /// zgx: the extra in the slot `handle` names, if any.
    #[inline]
    pub(crate) fn slot(&self, handle: u16) -> Option<&Arc<CellExtra>> {
        let slot = (handle as usize).checked_sub(1)?;
        debug_assert!(slot < self.extras.len(), "handle {handle} past the table");
        self.extras.get(slot)
    }

    /// zgx: the handle for `extra` given the cell at `column`, which lets go
    /// of its own: the last slot where it's that one already (along a run
    /// written with the same pen), otherwise a new one.
    #[inline]
    pub(crate) fn slot_for(&mut self, column: Column, extra: &Arc<CellExtra>) -> u16
    where
        T: GridCell,
    {
        match self.extras.last() {
            Some(last) if Arc::ptr_eq(last, extra) => self.extras.len() as u16,
            _ => self.push_slot(column.0, extra.clone()),
        }
    }

    /// zgx: the most slots the table holds before it's compacted.
    #[inline]
    fn table_limit(&self) -> usize {
        max(self.inner.len(), 8).min(u16::MAX as usize - 1)
    }

    /// zgx: a new slot for `extra`, given the cell at `column`, which lets go
    /// of its own; the table compacted first where it's full. Bounded so: at
    /// most a slot for each other cell, and this one.
    fn push_slot(&mut self, column: usize, extra: Arc<CellExtra>) -> u16
    where
        T: GridCell,
    {
        if self.extras.len() >= self.table_limit() {
            self.compact(Some(column));
        }
        self.extras.push(extra);
        self.extras.len() as u16
    }

    /// zgx: room in the table for `count` slots more, compacted first where
    /// they wouldn't fit; where they start.
    fn make_room(&mut self, count: usize) -> u16
    where
        T: GridCell,
    {
        if count != 0 && self.extras.len() + count > self.table_limit() {
            self.compact(None);
        }
        self.extras.len() as u16
    }

    /// zgx: the table compacted where it's grown past its limit.
    fn keep_bounded(&mut self)
    where
        T: GridCell,
    {
        if self.extras.len() > self.table_limit() {
            self.compact(None);
        }
    }

    /// zgx: the table rebuilt with only the slots its cells hold (but that of
    /// the cell at `except`, which lets go of it), in the order they're first
    /// held, the cells renumbered.
    #[cold]
    #[inline(never)]
    fn compact(&mut self, except: Option<usize>)
    where
        T: GridCell,
    {
        let old = self.extras.take();
        let mut renumbered = vec![0u16; old.len()];
        let occ = min(self.occ, self.inner.len());
        for (at, cell) in self.inner[..occ].iter_mut().enumerate() {
            let handle = cell.extra_handle();
            if handle == 0 {
                continue;
            }
            if except == Some(at) {
                cell.set_extra_handle(0);
                continue;
            }
            let new = &mut renumbered[handle as usize - 1];
            if *new == 0 {
                self.extras.push(old[handle as usize - 1].clone());
                *new = self.extras.len() as u16;
            }
            cell.set_extra_handle(*new);
        }
    }
}

/// zgx: the extra-backed parts of a cell, read and written by its column.
impl Row<Cell> {
    /// The extra the cell at `column` holds: joined characters, an underline
    /// colour, a hyperlink.
    #[inline]
    pub fn extra(&self, column: Column) -> Option<&CellExtra> {
        self.slot(self.inner[column.0].extra).map(|extra| &**extra)
    }

    /// Zerowidth characters joined to the cell at `column`: some (maybe
    /// none) wherever it holds an extra.
    #[inline]
    pub fn zerowidth(&self, column: Column) -> Option<&[char]> {
        self.extra(column).map(CellExtra::zerowidth)
    }

    /// The underline colour of the cell at `column`, if one of its own.
    #[inline]
    pub fn underline_color(&self, column: Column) -> Option<Color> {
        self.extra(column)?.underline_color()
    }

    /// The hyperlink (OSC 8) of the cell at `column`.
    #[inline]
    pub fn hyperlink(&self, column: Column) -> Option<Hyperlink> {
        self.extra(column)?.hyperlink().cloned()
    }

    /// Join `character` to the cell at `column`.
    #[inline]
    pub fn push_zerowidth(&mut self, column: Column, character: char) {
        self.update_extra(column, |extra| CellExtra::push_zerowidth(extra, character));
    }

    /// Remove all wide char data from the cell at `column`.
    #[inline(never)]
    pub fn clear_wide(&mut self, column: Column) {
        let cell = &mut self[column];
        cell.flags.remove(Flags::WIDE_CHAR);
        cell.c = ' ';
        if cell.extra != 0 {
            self.update_extra(column, CellExtra::clear_zerowidth);
        }
    }

    /// Set the underline colour of the cell at `column`.
    #[inline]
    pub fn set_underline_color(&mut self, column: Column, color: Option<Color>) {
        self.update_extra(column, |extra| CellExtra::set_underline_color(extra, color));
    }

    /// Set the hyperlink of the cell at `column`.
    #[inline]
    pub fn set_hyperlink(&mut self, column: Column, hyperlink: Option<Hyperlink>) {
        self.update_extra(column, |extra| CellExtra::set_hyperlink(extra, hyperlink));
    }

    /// Set the extra of the cell at `column` (none: it holds none).
    #[inline]
    pub fn set_extra(&mut self, column: Column, extra: Option<Arc<CellExtra>>) {
        let handle = match &extra {
            Some(extra) => self.slot_for(column, extra),
            None => 0,
        };
        self[column].extra = handle;
    }

    /// The extra of the cell at `column` changed by `update`: a copy where
    /// it's in the table (other cells may hold its slot), in a slot of its
    /// own.
    fn update_extra(&mut self, column: Column, update: impl FnOnce(&mut Option<Arc<CellExtra>>)) {
        let mut extra = self.slot(self.inner[column.0].extra).cloned();
        update(&mut extra);
        self.set_extra(column, extra);
    }
}

/// zgx: `cells` moved in after `base` slots of their new table: their
/// handles from their own table's moved past them.
fn renumber<T: GridCell>(cells: &mut [T], base: u16) {
    if base == 0 {
        return;
    }
    for cell in cells {
        let handle = cell.extra_handle();
        if handle != 0 {
            cell.set_extra_handle(handle + base);
        }
    }
}

/// zgx: cells moved from one row to another (a reflow), with the extras
/// they hold: their handles are into `extras`, a table of their own.
#[derive(Debug)]
pub struct Cells<T> {
    cells: Vec<T>,
    extras: Vec<Arc<CellExtra>>,
}

impl<T> Default for Cells<T> {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            extras: Vec::new(),
        }
    }
}

impl<T: GridCell> Cells<T> {
    /// `cells` of a row whose table is `table`, the extras they hold taken
    /// into their own.
    fn take(mut cells: Vec<T>, table: &[Arc<CellExtra>]) -> Self {
        let mut extras = Vec::new();
        if !table.is_empty() {
            let mut renumbered = vec![0u16; table.len()];
            for cell in &mut cells {
                let handle = cell.extra_handle();
                if handle == 0 {
                    continue;
                }
                let new = &mut renumbered[handle as usize - 1];
                if *new == 0 {
                    extras.push(table[handle as usize - 1].clone());
                    *new = extras.len() as u16;
                }
                cell.set_extra_handle(*new);
            }
        }
        Self { cells, extras }
    }

    /// `cell`, which holds no extra, after the others.
    #[inline]
    pub fn push(&mut self, cell: T) {
        debug_assert_eq!(cell.extra_handle(), 0);
        self.cells.push(cell);
    }

    /// `cell` before the others, holding `extra`.
    pub fn insert_front(&mut self, mut cell: T, extra: Option<Arc<CellExtra>>) {
        let handle = match extra {
            Some(extra) => {
                self.extras.push(extra);
                self.extras.len() as u16
            }
            None => 0,
        };
        cell.set_extra_handle(handle);
        self.cells.insert(0, cell);
    }

    /// The first `len` cells kept.
    #[inline]
    pub fn truncate(&mut self, len: usize) {
        self.cells.truncate(len);
    }

    /// As many as `len` cells, new ones from `cell`.
    #[inline]
    pub fn resize_with(&mut self, len: usize, cell: impl FnMut() -> T) {
        self.cells.resize_with(len, cell);
    }
}

impl<T> Deref for Cells<T> {
    type Target = [T];

    #[inline]
    fn deref(&self) -> &[T] {
        &self.cells
    }
}

impl<T> DerefMut for Cells<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.cells
    }
}

impl<'a, T> IntoIterator for &'a Row<T> {
    type IntoIter = slice::Iter<'a, T>;
    type Item = &'a T;

    #[inline]
    fn into_iter(self) -> slice::Iter<'a, T> {
        self.inner.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Row<T> {
    type IntoIter = slice::IterMut<'a, T>;
    type Item = &'a mut T;

    #[inline]
    fn into_iter(self) -> slice::IterMut<'a, T> {
        self.occ = self.len();
        self.inner.iter_mut()
    }
}

impl<T> Index<Column> for Row<T> {
    type Output = T;

    #[inline]
    fn index(&self, index: Column) -> &T {
        &self.inner[index.0]
    }
}

impl<T> IndexMut<Column> for Row<T> {
    #[inline]
    fn index_mut(&mut self, index: Column) -> &mut T {
        self.occ = max(self.occ, *index + 1);
        &mut self.inner[index.0]
    }
}

impl<T> Index<Range<Column>> for Row<T> {
    type Output = [T];

    #[inline]
    fn index(&self, index: Range<Column>) -> &[T] {
        &self.inner[(index.start.0)..(index.end.0)]
    }
}

impl<T> IndexMut<Range<Column>> for Row<T> {
    #[inline]
    fn index_mut(&mut self, index: Range<Column>) -> &mut [T] {
        self.occ = max(self.occ, *index.end);
        &mut self.inner[(index.start.0)..(index.end.0)]
    }
}

impl<T> Index<RangeTo<Column>> for Row<T> {
    type Output = [T];

    #[inline]
    fn index(&self, index: RangeTo<Column>) -> &[T] {
        &self.inner[..(index.end.0)]
    }
}

impl<T> IndexMut<RangeTo<Column>> for Row<T> {
    #[inline]
    fn index_mut(&mut self, index: RangeTo<Column>) -> &mut [T] {
        self.occ = max(self.occ, *index.end);
        &mut self.inner[..(index.end.0)]
    }
}

impl<T> Index<RangeFrom<Column>> for Row<T> {
    type Output = [T];

    #[inline]
    fn index(&self, index: RangeFrom<Column>) -> &[T] {
        &self.inner[(index.start.0)..]
    }
}

impl<T> IndexMut<RangeFrom<Column>> for Row<T> {
    #[inline]
    fn index_mut(&mut self, index: RangeFrom<Column>) -> &mut [T] {
        self.occ = self.len();
        &mut self.inner[(index.start.0)..]
    }
}

impl<T> Index<RangeFull> for Row<T> {
    type Output = [T];

    #[inline]
    fn index(&self, _: RangeFull) -> &[T] {
        &self.inner[..]
    }
}

impl<T> IndexMut<RangeFull> for Row<T> {
    #[inline]
    fn index_mut(&mut self, _: RangeFull) -> &mut [T] {
        self.occ = self.len();
        &mut self.inner[..]
    }
}

impl<T> Index<RangeToInclusive<Column>> for Row<T> {
    type Output = [T];

    #[inline]
    fn index(&self, index: RangeToInclusive<Column>) -> &[T] {
        &self.inner[..=(index.end.0)]
    }
}

impl<T> IndexMut<RangeToInclusive<Column>> for Row<T> {
    #[inline]
    fn index_mut(&mut self, index: RangeToInclusive<Column>) -> &mut [T] {
        self.occ = max(self.occ, *index.end + 1);
        &mut self.inner[..=(index.end.0)]
    }
}
