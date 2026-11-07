//! Elastic (rubber-band) scrolling, as macOS's scroll views and Safari's pages
//! do it: a trackpad gesture pushed past a scroll view's edge stretches it, a
//! twentieth of the push, and letting go springs it back. Momentum that reaches
//! an edge bounces. The offset an app sees never leaves its range;
//! only the content's translation shows the stretch.
//!
//! The host turns it on (`Ui::set_elastic_scrolling`) and reports a trackpad
//! gesture's phases (`InputEvent::ScrollGesture`); a mouse wheel's notches,
//! and scrolling an app does, never stretch. Off, nothing here runs.
use crate::{
    input::{DispatchResult, InputEvent, ScrollGesture},
    reactive::Signal,
    scene::NodeId,
    widgets::Ui,
};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
    time::Instant,
};

/// How much push a point of stretch takes. Measured on an `NSScrollView`, and
/// the same in WebKit's and Chromium's: linear, with no limit.
const PULL: f32 = 20.;
/// How fast a stretch comes back, per second: gone in about 0.4 s.
const REBOUND: f32 = 12.5;
/// How far an edge's speed carries past it: a bounce peaks 80 ms in, at
/// about 0.009 of the speed in points a second.
const AMPLITUDE: f32 = 0.31;
/// The fastest a bounce starts, in points a second.
const MOST_SPEED: f32 = 20_000.;
/// How much a new speed counts against the speed so far, measuring the
/// fingers' speed (`Speed`), and the shortest and longest time between two
/// of their scrolls it counts: as AppKit's scroll views measure it.
const SPEED_WEIGHT: f32 = 0.8;
const SPEED_SOONEST: f32 = 0.002;
const SPEED_LATEST: f32 = 0.2;

/// One scroll view's stretch.
pub(crate) struct Elastic {
    pub(crate) horizontal: bool,
    /// The view's size and its content's (along both axes).
    size: Signal<(f32, f32)>,
    extent: Signal<(f32, f32)>,
    offset: Signal<f32>,
    /// What the gesture pushed past the edge: before `PULL`
    /// (negative before the start, positive after the end).
    pushed: Cell<f32>,
    /// What shows: `pushed` over `PULL`, or the spring's position.
    pub(crate) shown: Signal<f32>,
    /// Springing back: from where, at what speed and since when, and its
    /// next step.
    spring: Cell<Option<(f32, f32, Instant)>>,
    next: Cell<Option<Instant>>,
}
impl Elastic {
    fn main(&self, size: (f32, f32)) -> f32 {
        if self.horizontal { size.0 } else { size.1 }
    }
    fn length(&self) -> f32 {
        self.main(self.size.with_untracked(|s| *s))
    }
    fn limit(&self) -> f32 {
        (self.main(self.extent.with_untracked(|s| *s)) - self.length()).max(0.)
    }
    fn push(&self, delta: f32) {
        // Caught as it springs back: on from where it shows.
        if self.spring.take().is_some() {
            self.pushed.set(self.shown.with_untracked(|s| *s) * PULL);
            self.next.set(None);
        }
        let pushed = self.pushed.get() + delta;
        self.pushed.set(pushed);
        self.shown.set(pushed / PULL);
    }
    /// Let go of at `speed`, the fingers' along its axis: a stretch still
    /// growing goes on a moment before it comes back, as on an `NSScrollView`
    /// (at 1500 points a second of push, 2 points further for 15 ms).
    fn release(&self, now: Instant, speed: f32) {
        if self.shown.with_untracked(|s| *s) == 0. {
            return;
        }
        self.spring.set(Some((
            self.shown.with_untracked(|s| *s),
            speed.clamp(-MOST_SPEED, MOST_SPEED),
            now,
        )));
        self.next.set(Some(now));
    }
    fn stretched(&self) -> bool {
        self.shown.with_untracked(|s| *s) != 0.
    }
    /// Whether its content fits: it can't scroll, so nothing stretches it.
    fn fits(&self) -> bool {
        self.limit() <= 0.
    }
    /// Back in place at once, its push and spring forgotten.
    fn snap(&self) {
        self.spring.set(None);
        self.next.set(None);
        self.pushed.set(0.);
        if self.stretched() {
            self.shown.set(0.);
        }
    }
    /// Whether it is still on its way back: stretched, or due a step.
    fn moving(&self) -> bool {
        self.stretched() || self.next.get().is_some()
    }
}

