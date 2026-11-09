//! Scroll views that come to rest on whole pages (`View::paging`), each page the
//! view's own length, as `UIScrollView.pagingEnabled` does on iOS.
//!
//! A gesture moves the content with the fingers. Lifting them picks a page as
//! UIKit does, and the view goes there on UIKit's own spring, whatever momentum
//! the platform goes on sending. A mouse wheel goes a page a notch. Anything else
//! that moves it (a key, a reveal of what took focus, a script) settles on the
//! nearest page once it stops. A paged view has this record; a view that doesn't
//! page has nothing, and nothing here runs for it.
use crate::{input::ScrollGesture, reactive::Signal, widgets::Ui};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

/// UIKit's paging spring, a step a millisecond: `v = (v + (target - x) * PULL) *
/// FRICTION; x += v` (`-[UIScrollView _getPagingDecelerationOffset:forTimeInterval:]`;
/// `_pagingSpringPull` and `_pagingFriction` read at runtime, on a view 402 long).
const PULL: f32 = 0.000_284_858;
const FRICTION: f32 = 0.970_815;
/// A lift at this speed or more, in points a second, goes on to the next page
/// wherever it is; slower, to the nearest. Measured on a `UIScrollView`: at
/// 0.2999 points a millisecond it stayed, at 0.3337 went on, from 10 to 190 points
/// along a page of 402; what the threshold is exactly wasn't read.
const FLICK: f32 = 300.;
/// How long a move that is no gesture's rests before it settles.
const IDLE: Duration = Duration::from_millis(150);
/// Scrolls this far apart are separate notches of a wheel; closer, one notch's
/// ease, unless they add up to half a page.
const NOTCH_GAP: Duration = Duration::from_millis(180);
/// The most of a stall's motion a step makes up, in milliseconds.
const MOST_STEPS: u128 = 64;

/// How far back a lift's speed is measured.
const LIFT_WINDOW: Duration = Duration::from_millis(100);
/// Fingers rested this long before lifting have no speed.
const LIFT_LATEST: Duration = Duration::from_millis(200);

/// The speed fingers lift at: what they moved over the last `LIFT_WINDOW`.
/// Scrolls arrive when the app gets to them, in batches while it draws, so the
/// gaps between them say little: a slow swipe's last gap could read as a flick.
#[derive(Default)]
struct LiftSpeed {
    /// When what `recent` adds up to began.
    from: Option<Instant>,
    recent: VecDeque<(Instant, f32)>,
}
impl LiftSpeed {
    fn starting(now: Instant) -> Self {
        Self {
            from: Some(now),
            recent: VecDeque::new(),
        }
    }
    fn scrolled(&mut self, delta: f32, now: Instant) {
        if delta == 0. || !delta.is_finite() {
            return;
        }
        // Turned back: the speed is the way it goes now.
        if let Some(&(at, last)) = self.recent.back()
            && last * delta < 0.
        {
            self.recent.clear();
            self.from = Some(at);
        }
        self.from.get_or_insert(now);
        self.recent.push_back((now, delta));
        while let Some(&(at, _)) = self.recent.front()
            && self.recent.len() > 1
            && now.saturating_duration_since(at) > LIFT_WINDOW
        {
            self.from = Some(at);
            self.recent.pop_front();
        }
    }
    /// In points a second, at `now`.
    fn at(&self, now: Instant) -> f32 {
        let (Some(from), Some(&(last, _))) = (self.from, self.recent.back()) else {
            return 0.;
        };
        if now.saturating_duration_since(last) > LIFT_LATEST {
            return 0.;
        }
        let moved: f32 = self.recent.iter().map(|(_, delta)| delta).sum();
        moved
            / last
                .saturating_duration_since(from)
                .as_secs_f32()
                .max(0.001)
    }
}

