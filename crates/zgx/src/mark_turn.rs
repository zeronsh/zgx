//! A text area's line marks turning (`LineMark::rotate`: a toggle's chevron
//! opening): the app says the angle a mark ends at, and the mark eases there
//! over `motion.duration.fast`, easing out, as the web's `transition`. The
//! host wakes for each step as for any other interaction deadline
//! (`Ui::next_interaction_deadline`), so a turn costs a few small frames and
//! no frame loop, and once every turn settles nothing is scheduled: an idle
//! window does no work.
use crate::{scene::NodeId, widgets::Ui};
use std::time::{Duration, Instant};

/// How often a turning mark steps: as often as a 120 Hz display draws.
const STEP: Duration = Duration::from_micros(8_333);

/// A text area's marks' turns, by line and place among its marks.
pub(crate) type Turns = std::collections::HashMap<(usize, usize), Turn>;
/// One mark's turn (`LineMark::rotate`): from what angle, to what, since
/// when, in degrees.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Turn {
    from: f32,
    to: f32,
    start: Instant,
}

impl Turn {
    /// A mark shown at `to` from the first: at rest, no turn.
    pub(crate) fn at(to: f32, now: Instant) -> Self {
        Self {
            from: to,
            to,
            start: now,
        }
    }
    /// Where it stands at `now`, turning over `duration`, and whether it is
    /// still turning.
    pub(crate) fn angle(&self, now: Instant, duration: Duration) -> (f32, bool) {
        if self.from == self.to || duration.is_zero() {
            return (self.to, false);
        }
        let t = (now.saturating_duration_since(self.start).as_secs_f32() / duration.as_secs_f32())
            .clamp(0., 1.);
        let eased = 1. - (1. - t).powi(3);
        (self.from + (self.to - self.from) * eased, t < 1.)
    }
    /// Turn toward `to` from where it stands at `now`: a change mid-turn
    /// starts from there, not from its old end.
    pub(crate) fn toward(&mut self, to: f32, now: Instant, duration: Duration) {
        if to != self.to {
            let (here, _) = self.angle(now, duration);
            *self = Self {
                from: here,
                to,
                start: now,
            };
        }
    }
}

/// The text areas with a mark turning in a window, and when they next step.
#[derive(Default)]
pub(crate) struct State {
    turning: Vec<NodeId>,
    next: Option<Instant>,
}

/// Text area `node` has a mark still turning: drawn again one step on.
pub(crate) fn turning(state: &std::cell::RefCell<State>, node: NodeId, now: Instant) {
    let mut state = state.borrow_mut();
    if !state.turning.contains(&node) {
        state.turning.push(node);
    }
    let next = now + STEP;
    state.next = Some(state.next.map_or(next, |at| at.min(next)));
}

/// When the next turning mark steps; none while nothing turns.
pub(crate) fn next_step(ui: &Ui) -> Option<Instant> {
    ui.mark_turn_state().borrow().next
}

/// Draw again each text area whose marks were turning, due by `now`: each
/// draw steps its marks, and asks again only while one still turns. Whether
/// any was drawn.
pub(crate) fn advance(ui: &Ui, now: Instant) -> bool {
    let nodes = {
        let mut state = ui.mark_turn_state().borrow_mut();
        if state.next.is_none_or(|next| next > now) {
            return false;
        }
        state.next = None;
        std::mem::take(&mut state.turning)
    };
    let mut drawn = false;
    for node in nodes {
        if let Some(editor) = ui.editor(node) {
            editor.refresh();
            drawn = true;
        }
    }
    drawn
}