/// The document's elastic state.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) enabled: bool,
    /// Every scroll view's, by its root.
    pub(crate) views: Vec<(NodeId, Weak<Elastic>)>,
    /// The trackpad's fingers are down, or its momentum runs.
    fingers: bool,
    momentum: bool,
    /// The momentum bounced off an edge: the rest of it is spent there, as
    /// AppKit's is, not bounced again each time the spring settles.
    bounced: bool,
    /// The view the gesture stretches.
    active: Option<Weak<Elastic>>,
    /// The fingers' speed along either axis (horizontal, vertical): what a
    /// stretch they let go of starts back at.
    speed: [Speed; 2],
    /// Every view stretched or springing back (`active` among them): each
    /// springs back on its own, whichever the next gesture takes. Empty
    /// unless something is stretched.
    moving: Vec<Weak<Elastic>>,
    /// Swipes between pages are on (`Ui::set_navigation_swipes`).
    pub(crate) swipes: bool,
    /// The gesture's first scroll is still to come: only it starts a swipe.
    first: bool,
    /// The gesture became a swipe between pages: its scrolls go nowhere.
    swiping: bool,
    /// A swipe the host has yet to hear of (`Ui::take_navigation_swipe`).
    pub(crate) asked: bool,
}

/// `root`'s scroll view stretches: its translation adds `shown`.
pub(crate) fn register(
    ui: &mut Ui,
    root: NodeId,
    horizontal: bool,
    size: Signal<(f32, f32)>,
    extent: Signal<(f32, f32)>,
    offset: Signal<f32>,
) -> Signal<f32> {
    let shown = ui.signal(0_f32);
    let elastic = Rc::new(Elastic {
        horizontal,
        size,
        extent,
        offset,
        pushed: Cell::new(0.),
        shown: shown.clone(),
        spring: Cell::new(None),
        next: Cell::new(None),
    });
    {
        let mut state = ui.elastic_state().borrow_mut();
        // Unmounted views' entries go now and then, not on every mount.
        if state.views.len() >= 64 && state.views.len().is_power_of_two() {
            state.views.retain(|(_, view)| view.strong_count() > 0);
        }
        state.views.push((root, Rc::downgrade(&elastic)));
    }
    ui.retain(root, elastic);
    shown
}

/// `view` is stretched (or springing back): kept with the others.
fn keep_moving(ui: &Ui, view: &Rc<Elastic>) {
    let mut state = ui.elastic_state().borrow_mut();
    if !state
        .moving
        .iter()
        .any(|moving| moving.as_ptr() == Rc::as_ptr(view))
    {
        state.moving.push(Rc::downgrade(view));
    }
}

/// The views stretched or springing back, alive.
fn moving(ui: &Ui) -> Vec<Rc<Elastic>> {
    let state = ui.elastic_state().borrow();
    state.moving.iter().filter_map(Weak::upgrade).collect()
}

/// `id`'s scroll view's offset, how far it can scroll and its stretch, if
/// `id` is a scroll view (its root, or the viewport or content inside).
pub(crate) fn report(ui: &Ui, id: NodeId) -> Option<(f32, f32, f32)> {
    let scene = ui.scene.borrow();
    let state = ui.elastic_state().borrow();
    std::iter::once(id)
        .chain(scene.ancestors(id).take(2))
        .find_map(|node| {
            state
                .views
                .iter()
                .find(|(root, _)| *root == node)
                .and_then(|(_, view)| view.upgrade())
        })
        .map(|view| {
            (
                view.offset.with_untracked(|o| *o),
                view.limit(),
                view.shown.with_untracked(|s| *s),
            )
        })
}

