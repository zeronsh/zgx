//! Effects on buses, driven with effects of the test's own: where in a bus
//! they run, their parameters on exact frames, tails that outlast their
//! input, side chains, taking one off, and the debug check that names an
//! effect breaking the audio thread's rules.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use zgx_audio::{
    Audio, Checked, Config, Context, Effect, Error, Format, Parameter, Play, Ramp, Sound, Time,
};

// Counts nothing unless the mixer arms it, around an effect's process.
#[global_allocator]
static ALLOCATOR: Checked = Checked(std::alloc::System);

const RATE: u32 = 48_000;

fn unlimited() -> Config {
    Config {
        limiter: None,
        ..Config::default()
    }
}

/// A constant, looped from its second frame (its first is zero: no fade in).
fn flat(value: f32) -> Sound {
    Sound::from_samples(&[0., value], 1, RATE).unwrap()
}

fn left(samples: &[f32]) -> Vec<f32> {
    samples.as_chunks::<2>().0.iter().map(|f| f[0]).collect()
}

fn right(samples: &[f32]) -> Vec<f32> {
    samples.as_chunks::<2>().0.iter().map(|f| f[1]).collect()
}

/// Renders `frames` of what `build` sets up in blocks of several sizes,
/// checks they agree, and returns them.
fn render(frames: usize, build: impl Fn(&Audio)) -> Vec<f32> {
    let mut first: Option<Vec<f32>> = None;
    for block in [512, 1, 37, 128, 300] {
        let (audio, mut mixer) = Audio::offline(unlimited());
        build(&audio);
        let mut out = vec![0.; frames * 2];
        for chunk in out.chunks_mut(block * 2) {
            mixer.render(chunk);
        }
        match &first {
            None => first = Some(out),
            Some(first) => {
                let at = first.iter().zip(&out).position(|(a, b)| a != b);
                assert_eq!(at, None, "blocks of {block} differ from 512's at {at:?}");
            }
        }
    }
    first.unwrap()
}

struct Scale(f32);
impl Effect for Scale {
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, block: &mut [f32], _: &Context<'_>) {
        block.iter_mut().for_each(|s| *s *= self.0);
    }
}

struct Offset(f32);
impl Effect for Offset {
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, block: &mut [f32], _: &Context<'_>) {
        block.iter_mut().for_each(|s| *s += self.0);
    }
}

#[test]
fn effects_run_in_order_on_the_bus_sum_before_its_gain() {
    let out = left(&render(400, |audio| {
        let music = audio.bus("music").unwrap();
        music.set_gain(0.5, Ramp::NONE).unwrap();
        audio
            .play(&flat(0.25), Play::new().bus(&music).loop_frames(1..2))
            .unwrap();
        audio
            .play(&flat(0.125), Play::new().bus(&music).loop_frames(1..2))
            .unwrap();
        music.add_effect(Offset(0.125)).unwrap();
        music.add_effect(Scale(2.)).unwrap();
    }));
    // ((0.25 + 0.125) + 0.125) * 2, then the bus's 0.5.
    assert_eq!(out[1], 0.5);
    assert_eq!(out[399], 0.5);
}

/// Writes its two parameters' values as its output, left and right.
struct Show;
impl Effect for Show {
    fn parameters(&self) -> &[Parameter] {
        const PARAMS: [Parameter; 2] = [
            Parameter::new("a", 0.5, 0., 1.),
            Parameter::new("b", -1., -2., 2.),
        ];
        &PARAMS
    }
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        assert_eq!(block.len(), context.frames() * 2);
        assert_eq!(
            context.format(),
            Format {
                rate: RATE,
                channels: 2
            }
        );
        for (i, frame) in block.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            *frame = [context.value_at(0, i), context.value_at(1, i)];
        }
    }
}

