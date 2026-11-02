use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use zgx::scene::Rect;

/// Signals one frame may be written before it counts as thrashing.
pub const THRASH_WRITES: u32 = 3;
/// App frames watched before one can become the worst: the first frames of a
/// window (fonts, the first layout) are startup, not steady state.
const WORST_WARMUP: u64 = 8;
pub const WAIT: &str = "wait";

/// What one phase of a frame took: its microseconds, and the instructions it
/// retired where the engine counts them (0 where not: a count costs a system
/// call, so nested phases are timed only).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhaseCost {
    pub name: &'static str,
    pub micros: f32,
    pub instructions: u64,
}

/// An effect that ran in a frame, by its cost: what its maker named it
/// (`app.luau:30 effect`, `@12 binding`) and how long all its runs in the
/// frame took, their own work included. The signal whose write queued it,
/// where known (its address, `Signal::addr`).
#[derive(Clone, Debug, PartialEq)]
pub struct SlowRun {
    pub name: String,
    pub micros: f32,
    pub cause: Option<usize>,
}

/// One finished frame, as the profiler saw it.
#[derive(Clone, Debug, Default)]
pub struct FrameRecord {
    /// Milliseconds since the profiler attached.
    pub at_ms: f64,
    /// Time since the frame before it, milliseconds.
    pub interval_ms: f32,
    /// Areas the frame repainted, logical points.
    pub damage: Vec<Rect>,
    /// Their total area, square logical points.
    pub damage_area: f32,
    /// What each phase of it took.
    pub phases: Vec<PhaseCost>,
    /// Instructions its phases retired, where counted.
    pub instructions: u64,
    /// Signals written in it.
    pub writes: u32,
    /// Effects that ran in it.
    pub effects: u32,
    /// Effects run per written signal.
    pub cascade: f32,
    /// Signals written more than [`THRASH_WRITES`] times, with their counts.
    pub thrashing: Vec<(usize, u32)>,
    /// The costliest named effects that ran in it.
    pub slow: Vec<SlowRun>,
    /// Why it did what it did, where the engine said (`zgx::trace`'s notes).
    pub notes: Vec<String>,
    /// The GPU's own time for it, microseconds, where measured.
    pub gpu_micros: Option<f32>,
    /// Only the profiler's own overlay repainted.
    pub hud_only: bool,
    /// Nodes laid out, painted and composited again.
    pub layout_nodes: usize,
    pub paint_nodes: usize,
    pub composite_nodes: usize,
    /// It repainted the whole window rather than its damage.
    pub full: bool,
    pub rss_mib: f32,
}
impl FrameRecord {
    /// Its phases' time together, microseconds.
    pub fn total_micros(&self) -> f32 {
        self.phases.iter().map(|phase| phase.micros).sum()
    }
    /// The phase that took the most, if it had any.
    pub fn dominant(&self) -> Option<&PhaseCost> {
        self.phases
            .iter()
            .filter(|phase| phase.name != WAIT)
            .max_by(|a, b| a.micros.total_cmp(&b.micros))
    }
}

/// One bar of the pacing graph: an app frame's interval, and the phase that
/// took the most of it, so a spike reads as what it was made of.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bar {
    pub ms: f32,
    pub dominant: Option<&'static str>,
}

/// The worst frame kept since the profiler attached or was cleared: the app
/// frame that arrived latest relative to its own recording — a frame late by
/// the profiler's dropped definition (more than 1.5× the median), never a
/// steady frame, however slow the app is on the whole.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorstFrame {
    pub at_ms: f64,
    pub interval_ms: f32,
    pub phases: Vec<PhaseCost>,
    pub instructions: u64,
    pub slow: Vec<SlowRun>,
    pub notes: Vec<String>,
}
impl WorstFrame {
    /// The phase that took the most of it, if it had any.
    pub fn dominant(&self) -> Option<&PhaseCost> {
        self.phases
            .iter()
            .filter(|phase| phase.name != WAIT)
            .max_by(|a, b| a.micros.total_cmp(&b.micros))
    }
}

