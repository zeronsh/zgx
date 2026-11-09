//! Work a window does with the time its frames leave over: what a browser
//! does off the critical path. A virtual list builds the rows about to come
//! into view here, one at a time, and keeps those it has built, so a scroll
//! frame finds its rows there already.
//!
//! A host says it has spare time (`Ui::host_spare_time`) and spends it as a
//! refresh begins, before the frame's own work (`Ui::run_spare_work`), asking
//! for frames while `Ui::spare_work_wanted` holds; once everything is built,
//! nothing is left to wake it. Not after the frame: a window holds its frame
//! to hand it over late in the refresh, and what follows has no time left,
//! where what comes before it spends only that wait. A host that doesn't (a headless one) has none of this: its
//! lists build the rows in view and their overscan, and drop the rest.
use crate::{reactive::Signal, scene::NodeId};
use std::{
    cell::Cell,
    collections::HashMap,
    hash::Hash,
    ops::Range,
    rc::Rc,
    time::{Duration, Instant},
};

/// Viewports of rows built beyond each edge of the view, unless a list says.
pub(crate) const AHEAD_VIEWPORTS: f32 = 1.;
/// Viewports beyond the rows built ahead that rows already built stay for.
pub(crate) const KEPT_VIEWPORTS: f32 = 4.;
/// The most rows a list keeps, unless the rows it builds ahead are more.
pub(crate) const KEPT_ROWS: usize = 256;
/// The most scene nodes (counted as they are kept) a list's rows keep beyond
/// those it builds ahead: tens of megabytes at most, with what they hold.
pub(crate) const KEPT_NODES: usize = 8_000;
/// How long a window's frames stay stopped before its lists let go of the
/// rows they keep beyond those built ahead (`Ui::release_kept_rows`).
pub const KEPT_REST: Duration = Duration::from_secs(30);
/// Spare frames a row that doesn't fit waits before it is built anyway.
const MOST_WAITED: u32 = 4;

/// A list's share of its window's spare time: shared by its pass and
/// the window's queue.
pub(crate) struct ListAhead {
    /// Woken to run a pass that may build a row.
    tick: Signal<u64>,
    may_build: Cell<bool>,
    wants: Cell<bool>,
    queued: Cell<bool>,
    passes: Cell<u64>,
    /// What a row has cost, seconds: built and laid out.
    cost: Cell<f32>,
    /// Spare frames its next row has not fit in.
    waited: Cell<u32>,
    resting: Cell<bool>,
    viewports: f32,
}
/// A list's rows by what it does with them, around the view.
pub(crate) struct Rings {
    /// Rows to build.
    pub(crate) build: Range<usize>,
    /// Rows built that stay.
    pub(crate) keep: Range<usize>,
}
impl ListAhead {
    pub(crate) fn new(tick: Signal<u64>, viewports: f32) -> Self {
        Self {
            tick,
            may_build: Cell::new(false),
            wants: Cell::new(false),
            queued: Cell::new(false),
            passes: Cell::new(0),
            cost: Cell::new(0.),
            waited: Cell::new(0),
            // Until its view first moves: a list that never scrolls builds
            // what it always built.
            resting: Cell::new(true),
            viewports,
        }
    }
    /// A pass of the list, subscribing it to its share of spare time.
    pub(crate) fn pass(&self) -> u64 {
        self.passes.set(self.passes.get().wrapping_add(1));
        self.tick.get()
    }
    /// Whether this pass may build a row: once for each that was asked.
    pub(crate) fn take_may_build(&self) -> bool {
        self.may_build.take()
    }
    /// The view moved: rows are kept again.
    pub(crate) fn moved(&self) {
        self.resting.set(false);
    }
    /// The rows around a view from `top`, `height` tall, of `count`, as far
    /// as `row_at` says, and `overscan` rows at least; `shown`, those in
    /// view, are built whatever this says.
    pub(crate) fn rings(
        &self,
        top: f32,
        height: f32,
        count: usize,
        overscan: usize,
        shown: &Range<usize>,
        row_at: &dyn Fn(f32) -> Option<usize>,
    ) -> Rings {
        let least =
            shown.start.saturating_sub(overscan)..shown.end.saturating_add(overscan).min(count);
        let around = |distance: f32| {
            if height <= 0. || count == 0 {
                return least.clone();
            }
            let start = row_at(top - distance).unwrap_or(0).min(least.start);
            let end = row_at(top + height + distance)
                .map_or(count, |row| row + 1)
                .max(least.end);
            start
                .max(shown.start.saturating_sub(KEPT_ROWS))
                .min(shown.start)
                ..end.min(shown.end.saturating_add(KEPT_ROWS)).max(shown.end)
        };
        // At rest, what shows and its overscan, as a list with no spare time.
        if self.resting.get() {
            return Rings {
                build: least.clone(),
                keep: least,
            };
        }
        let ahead = height * self.viewports;
        Rings {
            build: around(ahead),
            keep: around(ahead + height * KEPT_VIEWPORTS),
        }
    }
}
impl Rings {
    /// The row to build next of those not in `built` (sorted): the nearest
    /// the way the view last went, then the nearest the other way. At most
    /// `KEPT_ROWS` a side are looked at, over rows `can` says can't be.
    pub(crate) fn next(
        &self,
        shown: &Range<usize>,
        down: bool,
        built: &[usize],
        can: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let free = |row: &usize| built.binary_search(row).is_err() && can(*row);
        let below = || (shown.end..self.build.end).take(KEPT_ROWS).find(free);
        let above = || {
            (self.build.start..shown.start)
                .rev()
                .take(KEPT_ROWS)
                .find(free)
        };
        if down {
            below().or_else(above)
        } else {
            above().or_else(below)
        }
    }
}

