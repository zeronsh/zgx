use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use zgx::scene::Rect;

/// Signals one frame may be written before it counts as thrashing.
pub const THRASH_WRITES: u32 = 3;

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
    /// What each phase of it took, microseconds.
    pub phases: Vec<(&'static str, f32)>,
    /// Signals written in it.
    pub writes: u32,
    /// Effects that ran in it.
    pub effects: u32,
    /// Effects run per written signal.
    pub cascade: f32,
    /// Signals written more than [`THRASH_WRITES`] times, with their counts.
    pub thrashing: Vec<(usize, u32)>,
    /// The GPU's own time for it, microseconds, where measured.
    pub gpu_micros: Option<f32>,
    /// Only the profiler's own overlay repainted.
    pub hud_only: bool,
    /// Nodes laid out and painted.
    pub layout_nodes: usize,
    pub paint_nodes: usize,
}
impl FrameRecord {
    /// Its phases' time together, microseconds.
    pub fn total_micros(&self) -> f32 {
        self.phases.iter().map(|(_, micros)| micros).sum()
    }
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
    pub phase_micros: Vec<(&'static str, f32)>,
    pub gpu_micros: Option<f32>,
    pub rss_mib: f32,
    pub idle: bool,
}
impl Summary {
    pub fn frame_micros(&self) -> f32 {
        self.phase_micros.iter().map(|(_, micros)| micros).sum()
    }
}

/// The frames kept, and what they add up to.
pub struct History {
    frames: VecDeque<FrameRecord>,
    capacity: usize,
    intervals: Vec<f32>,
    app_frames: u64,
    hud_frames: u64,
    total_writes: u64,
    total_effects: u64,
    total_thrash: u64,
    began: Instant,
    last: Option<Instant>,
    last_at: Option<Instant>,
    last_app: Option<FrameRecord>,
    rss_mib: f32,
}
/// Intervals kept for percentiles.
const PACING_SAMPLES: usize = 240;
impl History {
    pub fn new(capacity: usize, began: Instant) -> Self {
        let capacity = capacity.max(1);
        Self {
            frames: VecDeque::with_capacity(capacity.min(4096)),
            capacity,
            intervals: Vec::with_capacity(PACING_SAMPLES),
            app_frames: 0,
            hud_frames: 0,
            total_writes: 0,
            total_effects: 0,
            total_thrash: 0,
            began,
            last: None,
            last_at: None,
            last_app: None,
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
    /// The recent intervals of the app's own frames, oldest first, for a
    /// pacing graph.
    pub fn intervals(&self) -> &[f32] {
        &self.intervals
    }
    pub fn set_rss(&mut self, mib: f32) {
        self.rss_mib = mib;
    }
    /// The frame's interval from the one before it, in milliseconds.
    pub fn interval(&mut self, at: Option<Instant>) -> Option<f32> {
        let at = at?;
        let interval = self.last.map(|last| (at - last).as_secs_f32() * 1e3);
        self.last = Some(at);
        interval
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
                if self.intervals.len() == PACING_SAMPLES {
                    self.intervals.remove(0);
                }
                self.intervals.push(record.interval_ms);
            }
        }
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
        }
        self.frames.push_back(record);
    }
    pub fn clear(&mut self) {
        self.frames.clear();
        self.intervals.clear();
        self.app_frames = 0;
        self.hud_frames = 0;
        self.total_writes = 0;
        self.total_effects = 0;
        self.total_thrash = 0;
        self.began = Instant::now();
        self.last = None;
        self.last_at = None;
        self.last_app = None;
    }
    pub fn summary(&self, idle_after: Duration) -> Summary {
        let intervals = self.intervals.clone();
        let mut sorted = intervals.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let median = percentile(&sorted, 0.5);
        let dropped = sorted.iter().filter(|ms| **ms > median * 1.5).count() as u64;
        let last = self.last_app.as_ref();
        let count = sorted.len();
        let mean = intervals.iter().sum::<f32>() / count.max(1) as f32;
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
            gpu_micros: last.and_then(|frame| frame.gpu_micros),
            rss_mib: self.rss_mib,
            idle: self.last_at.is_none_or(|at| at.elapsed() > idle_after),
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
