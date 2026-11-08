//! A video with sound costs the encoder no more memory than one without:
//! ffmpeg isn't asked to end at the shorter stream (`-shortest`), which in a
//! video longer than 10 s came to hold 10 s of decoded frames (1.8 GB at
//! 1080p and 60 fps), as the mix is never longer than the frames. Measured as the largest child's peak resident
//! memory, which the system keeps for every child a process has waited on:
//! a process of its own, so no other test's children count.
#![cfg(unix)]
mod common;
use common::*;
use zgx_video::Format;
use zgx_video::encode::{EncodeOptions, Sink};

#[repr(C)]
struct Rusage {
    user: [i64; 2],
    system: [i64; 2],
    maxrss: i64,
    rest: [i64; 13],
}
unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut Rusage) -> i32;
}
/// The largest peak, in bytes, of the children waited on so far.
fn children_peak() -> u64 {
    const CHILDREN: i32 = -1;
    let mut usage = Rusage {
        user: [0; 2],
        system: [0; 2],
        maxrss: 0,
        rest: [0; 13],
    };
    unsafe { getrusage(CHILDREN, &mut usage) };
    // Bytes on macOS, kilobytes on Linux.
    let unit = if cfg!(target_os = "macos") { 1 } else { 1024 };
    usage.maxrss as u64 * unit
}

#[test]
fn sound_adds_no_frames_held_by_the_encoder() {
    if zgx_video::encode::ffmpeg().is_none() {
        eprintln!("no ffmpeg here: skipped");
        return;
    }
    let dir = temp_dir("encoder-memory");
    let wav = dir.join("tone.wav");
    // Longer than the 10 s `-shortest` buffers: 25 s at 60 fps, 640 x 360.
    // What it held, 600 frames of 345,600 bytes, is 207 MB.
    let (width, height, frames) = (640u32, 360u32, 1500usize);
    let tone: Vec<f32> = (0..48000 * 2 * frames / 60)
        .map(|i| (i as f32 * 0.02).sin() * 0.3)
        .collect();
    zgx_video::audio::write_wav(&wav, &tone).unwrap();
    let encode = |out: &str, audio: Option<std::path::PathBuf>| {
        let mut sink = Sink::start(EncodeOptions {
            format: Format::H264,
            output: dir.join(out),
            size: (width, height),
            fps: 60.,
            quality: None,
            audio,
            first_index: 0,
        })
        .unwrap();
        for frame in 0..frames {
            let mut pixels = sink.buffer();
            pixels.clear();
            // A grey that steps each frame: cheap to make and to encode.
            pixels.resize((width * height * 4) as usize, (frame % 200) as u8 + 20);
            sink.push(pixels).unwrap();
        }
        sink.finish().unwrap();
        children_peak()
    };
    let mute = encode("mute.mp4", None);
    let sound = encode("sound.mp4", Some(wav.clone()));
    eprintln!(
        "encoder peak: {} MB muted, {} MB with sound",
        mute >> 20,
        sound >> 20
    );
    assert!(
        sound < mute + (64 << 20),
        "with sound the encoder held {} MB more",
        (sound - mute) >> 20
    );
}
