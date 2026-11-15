//! A list following its end that glides there rather than jumping
//! (`VariableHeights::glide_end`): as a reply is written into a chat, the view
//! keeps up with it on a spring instead of stepping a line at a time.
//!
//! The spring is a stick-to-bottom one, as stackblitz's use-stick-to-bottom
//! and Zeron's own transcript before zgx moved theirs: a velocity relaxing
//! toward the gap each 60 Hz frame, plus a feed-forward of how fast the end
//! grows (smoothed), chasing a point a little short of the end while it grows
//! (at most `MOST_LEAD`), so that text arriving in bursts reads as one glide.
//!
//! The list's pass starts it, and moves its target as the end moves
//! (`chase`); each step writes the list's offset, and the pass knows its
//! own steps by what was written (`wrote`). Any other move of the offset (a
//! wheel, a script) is someone else's: the pass stops it. While it chases,
//! the list says it rests at its end (`following`). A list without it has no
//! record, and nothing here runs.
use crate::time::Instant;
use crate::{reactive::Signal, widgets::Ui};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
    time::Duration,
};

/// How much of its velocity the spring keeps a frame.
const DAMPING: f32 = 0.7;
/// How hard the gap pulls.
const STIFFNESS: f32 = 0.05;
/// How slowly it starts and stops.
const MASS: f32 = 1.25;
/// The frame its constants are for (60 Hz).
const FRAME_MS: f32 = 1000. / 60.;
/// The most frames a step makes up after a stall: a hitch catches up rather
/// than teleporting.
const MOST_FRAMES: f32 = 8.;
/// How much a frame's growth counts against the growth so far.
const GROWTH_WEIGHT: f32 = 0.12;
/// How far short of the end it chases while the end grows, at most.
const MOST_LEAD: f32 = 32.;
/// How long a glide that landed keeps its speed: text that pauses a moment
/// goes on at the pace it had.
const KEEP_PACE: Duration = Duration::from_millis(500);