#[test]
fn parameters_move_by_index_on_their_frames_and_keep_to_their_ranges() {
    let out = render(1000, |audio| {
        let world = audio.bus("world").unwrap();
        audio
            .play(&flat(0.), Play::new().bus(&world).loop_frames(1..2))
            .unwrap();
        let show = world.add_effect(Show).unwrap();
        // 100 frames from 0.5 to 1, from frame 100.
        show.at(Time(100))
            .set(0, 1., Ramp::seconds(100. / f64::from(RATE)))
            .unwrap();
        // Past its range: kept to 2. Not a number: its least.
        show.at(Time(400)).set(1, 5., Ramp::NONE).unwrap();
        show.at(Time(600)).set(0, f32::NAN, Ramp::NONE).unwrap();
        // An index it hasn't: nothing.
        show.at(Time(700)).set(9, 3., Ramp::NONE).unwrap();
    });
    let (a, b) = (left(&out), right(&out));
    assert_eq!(&a[..101], &[0.5; 101][..], "its default until frame 100");
    assert_eq!(a[150], 0.75);
    assert_eq!(&a[200..600], &[1.; 400][..]);
    assert_eq!(&a[600..], &[0.; 400][..]);
    assert_eq!(&b[..400], &[-1.; 400][..]);
    assert_eq!(&b[400..], &[2.; 600][..]);
}

/// A delay of `frames`, ringing that long after its input stops.
struct Echo {
    frames: usize,
    ring: Vec<f32>,
    at: usize,
    calls: Arc<AtomicUsize>,
}
impl Effect for Echo {
    fn prepare(&mut self, _: Format, _: usize) {
        self.ring = vec![0.; self.frames * 2];
    }
    fn process(&mut self, block: &mut [f32], _: &Context<'_>) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        for s in block.iter_mut() {
            std::mem::swap(s, &mut self.ring[self.at]);
            self.at = (self.at + 1) % self.ring.len();
        }
    }
    fn tail(&self) -> Duration {
        Duration::from_secs_f64(self.frames as f64 / f64::from(RATE))
    }
}

#[test]
fn a_tail_runs_on_silence_after_its_input_stops_then_rests() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (audio, mut mixer) = Audio::offline(unlimited());
    let world = audio.bus("world").unwrap();
    world
        .add_effect(Echo {
            frames: 1000,
            ring: Vec::new(),
            at: 0,
            calls: calls.clone(),
        })
        .unwrap();
    let click = Sound::from_samples(&[0., 0.5, -0.25, 0.1], 1, RATE).unwrap();
    audio.play(&click, Play::new().bus(&world)).unwrap();
    let out = left(&mixer.frames(4000));
    // The voice ended after 4 frames; its echo comes 1000 later.
    assert_eq!(&out[1000..1005], &[0., 0.5, -0.25, 0.1, 0.]);
    assert!(out[..1000].iter().all(|&s| s == 0.));
    // After the tail, the effect isn't run on silence.
    let ran = calls.load(Ordering::Relaxed);
    mixer.frames(4000);
    assert_eq!(calls.load(Ordering::Relaxed), ran, "run past its tail");
    assert!(ran * 128 < 2000, "ran on {ran} blocks of silence");
}

/// Writes the level its side chain gives it.
struct Key;
impl Effect for Key {
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, block: &mut [f32], context: &Context<'_>) {
        let level = context.side_chain().expect("added with a side chain");
        block.iter_mut().for_each(|s| *s = level);
    }
}

