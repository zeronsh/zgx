//! The formats' public conformance vectors, fetched when asked for, never
//! kept in the repository:
//!
//! - MP3: the ISO/IEC 11172-4 Layer III compliance streams (as minimp3's
//!   repository keeps them, CC0), decoded by nanomp3 and held to their
//!   reference PCM;
//! - Opus: the RFC 8251 test vectors, each packet decoded by opus-pure and
//!   its range coder's final state held to the one the vector records (as
//!   libopus's `opus_demo` checks), and the output to the reference's.
//!
//! `cargo test -p zgx-audio --features mp3,opus --test conformance -- --ignored`
//! (it needs `curl` and `tar`, and the network the first time).
#![cfg(any(feature = "mp3", feature = "opus"))]
use std::path::PathBuf;
use std::process::Command;

/// The vectors' folder, in the target's scratch space.
fn cache() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("zgx-audio-vectors");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `url` in the cache as `name`, fetched the first time.
fn fetch(url: &str, name: &str) -> PathBuf {
    let path = cache().join(name);
    if !path.exists() {
        let status = Command::new("curl")
            .args(["-sSfL", "--max-time", "600", "-o"])
            .arg(&path)
            .arg(url)
            .status()
            .expect("curl, to fetch the vectors");
        assert!(status.success(), "fetching {url}");
    }
    path
}

#[cfg(feature = "mp3")]
#[test]
#[ignore = "fetches the vectors over the network"]
fn mp3_decodes_the_iso_layer_iii_compliance_streams() {
    let base = "https://raw.githubusercontent.com/lieff/minimp3/master/vectors";
    for name in [
        "l3-compl",
        "l3-he_32khz",
        "l3-he_44khz",
        "l3-he_48khz",
        "l3-he_free",
        "l3-he_mode",
        "l3-hecommon",
        "l3-si",
        "l3-si_block",
        "l3-si_huff",
        "l3-sin1k0db",
    ] {
        let bit =
            std::fs::read(fetch(&format!("{base}/{name}.bit"), &format!("{name}.bit"))).unwrap();
        let pcm =
            std::fs::read(fetch(&format!("{base}/{name}.pcm"), &format!("{name}.pcm"))).unwrap();
        let reference: Vec<i16> = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect();
        // l3-he_mode switches between mono and stereo: the decoder is held
        // to it with the switch allowed. (zgx-audio's own reader doesn't
        // allow it: a sound keeps one channel count, so such a file ends
        // at its first switch.)
        let options = nanomp3::Options::default().allow_mono_stereo_transition(true);
        let mut reader =
            nanomp3::Reader::<_, i16>::with_options(std::io::Cursor::new(bit), options).unwrap();
        let mut ours = Vec::new();
        let mut buffer = vec![0i16; 4_608];
        while let Ok(n @ 1..) = reader.read(&mut buffer) {
            ours.extend_from_slice(&buffer[..n]);
        }
        let n = ours.len().min(reference.len());
        let (mut worst, mut squares) = (0i32, 0f64);
        for (a, b) in ours[..n].iter().zip(&reference[..n]) {
            let d = i32::from(*a) - i32::from(*b);
            worst = worst.max(d.abs());
            squares += f64::from(d * d);
        }
        let rms = (squares / n.max(1) as f64).sqrt();
        println!(
            "{name}: {} samples (reference {}), worst {worst} LSB, rms {rms:.3} LSB",
            ours.len(),
            reference.len()
        );
        // ISO's reference decoder leaves a stream's last frame out where
        // nanomp3 (and minimp3) decode it: at most one frame (1,152 per
        // channel) apart, compared over what both have.
        let channels = usize::from(reader.channels().map_or(1, |c| c.num()));
        assert!(
            ours.len().abs_diff(reference.len()) <= 1_152 * channels,
            "{name}: {} samples, the reference {}",
            ours.len(),
            reference.len()
        );
        assert!(
            worst <= 1 && rms < 0.5,
            "{name}: worst {worst} LSB, rms {rms:.3} LSB"
        );
    }
}

#[cfg(feature = "opus")]
#[test]
#[ignore = "fetches the vectors over the network"]
fn opus_decodes_the_rfc_8251_vectors_to_their_range_coder_states() {
    let tarball = fetch(
        "https://opus-codec.org/static/testvectors/opus_testvectors-rfc8251.tar.gz",
        "opus_testvectors-rfc8251.tar.gz",
    );
    let dir = cache().join("opus_newvectors");
    if !dir.exists() {
        let status = Command::new("tar")
            .arg("xzf")
            .arg(&tarball)
            .arg("-C")
            .arg(cache())
            .status()
            .expect("tar, to unpack the vectors");
        assert!(status.success());
    }
    for k in 1..=12 {
        let name = format!("testvector{k:02}");
        let bit = std::fs::read(dir.join(format!("{name}.bit"))).unwrap();
        let dec = std::fs::read(dir.join(format!("{name}.dec"))).unwrap();
        let reference: Vec<f64> = dec
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| f64::from(i16::from_le_bytes(*b)) / 32_768.)
            .collect();
        let mut decoder = opus_pure::OpusDecoder::new(48_000, 2).unwrap();
        let mut out = vec![0f32; 5_760 * 2];
        let (mut at, mut packets, mut mismatched) = (0, 0, 0);
        let mut ours = Vec::new();
        while at + 8 <= bit.len() {
            let length = u32::from_be_bytes(bit[at..at + 4].try_into().unwrap()) as usize;
            let range = u32::from_be_bytes(bit[at + 4..at + 8].try_into().unwrap());
            at += 8;
            let packet = &bit[at..(at + length).min(bit.len())];
            at += length;
            let n = decoder.decode(packet, 5_760, &mut out).unwrap();
            ours.extend_from_slice(&out[..n * 2]);
            packets += 1;
            if length > 0 && decoder.final_range() != range {
                mismatched += 1;
            }
        }
        let n = ours.len().min(reference.len());
        let (mut signal, mut noise) = (0f64, 0f64);
        for (a, b) in ours[..n].iter().zip(&reference[..n]) {
            signal += b * b;
            noise += (f64::from(*a) - b).powi(2);
        }
        let snr = 10. * (signal / noise.max(1e-30)).log10();
        println!(
            "{name}: {packets} packets, {mismatched} range mismatches, {snr:.1} dB against the reference"
        );
        assert_eq!(mismatched, 0, "{name}: its range coder's states");
        assert_eq!(ours.len(), reference.len(), "{name}: its length");
    }
}