/// One named stretch of a capture: the frames between a marker and the next
/// (or the end), and what they add up to. A marker begins its segment, so two
/// markers name two scenarios to compare.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Segment {
    pub name: String,
    /// When it began and ended, milliseconds since the profiler attached.
    pub from_ms: f64,
    pub to_ms: f64,
    /// The app frames in it.
    pub frames: u64,
    pub fps: f64,
    pub p50_ms: f32,
    pub p95_ms: f32,
    /// Mean damaged area a frame, square logical points.
    pub damage_area: f32,
    /// Mean cost a frame, per phase that ran in the segment.
    pub phase_micros: Vec<PhaseCost>,
}

/// What the profiler reports: pacing, the last frame's split, and totals.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub app_frames: u64,
    pub hud_frames: u64,
    pub seconds: f64,
    pub fps: f64,
    pub p50_ms: f32,
    pub p95_ms: f32,
    pub p99_ms: f32,
    pub dropped: u64,
    pub damage_rects: usize,
    pub damage_area: f32,
    pub writes: u32,
    pub effects: u32,
    pub cascade: f32,
    pub thrash: u32,
    pub total_writes: u64,
    pub total_effects: u64,
    pub total_thrash: u64,
    pub phase_micros: Vec<PhaseCost>,
    /// Instructions the last app frame's phases retired, where counted.
    pub instructions: u64,
    /// The costliest named effects of the last app frame.
    pub slow: Vec<SlowRun>,
    /// Why the last app frame did what it did.
    pub notes: Vec<String>,
    /// The last app frame repainted the whole window.
    pub full: bool,
    pub worst: Option<WorstFrame>,
    pub gpu_micros: Option<f32>,
    pub rss_mib: f32,
    pub idle: bool,
}
impl Summary {
    pub fn frame_micros(&self) -> f32 {
        self.phase_micros
            .iter()
            .filter(|phase| phase.name != WAIT)
            .map(|phase| phase.micros)
            .sum()
    }
}

