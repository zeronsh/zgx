//! Devtools' tapes of wheel input (`Ui::wheel_tape`): what a window received
//! from the platform, recorded with its times and phases, and played back
//! through the window's own input path, as if received again. A trackpad's
//! fingers and momentum, a wheel's notches: resampling, the trackpad's
//! phases and elastic edges then run as they do for the platform's events,
//! and nothing reaches the desktop.
//!
//! A tape is text, an event a line: milliseconds since the first, its kind
//! (`pixel` motion in points, or wheel `lines`), its motion across and down
//! as the platform reported it, and its phase:
//!
//! ```text
//! # zgx wheel tape 1
//! 0.000 pixel 0 -2.5 started
//! 8.312 pixel 0 -6.75 moved
//! ```
//!
//! A tape may also move the pointer, for an interaction a script plays
//! across frames (a drag's moves, each in a frame of its own): `pointer`,
//! where it is from the target's centre in points, across and down, and
//! `down` (the primary button pressed there), `moved` or `up`. Recordings
//! hold wheel input only.
//!
//! ```text
//! 0 pointer 0 0 down
//! 8.3 pointer 4 0 moved
//! 16.7 pointer 8 0 up
//! ```
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use winit::event::{MouseScrollDelta, TouchPhase};

const HEADER: &str = "# zgx wheel tape 1";

/// One event of a tape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Entry {
    /// Since the tape's first event.
    pub after: Duration,
    /// Pixel motion in points (`true`), or wheel lines.
    pub pixels: bool,
    /// The wheel's motion, or the pointer's place from the target's centre.
    pub delta: (f32, f32),
    pub phase: TouchPhase,
    /// A pointer event (`delta` its place), not the wheel's.
    pub pointer: Option<Press>,
}

/// What a tape's pointer event does where it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Press {
    /// The primary button goes down.
    Down,
    Moved,
    /// The primary button comes up.
    Up,
}

/// A tape's event due, as the platform would report it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Due {
    Wheel(MouseScrollDelta, TouchPhase),
    /// The pointer at a place in the window, in points.
    Pointer((f32, f32), Press),
}

fn phase_name(phase: TouchPhase) -> &'static str {
    match phase {
        TouchPhase::Started => "started",
        TouchPhase::Moved => "moved",
        TouchPhase::Ended => "ended",
        TouchPhase::Cancelled => "cancelled",
    }
}

/// `entries` as a tape's text.
pub(crate) fn format(entries: &[Entry]) -> String {
    let mut out = String::from(HEADER);
    for e in entries {
        if let Some(press) = e.pointer {
            out.push_str(&format!(
                "\n{:.3} pointer {} {} {}",
                e.after.as_secs_f64() * 1e3,
                e.delta.0,
                e.delta.1,
                match press {
                    Press::Down => "down",
                    Press::Moved => "moved",
                    Press::Up => "up",
                }
            ));
            continue;
        }
        out.push_str(&format!(
            "\n{:.3} {} {} {} {}",
            e.after.as_secs_f64() * 1e3,
            if e.pixels { "pixel" } else { "lines" },
            e.delta.0,
            e.delta.1,
            phase_name(e.phase)
        ));
    }
    out.push('\n');
    out
}

/// A tape's events, in order; what is wrong with it, at its line.
pub(crate) fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = || {
            format!(
                "tape line {}: `{line}` is not `ms pixel|lines dx dy phase` or `ms pointer x y \
                 down|moved|up`",
                i + 1
            )
        };
        let words: Vec<&str> = line.split_whitespace().collect();
        let [ms, kind, dx, dy, phase] = words[..] else {
            return Err(bad());
        };
        let number = |w: &str| w.parse::<f32>().ok().filter(|v| v.is_finite());
        let ms = ms.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.);
        let (Some(ms), Some(dx), Some(dy)) = (ms, number(dx), number(dy)) else {
            return Err(bad());
        };
        let pixels = match kind {
            "pixel" => true,
            "lines" | "pointer" => false,
            _ => return Err(bad()),
        };
        let pointer = match (kind, phase) {
            ("pointer", "down") => Some(Press::Down),
            ("pointer", "moved") => Some(Press::Moved),
            ("pointer", "up") => Some(Press::Up),
            ("pointer", _) => return Err(bad()),
            _ => None,
        };
        let phase = match phase {
            _ if pointer.is_some() => TouchPhase::Moved,
            "started" => TouchPhase::Started,
            "moved" => TouchPhase::Moved,
            "ended" => TouchPhase::Ended,
            "cancelled" => TouchPhase::Cancelled,
            _ => return Err(bad()),
        };
        let after = Duration::from_secs_f64(ms / 1e3);
        if entries
            .last()
            .is_some_and(|last: &Entry| last.after > after)
        {
            return Err(format!("tape line {}: times go back", i + 1));
        }
        entries.push(Entry {
            after,
            pixels,
            delta: (dx, dy),
            phase,
            pointer,
        });
    }
    if entries.is_empty() {
        return Err("the tape holds no events".into());
    }
    Ok(entries)
}