impl Rings {
    /// How far from `shown` rows go, those at least this far, for the rest
    /// of `rows` (each's index and nodes) to fit what a list keeps; none of
    /// those shown or to build go.
    pub(crate) fn cut(
        &self,
        shown: &Range<usize>,
        rows: impl Iterator<Item = (usize, usize)> + Clone,
    ) -> Option<usize> {
        let (mut count, mut nodes) = (0, 0);
        for (_, row_nodes) in rows.clone() {
            count += 1;
            nodes += row_nodes;
        }
        let most = KEPT_ROWS.max(self.build.len().saturating_add(shown.len()));
        if count <= most && nodes <= KEPT_NODES {
            return None;
        }
        let mut far: Vec<(usize, usize)> = rows
            .filter(|(index, _)| !self.build.contains(index) && !shown.contains(index))
            .map(|(index, nodes)| (distance(shown, index), nodes))
            .collect();
        far.sort_unstable_by_key(|(distance, _)| std::cmp::Reverse(*distance));
        let mut cut = None;
        for (distance, row_nodes) in far {
            if count <= most && nodes <= KEPT_NODES {
                break;
            }
            cut = Some(distance);
            count -= 1;
            nodes = nodes.saturating_sub(row_nodes);
        }
        cut
    }
    /// Whether row `index` goes, by `cut`.
    pub(crate) fn beyond(&self, shown: &Range<usize>, index: usize, cut: usize) -> bool {
        !self.build.contains(&index) && !shown.contains(&index) && distance(shown, index) >= cut
    }
}
fn distance(shown: &Range<usize>, index: usize) -> usize {
    index
        .saturating_sub(shown.end)
        .max(shown.start.saturating_sub(index))
}

/// A row of a list that keeps rows, as [`Kept`] sees it.
pub(crate) trait KeptRow {
    fn node(&self) -> NodeId;
    fn place(&self) -> &Signal<usize>;
    /// The run that last showed or kept it.
    fn run(&self) -> u64;
    fn set_run(&mut self, run: u64);
}
type Rows<K, R> = HashMap<K, R, rustc_hash::FxBuildHasher>;