fn active(ui: &Ui) -> Option<Rc<Elastic>> {
    ui.elastic_state().borrow().active.as_ref()?.upgrade()
}

/// A trackpad gesture's phase: what lifting the fingers or the momentum's
/// end lets spring back.
pub(crate) fn gesture(ui: &Ui, phase: ScrollGesture) {
    let now = crate::clock::now();
    let release = {
        let mut state = ui.elastic_state().borrow_mut();
        match phase {
            ScrollGesture::Began => {
                state.fingers = true;
                state.speed = [Speed::starting(now); 2];
                state.momentum = false;
                state.bounced = false;
                state.first = true;
                state.swiping = false;
                // A swipe a gesture's last scroll asked for, too late to track.
                state.asked = false;
                false
            }
            ScrollGesture::Ended => {
                state.fingers = false;
                state.first = false;
                true
            }
            ScrollGesture::MomentumBegan => {
                state.momentum = true;
                state.bounced = false;
                state.first = false;
                false
            }
            ScrollGesture::MomentumEnded => {
                state.momentum = false;
                true
            }
        }
    };
    if release && !ui.elastic_state().borrow().moving.is_empty() {
        // Every stretched view, not the active one alone: one a gesture
        // left as another took over springs back too, from rest. The one the
        // fingers held goes at their speed; a momentum's end lets go at none.
        let lifted = (phase == ScrollGesture::Ended)
            .then(|| active(ui))
            .flatten();
        let speed = ui.elastic_state().borrow().speed;
        for view in moving(ui) {
            let speed = match &lifted {
                Some(lifted) if Rc::ptr_eq(lifted, &view) => {
                    speed[usize::from(!view.horizontal)].at(now)
                }
                _ => 0.,
            };
            match view.fits() {
                true => view.snap(),
                false => view.release(now, speed),
            }
        }
    }
}

/// Before a scroll is dispatched: a stretched view takes what pulls it back
/// first, and only the rest scrolls. Returns the event to dispatch, if any.
pub(crate) fn before_scroll(ui: &Ui, event: InputEvent) -> Option<InputEvent> {
    let InputEvent::Scroll {
        x,
        y,
        mut delta_x,
        mut delta_y,
    } = event
    else {
        return Some(event);
    };
    // A swipe between pages' gesture scrolls nothing.
    if ui.elastic_state().borrow().swiping {
        return None;
    }
    {
        let mut state = ui.elastic_state().borrow_mut();
        if state.fingers {
            let now = Instant::now();
            state.speed[0].scrolled(delta_x, now);
            state.speed[1].scrolled(delta_y, now);
        }
    }
    let Some(view) = active(ui).filter(|view| view.stretched() && view.spring.get().is_none())
    else {
        return Some(event);
    };
    let delta = if view.horizontal {
        &mut delta_x
    } else {
        &mut delta_y
    };
    let pushed = view.pushed.get();
    // Toward the content: what shows comes back as far as the hand moves, then
    // what is left scrolls.
    if pushed * *delta < 0. {
        let back = delta.abs().min(pushed.abs() / PULL).copysign(*delta);
        view.push(back * PULL);
        *delta -= back;
        if view.pushed.get().abs() < 0.5 {
            view.pushed.set(0.);
            view.shown.set(0.);
        }
    }
    (*delta != 0. || delta_x != 0. || delta_y != 0.).then_some(InputEvent::Scroll {
        x,
        y,
        delta_x,
        delta_y,
    })
}