/// One paged view.
pub(crate) struct Paged {
    horizontal: bool,
    size: Signal<(f32, f32)>,
    extent: Signal<(f32, f32)>,
    offset: Signal<f32>,
    /// The page the gesture began on: it ends at most one page from it.
    origin: Cell<f32>,
    /// The gesture scrolled this view, and how fast (along its axis).
    touched: Cell<bool>,
    speed: RefCell<LiftSpeed>,
    /// The spring: where it goes, where it is and how fast (points a
    /// millisecond), and the time it has stepped to.
    target: Cell<Option<f32>>,
    at: Cell<f32>,
    velocity: Cell<f32>,
    stepped: Cell<Instant>,
    /// A gesture's lift set it going: what the platform's momentum sends
    /// meanwhile moves nothing.
    lifted: Cell<bool>,
    /// When it next steps, or settles after a move that was no gesture's.
    next: Cell<Option<Instant>>,
    /// What this last wrote to `offset`, to tell a move of someone else's.
    written: Cell<f32>,
    /// The view's length last seen: resized, it stays on its page.
    length: Cell<f32>,
    /// The wheel's last scroll, and what its notch has added up to.
    wheeled: Cell<Option<Instant>>,
    notch: Cell<f32>,
}
impl Paged {
    fn main(&self, size: (f32, f32)) -> f32 {
        if self.horizontal { size.0 } else { size.1 }
    }
    fn page_length(&self) -> f32 {
        self.main(self.size.with_untracked(|size| *size))
    }
    fn limit(&self) -> f32 {
        (self.main(self.extent.with_untracked(|size| *size)) - self.page_length()).max(0.)
    }
    fn position(&self) -> f32 {
        self.offset.with_untracked(|offset| *offset)
    }
    /// The last page: its start at the end of what scrolls, if the content
    /// isn't a whole number of pages.
    fn last_page(&self) -> f32 {
        (self.limit() / self.page_length().max(1.) - 0.001)
            .ceil()
            .max(0.)
    }
    fn start_of(&self, page: f32) -> f32 {
        (page * self.page_length()).min(self.limit())
    }
    /// The page `position` is nearest.
    fn nearest(&self, position: f32) -> f32 {
        let last = self.last_page();
        let length = self.page_length().max(1.);
        if position >= self.limit() {
            return last;
        }
        (position / length).round().clamp(0., last)
    }
    fn go_to(&self, page: f32, velocity: f32, now: Instant) {
        let target = self.start_of(page);
        let at = self.position();
        self.at.set(at);
        self.written.set(at);
        self.stepped.set(now);
        if (target - at).abs() < 0.01 && velocity.abs() < 0.01 {
            self.stop();
            return;
        }
        self.velocity.set(velocity);
        self.target.set(Some(target));
        self.next.set(Some(now));
    }
    fn stop(&self) {
        self.target.set(None);
        self.next.set(None);
        self.lifted.set(false);
    }
    /// The page a lift at `speed` (points a second) goes to.
    fn page_for_lift(&self, speed: f32) -> f32 {
        let last = self.last_page();
        let length = self.page_length().max(1.);
        let along = self.position() / length;
        let origin = self.origin.get();
        let page = if speed >= FLICK {
            along.ceil()
        } else if speed <= -FLICK {
            along.floor()
        } else {
            self.nearest(self.position())
        };
        page.clamp((origin - 1.).max(0.), (origin + 1.).min(last))
    }
    /// A scroll along it (`delta` points on from its start, as its offset counts
    /// them): whether this takes it, so that nothing else moves for it.
    pub(crate) fn scrolled(&self, ui: &Ui, delta: f32) -> bool {
        let (fingers, momentum) = {
            let state = ui.paging_state().borrow();
            (state.fingers, state.momentum)
        };
        let now = crate::clock::now();
        if fingers {
            self.touched.set(true);
            self.speed.borrow_mut().scrolled(delta, now);
            return false;
        }
        if momentum || self.lifted.get() {
            return true;
        }
        if delta == 0. {
            return false;
        }
        // A wheel. A notch eases over a few scrolls: it is one page however many.
        let apart = self
            .wheeled
            .get()
            .is_none_or(|at| now.saturating_duration_since(at) > NOTCH_GAP);
        let turned = self.notch.get() * delta < 0.;
        let first = apart || turned;
        let notch = if first {
            delta
        } else {
            self.notch.get() + delta
        };
        self.wheeled.set(Some(now));
        let page_length = self.page_length().max(1.);
        if !first && notch.abs() < page_length * 0.5 {
            self.notch.set(notch);
            return true;
        }
        self.notch.set(if first { notch } else { 0. });
        let from = match self.target.get() {
            Some(target) => self.nearest(target),
            None => self.nearest(self.position()),
        };
        let page = (from + delta.signum()).clamp(0., self.last_page());
        if page == from && self.target.get().is_none() {
            // At its last page: the wheel goes on to what's around it.
            return false;
        }
        let velocity = if self.target.get().is_some() {
            self.velocity.get()
        } else {
            0.
        };
        self.go_to(page, velocity, now);
        true
    }
    /// The view's size or offset changed.
    fn observed(&self, ui: &Ui) {
        let length = self.page_length();
        let position = self.position();
        let before = self.length.replace(length);
        if before != length && before > 0. && length > 0. {
            // Resized: the same page, not the same distance.
            let page = (self.written.get() / before).round();
            let start = self.start_of(page);
            self.stop();
            self.written.set(start);
            if start != position {
                self.offset.set(start);
            }
            return;
        }
        if position == self.written.get() {
            return;
        }
        if ui.paging_state().borrow().fingers {
            self.written.set(position);
            return;
        }
        // Someone else moved it: it settles once they've stopped.
        self.written.set(position);
        self.target.set(None);
        self.lifted.set(false);
        self.next.set(Some(crate::clock::now() + IDLE));
    }
    /// Steps due by `now`; whether it moved.
    fn step(&self, ui: &Ui, now: Instant) -> bool {
        if self.next.get().is_none_or(|next| next > now) {
            return false;
        }
        let Some(target) = self.target.get() else {
            // A move of someone else's has rested.
            self.next.set(None);
            self.go_to(self.nearest(self.position()), 0., now);
            return false;
        };
        let elapsed = now
            .saturating_duration_since(self.stepped.get())
            .as_millis();
        self.stepped
            .set(self.stepped.get() + Duration::from_millis(elapsed as u64));
        let (mut at, mut velocity) = (self.at.get(), self.velocity.get());
        for _ in 0..elapsed.min(MOST_STEPS) {
            velocity = (velocity + (target - at) * PULL) * FRICTION;
            at += velocity;
        }
        let limit = self.limit();
        if at < 0. || at > limit {
            at = at.clamp(0., limit);
            velocity = 0.;
        }
        let done = (target - at).abs() < 0.25 && velocity.abs() < 0.02;
        if done {
            at = target;
            self.stop();
        } else {
            self.next.set(Some(now + ui.refresh_interval()));
        }
        self.at.set(at);
        self.velocity.set(velocity);
        self.written.set(at);
        self.offset.set(at);
        true
    }
}