/// The frames kept, and what they add up to.
pub struct History {
    frames: VecDeque<FrameRecord>,
    capacity: usize,
    bars: Vec<Bar>,
    app_frames: u64,
    hud_frames: u64,
    total_writes: u64,
    total_effects: u64,
    total_thrash: u64,
    began: Instant,
    last: Option<Instant>,
    last_at: Option<Instant>,
    last_app: Option<FrameRecord>,
    worst: Option<WorstFrame>,
    rss_mib: f32,
}
/// Graph bars and pacing samples kept.
const PACING_SAMPLES: usize = 240;
impl History {
    pub fn new(capacity: usize, began: Instant) -> Self {
        let capacity = capacity.max(1);
        Self {
            frames: VecDeque::with_capacity(capacity.min(4096)),
            capacity,
            bars: Vec::with_capacity(PACING_SAMPLES),
            app_frames: 0,
            hud_frames: 0,
            total_writes: 0,
            total_effects: 0,
            total_thrash: 0,
            began,
            last: None,
            last_at: None,
            last_app: None,
            worst: None,
            rss_mib: 0.,
        }
    }
    pub fn frames(&self) -> &VecDeque<FrameRecord> {
        &self.frames
    }
    /// The last frame the app drew (not the overlay's own), for the split the
    /// HUD shows.
    pub fn last_app(&self) -> Option<&FrameRecord> {
        self.last_app.as_ref()
    }
    /// The recent bars of the app's own frames, oldest first, for a pacing
    /// graph.
    pub fn bars(&self) -> &[Bar] {
        &self.bars
    }
    /// The worst app frame kept, if one arrived late enough to count.
    pub fn worst(&self) -> Option<&WorstFrame> {
        self.worst.as_ref()
    }
    pub fn set_rss(&mut self, mib: f32) {
        self.rss_mib = mib;
    }
    pub fn rss_mib(&self) -> f32 {
        self.rss_mib
    }
    /// The frame's interval from the one before it, in milliseconds.
    pub fn interval(&mut self, at: Option<Instant>, idle_after: Duration) -> Option<f32> {
        let at = at?;
        let gap = self.last.replace(at).map(|last| at - last)?;
        (gap <= idle_after).then_some(gap.as_secs_f32() * 1e3)
    }
    pub fn push(&mut self, record: FrameRecord, at: Instant) {
        self.last_at = Some(at);
        self.total_writes += u64::from(record.writes);
        self.total_effects += u64::from(record.effects);
        self.total_thrash += record.thrashing.len() as u64;
        if record.hud_only {
            self.hud_frames += 1;
        } else {
            self.app_frames += 1;
            self.last_app = Some(record.clone());
            if record.interval_ms.is_finite() && record.interval_ms > 0. {
                if self.bars.len() == PACING_SAMPLES {
                    self.bars.remove(0);
                }
                self.bars.push(Bar {
                    ms: record.interval_ms,
                    dominant: record.dominant().map(|phase| phase.name),
                });
            }
            if self.app_frames > WORST_WARMUP && record.interval_ms >= 2. {
                self.note_worst(&record);
            }
        }
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(record);
    }
    /// Keep `record` as the worst if it arrived later than the one kept, and
    /// (for a first) late by the profiler's dropped definition.
    fn note_worst(&mut self, record: &FrameRecord) {
        if self
            .worst
            .as_ref()
            .is_some_and(|worst| worst.interval_ms >= record.interval_ms)
        {
            return;
        }
        if self.worst.is_none() {
            let mut sorted: Vec<f32> = self.bars.iter().map(|bar| bar.ms).collect();
            sorted.sort_by(|a, b| a.total_cmp(b));
            if record.interval_ms <= percentile(&sorted, 0.5) * 1.5 {
                return;
            }
        }
        self.worst = Some(WorstFrame {
            at_ms: record.at_ms,
            interval_ms: record.interval_ms,
            phases: record.phases.clone(),
            instructions: record.instructions,
            slow: record.slow.clone(),
            notes: record.notes.clone(),
        });
    }
    pub fn clear(&mut self) {
        self.frames.clear();
        self.bars.clear();
        self.app_frames = 0;
        self.hud_frames = 0;
        self.total_writes = 0;
        self.total_effects = 0;
        self.total_thrash = 0;
        self.began = Instant::now();
        self.last = None;
        self.last_at = None;
        self.last_app = None;
        self.worst = None;
    }
    pub fn summary(&self, idle_after: Duration) -> Summary {
        let mut sorted: Vec<f32> = self.bars.iter().map(|bar| bar.ms).collect();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let median = percentile(&sorted, 0.5);
        let dropped = sorted.iter().filter(|ms| **ms > median * 1.5).count() as u64;
        let last = self.last_app.as_ref();
        let count = sorted.len();
        let mean = self.bars.iter().map(|bar| bar.ms).sum::<f32>() / count.max(1) as f32;
        Summary {
            app_frames: self.app_frames,
            hud_frames: self.hud_frames,
            seconds: (Instant::now() - self.began).as_secs_f64(),
            fps: if mean > 0. {
                1000. / f64::from(mean)
            } else {
                0.
            },
            p50_ms: median,
            p95_ms: percentile(&sorted, 0.95),
            p99_ms: percentile(&sorted, 0.99),
            dropped,
            damage_rects: last.map_or(0, |frame| frame.damage.len()),
            damage_area: last.map_or(0., |frame| frame.damage_area),
            writes: last.map_or(0, |frame| frame.writes),
            effects: last.map_or(0, |frame| frame.effects),
            cascade: last.map_or(0., |frame| frame.cascade),
            thrash: last.map_or(0, |frame| frame.thrashing.len() as u32),
            total_writes: self.total_writes,
            total_effects: self.total_effects,
            total_thrash: self.total_thrash,
            phase_micros: last.map_or_else(Vec::new, |frame| frame.phases.clone()),
            instructions: last.map_or(0, |frame| frame.instructions),
            slow: last.map_or_else(Vec::new, |frame| frame.slow.clone()),
            notes: last.map_or_else(Vec::new, |frame| frame.notes.clone()),
            full: last.is_some_and(|frame| frame.full),
            worst: self.worst.clone(),
            gpu_micros: last.and_then(|frame| frame.gpu_micros),
            rss_mib: self.rss_mib,
            idle: self.last_at.is_none_or(|at| at.elapsed() > idle_after),
        }
    }
    /// The marker-delimited stretches of the recording, in order: each marker
    /// begins a segment that lasts until the next marker or the last frame.
    /// The stretch before the first marker is nameless.
    pub fn segments(&self, markers: &[(f64, String)]) -> Vec<Segment> {
        let Some(end) = self.frames.back().map(|frame| frame.at_ms) else {
            return Vec::new();
        };
        if markers.is_empty() {
            return Vec::new();
        }
        let mut starts: Vec<(f64, &str)> = Vec::with_capacity(markers.len() + 1);
        starts.push((0., ""));
        for (at, name) in markers {
            starts.push((*at, name.as_str()));
        }
        let mut segments: Vec<Segment> = starts
            .iter()
            .enumerate()
            .filter_map(|(i, &(from, name))| {
                let to = starts.get(i + 1).map_or(end, |next| next.0.min(end));
                (to > from).then(|| Segment {
                    name: name.to_string(),
                    from_ms: from,
                    to_ms: to,
                    ..Default::default()
                })
            })
            .collect();
        let last = segments.len().saturating_sub(1);
        let mut intervals: Vec<Vec<f32>> = vec![Vec::new(); segments.len()];
        for frame in self.frames.iter().filter(|frame| !frame.hud_only) {
            let inside = |i: usize| {
                frame.at_ms >= segments[i].from_ms
                    && (frame.at_ms < segments[i].to_ms
                        || (i == last && frame.at_ms <= segments[i].to_ms))
            };
            let Some(i) = (0..segments.len()).find(|&i| inside(i)) else {
                continue;
            };
            let segment = &mut segments[i];
            segment.frames += 1;
            segment.damage_area += frame.damage_area;
            collect(&mut segment.phase_micros, frame);
            if frame.interval_ms.is_finite() && frame.interval_ms > 0. {
                intervals[i].push(frame.interval_ms);
            }
        }
        for (i, segment) in segments.iter_mut().enumerate() {
            let mut sorted = intervals[i].clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            if segment.frames > 0 {
                segment.damage_area /= segment.frames as f32;
                let frames = segment.frames as f32;
                for phase in &mut segment.phase_micros {
                    phase.micros /= frames;
                    phase.instructions = (phase.instructions as f32 / frames) as u64;
                }
            }
            segment
                .phase_micros
                .sort_by(|a, b| b.micros.total_cmp(&a.micros));
            segment.p50_ms = percentile(&sorted, 0.5);
            segment.p95_ms = percentile(&sorted, 0.95);
            let mean = sorted.iter().sum::<f32>() / sorted.len().max(1) as f32;
            segment.fps = if mean > 0. {
                1000. / f64::from(mean)
            } else {
                0.
            };
        }
        segments
    }
}

