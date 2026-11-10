//! Effects on buses: the [`Effect`] trait an app's own DSP implements, and
//! what the mixer keeps of each. A bus runs its effects in order on its sum,
//! before its gain; master's run before its gain and the limiter.
//!
//! **Stability, in 0.x:** [`Effect`], [`Parameter`] and [`Context`], and
//! that [`Effect::process`] is called with at most the `max_block` frames
//! [`Effect::prepare`] was given, are not broken in a patch release, and in
//! a minor release only with a changelog entry saying how.
use crate::device::Format;
use crate::engine::Time;
use crate::mixer::MAX_BLOCK;
use crate::ramp::Smoothed;
use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::time::Duration;

/// Sound processing on a bus: a filter, a delay, a reverb, a compressor,
/// or an app's own. Made and [prepared](Effect::prepare) on the main
/// thread, then moved to the audio thread, where only
/// [`process`](Effect::process) runs; dropped on the main thread again
/// once removed.
///
/// ```
/// use zgx_audio::{Context, Effect, Format, Parameter};
///
/// /// A gain, in decibels, smoothed by the engine.
/// struct Trim;
/// impl Effect for Trim {
///     fn parameters(&self) -> &[Parameter] {
///         const DB: [Parameter; 1] = [Parameter::new("db", 0., -60., 24.)];
///         &DB
///     }
///     fn prepare(&mut self, _format: Format, _max_block: usize) {}
///     fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
///         for (i, [left, right]) in block.as_chunks_mut::<2>().0.iter_mut().enumerate() {
///             let gain = 10f32.powf(context.value_at(0, i) / 20.);
///             *left *= gain;
///             *right *= gain;
///         }
///     }
/// }
/// ```
pub trait Effect: Send + 'static {
    /// Its parameters, by index: what [`EffectHandle::set`] moves and
    /// [`Context`] gives back each block, smoothed. Read once, when the
    /// effect is added.
    ///
    /// [`EffectHandle::set`]: crate::EffectHandle::set
    fn parameters(&self) -> &[Parameter] {
        &[]
    }

    /// Called off the audio thread before first use, and again when the
    /// format changes: make every buffer here. `max_block` is the most
    /// frames [`process`](Effect::process) will be given.
    fn prepare(&mut self, format: Format, max_block: usize);

    /// Called on the audio thread with the bus's frames, interleaved (two
    /// channels: the mixer is stereo), to change in place: while the bus
    /// brings sound, and for the effect's [`tail`](Effect::tail) after. On
    /// silence otherwise it isn't called: silence in, silence out. What it
    /// leaves is made finite and kept within ±2^18 on the way out. No locks,
    /// allocation, freeing or system calls: a debug build of zgx-audio
    /// with [`Checked`] as the global allocator panics, naming the effect,
    /// when one allocates or frees here.
    fn process(&mut self, block: &mut [f32], context: &Context<'_>);

    /// How long sound continues after its input stops (a reverb's tail, a
    /// delay's echoes): the bus keeps running it on silence that long, and
    /// the device isn't stopped under it. Asked on the audio thread (under
    /// the same rules as `process`) every 128 frames of the clock after one
    /// that brought sound, so it can follow the effect's parameters; how
    /// long its sound outlasts the last sound in, to within those 128
    /// frames.
    fn tail(&self) -> Duration {
        Duration::ZERO
    }
}

/// One of an effect's parameters: its name, the value it starts at, and
/// the range a value set is kept to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parameter {
    pub name: &'static str,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

impl Parameter {
    pub const fn new(name: &'static str, default: f32, min: f32, max: f32) -> Parameter {
        Parameter {
            name,
            default,
            min,
            max,
        }
    }
}

/// What an effect is told with each block.
pub struct Context<'a> {
    time: Time,
    format: Format,
    frames: usize,
    params: &'a [Param],
    side: Option<f32>,
}

impl Context<'_> {
    /// The frame on the mixer's clock of the block's first.
    pub fn time(&self) -> Time {
        self.time
    }

    /// The format it was prepared for.
    pub fn format(&self) -> Format {
        self.format
    }

    /// Frames in this block.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Parameter `index`'s value at the block's first frame.
    pub fn value(&self, index: usize) -> f32 {
        self.params[index].first
    }

    /// Parameter `index`'s value at each frame, while it moves; `None`
    /// while it holds still (then every frame's is [`value`](Context::value)).
    pub fn values(&self, index: usize) -> Option<&[f32]> {
        let param = &self.params[index];
        param.moving.then(|| &param.values[..self.frames])
    }

    /// Parameter `index`'s value at frame `frame` of the block.
    pub fn value_at(&self, index: usize, frame: usize) -> f32 {
        self.values(index).map_or(self.value(index), |v| v[frame])
    }

    /// The level of the bus this effect was given as its side chain, over
    /// the block before this one, after that bus's gain: the root of its
    /// mean square. `None` for an effect added without one.
    pub fn side_chain(&self) -> Option<f32> {
        self.side
    }
}

/// A parameter as the mixer smooths it.
pub(crate) struct Param {
    pub smoothed: Smoothed,
    pub min: f32,
    pub max: f32,
    /// This block's: the first frame's value, and each frame's while it
    /// moves.
    first: f32,
    moving: bool,
    values: Box<[f32; MAX_BLOCK]>,
}