/// The document's paged views.
#[derive(Default)]
pub(crate) struct State {
    views: Vec<Weak<Paged>>,
    /// A gesture's fingers are down; its momentum runs.
    fingers: bool,
    momentum: bool,
}

/// `root`'s scroll view pages: its record, for the view to hold and give scrolls to.
pub(crate) fn register(
    ui: &mut Ui,
    horizontal: bool,
    size: Signal<(f32, f32)>,
    extent: Signal<(f32, f32)>,
    offset: Signal<f32>,
) -> Rc<Paged> {
    let now = crate::clock::now();
    let position = offset.with_untracked(|offset| *offset);
    let paged = Rc::new(Paged {
        horizontal,
        size: size.clone(),
        extent: extent.clone(),
        offset: offset.clone(),
        origin: Cell::new(0.),
        touched: Cell::new(false),
        speed: RefCell::default(),
        target: Cell::new(None),
        at: Cell::new(position),
        velocity: Cell::new(0.),
        stepped: Cell::new(now),
        lifted: Cell::new(false),
        next: Cell::new(None),
        written: Cell::new(position),
        length: Cell::new(0.),
        wheeled: Cell::new(None),
        notch: Cell::new(0.),
    });
    {
        let mut state = ui.paging_state().borrow_mut();
        state.views.retain(|view| view.strong_count() > 0);
        state.views.push(Rc::downgrade(&paged));
    }
    paged
}
/// What the view's binding calls, reading what it depends on.
pub(crate) fn observe(ui: &Ui, paged: &Paged) {
    let _ = (paged.size.get(), paged.offset.get());
    paged.observed(ui);
}
fn views(ui: &Ui) -> Vec<Rc<Paged>> {
    let state = ui.paging_state().borrow();
    state.views.iter().filter_map(Weak::upgrade).collect()
}

/// A trackpad gesture's phase, or a touch's.
pub(crate) fn gesture(ui: &Ui, phase: ScrollGesture) {
    if ui.paging_state().borrow().views.is_empty() {
        return;
    }
    let now = crate::clock::now();
    match phase {
        ScrollGesture::Began => {
            {
                let mut state = ui.paging_state().borrow_mut();
                state.fingers = true;
                state.momentum = false;
            }
            for view in views(ui) {
                // Caught as it moves: on from where it shows.
                view.stop();
                view.touched.set(false);
                *view.speed.borrow_mut() = LiftSpeed::starting(now);
                view.origin.set(view.nearest(view.position()));
                view.written.set(view.position());
            }
        }
        ScrollGesture::Ended => {
            ui.paging_state().borrow_mut().fingers = false;
            for view in views(ui) {
                let speed = match view.touched.take() {
                    true => view.speed.borrow().at(now),
                    false => 0.,
                };
                let page = view.page_for_lift(speed);
                // The speed it lifted at carries on into the spring, as UIKit's does.
                view.go_to(page, speed / 1000., now);
                view.lifted.set(view.target.get().is_some());
            }
        }
        ScrollGesture::MomentumBegan => ui.paging_state().borrow_mut().momentum = true,
        ScrollGesture::MomentumEnded => ui.paging_state().borrow_mut().momentum = false,
    }
}

/// When a paged view next steps. None moving, nothing.
pub(crate) fn next_step(ui: &Ui) -> Option<Instant> {
    let state = ui.paging_state().borrow();
    state
        .views
        .iter()
        .filter_map(|view| view.upgrade()?.next.get())
        .min()
}

/// Step what's due by `now`; whether anything moved.
pub(crate) fn advance(ui: &Ui, now: Instant) -> bool {
    if ui.paging_state().borrow().views.is_empty() {
        return false;
    }
    let mut moved = false;
    for view in views(ui) {
        moved |= view.step(ui, now);
    }
    moved
}
