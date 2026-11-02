//! A GIF's memory is bounded by its area, not its length: one too large to
//! keep decoded (`assets::GIF_KEPT`) holds its file, its canvas and a frame
//! or two however long it is, decoding, playing and looping, and gives all
//! of it back as it goes. A counting allocator of this test's own process
//! measures it. Also what a second of playback costs each way, printed.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};

struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
// SAFETY: Every call goes to the system allocator unchanged; only sizes are
// counted.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let at = unsafe { System.alloc(layout) };
        if !at.is_null() {
            let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        at
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let at = unsafe { System.alloc_zeroed(layout) };
        if !at.is_null() {
            let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        at
    }
    unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
        unsafe { System.dealloc(at, layout) };
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, at: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(at, layout, size) };
        if !moved.is_null() {
            let now = if size >= layout.size() {
                NOW.fetch_add(size - layout.size(), Ordering::Relaxed) + size - layout.size()
            } else {
                NOW.fetch_sub(layout.size() - size, Ordering::Relaxed) - (layout.size() - size)
            };
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        moved
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;
/// One test at a time measures: the counts are the process's.
static ALONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A `width` by `height` GIF of `frames` full frames, frame `i` in palette
/// colour `i % 256`, 20 ms each, looping.
fn gif(width: u16, height: u16, frames: usize) -> Vec<u8> {
    let palette: Vec<u8> = (0..256u32)
        .flat_map(|i| [i as u8, 255 - i as u8, (i * 7) as u8])
        .collect();
    let mut bytes = Vec::new();
    {
        let mut writer = gif::Encoder::new(&mut bytes, width, height, &palette).unwrap();
        writer.set_repeat(gif::Repeat::Infinite).unwrap();
        for i in 0..frames {
            writer
                .write_frame(&gif::Frame {
                    width,
                    height,
                    delay: 2,
                    buffer: std::borrow::Cow::Owned(vec![
                        (i % 256) as u8;
                        usize::from(width) * usize::from(height)
                    ]),
                    ..Default::default()
                })
                .unwrap();
        }
    }
    bytes
}
const MB: f64 = 1024. * 1024.;

#[test]
fn a_long_gif_holds_a_few_frames_and_gives_them_back() {
    let _alone = ALONE.lock().unwrap_or_else(|p| p.into_inner());
    // 400 x 300, 300 frames: 137 MiB of frames decoded, 6 s a loop.
    let (width, height, frames) = (400u16, 300u16, 300usize);
    let encoded: std::sync::Arc<[u8]> = gif(width, height, frames).into();
    let frame = usize::from(width) * usize::from(height) * 4;
    let base = NOW.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let animation = zgx_gpu::assets::decode_gif_shared(encoded.clone()).unwrap();
    let decoded_peak = PEAK.load(Ordering::Relaxed) - base;
    let retained = NOW.load(Ordering::Relaxed) - base;
    assert!(animation.is_streamed());
    // Every frame, twice over (a loop back to the start), as playback asks.
    PEAK.store(NOW.load(Ordering::Relaxed), Ordering::Relaxed);
    for i in (0..frames).chain(0..frames) {
        drop(animation.frame(i));
    }
    let playing_peak = PEAK.load(Ordering::Relaxed) - base;
    let reported = animation.bytes();
    drop(animation);
    let after = NOW.load(Ordering::Relaxed) as i64 - base as i64;
    eprintln!(
        "GIF {width}x{height}, {frames} frames ({:.0} MiB decoded whole), {:.2} MiB encoded: \
         decode peak {:.2} MiB, retained {:.2} MiB, playing peak {:.2} MiB, \
         its bytes() {:.2} MiB, after drop {after} bytes",
        (frame * frames) as f64 / MB,
        encoded.len() as f64 / MB,
        decoded_peak as f64 / MB,
        retained as f64 / MB,
        playing_peak as f64 / MB,
        reported as f64 / MB,
    );
    // A few frames' worth, whatever its length: canvas, a frame, one held,
    // the decoder's buffers (its file is the caller's, shared). Decoding,
    // frames are kept until they pass `GIF_KEPT`, then let go.
    assert!(
        decoded_peak < zgx_gpu::assets::GIF_KEPT + 4 * frame,
        "decode peak {decoded_peak}"
    );
    assert!(retained < 3 * frame, "retained {retained}");
    // Its first frame, the canvas, the decoder's frame, the one made, the
    // decoder's own buffers: the same for 30 frames or 3,000.
    assert!(playing_peak < 8 * frame, "playing peak {playing_peak}");
    assert!(
        after.unsigned_abs() < 4096,
        "back to where it started: {after}"
    );
}

/// What a second of playback costs, kept (a small GIF, decoded once) and
/// streamed (a long one, each frame decoded as it comes): printed, for the
/// record; release builds measure it (`cargo test --release`).
#[test]
fn what_a_second_of_playback_costs() {
    let _alone = ALONE.lock().unwrap_or_else(|p| p.into_inner());
    for (what, width, height, frames) in [
        ("kept", 120u16, 120u16, 50usize),
        ("streamed", 400, 300, 300),
    ] {
        let animation = zgx_gpu::assets::decode_gif(&gif(width, height, frames)).unwrap();
        assert_eq!(animation.is_streamed(), what == "streamed");
        // Twice through, as frames come (20 ms each): the frames a loop of
        // playback asks for.
        let start = std::time::Instant::now();
        for i in (0..frames).chain(0..frames) {
            drop(animation.frame(i));
        }
        let spent = start.elapsed();
        let played = animation.cycle_duration() * 2;
        eprintln!(
            "{what} {width}x{height}, {frames} frames: {:.3} ms of frames per second played",
            spent.as_secs_f64() * 1e3 / played.as_secs_f64()
        );
    }
}