/// A window's recording, and what it plays back.
#[derive(Default)]
pub(crate) struct Tape {
    /// Recording since then, what was received so far.
    recording: Option<(Option<Instant>, Vec<Entry>)>,
    /// To deliver, each at its time, at a point in the window.
    playing: VecDeque<(Instant, Entry)>,
    pub at: (f32, f32),
}

impl Tape {
    pub fn record(&mut self) {
        self.recording = Some((None, Vec::new()));
    }
    /// The recording so far, as text, and recording stops.
    pub fn stop(&mut self) -> Result<String, String> {
        match self.recording.take() {
            Some((_, entries)) if !entries.is_empty() => Ok(format(&entries)),
            Some(_) => Err("nothing was received while recording".into()),
            None => Err("not recording".into()),
        }
    }
    /// What the window received at `now`, kept if recording. `delta` is the
    /// platform's, pixels in points.
    pub fn received(
        &mut self,
        now: Instant,
        delta: MouseScrollDelta,
        scale: f64,
        phase: TouchPhase,
    ) {
        let Some((start, entries)) = &mut self.recording else {
            return;
        };
        let start = *start.get_or_insert(now);
        let (pixels, delta) = match delta {
            MouseScrollDelta::PixelDelta(p) => {
                let p = p.to_logical::<f32>(scale);
                (true, (p.x, p.y))
            }
            MouseScrollDelta::LineDelta(x, y) => (false, (x, y)),
        };
        entries.push(Entry {
            after: now.saturating_duration_since(start),
            pixels,
            delta,
            phase,
            pointer: None,
        });
    }
    /// Play `entries` from `now`, at `at`; whatever was playing stops.
    pub fn play(&mut self, now: Instant, entries: Vec<Entry>, at: (f32, f32)) {
        self.at = at;
        self.playing = entries.into_iter().map(|e| (now + e.after, e)).collect();
    }
    /// When the next event is due.
    pub fn deadline(&self) -> Option<Instant> {
        self.playing.front().map(|(at, _)| *at)
    }
    /// The next event due by `now`, as the platform would report it.
    pub fn due(&mut self, now: Instant, scale: f64) -> Option<Due> {
        if self.playing.front().is_none_or(|(at, _)| *at > now) {
            return None;
        }
        let (_, mut e) = self.playing.pop_front()?;
        // The pointer's moves due by now come as one, the last: a platform
        // hands an app one move a turn, and each move handled lays out a
        // window it resizes. A window slower than the tape would otherwise
        // pay for moves no hand's drag would have sent it.
        while e.pointer == Some(Press::Moved)
            && let Some((at, next)) = self.playing.front()
            && *at <= now
            && next.pointer == Some(Press::Moved)
        {
            e = self.playing.pop_front()?.1;
        }
        if let Some(press) = e.pointer {
            let at = (self.at.0 + e.delta.0, self.at.1 + e.delta.1);
            return Some(Due::Pointer(at, press));
        }
        let delta = if e.pixels {
            MouseScrollDelta::PixelDelta(
                winit::dpi::LogicalPosition::new(e.delta.0 as f64, e.delta.1 as f64)
                    .to_physical(scale),
            )
        } else {
            MouseScrollDelta::LineDelta(e.delta.0, e.delta.1)
        };
        Some(Due::Wheel(delta, e.phase))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;

    #[test]
    fn a_recording_plays_back_as_received() {
        let start = Instant::now();
        let mut tape = Tape::default();
        tape.record();
        let ms = |n: u64| start + Duration::from_millis(n);
        tape.received(
            ms(5),
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(0., -5.)),
            2.,
            TouchPhase::Started,
        );
        tape.received(
            ms(13),
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(1., -13.5)),
            2.,
            TouchPhase::Moved,
        );
        tape.received(
            ms(30),
            MouseScrollDelta::LineDelta(0., -1.),
            2.,
            TouchPhase::Moved,
        );
        let text = tape.stop().unwrap();
        assert_eq!(
            text,
            "# zgx wheel tape 1\n0.000 pixel 0 -2.5 started\n8.000 pixel 0.5 -6.75 moved\n25.000 lines 0 -1 moved\n"
        );
        let entries = parse(&text).unwrap();
        tape.play(start, entries, (10., 20.));
        assert_eq!(tape.deadline(), Some(start));
        // At another scale: the same points.
        assert_eq!(
            tape.due(start, 1.),
            Some(Due::Wheel(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0., -2.5)),
                TouchPhase::Started
            ))
        );
        assert_eq!(tape.due(ms(7), 1.), None, "not due yet");
        assert_eq!(
            tape.due(ms(8), 1.),
            Some(Due::Wheel(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.5, -6.75)),
                TouchPhase::Moved
            ))
        );
        assert_eq!(
            tape.due(ms(40), 1.),
            Some(Due::Wheel(
                MouseScrollDelta::LineDelta(0., -1.),
                TouchPhase::Moved
            ))
        );
        assert_eq!(tape.deadline(), None, "played");
        assert_eq!(tape.stop(), Err("not recording".into()));
    }

    #[test]
    fn a_bad_tape_says_where() {
        assert!(parse("").unwrap_err().contains("no events"));
        assert!(
            parse("0 pixel 1 2 started\nx")
                .unwrap_err()
                .contains("line 2")
        );
        assert!(
            parse("5 pixel 0 1 moved\n2 pixel 0 1 moved")
                .unwrap_err()
                .contains("back")
        );
        assert!(parse("0 pixel 0 NaN moved").is_err());
        assert!(parse("0 drag 0 1 moved").is_err());
        assert!(parse("0 pointer 0 1 started").is_err());
    }

    #[test]
    fn a_tape_moves_the_pointer_from_the_targets_centre() {
        let start = Instant::now();
        let text = "0 pointer 0 0 down\n8 pointer 4.5 -1 moved\n16 pointer 9 -2 up\n";
        let entries = parse(text).unwrap();
        assert_eq!(
            format(&entries),
            "# zgx wheel tape 1\n0.000 pointer 0 0 down\n8.000 pointer 4.5 -1 moved\n16.000 pointer 9 -2 up\n"
        );
        let mut tape = Tape::default();
        tape.play(start, entries, (100., 50.));
        let ms = |n: u64| start + Duration::from_millis(n);
        assert_eq!(
            tape.due(start, 2.),
            Some(Due::Pointer((100., 50.), Press::Down))
        );
        assert_eq!(tape.due(ms(7), 2.), None, "not due yet");
        assert_eq!(
            tape.due(ms(8), 2.),
            Some(Due::Pointer((104.5, 49.), Press::Moved))
        );
        assert_eq!(
            tape.due(ms(16), 2.),
            Some(Due::Pointer((109., 48.), Press::Up))
        );
        assert_eq!(tape.deadline(), None, "played");
    }

    #[test]
    fn moves_due_at_once_come_as_the_last_and_presses_stay() {
        let start = Instant::now();
        let text = "0 pointer 0 0 down\n8 pointer 1 0 moved\n16 pointer 2 0 moved\n\
                    24 pointer 3 0 moved\n32 pointer 4 0 up\n40 pointer 5 0 moved\n";
        let mut tape = Tape::default();
        tape.play(start, parse(text).unwrap(), (0., 0.));
        let ms = |n: u64| start + Duration::from_millis(n);
        // A window behind by three moves and a release: the press first, the
        // last move for the three, the release in its place, and a move not
        // yet due left for later.
        let now = ms(35);
        assert_eq!(tape.due(now, 2.), Some(Due::Pointer((0., 0.), Press::Down)));
        assert_eq!(
            tape.due(now, 2.),
            Some(Due::Pointer((3., 0.), Press::Moved))
        );
        assert_eq!(tape.due(now, 2.), Some(Due::Pointer((4., 0.), Press::Up)));
        assert_eq!(tape.due(now, 2.), None);
        assert_eq!(
            tape.due(ms(40), 2.),
            Some(Due::Pointer((5., 0.), Press::Moved))
        );
    }
}