/// What a list does with spare time, apart from the list's own pass: all of
/// it behind one `Option` there and out of line here, so that a list with
/// none (a headless host's) runs what it always ran.
pub(crate) struct Kept {
    ahead: Rc<ListAhead>,
    overscan: usize,
    /// Spare time's turn the last pass saw.
    turn: u64,
    /// The rows' width and what says rows may have moved, last pass.
    width: u32,
    structure: u64,
    /// A fixed list's inputs, last pass (`woken`).
    inputs: Option<(usize, u32, u32, u32, u64)>,
    top: Option<f32>,
    /// The way the view last moved: up.
    up: bool,
    /// The heights rows were last placed by.
    pub(crate) placed: u64,
    /// This pass's rows in view and rings.
    shown: Range<usize>,
    rings: Rings,
    /// Rows built, sorted: kept to reuse its allocation.
    built: Vec<usize>,
    /// What each row mounted with, counted once it is kept.
    nodes: crate::scene::NodeMap<usize>,
}
/// What [`Kept::keep`] did that a list's pass goes on to do something about.
#[derive(Default)]
pub(crate) struct KeptPass {
    /// A row's index changed.
    pub(crate) reindexed: bool,
    /// A row was built.
    pub(crate) built: bool,
}
impl Kept {
    /// A list's, if `ahead` says it has spare time.
    pub(crate) fn new(ahead: Option<Rc<ListAhead>>, overscan: usize) -> Option<Box<Self>> {
        Some(Box::new(Self {
            ahead: ahead?,
            overscan,
            turn: 0,
            width: 0,
            structure: 0,
            inputs: None,
            top: None,
            up: false,
            placed: 0,
            shown: 0..0,
            rings: Rings {
                build: 0..0,
                keep: 0..0,
            },
            built: Vec::new(),
            nodes: Default::default(),
        }))
    }
    /// A pass begins, subscribed to spare time: whether it is spare time's
    /// turn (it has rows to build, or to let go).
    pub(crate) fn pass(&mut self) -> bool {
        let turn = self.ahead.pass();
        std::mem::replace(&mut self.turn, turn) != turn
    }
    /// Whether a fixed list's pass was woken by none of its `inputs` (by
    /// what its `key` reads).
    pub(crate) fn woken(&mut self, inputs: (usize, u32, u32, u32)) -> bool {
        let inputs = (inputs.0, inputs.1, inputs.2, inputs.3, self.turn);
        self.inputs.replace(inputs) == Some(inputs)
    }
    /// The rows in view are `shown` of `count`, in a view from `top`,
    /// `height` tall. `settled`: a move now is rows' heights settling around
    /// what shows (or a reply growing under a list that follows it), no one
    /// scrolling.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn around(
        &mut self,
        top: f32,
        height: f32,
        count: usize,
        shown: Range<usize>,
        settled: bool,
        row_at: &dyn Fn(f32) -> Option<usize>,
    ) {
        if let Some(was) = self.top.replace(top)
            && was != top
            && !settled
        {
            self.up = top < was;
            self.ahead.moved();
        }
        self.rings = self
            .ahead
            .rings(top, height, count, self.overscan, &shown, &row_at);
        self.shown = shown;
    }
    /// Stamp the rows that stay with `run`, those shown being stamped: the
    /// rest go. Finding them by `key` where rows may have moved (`structure`
    /// changed, or `woken`). At spare time's turn, build the next.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    pub(crate) fn keep<K: Eq + Hash + Clone, R: KeptRow>(
        &mut self,
        ui: &mut crate::widgets::Ui,
        entries: &mut Rows<K, R>,
        run: u64,
        width: f32,
        structure: u64,
        woken: bool,
        focus_goes: bool,
        key: &mut dyn FnMut(usize) -> K,
        can: &dyn Fn(usize) -> bool,
        mount: &mut dyn FnMut(&mut crate::widgets::Ui, &K, usize) -> R,
    ) -> KeptPass {
        let mut pass = KeptPass::default();
        let index_of = |row: &R| row.place().with_untracked(|index| *index);
        // A new width measures every row again: those out of view are built
        // again as they are wanted.
        let same_width = std::mem::replace(&mut self.width, width.to_bits()) == width.to_bits();
        let rekey = std::mem::replace(&mut self.structure, structure) != structure || woken;
        if same_width && rekey {
            for index in self.rings.keep.clone() {
                if self.shown.contains(&index) || !can(index) {
                    continue;
                }
                let row_key = key(index);
                if let Some(row) = entries.get_mut(&row_key)
                    && row.run() != run
                {
                    row.set_run(run);
                    if row.place().with_untracked(|was| *was != index) {
                        row.place().set(index);
                        pass.reindexed = true;
                    }
                }
            }
        } else if same_width {
            // (One whose place has come into view and wasn't asked for
            // there is no longer that row.)
            for row in entries.values_mut() {
                let index = index_of(row);
                if row.run() != run
                    && self.rings.keep.contains(&index)
                    && !self.shown.contains(&index)
                    && can(index)
                {
                    row.set_run(run);
                }
            }
        }
        // Holding the keyboard's focus or what's open from it, a row goes
        // as it leaves, as ever.
        let focused = if focus_goes { ui.input.focused() } else { None };
        let scope = ui.input.focus_scope();
        if focused.is_some() || scope.is_some() {
            for row in entries.values_mut() {
                let index = index_of(row);
                if row.run() != run || self.shown.contains(&index) {
                    continue;
                }
                let node = row.node();
                let holds = focused.is_some_and(|focused| ui.opened_from(focused, node));
                if holds || scope.is_some_and(|scope| ui.opened_from(scope, node)) {
                    row.set_run(run.wrapping_sub(1));
                }
            }
        }
        // Past what a list keeps, the farthest go.
        {
            let scene = ui.scene.borrow();
            for row in entries.values() {
                if row.run() == run && !self.nodes.contains_key(&row.node()) {
                    self.nodes
                        .insert(row.node(), nodes_under(&scene, row.node()));
                }
            }
            if self.nodes.len() > entries.len() {
                self.nodes.retain(|node, _| scene.contains(*node));
            }
        }
        let rows = entries.values().filter(|row| row.run() == run).map(|row| {
            (
                index_of(row),
                self.nodes.get(&row.node()).copied().unwrap_or(1),
            )
        });
        if let Some(cut) = self.rings.cut(&self.shown, rows) {
            for row in entries.values_mut() {
                if row.run() == run && self.rings.beyond(&self.shown, index_of(row), cut) {
                    row.set_run(run.wrapping_sub(1));
                }
            }
        }
        // Spare time's row, if this is its turn.
        self.built.clear();
        self.built.extend(
            entries
                .values()
                .filter(|row| row.run() == run)
                .map(index_of),
        );
        self.built.sort_unstable();
        let mut next = self.rings.next(&self.shown, !self.up, &self.built, can);
        if self.ahead.take_may_build()
            && let Some(index) = next
        {
            let row_key = key(index);
            match entries.get_mut(&row_key) {
                // Rows moved where nothing said so: it is this row now, and
                // the next pass that moves looks again.
                Some(row) => {
                    row.set_run(run);
                    row.place().set(index);
                    pass.reindexed = true;
                    next = None;
                }
                None => {
                    let row = mount(ui, &row_key, index);
                    entries.insert(row_key, row);
                    pass.built = true;
                    let at = self.built.partition_point(|row| *row < index);
                    self.built.insert(at, index);
                    next = self.rings.next(&self.shown, !self.up, &self.built, can);
                }
            }
        }
        ui.list_wants(&self.ahead, next.is_some());
        pass
    }
    /// `order`, every row in the order it reads in: Tab and the accessible
    /// tree go through rows out of view as through the list.
    #[inline(never)]
    pub(crate) fn order<K, R: KeptRow>(entries: &Rows<K, R>, order: &mut Vec<NodeId>) {
        let mut rows: Vec<(usize, NodeId)> = entries
            .values()
            .map(|row| (row.place().with_untracked(|index| *index), row.node()))
            .collect();
        rows.sort_unstable_by_key(|(index, _)| *index);
        order.clear();
        order.extend(rows.into_iter().map(|(_, node)| node));
    }
}
fn nodes_under(scene: &crate::scene::Scene, row: NodeId) -> usize {
    let mut stack = vec![row];
    let mut count = 0;
    while let Some(node) = stack.pop() {
        count += 1;
        stack.extend_from_slice(scene.children(node));
    }
    count
}