/// After a scroll no view took: during a trackpad gesture, the view it is
/// latched to stretches, or the outermost one under the pointer that
/// scrolls along its axis (as a page bounces, not a box inside it).
pub(crate) fn after_scroll(ui: &Ui, event: &InputEvent, result: &DispatchResult) {
    let InputEvent::Scroll {
        delta_x, delta_y, ..
    } = *event
    else {
        return;
    };
    let (fingers, momentum, first) = {
        let mut state = ui.elastic_state().borrow_mut();
        (
            state.fingers,
            state.momentum,
            std::mem::take(&mut state.first),
        )
    };
    if result.default_prevented || !(fingers || momentum) {
        return;
    }
    let horizontal = delta_x.abs() > delta_y.abs();
    let delta = if horizontal { delta_x } else { delta_y };
    if delta == 0. || !delta.is_finite() {
        return;
    }
    // A gesture starting across where nothing scrolls that way, over a view
    // that navigates: the platform's swipe between pages, not a stretch.
    if first && fingers && horizontal && swipe(ui, result.target) {
        return;
    }
    let now = crate::clock::now();
    let view = active(ui)
        .filter(|view| view.horizontal == horizontal && view.stretched())
        .or_else(|| {
            let target = result.target?;
            let scene = ui.scene.borrow();
            let state = ui.elastic_state().borrow();
            std::iter::once(target)
                .chain(scene.ancestors(target))
                .filter_map(|node| {
                    state
                        .views
                        .iter()
                        .find(|(root, _)| *root == node)
                        .and_then(|(_, view)| view.upgrade())
                })
                .filter(|view| view.horizontal == horizontal && view.limit() > 0.)
                .last()
        });
    let Some(view) = view else {
        return;
    };
    if view.fits() {
        view.snap();
        return;
    }
    // Only past the edge the delta pushes toward.
    let at = view.offset.with_untracked(|o| *o);
    let limit = view.limit();
    let outward = (delta < 0. && at <= 0.) || (delta > 0. && at >= limit);
    if !outward {
        return;
    }
    if view.spring.get().is_some() && !fingers {
        // Momentum toward an edge springing back is spent in the spring.
        ui.elastic_state().borrow_mut().bounced = true;
        return;
    }
    if momentum && !fingers {
        // Momentum bounces off the edge, at the speed it got there. Once a
        // gesture's momentum: what is left of it stops at the edge.
        if ui.elastic_state().borrow().bounced {
            return;
        }
        ui.elastic_state().borrow_mut().bounced = true;
        let speed = delta / ui.refresh_interval().as_secs_f32().max(0.001);
        view.spring
            .set(Some((0., speed.clamp(-MOST_SPEED, MOST_SPEED), now)));
        view.next.set(Some(now));
        keep_moving(ui, &view);
        return;
    }
    view.push(delta);
    keep_moving(ui, &view);
    let previous = ui
        .elastic_state()
        .borrow_mut()
        .active
        .replace(Rc::downgrade(&view));
    // Another view the gesture left stretched springs back from where it
    // shows (it's in `moving`, so it steps on its own).
    if let Some(previous) = previous.and_then(|previous| previous.upgrade())
        && !Rc::ptr_eq(&previous, &view)
        && previous.spring.get().is_none()
    {
        previous.release(now, 0.);
    }
}

/// Whether the gesture over `target` becomes a swipe between pages
/// (`Ui::set_navigation_swipes`): it does where a view takes navigation.
fn swipe(ui: &Ui, target: Option<NodeId>) -> bool {
    if !ui.elastic_state().borrow().swipes {
        return false;
    }
    let navigates = target.is_some_and(|target| ui.input.navigates(&ui.scene.borrow(), target));
    if navigates {
        let mut state = ui.elastic_state().borrow_mut();
        state.swiping = true;
        state.asked = true;
    }
    navigates
}

/// When the stretched view next steps back. Nothing stretched, nothing.
pub(crate) fn next_step(ui: &Ui) -> Option<Instant> {
    let state = ui.elastic_state().borrow();
    if state.moving.is_empty() {
        return None;
    }
    state
        .moving
        .iter()
        .filter_map(|view| view.upgrade()?.next.get())
        .min()
}

