//! Files made to break the decoders: empty, cut short at every length,
//! headers saying absurd things, and our tones with bytes flipped at
//! random. Loaded whole and streamed, each must come back (an error or
//! some audio) without a panic, without hanging (a stream must end), and
//! without allocating far past what the file could hold. Its own binary:
//! the allocator below counts every byte live in the process, and the one
//! test runs alone.
#![cfg(any(feature = "vorbis", feature = "mp3", feature = "opus", feature = "aac"))]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use zgx_audio::{Audio, Config, Play, Source};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        if new > layout.size() {
            let now = LIVE.fetch_add(new - layout.size(), Ordering::Relaxed) + new - layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        } else {
            LIVE.fetch_sub(layout.size() - new, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The most a hostile file may make live at once, beyond what was live
/// before it: well past a minute of decoded stereo, short of anything an
/// absurd header could ask for.
const MOST: usize = 64 << 20;

/// `bytes` loaded whole and streamed to its end, with a time limit: what
/// was allocated at its peak, beyond what was live before.
fn survive(what: &str, bytes: Vec<u8>) {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let began = Instant::now();
    let (done, wait) = std::sync::mpsc::channel();
    let name = what.to_string();
    std::thread::spawn(move || {
        let (audio, mut mixer) = Audio::offline(Config {
            limiter: None,
            ..Config::default()
        })
        .unwrap();
        let _ = audio.load(Source::bytes(bytes.clone()));
        if let Ok(voice) = audio.stream(Source::bytes(bytes), Play::new()) {
            // A stream of a broken file ends, or plays on as audio: either
            // way, within its own length's worth of frames and a bit.
            for _ in 0..200 {
                mixer.frames(512);
                audio.update();
                if !voice.playing() {
                    break;
                }
            }
            let _ = voice.stop();
        }
        let _ = done.send(name);
    });
    let finished = wait.recv_timeout(Duration::from_secs(20));
    assert!(
        finished.is_ok(),
        "{what}: still going after {:?}",
        began.elapsed()
    );
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    assert!(peak < MOST, "{what}: {peak} bytes at its peak");
}

/// `bytes` with `share` of them flipped, by a seed.
fn flipped(bytes: &[u8], share: f64, seed: u64) -> Vec<u8> {
    let mut s = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut out = bytes.to_vec();
    let n = (bytes.len() as f64 * share).max(1.) as usize;
    for _ in 0..n {
        let at = next() as usize % out.len();
        out[at] ^= (next() as u8) | 1;
    }
    out
}

/// Every way of breaking `file` this tries.
fn break_it(name: &str, file: &[u8]) {
    survive(&format!("{name}, empty"), Vec::new());
    for cut in (0..file.len()).step_by(file.len() / 97 + 1) {
        survive(&format!("{name}, cut at {cut}"), file[..cut].to_vec());
    }
    for seed in 0..50 {
        survive(
            &format!("{name}, flipped {seed}"),
            flipped(file, 0.01, seed),
        );
    }
}

#[test]
fn hostile_files_come_back_without_a_panic_a_hang_or_a_flood_of_memory() {
    #[cfg(feature = "mp3")]
    {
        let tone = include_bytes!("assets/tone.mp3");
        break_it("mp3", tone);
        // An ID3 tag saying it's 256 MiB long, then a little audio.
        let mut tag = b"ID3\x04\x00\x00\x7f\x7f\x7f\x7f".to_vec();
        tag.extend_from_slice(&tone[..2_000]);
        survive("mp3, a huge ID3 tag", tag);
        // Frame headers and nothing else, many of them.
        survive("mp3, headers only", b"\xff\xfb\x90\x00".repeat(20_000));
    }
    #[cfg(feature = "opus")]
    {
        let tone = include_bytes!("assets/tone.opus");
        break_it("opus", tone);
        // The OpusHead saying 255 channels in mapping family 255; and its
        // tags' vendor string saying it's 4 GiB long.
        let head = tone.windows(8).position(|w| w == b"OpusHead").unwrap();
        let mut many = tone.to_vec();
        many[head + 9] = 255;
        many[head + 18] = 255;
        survive("opus, 255 channels", many);
        let tags = tone.windows(8).position(|w| w == b"OpusTags").unwrap();
        let mut long = tone.to_vec();
        long[tags + 8..tags + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        survive("opus, a 4 GiB vendor string", long);
    }
    #[cfg(feature = "vorbis")]
    {
        break_it("vorbis", include_bytes!("assets/tone.ogg"));
    }
    #[cfg(all(feature = "aac", target_os = "macos"))]
    {
        break_it("m4a", include_bytes!("assets/tone.m4a"));
        break_it("adts", include_bytes!("assets/tone.aac"));
        // An ADTS header whose frame length runs far past the file.
        survive(
            "adts, a frame past the end",
            b"\xff\xf1\x50\x80\xff\xff\xfc".repeat(100),
        );
    }
}
