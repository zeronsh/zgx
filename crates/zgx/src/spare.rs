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
    pub(crate) fn pass(&self) {
        self.passes.set(self.passes.get().wrapping_add(1));
        self.tick.get();
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

/// What a list does with spare time, all of it deciding which rows its pass
/// goes through: the rows in view, those kept and the next to build, merged
/// (`merge`). The pass then builds, stamps, places, orders and drops them as
/// it does any, so a list with none (a headless host's) runs what it always
/// ran: this is one `Option` there, and out of line here.
pub(crate) struct Kept<K> {
    ahead: Rc<ListAhead>,
    overscan: usize,
    width: u32,
    structure: u64,
    top: Option<f32>,
    /// The way the view last moved: up.
    up: bool,
    /// The rows the last pass went through, by index.
    rows: Vec<(K, usize)>,
    /// What each row mounted with, counted once it is kept.
    nodes: crate::scene::NodeMap<usize>,
    /// The decodes each row out of view asked for its pictures in spare
    /// time (`ImageData::decode_ahead`), let go as it goes.
    pictures: crate::scene::NodeMap<Vec<crate::image::Ahead>>,
    /// Rows out of view haven't all asked for theirs: another turn is wanted
    /// while there's room for their decodes.
    unasked: bool,
}
/// A list's view as a pass sees it, for [`Kept::merge`].
pub(crate) struct KeptView {
    pub(crate) top: f32,
    pub(crate) height: f32,
    pub(crate) count: usize,
    pub(crate) width: f32,
    /// What says rows may have moved.
    pub(crate) structure: u64,
    /// A move now is rows' heights settling around what shows (or a reply
    /// growing under a list that follows it), no one scrolling.
    pub(crate) settled: bool,
    /// A row holding the keyboard's focus goes as it leaves.
    pub(crate) focus_goes: bool,
}
impl<K: Eq + Hash + Clone> Kept<K> {
    /// A list's, if it has a share of spare time.
    pub(crate) fn new(ahead: Rc<ListAhead>, overscan: usize) -> Box<Self> {
        Box::new(Self {
            ahead,
            overscan,
            width: 0,
            structure: 0,
            top: None,
            up: false,
            rows: Vec::new(),
            nodes: Default::default(),
            pictures: Default::default(),
            unasked: false,
        })
    }
    /// A pass begins, subscribed to its turn of spare time.
    pub(crate) fn pass(&self) {
        self.ahead.pass();
    }
    /// The rows this pass goes through, by index: `shown` (those in view, by
    /// index), those that stay, and at spare time's turn the next to build.
    /// `row_at`, `can` and `node_of` say where rows are, whether one can be
    /// built, and the row built for a key.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn merge(
        &mut self,
        ui: &crate::widgets::Ui,
        view: KeptView,
        shown: Vec<(K, usize)>,
        key: &mut dyn FnMut(usize) -> K,
        row_at: &dyn Fn(f32) -> Option<usize>,
        can: &dyn Fn(usize) -> bool,
        node_of: &dyn Fn(&K) -> Option<NodeId>,
    ) -> Vec<(K, usize)> {
        if let Some(was) = self.top.replace(view.top)
            && was != view.top
            && !view.settled
        {
            self.up = view.top < was;
            self.ahead.moved();
        }
        let range = match (shown.first(), shown.last()) {
            (Some((_, first)), Some((_, last))) => *first..*last + 1,
            _ => 0..0,
        };
        let rings = self.ahead.rings(
            view.top,
            view.height,
            view.count,
            self.overscan,
            &range,
            row_at,
        );
        // A new width measures every row again: those out of view are built
        // again as they are wanted.
        let width = view.width.to_bits();
        let same_width = std::mem::replace(&mut self.width, width) == width;
        // Rows may have moved: asked, or a row in view isn't what it was.
        let moved = std::mem::replace(&mut self.structure, view.structure) != view.structure
            || shown.iter().any(|(row, index)| {
                self.rows
                    .binary_search_by_key(index, |(_, at)| *at)
                    .is_ok_and(|at| self.rows[at].0 != *row)
            });
        let mut kept: Vec<(K, usize)> = Vec::new();
        let was = std::mem::take(&mut self.rows);
        if same_width && moved {
            for index in rings.keep.clone() {
                if !range.contains(&index) && can(index) {
                    let row = key(index);
                    if node_of(&row).is_some() {
                        kept.push((row, index));
                    }
                }
            }
        } else if same_width {
            kept.extend(was.into_iter().filter(|(_, index)| {
                rings.keep.contains(index) && !range.contains(index) && can(*index)
            }));
        }
        let in_view: std::collections::HashSet<&K, rustc_hash::FxBuildHasher> =
            shown.iter().map(|(row, _)| row).collect();
        kept.retain(|(row, _)| !in_view.contains(row));
        // Holding the keyboard's focus or what's open from it, a row goes
        // as it leaves, as ever.
        let focused = if view.focus_goes {
            ui.input.focused()
        } else {
            None
        };
        let scope = ui.input.focus_scope();
        if focused.is_some() || scope.is_some() {
            kept.retain(|(row, _)| {
                node_of(row).is_none_or(|node| {
                    focused.is_none_or(|focused| !ui.opened_from(focused, node))
                        && scope.is_none_or(|scope| !ui.opened_from(scope, node))
                })
            });
        }
        // Past what a list keeps, the farthest go.
        {
            let scene = ui.scene.borrow();
            for (row, _) in shown.iter().chain(&kept) {
                if let Some(node) = node_of(row)
                    && !self.nodes.contains_key(&node)
                {
                    self.nodes.insert(node, nodes_under(&scene, node));
                }
            }
            if self.nodes.len() > shown.len() + kept.len() {
                self.nodes.retain(|node, _| scene.contains(*node));
            }
            if self.pictures.len() > shown.len() + kept.len() {
                self.pictures.retain(|node, _| scene.contains(*node));
            }
        }
        let nodes = |row: &K| {
            node_of(row)
                .and_then(|node| self.nodes.get(&node).copied())
                .unwrap_or(1)
        };
        let all = shown
            .iter()
            .chain(&kept)
            .map(|(row, index)| (*index, nodes(row)));
        if let Some(cut) = rings.cut(&range, all) {
            kept.retain(|(_, index)| !rings.beyond(&range, *index, cut));
        }
        // Spare time's row, if this is its turn.
        let mut built: Vec<usize> = shown.iter().chain(&kept).map(|(_, index)| *index).collect();
        built.sort_unstable();
        let mut next = rings.next(&range, !self.up, &built, can);
        let turn = self.ahead.take_may_build();
        // Spare time's turn: the rows out of view ask for their pictures
        // (laid out, all: a row built this turn is only after).
        if turn {
            self.unasked = self.decode_pictures(ui, &kept, &range, node_of);
        }
        if turn && let Some(index) = next {
            let row = key(index);
            // Rows moved where nothing said so: the next pass that moves
            // looks again.
            if in_view.contains(&row) || kept.iter().any(|(other, _)| *other == row) {
                next = None;
            } else {
                kept.push((row, index));
                let at = built.partition_point(|row| *row < index);
                built.insert(at, index);
                next = rings.next(&range, !self.up, &built, can);
                // It asks for its pictures next turn, laid out.
                self.unasked = true;
            }
        }
        let pictures = self.unasked && crate::image::ahead_room();
        ui.list_wants(&self.ahead, next.is_some() || pictures);
        drop(in_view);
        kept.extend(shown);
        kept.sort_unstable_by_key(|(_, index)| *index);
        self.rows = kept.clone();
        kept
    }
}
impl<K: Eq + Hash + Clone> Kept<K> {
    /// Rows out of view not yet asked ask for their pictures' pixels at the
    /// size they'll be drawn: those the way the view goes first, nearest
    /// first, while the decodes waiting are few enough (`AHEAD_MOST`). Each
    /// row's go as it does. Whether rows are left to ask (the room ran out).
    #[inline(never)]
    fn decode_pictures(
        &mut self,
        ui: &crate::widgets::Ui,
        kept: &[(K, usize)],
        shown: &Range<usize>,
        node_of: &dyn Fn(&K) -> Option<NodeId>,
    ) -> bool {
        let up = self.up;
        let mut rows: Vec<(bool, usize, NodeId)> = kept
            .iter()
            .filter_map(|(row, index)| {
                let node = node_of(row)?;
                (!self.pictures.contains_key(&node)).then(|| {
                    let behind = if up {
                        *index >= shown.end
                    } else {
                        *index < shown.start
                    };
                    (behind, distance(shown, *index), node)
                })
            })
            .collect();
        rows.sort_unstable_by_key(|(behind, distance, _)| (*behind, *distance));
        let scene = ui.scene.borrow();
        let scale = match scene.pixel_scale() {
            scale if scale > 0. => scale,
            _ => 1.,
        };
        for (_, _, node) in rows {
            if !crate::image::ahead_room() {
                return true;
            }
            let mut asked = Vec::new();
            let mut stack = vec![node];
            while let Some(at) = stack.pop() {
                if let crate::scene::NodeKind::Image(image) = scene.kind(at)
                    && let bounds = scene.bounds(at)
                    // (Drawing nothing, as a renderer passes it by.)
                    && image.paint_bounds(bounds).width > 0.
                    && image.paint_bounds(bounds).height > 0.
                {
                    // At the size a renderer draws it: its box, through its
                    // own transform, in device pixels.
                    let m = image.transform();
                    let stretch = (m.a.hypot(m.b)).max(m.c.hypot(m.d));
                    let drawn = (
                        bounds.width * scale * stretch,
                        bounds.height * scale * stretch,
                    );
                    let size =
                        crate::image::fitted_size(image.width(), image.height(), Some(drawn));
                    if let Some(ahead) = image.decode_ahead(size) {
                        asked.push(ahead);
                    }
                }
                stack.extend_from_slice(scene.children(at));
            }
            self.pictures.insert(node, asked);
        }
        false
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
    /// (`View::list_ahead`) doesn't turn it off: `tick` wakes its pass.
    pub(crate) fn list_ahead(
        &self,
        viewports: Option<f32>,
        tick: Signal<u64>,
    ) -> Option<Rc<ListAhead>> {
        let spare = self.spare();
        if !spare.hosted.get() {
            return None;
        }
        let viewports = viewports.unwrap_or(AHEAD_VIEWPORTS);
        if !viewports.is_finite() || viewports <= 0. {
            return None;
        }
        let list = Rc::new(ListAhead::new(tick, viewports));
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