/// Step the spring (or end a bounce) due by `now`; whether it moved.
pub(crate) fn advance(ui: &Ui, now: Instant) -> bool {
    if ui.elastic_state().borrow().moving.is_empty() {
        return false;
    }
    let mut moved = false;
    for view in moving(ui) {
        moved |= step(ui, &view, now);
    }
    // Settled views (and unmounted ones) leave the list; the active one
    // too, once settled.
    let mut state = ui.elastic_state().borrow_mut();
    state
        .moving
        .retain(|view| view.upgrade().is_some_and(|view| view.moving()));
    if state
        .active
        .as_ref()
        .and_then(Weak::upgrade)
        .is_none_or(|view| !view.moving())
    {
        state.active = None;
    }
    moved
}

/// Step `view`'s spring (or end its bounce) if due by `now`; whether it
/// moved.
fn step(ui: &Ui, view: &Rc<Elastic>, now: Instant) -> bool {
    if view.fits() && view.moving() {
        view.snap();
        return true;
    }
    if view.next.get().is_none_or(|next| next > now) {
        return false;
    }
    let Some((from, speed, since)) = view.spring.get() else {
        view.next.set(None);
        return false;
    };
    let t = now.saturating_duration_since(since).as_secs_f32();
    let shown = (from + AMPLITUDE * speed * t) * (-REBOUND * t).exp();
    // A bounce from the edge is done once under half a point past its
    // furthest, at 1 / REBOUND, or at once if it never gets half a point out
    // (`NSScrollView` ends it so: measured at 83-85 ms, 0.0086-0.0091 of the
    // speed at the edge, against 80 ms and 0.0091 here).
    let peak = AMPLITUDE * speed.abs() / REBOUND * (-1_f32).exp();
    let past = from != 0. || t > 1. / REBOUND || peak < 0.5;
    if shown.abs() < 0.5 && past {
        view.snap();
    } else {
        view.shown.set(shown);
        // A step a refresh (`Ui::set_refresh_interval`).
        view.next.set(Some(now + ui.refresh_interval()));
    }
    true
}

/// The fingers' speed along one axis, as AppKit's scroll views measure it:
/// each scroll's speed weighed against the speed so far. One sooner than
/// `SPEED_SOONEST` after the last waits for the next (its distance with it);
/// one later than `SPEED_LATEST`, or the other way, starts over.
#[derive(Clone, Copy, Default)]
pub(crate) struct Speed {
    speed: f32,
    /// When the last scroll counted, the distance since, and whether a speed
    /// has been measured yet.
    at: Option<Instant>,
    pending: f32,
    measured: bool,
}
impl Speed {
    fn starting(now: Instant) -> Self {
        Self {
            at: Some(now),
            ..Self::default()
        }
    }
    fn scrolled(&mut self, delta: f32, now: Instant) {
        if delta == 0. || !delta.is_finite() {
            return;
        }
        let Some(at) = self.at else {
            *self = Self::starting(now);
            return;
        };
        let dt = now.saturating_duration_since(at).as_secs_f32();
        if dt < SPEED_SOONEST {
            self.pending += delta;
            return;
        }
        let delta = delta + std::mem::take(&mut self.pending);
        let speed = delta / dt;
        self.speed = if !self.measured || dt > SPEED_LATEST || speed * self.speed < 0. {
            speed
        } else {
            SPEED_WEIGHT * speed + (1. - SPEED_WEIGHT) * self.speed
        };
        self.measured = true;
        self.at = Some(now);
    }
    /// The speed the fingers lift at, at `now`: none once they've rested
    /// `SPEED_LATEST` (on an `NSScrollView`, a push held 150 ms before lifting
    /// still went on, one held 300 ms didn't).
    fn at(&self, now: Instant) -> f32 {
        match self.at {
            Some(at)
                if self.measured
                    && now.saturating_duration_since(at).as_secs_f32() <= SPEED_LATEST =>
            {
                self.speed
            }
            _ => 0.,
        }
    }
}
