//! The glyph atlas's cells (`GpuRenderer::atlas_cell`): rows of cells
//! ("shelves") one height class tall, filled left to right, whose cells
//! are given back one at a time (`free`) and taken again by cells that
//! fit. A row emptied takes cells of any class up to its height; rows
//! emptied at the bottom are given back whole. So an atlas whose text
//! changes keeps the cells of the text it still holds where they are.

/// Cells of one atlas, `size` texels square. Sizes and positions are of
/// whole cells, gutters included.
pub(crate) struct Shelves {
    size: u32,
    shelves: Vec<Shelf>,
    /// Where the next new shelf starts: below every shelf.
    top: u32,
    /// Texels of the cells taken.
    area: u64,
    /// The rows of the shelves holding cells (`rows`), kept as they change:
    /// asked every frame (`GpuRenderer::atlas_filling`).
    rows: u32,
}

struct Shelf {
    y: u32,
    height: u32,
    /// Where the shelf's cells end: past them it is free to the edge.
    end: u32,
    /// Free spans `(x, width)` before `end`, in order.
    free: Vec<(u32, u32)>,
    /// Its cells taken.
    cells: u32,
}

impl Shelf {
    /// Where a cell `width` wide fits: the first free span it fits, else
    /// after the shelf's cells.
    fn fit(&self, width: u32, size: u32) -> Option<u32> {
        self.free
            .iter()
            .find(|(_, w)| *w >= width)
            .map(|(x, _)| *x)
            .or((self.end + width <= size).then_some(self.end))
    }
    fn take(&mut self, x: u32, width: u32) {
        self.cells += 1;
        if x == self.end {
            self.end += width;
            return;
        }
        let at = self
            .free
            .iter()
            .position(|(from, _)| *from == x)
            .expect("a free span");
        let (from, w) = self.free[at];
        if w > width {
            self.free[at] = (from + width, w - width);
        } else {
            self.free.remove(at);
        }
    }
    fn give(&mut self, x: u32, width: u32) {
        self.cells -= 1;
        if self.cells == 0 {
            self.free.clear();
            self.end = 0;
            return;
        }
        let at = self.free.partition_point(|(from, _)| *from < x);
        self.free.insert(at, (x, width));
        // Joined to the spans either side.
        if at + 1 < self.free.len() && self.free[at].0 + self.free[at].1 == self.free[at + 1].0 {
            self.free[at].1 += self.free[at + 1].1;
            self.free.remove(at + 1);
        }
        if at > 0 && self.free[at - 1].0 + self.free[at - 1].1 == self.free[at].0 {
            self.free[at - 1].1 += self.free[at].1;
            self.free.remove(at);
        }
        if let Some(&(from, w)) = self.free.last()
            && from + w == self.end
        {
            self.end = from;
            self.free.pop();
        }
    }
}

/// The height class of a cell `height` tall: shelves of a class hold
/// cells a little shorter too, wasting little.
fn class(height: u32) -> u32 {
    match height {
        0..=16 => height.next_multiple_of(2),
        17..=64 => height.next_multiple_of(4),
        _ => height.next_multiple_of(8),
    }
}