#[test]
fn a_side_chain_hears_its_bus_over_the_block_before() {
    let (audio, mut mixer) = Audio::offline(unlimited());
    let (ui, music) = (audio.bus("ui").unwrap(), audio.bus("music").unwrap());
    ui.set_gain(0.5, Ramp::NONE).unwrap();
    audio
        .play(&flat(0.5), Play::new().bus(&ui).loop_frames(1..2))
        .unwrap();
    audio
        .play(&flat(0.), Play::new().bus(&music).loop_frames(1..2))
        .unwrap();
    music.add_effect_keyed(Key, &ui).unwrap();
    let mut out = Vec::new();
    for _ in 0..4 {
        out.extend(left(&mixer.frames(64)));
    }
    // ui sends 0.25 (0.5 at a gain of 0.5): its first frame is zero, so
    // over its first block a little less; music shows that a block late.
    let first = 0.25 * (63f32 / 64.).sqrt();
    assert_eq!(out[10], 0.25, "nothing heard before the first block");
    assert!((out[64 + 10] - (0.25 + first)).abs() < 1e-6, "{}", out[74]);
    assert!((out[200] - 0.5).abs() < 1e-6, "{}", out[200]);

    // The other way round: a bus mixed before the one keyed on it is
    // still heard a block late, not as it is now.
    let (audio, mut mixer) = Audio::offline(unlimited());
    let (ui, world) = (audio.bus("ui").unwrap(), audio.bus("world").unwrap());
    audio
        .play(&flat(0.5), Play::new().bus(&world).loop_frames(1..2))
        .unwrap();
    audio
        .play(&flat(0.), Play::new().bus(&ui).loop_frames(1..2))
        .unwrap();
    ui.add_effect_keyed(Key, &world).unwrap();
    let out = left(&mixer.frames(64));
    assert_eq!(out[10], 0.5, "heard before its block was over: {}", out[10]);

    // Master too, though it's mixed after every other bus.
    let (audio, mut mixer) = Audio::offline(unlimited());
    let (ui, world) = (audio.bus("ui").unwrap(), audio.bus("world").unwrap());
    audio
        .play(&flat(0.5), Play::new().bus(&world).loop_frames(1..2))
        .unwrap();
    audio
        .play(&flat(0.), Play::new().bus(&ui).loop_frames(1..2))
        .unwrap();
    ui.set_gain(0.5, Ramp::NONE).unwrap();
    ui.add_effect_keyed(Key, &audio.master()).unwrap();
    mixer.frames(64);
    // Master was 0.5 from world and, once ui hears it, half of that again.
    let out = left(&mixer.frames(64));
    assert!(out[10] > 0.5, "master's level unheard: {}", out[10]);
}

struct Dropped(Arc<AtomicBool>);
impl Effect for Dropped {
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, block: &mut [f32], _: &Context<'_>) {
        block.iter_mut().for_each(|s| *s *= 2.);
    }
}
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[test]
fn a_removed_effect_stops_on_its_frame_and_is_dropped_by_the_main_thread() {
    let dropped = Arc::new(AtomicBool::new(false));
    let (audio, mut mixer) = Audio::offline(unlimited());
    let ui = audio.bus("ui").unwrap();
    audio
        .play(&flat(0.25), Play::new().bus(&ui).loop_frames(1..2))
        .unwrap();
    let effect = ui.add_effect(Dropped(dropped.clone())).unwrap();
    effect.at(Time(500)).remove().unwrap();
    let out = left(&mixer.frames(1000));
    assert_eq!((out[499], out[500]), (0.5, 0.25));
    assert!(!dropped.load(Ordering::Relaxed), "dropped by the mixer");
    audio.update();
    assert!(dropped.load(Ordering::Relaxed));
}

#[test]
fn a_bus_holds_eight_effects() {
    let audio = Audio::offline(unlimited()).0;
    let ui = audio.bus("ui").unwrap();
    let held: Vec<_> = (0..8).map(|_| ui.add_effect(Scale(1.)).unwrap()).collect();
    assert_eq!(ui.add_effect(Scale(1.)).unwrap_err(), Error::TooManyEffects);
    // Another bus has room of its own; one taken off makes room again.
    audio.bus("music").unwrap().add_effect(Scale(1.)).unwrap();
    held[3].remove().unwrap();
    ui.add_effect(Scale(1.)).unwrap();
}

struct Leaky;
impl Effect for Leaky {
    fn prepare(&mut self, _: Format, _: usize) {}
    fn process(&mut self, block: &mut [f32], _: &Context<'_>) {
        let scratch = std::hint::black_box(block.to_vec());
        block[0] = scratch[0];
    }
}

#[test]
#[cfg(debug_assertions)]
fn an_effect_that_allocates_on_the_audio_thread_is_named_in_a_panic() {
    let (audio, mut mixer) = Audio::offline(unlimited());
    let ui = audio.bus("ui").unwrap();
    audio
        .play(&flat(0.25), Play::new().bus(&ui).loop_frames(1..2))
        .unwrap();
    ui.add_effect(Leaky).unwrap();
    let mut out = vec![0.; 256];
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| mixer.render(&mut out)))
        .expect_err("allocating in process passed");
    let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(
        message.contains("effects::Leaky") && message.contains("2 time(s)"),
        "{message}"
    );
}
