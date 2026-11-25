//! A window keeps showing the frame it has while what it is changing to
//! comes (`Ui::hold_frames`): a page opened before its content has arrived
//! would show a frame without it, then one with it, a flash between the old
//! page and the new. Held, the host draws nothing until the hold is let go
//! (`Ui::release_frames`) or runs out, and then draws the change whole, in
//! one frame. Input still goes to the window and scripts still run; only
//! what shows waits. Holds last at most `FRAME_HOLD_MOST` from the first one
//! since a frame was drawn, however often one is asked for again, so content
//! that never comes costs a short pause, not a frozen window. A window that
//! has drawn no frame yet has none to keep showing, and is never held.
//!
//! What the window is doing to itself isn't held back: resized meanwhile, it
//! shows its last frame at the new size (macOS stretches it) until the hold
//! ends.
use crate::time::Instant;
use crate::widgets::Ui;
use std::time::Duration;

/// The longest a window holds its frame, however long it is asked to, and
/// however often, until it draws one.
#[doc(hidden)]
pub const FRAME_HOLD_MOST: Duration = Duration::from_millis(250);

impl Ui {
    /// Show no new frame for up to `longest`, until `release_frames`: what
    /// changes meanwhile shows together, as the hold ends. A hold already
    /// running ends at the later of the two, and none runs past
    /// `FRAME_HOLD_MOST` after the first since a frame was drawn
    /// (`held_frame_drawn`): asked again and again, or let go and asked
    /// again, the window still shows what changed by then. Does nothing
    /// until the window's first frame is drawn. A window resized while held
    /// shows its last frame at its new size until the hold ends.
    #[doc(hidden)]
    pub fn hold_frames(&self, longest: Duration) {
        if !self.storage.frame_drawn.get() {
            return;
        }
        let now = crate::clock::now();
        let since = self.storage.held_since.get().unwrap_or(now);
        let until = (now + longest.min(FRAME_HOLD_MOST)).min(since + FRAME_HOLD_MOST);
        if until <= now {
            return;
        }
        self.storage.held_since.set(Some(since));
        let held = &self.storage.frames_held;
        if held.get().is_none_or(|was| was < until) {
            held.set(Some(until));
        }
    }
    /// End a hold (`hold_frames`): the next frame shows all that changed.
    #[doc(hidden)]
    pub fn release_frames(&self) {
        self.storage.frames_held.set(None);
    }
    /// The host drew a frame, or had none to draw: what was held back shows,
    /// and a hold asked for from here has `FRAME_HOLD_MOST` of its own.
    #[doc(hidden)]
    pub fn held_frame_drawn(&self) {
        self.storage.frame_drawn.set(true);
        self.storage.held_since.set(None);
    }
    /// Until when frames are held at `now`, if they are: the host draws
    /// none until then, and draws at once if it passes. A hold that has run
    /// out ends here.
    #[doc(hidden)]
    pub fn frames_held(&self, now: Instant) -> Option<Instant> {
        let held = &self.storage.frames_held;
        match held.get() {
            Some(until) if until > now => Some(until),
            Some(_) => {
                held.set(None);
                None
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FRAME_HOLD_MOST;
    use crate::clock::VirtualClock;
    use crate::widgets::Ui;
    use std::time::Duration;

    const MS: Duration = Duration::from_millis(1);
    fn at(clock: &VirtualClock, ms: u32) {
        clock.advance(ms * MS, || {});
    }
    /// A window that has drawn its first frame.
    fn drawn() -> Ui {
        let ui = Ui::new(100., 100.);
        ui.held_frame_drawn();
        ui
    }

    #[test]
    fn a_hold_before_the_first_frame_does_nothing() {
        let clock = VirtualClock::new();
        let ui = Ui::new(100., 100.);
        ui.hold_frames(FRAME_HOLD_MOST);
        assert_eq!(ui.frames_held(clock.now()), None, "no frame to keep");
        // Nor does it use up the time of the first hold that counts.
        at(&clock, 200);
        ui.held_frame_drawn();
        assert_eq!(ui.frames_held(clock.now()), None, "and none is owed");
        ui.hold_frames(FRAME_HOLD_MOST);
        assert_eq!(
            ui.frames_held(clock.now()),
            Some(clock.now() + FRAME_HOLD_MOST)
        );
    }

    #[test]
    fn a_hold_lasts_until_released() {
        let clock = VirtualClock::new();
        let ui = drawn();
        assert_eq!(ui.frames_held(clock.now()), None, "nothing held at first");
        ui.hold_frames(100 * MS);
        assert_eq!(ui.frames_held(clock.now()), Some(clock.start() + 100 * MS));
        ui.release_frames();
        assert_eq!(ui.frames_held(clock.now()), None, "released");
    }

    #[test]
    fn a_hold_runs_out_and_is_never_longer_than_its_most() {
        let clock = VirtualClock::new();
        let ui = drawn();
        ui.hold_frames(Duration::from_secs(60));
        let until = clock.start() + FRAME_HOLD_MOST;
        assert_eq!(ui.frames_held(clock.now()), Some(until), "capped");
        // At its end: no longer held, and the hold is gone.
        at(&clock, 250);
        assert_eq!(ui.frames_held(until), None);
        assert_eq!(ui.frames_held(clock.start()), None);
    }

    #[test]
    fn a_second_hold_ends_at_the_later_of_the_two() {
        let clock = VirtualClock::new();
        let ui = drawn();
        ui.hold_frames(200 * MS);
        let long = clock.start() + 200 * MS;
        ui.hold_frames(10 * MS);
        assert_eq!(
            ui.frames_held(clock.now()),
            Some(long),
            "the shorter one doesn't cut it"
        );
        ui.release_frames();
        ui.hold_frames(10 * MS);
        assert!(ui.frames_held(clock.now()).expect("held") < long);
    }

    #[test]
    fn holds_asked_for_again_and_again_still_end() {
        for release in [false, true] {
            let clock = VirtualClock::new();
            let ui = drawn();
            let most = clock.start() + FRAME_HOLD_MOST;
            for ms in (0..1000).step_by(10) {
                at(&clock, ms);
                if release {
                    ui.release_frames();
                }
                ui.hold_frames(FRAME_HOLD_MOST);
                let held = ui.frames_held(clock.now());
                assert_eq!(held, (ms < 250).then_some(most), "at {ms} ms");
            }
            // A frame drawn: the next hold has its whole time.
            ui.held_frame_drawn();
            ui.hold_frames(FRAME_HOLD_MOST);
            assert_eq!(
                ui.frames_held(clock.now()),
                Some(clock.now() + FRAME_HOLD_MOST)
            );
        }
    }
}