impl Shelves {
    pub(crate) fn new(size: u32) -> Self {
        Self {
            size,
            shelves: Vec::new(),
            top: 0,
            area: 0,
            rows: 0,
        }
    }
    /// The atlas grown to `size` square: its shelves where they lie, now as
    /// wide as it, with the rows below them free.
    pub(crate) fn grow(&mut self, size: u32) {
        debug_assert!(size >= self.size);
        self.size = size;
    }
    /// Texels of the cells taken.
    pub(crate) fn area(&self) -> u64 {
        self.area
    }
    /// The rows its shelves holding cells take (an emptied shelf's not).
    pub(crate) fn rows(&self) -> u32 {
        self.rows
    }
    /// A cell `width` × `height`: its origin, or `None` where none fits.
    pub(crate) fn alloc(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width > self.size || height > self.size {
            return None;
        }
        let class = class(height).min(self.size);
        // Height classes first: a shelf of its class or a little taller in
        // use, then a new one, then the shortest emptied shelf that holds
        // it. Only once rows run out, the room left: the last shelf grown to
        // the cell, as a running row grows to its tallest (the newest
        // shelf, given back with the newest cells), then the shortest taller
        // shelf in use that has it (a first frame of many
        // sizes then fits as one filled a row at a time did, rather than
        // running out). Classes keep a colour animating through font
        // smoothing's levels from spending rows under its short cells.
        let near = |s: &Shelf| s.height >= class && s.height <= class + class / 4 + 2;
        let mut found = self
            .shelves
            .iter()
            .enumerate()
            .filter(|(_, s)| s.cells > 0 && near(s))
            .find_map(|(i, s)| s.fit(width, self.size).map(|x| (i, x)));
        if found.is_none() && self.top + class <= self.size {
            self.shelves.push(Shelf {
                y: self.top,
                height: class,
                end: 0,
                free: Vec::new(),
                cells: 0,
            });
            self.top += class;
            found = Some((self.shelves.len() - 1, 0));
        }
        if found.is_none() {
            found = self
                .shelves
                .iter()
                .enumerate()
                .filter(|(_, s)| s.cells == 0 && s.height >= class)
                .min_by_key(|(_, s)| s.height)
                .map(|(i, _)| (i, 0));
        }
        let count = self.shelves.len();
        if found.is_none()
            && let Some(last) = self.shelves.last_mut()
            && last.cells > 0
            && last.end + width <= self.size
            && self.top + class.saturating_sub(last.height) <= self.size
        {
            let taller = class.saturating_sub(last.height);
            last.height += taller;
            self.top += taller;
            self.rows += taller;
            found = Some((count - 1, last.end));
        }
        if found.is_none() {
            found = self
                .shelves
                .iter()
                .enumerate()
                .filter(|(_, s)| s.cells > 0 && s.height >= class)
                .filter_map(|(i, s)| s.fit(width, self.size).map(|x| (i, x, s.height)))
                .min_by_key(|(_, _, height)| *height)
                .map(|(i, x, _)| (i, x));
        }
        let (i, x) = found?;
        // An emptied shelf taller than the class: the rest of it a shelf
        // of its own.
        let shelf = &mut self.shelves[i];
        if shelf.cells == 0 && shelf.height > class {
            let rest = Shelf {
                y: shelf.y + class,
                height: shelf.height - class,
                end: 0,
                free: Vec::new(),
                cells: 0,
            };
            shelf.height = class;
            self.shelves.insert(i + 1, rest);
        }
        let shelf = &mut self.shelves[i];
        if shelf.cells == 0 {
            self.rows += shelf.height;
        }
        shelf.take(x, width);
        self.area += u64::from(width) * u64::from(height);
        Some((x, shelf.y))
    }
    /// Give back the cell `width` × `height` at `(x, y)` that `alloc` gave.
    pub(crate) fn free(&mut self, (x, y): (u32, u32), width: u32, height: u32) {
        let at = self.shelves.partition_point(|s| s.y < y);
        let shelf = &mut self.shelves[at];
        debug_assert_eq!(shelf.y, y, "a shelf's cell");
        shelf.give(x, width);
        self.area -= u64::from(width) * u64::from(height);
        // Emptied, it joins the emptied shelves either side: one taller.
        if self.shelves[at].cells == 0 {
            self.rows -= self.shelves[at].height;
            let mut at = at;
            if at > 0 && self.shelves[at - 1].cells == 0 {
                at -= 1;
                self.shelves[at].height += self.shelves.remove(at + 1).height;
            }
            if at + 1 < self.shelves.len() && self.shelves[at + 1].cells == 0 {
                self.shelves[at].height += self.shelves.remove(at + 1).height;
            }
        }
        // Empty shelves at the bottom given back whole.
        while self.shelves.last().is_some_and(|s| s.cells == 0) {
            let shelf = self.shelves.pop().expect("a shelf");
            self.top = shelf.y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No two cells overlap, and each lies within the atlas.
    fn disjoint(cells: &[(u32, u32, u32, u32)], size: u32) {
        for (i, a) in cells.iter().enumerate() {
            assert!(a.0 + a.2 <= size && a.1 + a.3 <= size, "{a:?} within");
            for b in &cells[i + 1..] {
                let apart =
                    a.0 + a.2 <= b.0 || b.0 + b.2 <= a.0 || a.1 + a.3 <= b.1 || b.1 + b.3 <= a.1;
                assert!(apart, "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn cells_freed_are_taken_again_and_never_overlap() {
        let mut shelves = Shelves::new(256);
        let mut cells = Vec::new();
        // A mix of glyph-like cells until it is full.
        let mut n = 0_u32;
        while let Some((x, y)) = shelves.alloc(6 + n * 7 % 23, 10 + n * 5 % 31) {
            cells.push((x, y, 6 + n * 7 % 23, 10 + n * 5 % 31));
            n += 1;
        }
        disjoint(&cells, 256);
        let full = shelves.area();
        // Every other cell given back: as many cells of the same sizes fit
        // again, where those were.
        let (kept, freed): (Vec<_>, Vec<_>) =
            cells.iter().enumerate().partition(|(i, _)| i % 2 == 0);
        for (_, c) in &freed {
            shelves.free((c.0, c.1), c.2, c.3);
        }
        let mut cells: Vec<_> = kept.into_iter().map(|(_, c)| *c).collect();
        for (_, c) in &freed {
            let (x, y) = shelves.alloc(c.2, c.3).expect("a freed cell's room");
            cells.push((x, y, c.2, c.3));
        }
        disjoint(&cells, 256);
        assert_eq!(shelves.area(), full);
    }

    #[test]
    fn emptied_shelves_join_and_hold_other_heights() {
        let mut shelves = Shelves::new(64);
        let small: Vec<_> = (0..8).map(|_| shelves.alloc(64, 8).unwrap()).collect();
        assert_eq!(shelves.alloc(64, 8), None, "full");
        assert_eq!(shelves.rows(), 64);
        // The middle four emptied: one shelf 32 rows tall, which a taller
        // cell takes the top of, and a shorter one the rest.
        for &cell in &small[2..6] {
            shelves.free(cell, 64, 8);
        }
        assert_eq!(shelves.rows(), 32);
        assert_eq!(shelves.alloc(64, 16), Some((0, 16)));
        assert_eq!(shelves.alloc(32, 6), Some((0, 32)));
        assert_eq!(shelves.rows(), 32 + 16 + 6);
        // The bottom shelves emptied join what is left above them, all of
        // it given back.
        for &cell in &small[6..] {
            shelves.free(cell, 64, 8);
        }
        assert_eq!(shelves.alloc(64, 24), Some((0, 38)));
        assert_eq!(shelves.area(), 64 * 8 * 2 + 64 * 16 + 32 * 6 + 64 * 24);
    }

    #[test]
    fn once_rows_run_out_cells_take_the_room_left() {
        let mut shelves = Shelves::new(64);
        // Classes first: each height a shelf of its own.
        assert_eq!(shelves.alloc(10, 40), Some((0, 0)));
        assert_eq!(shelves.alloc(10, 20), Some((0, 40)));
        assert_eq!(shelves.rows(), 60);
        // No rows for a shelf of its class: the shortest taller one with
        // room.
        assert_eq!(shelves.alloc(10, 8), Some((10, 40)));
        assert_eq!(shelves.rows(), 60);
    }

    #[test]
    fn with_no_room_left_the_last_shelf_grows_to_the_cell() {
        let mut shelves = Shelves::new(64);
        assert_eq!(shelves.alloc(64, 40), Some((0, 0)));
        assert_eq!(shelves.alloc(10, 20), Some((0, 40)));
        // No class shelf, no rows, no taller shelf with room: the last
        // shelf, 20 rows, grows to 24 for it.
        assert_eq!(shelves.alloc(10, 22), Some((10, 40)));
        assert_eq!(shelves.rows(), 64);
    }

    #[test]
    fn once_rows_run_out_an_emptied_taller_shelf_is_taken_again() {
        let mut shelves = Shelves::new(64);
        assert_eq!(shelves.alloc(20, 8), Some((0, 0)));
        let tall = shelves.alloc(20, 24).unwrap();
        assert_eq!(tall, (0, 8));
        assert_eq!(shelves.alloc(64, 8), Some((0, 32)));
        assert_eq!(shelves.alloc(64, 8), Some((0, 40)));
        // Emptied, it holds a cell that tall again, no rows being left.
        shelves.free(tall, 20, 24);
        assert_eq!(shelves.alloc(64, 24), Some((0, 8)));
        assert_eq!(shelves.area(), 20 * 8 + 64 * 8 * 2 + 64 * 24);
    }

    #[test]
    fn spans_freed_side_by_side_join() {
        let mut shelves = Shelves::new(64);
        let cells: Vec<_> = (0..4).map(|_| shelves.alloc(16, 8).unwrap()).collect();
        shelves.alloc(64, 8).unwrap();
        shelves.free(cells[1], 16, 8);
        shelves.free(cells[2], 16, 8);
        // The two spans joined hold a cell as wide as both.
        assert_eq!(shelves.alloc(32, 8), Some((16, 0)));
    }
}