/// An effect as a bus holds it.
pub(crate) struct Slot {
    pub id: u64,
    pub effect: Box<dyn Effect>,
    /// Its type's name, for the debug checks' panics.
    #[cfg(debug_assertions)]
    pub name: &'static str,
    pub params: Box<[Param]>,
    /// The rate it was prepared for.
    pub rate: u32,
    /// The bus whose level it's given, if any.
    pub side: Option<u16>,
}

impl Slot {
    /// Made, and prepared at `rate`, on the main thread: it allocates.
    pub fn new<E: Effect>(id: u64, mut effect: E, rate: u32, side: Option<u16>) -> Slot {
        let params = effect
            .parameters()
            .iter()
            .map(|p| Param {
                smoothed: Smoothed::new(p.default.clamp(p.min, p.max)),
                min: p.min,
                max: p.max,
                first: p.default,
                moving: false,
                values: Box::new([0.; MAX_BLOCK]),
            })
            .collect();
        effect.prepare(format(rate), MAX_BLOCK);
        Slot {
            id,
            effect: Box::new(effect),
            #[cfg(debug_assertions)]
            name: std::any::type_name::<E>(),
            params,
            rate,
            side,
        }
    }

    /// Prepared again for `rate`. Off the audio thread.
    pub fn prepare(&mut self, rate: u32) {
        self.effect.prepare(format(rate), MAX_BLOCK);
        self.rate = rate;
    }

    /// Its tail now, in frames.
    pub fn tail(&self) -> u64 {
        frames(self.effect.tail(), self.rate)
    }

    /// Runs the effect on `block` (`n` interleaved stereo frames from
    /// frame `time`, and maybe more after them), its parameters moved on
    /// by `n` frames.
    pub fn process(&mut self, block: &mut [f32], n: usize, time: u64, side: Option<f32>) {
        for param in self.params.iter_mut() {
            param.first = param.smoothed.value();
            param.moving = !param.smoothed.settled();
            if param.moving {
                param.smoothed.fill(&mut param.values[..n]);
            }
        }
        let context = Context {
            time: Time(time),
            format: format(self.rate),
            frames: n,
            params: &self.params,
            side,
        };
        #[cfg(debug_assertions)]
        let before = rules::arm();
        self.effect.process(&mut block[..n * 2], &context);
        #[cfg(debug_assertions)]
        rules::disarm(before, self.name);
    }

    /// Moves its parameters on by `n` frames without running it.
    pub fn skip(&mut self, n: usize) {
        for param in self.params.iter_mut() {
            param.smoothed.advance(n);
        }
    }
}

fn format(rate: u32) -> Format {
    Format { rate, channels: 2 }
}

fn frames(duration: Duration, rate: u32) -> u64 {
    (duration.as_secs_f64() * f64::from(rate)).ceil() as u64
}

/// A global allocator that catches an effect allocating or freeing on the
/// audio thread: in a debug build of zgx-audio, the mixer arms it around
/// each [`Effect::process`] and panics, naming the effect, if anything was
/// allocated or freed meanwhile. Every call goes on to the allocator it
/// wraps; outside those stretches, and in a release build, it only passes
/// them on.
///
/// ```
/// #[global_allocator]
/// static ALLOCATOR: zgx_audio::Checked = zgx_audio::Checked(std::alloc::System);
/// ```
///
/// A lock is caught when taking it allocates (std's `Mutex` on macOS, the
/// first time), not otherwise.
pub struct Checked<A = std::alloc::System>(pub A);

thread_local! {
    /// Set on the audio thread while an effect runs.
    static ARMED: Cell<bool> = const { Cell::new(false) };
    /// Allocations and frees while armed.
    static BROKE: Cell<u64> = const { Cell::new(0) };
}

fn note() {
    // A thread-local `Cell` with a constant start and no destructor:
    // reading or writing it allocates nothing, even on its first use.
    if ARMED.get() {
        BROKE.set(BROKE.get() + 1);
    }
}

// SAFETY: every call goes to the wrapped allocator unchanged; noting it
// touches two thread-local cells, which allocate nothing.
unsafe impl<A: GlobalAlloc> GlobalAlloc for Checked<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: as the caller promised.
        unsafe { self.0.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: as the caller promised.
        unsafe { self.0.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        // SAFETY: as the caller promised.
        unsafe { self.0.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        note();
        // SAFETY: as the caller promised.
        unsafe { self.0.realloc(ptr, layout, size) }
    }
}

#[cfg(debug_assertions)]
mod rules {
    use super::{ARMED, BROKE};

    /// Arms the check; returns the count before.
    pub fn arm() -> u64 {
        ARMED.set(true);
        BROKE.get()
    }

    /// Disarms it, and panics if `effect` allocated or freed since `before`.
    pub fn disarm(before: u64, effect: &str) {
        ARMED.set(false);
        let broke = BROKE.get() - before;
        if broke > 0 {
            panic!(
                "the effect {effect} allocated or freed memory {broke} time(s) in process(), \
                 on the audio thread: make its buffers in prepare()"
            );
        }
    }
}
