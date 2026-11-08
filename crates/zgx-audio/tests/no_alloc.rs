//! The audio thread's work allocates and frees nothing: playing, changing,
//! stopping and looping voices, a stream, and voices ending, counted by the
//! allocator over thousands of buffers.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use zgx_audio::{Info, Play, Sound, mixer};

struct Counting;
static COUNTING: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNTING.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn main() {
    let (mut control, mut mixer) = mixer::pair(48_000);
    let short = Sound::from_stereo(vec![0.25; 300]);
    let long = Sound::from_stereo(vec![0.1; 48_000]);
    let mut phase = 0f32;
    control.stream(Some(Box::new(move |out: &mut [f32], _: Info| {
        for sample in out.iter_mut() {
            phase = (phase + 0.01).fract();
            *sample = phase - 0.5;
        }
    })));
    let mut out = vec![0f32; 512 * 2];
    let mut voices = Vec::with_capacity(1000);
    let mut worst = 0;
    for buffer in 0..5000 {
        // The controller's side, before the count: what it sends.
        if buffer % 3 == 0 {
            voices.push(control.play(&short, Play::default()));
        }
        if buffer % 5 == 0 {
            // The mixer holds this one alone: when it ends, it's freed only
            // once it's back here.
            let once = Sound::from_stereo(vec![0.2; 200]);
            control.play(&once, Play::default());
        }
        if buffer % 50 == 0 {
            let looped = Play {
                looped: true,
                delay: 0.003,
                ..Play::default()
            };
            voices.push(control.play(&long, looped));
        }
        if buffer % 7 == 0
            && let Some(voice) = voices.pop()
        {
            control.set(voice, 0.5, -0.3);
            control.stop(voice);
        }
        control.collect();
        CALLS.store(0, Ordering::Relaxed);
        COUNTING.store(true, Ordering::Relaxed);
        mixer.render(&mut out);
        COUNTING.store(false, Ordering::Relaxed);
        worst = worst.max(CALLS.load(Ordering::Relaxed));
    }
    let reading = control.reading();
    assert!(reading.started > 1500, "voices played: {}", reading.started);
    assert_eq!(worst, 0, "the audio thread's buffer allocated or freed");
    println!(
        "no_alloc: 5000 buffers, {} voices, no allocation",
        reading.started
    );
}