/// One list's glide.
pub(crate) struct Glide {
    offset: Signal<f32>,
    /// Whether it glides toward the end now: the list rests at its end
    /// meanwhile, as far as anyone asks.
    pub(crate) following: Signal<bool>,
    at: Cell<f32>,
    target: Cell<f32>,
    /// Points per 60 Hz frame: the spring's, and the end's growth.
    velocity: Cell<f32>,
    growth: Cell<f32>,
    last_target: Cell<Option<f32>>,
    stepped: Cell<Instant>,
    /// When it next steps; none at rest.
    next: Cell<Option<Instant>>,
    /// When it last landed, its pace kept a moment (`KEEP_PACE`).
    landed: Cell<Option<Instant>>,
    /// What it last wrote to the offset.
    written: Cell<Option<f32>>,
}
impl Glide {
    /// Whether it is gliding toward the end.
    pub(crate) fn chasing(&self) -> bool {
        self.next.get().is_some()
    }
    /// Whether `requested` is what its last step wrote.
    pub(crate) fn wrote(&self, requested: f32) -> bool {
        self.written
            .get()
            .is_some_and(|w| w.to_bits() == requested.to_bits())
    }
    /// Glide from `at` (where the view now shows) to `target` (the end).
    /// Within half a point there's nothing to glide: it stops.
    pub(crate) fn chase(&self, ui: &Ui, at: f32, target: f32) {
        if target - at <= 0.5 {
            self.stop();
            return;
        }
        let now = crate::clock::now();
        if self.next.get().is_none() {
            // A pace kept from a moment ago goes on; else it starts still.
            if self
                .landed
                .get()
                .is_none_or(|landed| now.saturating_duration_since(landed) > KEEP_PACE)
            {
                self.velocity.set(0.);
                self.growth.set(0.);
                self.last_target.set(None);
            }
            self.stepped.set(now);
            self.next.set(Some(now + ui.refresh_interval()));
        }
        self.at.set(at);
        self.target.set(target);
        if !self.following.with_untracked(|f| *f) {
            self.following.set(true);
        }
    }
    /// Stop where it is: someone else moved the view, or it isn't wanted.
    pub(crate) fn stop(&self) {
        self.next.set(None);
        self.landed.set(None);
        self.written.set(None);
        if self.following.with_untracked(|f| *f) {
            self.following.set(false);
        }
    }
    /// Step if due by `now`; whether it moved.
    fn step(&self, ui: &Ui, now: Instant) -> bool {
        if self.next.get().is_none_or(|next| next > now) {
            return false;
        }
        let elapsed = now.saturating_duration_since(self.stepped.get());
        self.stepped.set(now);
        let mut frames = (elapsed.as_secs_f32() * 1000. / FRAME_MS).clamp(0.25, MOST_FRAMES);
        let target = self.target.get();
        let (at, done) = {
            let grew = self.last_target.get().map_or(0., |last| target - last);
            self.last_target.set(Some(target));
            if grew < -1. {
                // The end came up (a row folded, or gone): the pace so far
                // says nothing of it.
                self.growth.set(0.);
            } else {
                let observed = grew.max(0.) / frames;
                let growth = self.growth.get();
                self.growth
                    .set(growth + GROWTH_WEIGHT * (observed - growth));
            }
            let growth = self.growth.get();
            let chase = target - (growth * 9.).min(MOST_LEAD);
            let (mut at, mut velocity) = (self.at.get(), self.velocity.get());
            while frames > 0. {
                let h = frames.min(1.);
                frames -= h;
                let gap = (chase - at).max(0.);
                velocity += h * ((DAMPING * velocity + STIFFNESS * gap) / MASS - velocity);
                at = (at + (velocity + growth) * h).min(target);
            }
            self.velocity.set(velocity);
            if target - at <= 0.5 {
                (target, true)
            } else {
                (at, false)
            }
        };
        self.at.set(at);
        if done {
            self.next.set(None);
            self.landed.set(Some(now));
        } else {
            self.next.set(Some(now + ui.refresh_interval()));
        }
        self.written.set(Some(at));
        self.offset.set(at);
        if done && self.following.with_untracked(|f| *f) {
            // Landed on the end: at it, as the list measures it.
            self.following.set(false);
        }
        true
    }
}

/// The window's gliding lists.
#[derive(Default)]
pub(crate) struct State {
    glides: Vec<Weak<Glide>>,
}

/// A glide for the list scrolled by `offset`, for its pass to hold.
pub(crate) fn register(ui: &Ui, offset: Signal<f32>) -> Rc<Glide> {
    let now = crate::clock::now();
    let glide = Rc::new(Glide {
        offset,
        following: ui.runtime.signal(false),
        at: Cell::new(0.),
        target: Cell::new(0.),
        velocity: Cell::new(0.),
        growth: Cell::new(0.),
        last_target: Cell::new(None),
        stepped: Cell::new(now),
        next: Cell::new(None),
        landed: Cell::new(None),
        written: Cell::new(None),
    });
    let mut state = ui.follow_glide_state().borrow_mut();
    state.glides.retain(|glide| glide.strong_count() > 0);
    state.glides.push(Rc::downgrade(&glide));
    glide
}

fn glides(ui: &Ui) -> Vec<Rc<Glide>> {
    let state = ui.follow_glide_state().borrow();
    state.glides.iter().filter_map(Weak::upgrade).collect()
}

/// When a glide next steps; none gliding, nothing.
pub(crate) fn next_step(ui: &Ui) -> Option<Instant> {
    let state = ui.follow_glide_state().borrow();
    state
        .glides
        .iter()
        .filter_map(|glide| glide.upgrade()?.next.get())
        .min()
}

/// Step what's due by `now`; whether anything moved.
pub(crate) fn advance(ui: &Ui, now: Instant) -> bool {
    if ui.follow_glide_state().borrow().glides.is_empty() {
        return false;
    }
    let mut moved = false;
    for glide in glides(ui) {
        moved |= glide.step(ui, now);
    }
    moved
}
