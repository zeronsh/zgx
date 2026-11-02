//! Animation data and a monotonic, seekable frame timeline: its frames
//! decoded and kept, or, for one too large to keep, decoded as they come
//! (`Animation::streamed`).
use crate::image::ImageData;
use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::Duration,
};
/// How many times an [`Animation`] plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopCount {
    /// Forever.
    Infinite,
    /// This many times, then it holds its last frame.
    Finite(NonZeroU32),
}
impl LoopCount {
    /// Play once.
    pub const ONCE: Self = Self::Finite(NonZeroU32::MIN);
}
/// One frame of an [`Animation`].
#[derive(Clone, Debug)]
pub struct Frame {
    /// Its decoded pixels.
    pub image: Arc<ImageData>,
    /// How long it shows.
    pub duration: Duration,
}
/// A streamed animation's frames, decoded in order as they are asked for
/// (`Animation::streamed`): a GIF's, each composited over the last.
pub trait FrameStream: Send {
    /// Back to before the first frame.
    fn restart(&mut self) -> Result<(), String>;
    /// The next frame's pixels, in order; `None` if it can't be decoded.
    fn next(&mut self) -> Option<Arc<ImageData>>;
    /// The bytes it holds for good (its file, its canvas), for budgets.
    fn bytes(&self) -> usize;
}
/// Where a streamed animation's frames come from, and the one it decoded
/// last.
struct Stream {
    source: Box<dyn FrameStream>,
    /// The index the next `source.next` gives.
    at: usize,
    last: Option<(usize, Arc<ImageData>)>,
}
/// An animated image's frames and how many times it plays.
pub struct Animation {
    /// Every frame, decoded; a streamed one's first alone.
    frames: Arc<[Frame]>,
    ends: Vec<Duration>,
    duration: Duration,
    loops: LoopCount,
    stream: Option<Mutex<Stream>>,
}
impl std::fmt::Debug for Animation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Animation")
            .field("frames", &self.len())
            .field("duration", &self.duration)
            .field("loops", &self.loops)
            .field("streamed", &self.stream.is_some())
            .finish()
    }
}
/// The most pixels an animation's decoded frames may hold between them.
const MAX_DECODED: usize = 64 * 1024 * 1024;
impl Animation {
    /// `frames`, played `loops` times. Errs unless there are 1 to 4096
    /// frames, all one size, each shown for 10 ms to a day, with at most
    /// 64 MiB of pixels between them.
    pub fn new(frames: Vec<Frame>, loops: LoopCount) -> Result<Self, &'static str> {
        if frames.is_empty() || frames.len() > 4096 {
            return Err("animation requires 1..=4096 frames");
        }
        let size = (frames[0].image.width(), frames[0].image.height());
        let mut bytes = 0usize;
        for frame in &frames {
            if (frame.image.width(), frame.image.height()) != size {
                return Err("animation frame sizes must match");
            }
            bytes = bytes
                .checked_add(frame.image.pixels().len())
                .ok_or("animation size overflow")?;
            if bytes > MAX_DECODED {
                return Err("decoded animation exceeds 64 MiB");
            }
        }
        let (ends, duration) = timeline(frames.iter().map(|frame| frame.duration))?;
        Ok(Self {
            frames: frames.into(),
            ends,
            duration,
            loops,
            stream: None,
        })
    }
    /// An animation too large to keep decoded: `first`, its first frame's
    /// pixels, and each frame after decoded from `source` as it comes, in
    /// order (back to the start for an earlier one), only the last kept.
    /// `durations`: each frame's, the first's included; 1 to 4096 of them.
    pub fn streamed(
        first: Arc<ImageData>,
        durations: Vec<Duration>,
        loops: LoopCount,
        source: Box<dyn FrameStream>,
    ) -> Result<Self, &'static str> {
        if durations.is_empty() || durations.len() > 4096 {
            return Err("animation requires 1..=4096 frames");
        }
        let (ends, duration) = timeline(durations.iter().copied())?;
        Ok(Self {
            frames: Arc::new([Frame {
                image: first,
                duration: durations[0],
            }]),
            ends,
            duration,
            loops,
            stream: Some(Mutex::new(Stream {
                source,
                at: 0,
                last: None,
            })),
        })
    }
    /// Its frames, decoded and kept, in order; a streamed animation's
    /// first alone (`frame` gives any).
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }
    /// How many frames it has.
    pub fn len(&self) -> usize {
        self.ends.len()
    }
    /// Whether it has no frames: never (`new`, `streamed`).
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }
    /// Frame `index`'s pixels: kept, or a streamed animation's decoded now
    /// (the frames before it decoded on the way, from the start if it is
    /// earlier than the last; the first frame's if decoding fails).
    pub fn frame(&self, index: usize) -> Arc<ImageData> {
        let Some(stream) = &self.stream else {
            return self.frames[index.min(self.frames.len() - 1)].image.clone();
        };
        let first = || self.frames[0].image.clone();
        if index == 0 {
            return first();
        }
        let mut stream = stream.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((at, image)) = &stream.last
            && *at == index
        {
            return image.clone();
        }
        // The frame it holds goes before the next is decoded: one at a time.
        stream.last = None;
        if index < stream.at {
            if stream.source.restart().is_err() {
                return first();
            }
            stream.at = 0;
        }
        let mut image = None;
        while stream.at <= index {
            image = stream.source.next();
            stream.at += 1;
            if image.is_none() {
                break;
            }
        }
        match image {
            Some(image) => {
                stream.last = Some((index, image.clone()));
                image
            }
            None => first(),
        }
    }
    /// Whether its frames are decoded as they come (`streamed`).
    pub fn is_streamed(&self) -> bool {
        self.stream.is_some()
    }
    /// The bytes it holds beyond its first frame's, for budgets: its other
    /// decoded frames, or a streamed one's stream (its file and canvas) and
    /// the frame it decoded last.
    pub fn bytes(&self) -> usize {
        let kept: usize = self.frames.iter().skip(1).map(|f| f.image.bytes()).sum();
        kept + self.stream.as_ref().map_or(0, |stream| {
            let stream = stream.lock().unwrap_or_else(|p| p.into_inner());
            stream.source.bytes() + stream.last.as_ref().map_or(0, |(_, i)| i.bytes())
        })
    }
    /// How many times it plays.
    pub fn loop_count(&self) -> LoopCount {
        self.loops
    }
    /// One play through every frame: their durations added up.
    pub fn cycle_duration(&self) -> Duration {
        self.duration
    }
    /// Frame and time until its next boundary. None means finite playback ended
    /// (last frame retained) or the image has only one frame.
    pub fn at(&self, elapsed: Duration) -> (usize, Option<Duration>) {
        let nanos = self.duration.as_nanos();
        if let LoopCount::Finite(count) = self.loops
            && elapsed.as_nanos() >= nanos * u128::from(count.get())
        {
            return (self.len() - 1, None);
        }
        if self.len() == 1 {
            return (0, None);
        }
        let local = elapsed.as_nanos() % nanos;
        let index = self.ends.partition_point(|end| end.as_nanos() <= local);
        let remaining = self.ends[index].as_nanos() - local;
        (
            index,
            Some(Duration::new(
                (remaining / 1_000_000_000) as u64,
                (remaining % 1_000_000_000) as u32,
            )),
        )
    }
}
/// Each frame's end, and the whole: each duration 10 ms to a day.
fn timeline(
    durations: impl Iterator<Item = Duration>,
) -> Result<(Vec<Duration>, Duration), &'static str> {
    let mut duration = Duration::ZERO;
    let mut ends = Vec::new();
    for frame in durations {
        if frame < Duration::from_millis(10) || frame > Duration::from_secs(86400) {
            return Err("frame duration must be between 10 ms and one day");
        }
        duration = duration
            .checked_add(frame)
            .ok_or("animation duration overflow")?;
        ends.push(duration);
    }
    Ok((ends, duration))
}