fn collect(phases: &mut Vec<PhaseCost>, frame: &FrameRecord) {
    for phase in &frame.phases {
        match phases.iter_mut().find(|known| known.name == phase.name) {
            Some(total) => {
                total.micros += phase.micros;
                total.instructions += phase.instructions;
            }
            None => phases.push(*phase),
        }
    }
}

/// The value at `p` of an ascending slice, milliseconds.
pub fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.;
    }
    let index = ((sorted.len() - 1) as f32 * p.clamp(0., 1.)).round() as usize;
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(at_ms: f64, interval_ms: f32, phases: &[(&'static str, f32, u64)]) -> FrameRecord {
        FrameRecord {
            at_ms,
            interval_ms,
            phases: phases
                .iter()
                .map(|&(name, micros, instructions)| PhaseCost {
                    name,
                    micros,
                    instructions,
                })
                .collect(),
            instructions: phases.iter().map(|&(_, _, i)| i).sum(),
            ..Default::default()
        }
    }

    fn logged(frames: Vec<FrameRecord>) -> History {
        let began = Instant::now() - Duration::from_secs(10);
        let mut history = History::new(4096, began);
        for frame in frames {
            let at = began + Duration::from_secs_f64(frame.at_ms / 1e3);
            history.push(frame, at);
        }
        history
    }

    #[test]
    fn bars_carry_their_dominant_phase() {
        let history = logged(vec![frame(
            10.,
            8.,
            &[("layout", 5000., 90_000), ("paint", 900., 10_000)],
        )]);
        assert_eq!(
            history.bars(),
            &[Bar {
                ms: 8.,
                dominant: Some("layout")
            }]
        );
        assert_eq!(history.last_app().unwrap().instructions, 100_000);
        assert_eq!(
            history.last_app().unwrap().dominant().unwrap().name,
            "layout"
        );
    }

    #[test]
    fn worst_is_a_late_frame_not_a_steady_one() {
        let mut frames: Vec<FrameRecord> = (0..10)
            .map(|i| frame(i as f64, 16., &[("paint", 300., 0)]))
            .collect();
        frames.push(frame(10., 40., &[("layout", 20_000., 0)]));
        frames.push(frame(11., 16., &[("paint", 300., 0)]));
        let history = logged(frames);
        let worst = history.worst().expect("a late worst frame");
        assert_eq!(worst.interval_ms, 40.);
        assert_eq!(worst.dominant().unwrap().name, "layout");

        let steady = logged((0..10).map(|i| frame(i as f64, 16., &[])).collect());
        assert!(steady.worst().is_none());

        let mut reset = logged((0..12).map(|i| frame(i as f64, 16., &[])).collect());
        reset.clear();
        assert!(reset.worst().is_none());
    }

    #[test]
    fn a_later_late_frame_replaces_the_worst() {
        let mut frames: Vec<FrameRecord> = (0..10)
            .map(|i| frame(i as f64, 16., &[("paint", 300., 0)]))
            .collect();
        frames.push(frame(10., 40., &[("paint", 300., 0)]));
        frames.extend((11..20).map(|i| frame(i as f64, 16., &[("paint", 300., 0)])));
        frames.push(frame(20., 50., &[("text", 5_000., 0)]));
        let history = logged(frames);
        let worst = history.worst().expect("a worst frame");
        assert_eq!(worst.interval_ms, 50.);
        assert_eq!(worst.dominant().unwrap().name, "text");
        assert_eq!(worst.at_ms, 20.);
    }

    #[test]
    fn segments_aggregate_the_frames_between_markers() {
        let mut frames = Vec::new();
        for i in 0..20 {
            let heavy = i < 10;
            frames.push(frame(
                i as f64,
                if heavy { 20. } else { 5. },
                &[("paint", if heavy { 2000. } else { 400. }, 100)],
            ));
        }
        let history = logged(frames);
        let markers = vec![(10., "scenario: on".into())];
        let segments = history.segments(&markers);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].name, "");
        assert_eq!(segments[0].frames, 10);
        assert_eq!(segments[0].p50_ms, 20.);
        assert!((segments[0].fps - 50.).abs() < 1.);
        assert_eq!(segments[0].phase_micros[0].name, "paint");
        assert_eq!(segments[0].phase_micros[0].micros, 2000.);
        assert_eq!(segments[1].name, "scenario: on");
        assert_eq!(segments[1].frames, 10);
        assert_eq!(segments[1].p50_ms, 5.);
        assert!((segments[1].fps - 200.).abs() < 1.);
        assert_eq!(segments[1].phase_micros[0].micros, 400.);
        assert_eq!(segments[1].phase_micros[0].instructions, 100);
    }

    #[test]
    fn segments_without_frames_or_markers_are_empty() {
        let history = History::new(64, Instant::now());
        assert!(history.segments(&[(10., "a".into())]).is_empty());
        let history = logged(vec![frame(1., 16., &[])]);
        assert!(history.segments(&[]).is_empty());
    }

    #[test]
    fn segments_skip_the_overlays_own_frames() {
        let mut overlay = frame(0., 16., &[("paint", 2000., 0)]);
        overlay.hud_only = true;
        let history = logged(vec![overlay, frame(1., 16., &[("paint", 400., 0)])]);
        let segments = history.segments(&[(0., "a".into())]);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].frames, 1);
        assert_eq!(segments[0].phase_micros[0].micros, 400.);
    }
}
