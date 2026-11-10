//! What an effect costs the mixer, in instructions retired on this thread
//! (macOS; elsewhere, nanoseconds) for each frame of each instance.
//!
//! `cargo run --release -p zgx-audio --example effect_cost -- [effect] [instances]`
//!
//! Each arm renders 10 seconds offline in blocks of 512 frames: a looping
//! stereo noise on each of `instances` buses (16 by default), each bus with
//! one instance of the effect, and prints the least of 5 runs interleaved
//! with a control of the same scene without the effects, which is
//! subtracted. Every arm checks that each instance ran on every block.
//! Effects: `none` (an effect that does nothing: what running one costs),
//! `gain`, `lowpass`, `lowpass24`, `highpass`, `peaking`, `delay`, `reverb`, and
//! `sweep` (a low-pass whose cutoff moves the whole time).
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use zgx_audio::effects::{Delay, HighPass, LowPass, Peaking, Reverb, Slope};
use zgx_audio::{Audio, Bus, Config, Context, Effect, EffectHandle, Format, Play, Ramp, Sound};

const SECONDS: usize = 10;
const BLOCK: usize = 512;

#[cfg(target_os = "macos")]
fn count() -> u64 {
    unsafe extern "C" {
        fn thread_selfcounts(kind: i32, buffer: *mut u64, size: usize) -> i32;
    }
    let mut counts = [0u64; 2];
    // SAFETY: kind 1 (THSC_CPI) fills two u64s: instructions, then cycles.
    let ok = unsafe { thread_selfcounts(1, counts.as_mut_ptr(), 16) } == 0;
    assert!(ok, "this kernel doesn't count a thread's instructions");
    counts[0]
}

#[cfg(not(target_os = "macos"))]
fn count() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_nanos() as u64
}

/// Counts the frames an effect was run on, so an arm proves it ran.
struct Counted<E> {
    effect: E,
    frames: Arc<AtomicU64>,
}

impl<E: Effect> Effect for Counted<E> {
    fn parameters(&self) -> &[zgx_audio::Parameter] {
        self.effect.parameters()
    }
    fn prepare(&mut self, format: Format, max_block: usize) {
        self.effect.prepare(format, max_block);
    }
    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        self.frames
            .fetch_add(context.frames() as u64, Ordering::Relaxed);
        self.effect.process(block, context);
    }
    fn tail(&self) -> std::time::Duration {
        self.effect.tail()
    }
}

struct Nothing;
impl Effect for Nothing {
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, _: &mut [f32], _: &Context<'_>) {}
}

struct Gain(f32);
impl Effect for Gain {
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, block: &mut [f32], _: &Context<'_>) {
        block.iter_mut().for_each(|s| *s *= self.0);
    }
}

fn add(name: &str, bus: &Bus, frames: Arc<AtomicU64>) -> EffectHandle {
    fn counted<E: Effect>(bus: &Bus, effect: E, frames: Arc<AtomicU64>) -> EffectHandle {
        bus.add_effect(Counted { effect, frames }).unwrap()
    }
    match name {
        "none" => counted(bus, Nothing, frames),
        "gain" => counted(bus, Gain(0.5), frames),
        "lowpass" | "sweep" => counted(bus, LowPass::new(2000.), frames),
        "lowpass24" => counted(bus, LowPass::new(2000.).slope(Slope::Db24), frames),
        "highpass" => counted(bus, HighPass::new(200.), frames),
        "peaking" => counted(bus, Peaking::new(1000., 6., 1.), frames),
        "reverb" => counted(bus, Reverb::new(1., 2.), frames),
        "delay" => counted(
            bus,
            Delay::new(0.25, 0.25).feedback(0.5).mix(0.5, 1.),
            frames,
        ),
        other => panic!("no effect named {other:?}"),
    }
}

/// Instructions for the scene's blocks after the first, and the frames the
/// effects ran on.
fn run(effect: Option<&str>, instances: usize, sounds: &[Sound]) -> (u64, u64) {
    let (audio, mut mixer) = Audio::offline(Config {
        limiter: None,
        ..Config::default()
    })
    .unwrap();
    let frames = Arc::new(AtomicU64::new(0));
    for i in 0..instances {
        let bus = audio.add_bus(&format!("bus {i}"), &audio.master()).unwrap();
        if let Some(name) = effect {
            let handle = add(name, &bus, frames.clone());
            if name == "sweep" {
                handle
                    .set(LowPass::CUTOFF, 200., Ramp::seconds(SECONDS as f64 + 1.))
                    .unwrap();
            }
        }
        let play = Play::new()
            .bus(&bus)
            .gain(0.5 / instances as f32)
            .looping(true);
        audio.play(&sounds[i % sounds.len()], play).unwrap();
    }
    let mut out = vec![0f32; BLOCK * 2];
    mixer.render(&mut out);
    let ran = frames.load(Ordering::Relaxed);
    let before = count();
    for _ in 1..SECONDS * 48_000 / BLOCK {
        mixer.render(&mut out);
    }
    let instructions = count() - before;
    std::hint::black_box(&out);
    assert!(out.iter().any(|&s| s != 0.), "sound came out");
    (instructions, frames.load(Ordering::Relaxed) - ran)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let effect = args.next().unwrap_or_else(|| "none".into());
    let instances: usize = args.next().map_or(16, |a| a.parse().expect("a number"));
    let sounds: Vec<Sound> = (0..16u64)
        .map(|v| {
            let mut seed = 0x2545_f491_4f6c_dd1du64 ^ (v.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
            let noise: Vec<f32> = (0..48_000 * 2)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
                })
                .collect();
            Sound::from_samples(&noise, 2, 48_000).unwrap()
        })
        .collect();
    let (mut control, mut arm) = (u64::MAX, u64::MAX);
    let mut ran = 0;
    for _ in 0..5 {
        control = control.min(run(None, instances, &sounds).0);
        let (instructions, frames) = run(Some(&effect), instances, &sounds);
        arm = arm.min(instructions);
        ran = frames;
    }
    let frames = ((SECONDS * 48_000 / BLOCK - 1) * BLOCK) as u64;
    assert_eq!(
        ran,
        frames * instances as u64,
        "every instance ran on every frame"
    );
    let unit = if cfg!(target_os = "macos") {
        "instructions"
    } else {
        "ns"
    };
    println!(
        "{effect} x {instances}: {:.2} {unit} an instance a frame (control {:.2} a frame)",
        arm.saturating_sub(control) as f64 / instances as f64 / frames as f64,
        control as f64 / frames as f64,
    );
}