/// What a window keeps of its lists' spare work.
#[derive(Default)]
pub(crate) struct SpareWork {
    /// Its host spends spare time (`Ui::host_spare_time`).
    pub(crate) hosted: Cell<bool>,
    /// Lists with rows to build.
    pub(crate) queue: std::cell::RefCell<Vec<Rc<ListAhead>>>,
    /// Every list that keeps rows.
    pub(crate) lists: std::cell::RefCell<Vec<std::rc::Weak<ListAhead>>>,
}
impl crate::widgets::Ui {
    /// This window's host calls [`run_spare_work`](Self::run_spare_work)
    /// as a refresh begins, and asks for frames while
    /// [`spare_work_wanted`](Self::spare_work_wanted) holds. Say so before
    /// the content mounts: lists mounted before it stay as a headless
    /// host's do.
    pub fn host_spare_time(&self) {
        self.spare().hosted.set(true);
    }
    /// Whether anything waits for spare time, to ask for a frame.
    pub fn spare_work_wanted(&self) -> bool {
        !self.spare().queue.borrow().is_empty()
    }
    /// Spend time in a refresh that began at `started`, on a display
    /// refreshing each `interval`, up to half a refresh from its start: the
    /// frame's own work comes after. Returns the time left of that for the
    /// host's own spare work.
    ///
    /// A row is built whole, so one that can't fit waits for a frame it
    /// does (a guess, from what rows have cost). So that a row longer than
    /// the budget still gets its turn, one that has waited a few frames is
    /// built in the next that began only just now.
    pub fn run_spare_work(
        &self,
        started: Instant,
        interval: Duration,
    ) -> Result<Duration, crate::widgets::LayoutFeedbackError> {
        let deadline = started + interval / 2;
        let lists: Vec<Rc<ListAhead>> = self.spare().queue.borrow().clone();
        crate::profile::spare_time(|| self.spend(&lists, started, interval, deadline))?;
        Ok(deadline.saturating_duration_since(Instant::now()))
    }
    fn spend(
        &self,
        lists: &[Rc<ListAhead>],
        started: Instant,
        interval: Duration,
        deadline: Instant,
    ) -> Result<(), crate::widgets::LayoutFeedbackError> {
        for list in lists {
            while list.wants.get() {
                let now = Instant::now();
                let cost = Duration::from_secs_f32(list.cost.get());
                let waited = list.waited.get() >= MOST_WAITED && now < started + interval / 4;
                if now + cost > deadline && !waited {
                    list.waited.set(list.waited.get() + 1);
                    return Ok(());
                }
                list.waited.set(0);
                let passes = list.passes.get();
                list.may_build.set(true);
                list.tick.update(|tick| *tick = tick.wrapping_add(1));
                self.try_prepare_frame()?;
                // Its list has gone with its window's scene.
                if list.passes.get() == passes {
                    list.may_build.set(false);
                    list.wants.set(false);
                }
                let took = now.elapsed().as_secs_f32();
                // What a row costs now, a long one remembered a while.
                list.cost.set(took.max((list.cost.get() + took) / 2.));
            }
            list.queued.set(false);
            self.spare()
                .queue
                .borrow_mut()
                .retain(|other| !Rc::ptr_eq(other, list));
        }
        Ok(())
    }
    /// Lists go back to what shows and its overscan, until their views next
    /// move, and what they let go of is freed now: no frame need come, a
    /// covered window's as a shown one's. Hosts call this once its frames
    /// have stopped a while ([`KEPT_REST`]).
    pub fn release_kept_rows(&self) -> Result<(), crate::widgets::LayoutFeedbackError> {
        let lists: Vec<Rc<ListAhead>> = {
            let mut lists = self.spare().lists.borrow_mut();
            lists.retain(|list| list.strong_count() > 0);
            lists.iter().filter_map(std::rc::Weak::upgrade).collect()
        };
        for list in lists {
            list.resting.set(true);
            list.tick.update(|tick| *tick = tick.wrapping_add(1));
        }
        self.try_prepare_frame()?;
        while self.reclaim(Duration::from_millis(4)) {}
        Ok(())
    }
    /// A list's share of spare time, if its host has any and `viewports`
    /// (`View::list_ahead`) doesn't turn it off.
    pub(crate) fn list_ahead(&self, viewports: Option<f32>) -> Option<Rc<ListAhead>> {
        let spare = self.spare();
        if !spare.hosted.get() {
            return None;
        }
        let viewports = viewports.unwrap_or(AHEAD_VIEWPORTS);
        if !viewports.is_finite() || viewports <= 0. {
            return None;
        }
        let list = Rc::new(ListAhead::new(self.signal(0), viewports));
        spare.lists.borrow_mut().push(Rc::downgrade(&list));
        Some(list)
    }
    /// Whether `list` has rows to build: it joins the queue, or leaves it
    /// as `run_spare_work` next looks.
    pub(crate) fn list_wants(&self, list: &Rc<ListAhead>, wants: bool) {
        list.wants.set(wants);
        if wants && !list.queued.replace(true) {
            self.spare().queue.borrow_mut().push(list.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::Runtime;

    fn rings(build: Range<usize>) -> Rings {
        Rings {
            keep: build.clone(),
            build,
        }
    }

    #[test]
    fn the_next_row_is_the_nearest_the_way_the_view_went() {
        let rings = rings(5..15);
        let shown = 8..11;
        assert_eq!(rings.next(&shown, true, &[], |_| true), Some(11));
        assert_eq!(rings.next(&shown, false, &[], |_| true), Some(7));
        assert_eq!(
            rings.next(&shown, true, &[11, 12, 13, 14], |_| true),
            Some(7)
        );
        assert_eq!(rings.next(&shown, true, &[], |row| row != 11), Some(12));
        let all: Vec<usize> = (5..15).collect();
        assert_eq!(rings.next(&shown, true, &all, |_| true), None);
    }

    #[test]
    fn rings_go_by_distance_and_stop_at_a_cap_of_rows() {
        let runtime = Runtime::new();
        let list = ListAhead::new(runtime.signal(0), 1.);
        list.moved();
        // Rows of ten points: a view of a hundred is ten rows.
        let row_at = |y: f32| Some((y.max(0.) / 10.) as usize);
        let around = list.rings(1000., 100., 1000, 0, &(100..110), &row_at);
        assert_eq!(around.build, 90..121);
        assert_eq!(around.keep, 50..161);
        // At rest, what shows and its overscan.
        list.resting.set(true);
        let around = list.rings(1000., 100., 1000, 2, &(100..110), &row_at);
        assert_eq!((around.build, around.keep), (98..112, 98..112));
        // Rows of a point: the cap, not the distance.
        let row_at = |y: f32| Some(y.max(0.) as usize);
        list.resting.set(false);
        let around = list.rings(5000., 1000., 100_000, 0, &(5000..6000), &row_at);
        assert_eq!(around.keep, 5000 - KEPT_ROWS..6000 + KEPT_ROWS);
        // A list's overscan goes further than the distance, over tall rows.
        let row_at = |y: f32| Some((y.max(0.) / 1000.) as usize);
        let around = list.rings(10_000., 100., 1000, 3, &(10..11), &row_at);
        assert_eq!(around.build, 7..14);
    }

    #[cfg(feature = "trace")]
    #[test]
    fn spare_time_is_an_entry_of_its_own_and_the_frame_under_way_goes_on() {
        let frame = crate::trace::frame_index();
        crate::trace::set_enabled(true);
        crate::trace::input();
        crate::profile::spare_time(|| {
            let _update = crate::profile::span(crate::profile::Phase::Update);
        });
        crate::profile::frame();
        let frames = crate::trace::frames(usize::MAX);
        crate::trace::set_enabled(false);
        let frames: Vec<_> = frames.iter().filter(|f| f.index > frame).collect();
        assert_eq!(frames.len(), 2, "{}", frames.len());
        assert!(frames[0].spare && frames[0].events == 0);
        assert!(frames[0].phases.iter().any(|p| p.0 == "update"));
        assert!(!frames[1].spare);
        assert_eq!(frames[1].events, 1, "the input stays the frame's");
        assert!(frames[1].phases.iter().all(|p| p.0 != "update"));
    }
}
