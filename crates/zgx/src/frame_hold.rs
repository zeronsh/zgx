//! A window keeps showing the frame it has while what it is changing to
//! comes (`Ui::hold_frames`): a page opened before its content has arrived
//! would show a frame without it, then one with it, a flash between the old
//! page and the new. Held, the host draws nothing until the hold is let go
//! (`Ui::release_frames`) or runs out, and then draws the change whole, in
//! one frame. Input still goes to the window and scripts still run; only
//! what shows waits. A hold lasts at most `FRAME_HOLD_MOST`, so content
//! that never comes costs a short pause, not a frozen window.
use crate::widgets::Ui;
use std::time::{Duration, Instant};

/// The longest a window holds its frame, however long it is asked to.
pub const FRAME_HOLD_MOST: Duration = Duration::from_millis(250);

impl Ui {
    /// Show no new frame for up to `longest` (at most `FRAME_HOLD_MOST`),
    /// until `release_frames`: what changes meanwhile shows together, as
    /// the hold ends. A hold already running ends at the later of the two.
    pub fn hold_frames(&self, longest: Duration) {
        let until = Instant::now() + longest.min(FRAME_HOLD_MOST);
        let held = &self.storage.frames_held;
        if held.get().is_none_or(|was| was < until) {
            held.set(Some(until));
        }
    }
    /// End a hold (`hold_frames`): the next frame shows all that changed.
    pub fn release_frames(&self) {
        self.storage.frames_held.set(None);
    }
    /// Until when frames are held at `now`, if they are: the host draws
    /// none until then, and draws at once if it passes. A hold that has run
    /// out ends here.
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
    use crate::widgets::Ui;
    use std::time::{Duration, Instant};

    #[test]
    fn a_hold_lasts_until_released() {
        let ui = Ui::new(100., 100.);
        let now = Instant::now();
        assert_eq!(ui.frames_held(now), None, "nothing held at first");
        ui.hold_frames(Duration::from_millis(100));
        let until = ui.frames_held(now).expect("held");
        assert!(until > now && until <= Instant::now() + Duration::from_millis(100));
        ui.release_frames();
        assert_eq!(ui.frames_held(now), None, "released");
    }

    #[test]
    fn a_hold_runs_out_and_is_never_longer_than_its_most() {
        let ui = Ui::new(100., 100.);
        ui.hold_frames(Duration::from_secs(60));
        let until = ui.frames_held(Instant::now()).expect("held");
        assert!(until <= Instant::now() + FRAME_HOLD_MOST, "capped");
        // Past its end: no longer held, and the hold is gone.
        assert_eq!(ui.frames_held(until), None);
        assert_eq!(ui.frames_held(Instant::now()), None);
    }

    #[test]
    fn a_second_hold_ends_at_the_later_of_the_two() {
        let ui = Ui::new(100., 100.);
        ui.hold_frames(Duration::from_millis(200));
        let long = ui.frames_held(Instant::now()).expect("held");
        ui.hold_frames(Duration::from_millis(10));
        assert_eq!(
            ui.frames_held(Instant::now()),
            Some(long),
            "the shorter one doesn't cut it"
        );
        ui.release_frames();
        ui.hold_frames(Duration::from_millis(10));
        assert!(ui.frames_held(Instant::now()).expect("held") < long);
    }
}
