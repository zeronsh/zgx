//! A video's sounds, mixed to one stereo track, sample by sample, the same
//! each time. WAV files are read here; other formats go through ffmpeg, or
//! macOS's own afconvert.
use crate::script::AudioClip;
use std::path::{Path, PathBuf};

/// The rate a mix is made at.
pub const SAMPLE_RATE: u32 = 48_000;

/// Decoded sound: interleaved stereo at [`SAMPLE_RATE`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sound(pub Vec<f32>);

fn read_wav(bytes: &[u8]) -> Result<Sound, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut at = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            let word = |i: usize| u16::from_le_bytes(body[i..i + 2].try_into().unwrap());
            let mut tag = word(0);
            // Extensible: its real tag leads its sub-format.
            if tag == 0xFFFE && body.len() >= 26 {
                tag = word(24);
            }
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            format = Some((tag, word(2), rate, word(14)));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format.ok_or("WAV data before its format")?;
            let width = usize::from(bits / 8);
            if width == 0 || channels == 0 {
                return Err("a WAV of no samples".into());
            }
            let channels = usize::from(channels);
            let sample = |raw: &[u8]| -> Option<f32> {
                Some(match (tag, width) {
                    (1, 1) => (f32::from(raw[0]) - 128.) / 128.,
                    (1, 2) => f32::from(i16::from_le_bytes([raw[0], raw[1]])) / 32768.,
                    (1, 3) => {
                        (i32::from_le_bytes([0, raw[0], raw[1], raw[2]]) >> 8) as f32 / 8_388_608.
                    }
                    (1, 4) => i32::from_le_bytes(raw.try_into().ok()?) as f32 / 2_147_483_648.,
                    (3, 4) => f32::from_le_bytes(raw.try_into().ok()?),
                    (3, 8) => f64::from_le_bytes(raw.try_into().ok()?) as f32,
                    _ => return None,
                })
            };
            let mut stereo = Vec::with_capacity(body.len() / width / channels * 2);
            for frame in body.chunks_exact(width * channels) {
                let left = sample(&frame[..width])
                    .ok_or_else(|| format!("a WAV of {bits}-bit samples, format {tag}"))?;
                let right = if channels > 1 {
                    sample(&frame[width..width * 2]).unwrap_or(left)
                } else {
                    left
                };
                stereo.extend([left, right]);
            }
            return Ok(Sound(resample(&stereo, rate)));
        }
        at += 8 + size + size % 2;
    }
    Err("a WAV without data".into())
}

/// Interleaved stereo at `rate` taken to [`SAMPLE_RATE`], in straight lines.
fn resample(stereo: &[f32], rate: u32) -> Vec<f32> {
    if rate == SAMPLE_RATE || rate == 0 {
        return stereo.to_vec();
    }
    let frames = stereo.len() / 2;
    let out = (frames as u64 * u64::from(SAMPLE_RATE) / u64::from(rate)) as usize;
    let mut mixed = Vec::with_capacity(out * 2);
    for i in 0..out {
        let at = i as f64 * f64::from(rate) / f64::from(SAMPLE_RATE);
        let (whole, part) = (at as usize, at.fract() as f32);
        let next = (whole + 1).min(frames - 1);
        for channel in 0..2 {
            let a = stereo[whole * 2 + channel];
            let b = stereo[next * 2 + channel];
            mixed.push(a + (b - a) * part);
        }
    }
    mixed
}

/// Read `path`, whatever it holds if ffmpeg or afconvert can.
pub fn decode(path: &Path) -> Result<Sound, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.starts_with(b"RIFF") {
        return read_wav(&bytes).map_err(|error| format!("{}: {error}", path.display()));
    }
    let converted = std::env::temp_dir().join(format!("zgx-video-{}.wav", std::process::id()));
    let ran = if let Some(ffmpeg) = crate::encode::ffmpeg() {
        std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(path)
            .args(["-vn", "-ac", "2", "-ar", "48000", "-c:a", "pcm_f32le"])
            .arg(&converted)
            .status()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/afconvert")
            .args(["-f", "WAVE", "-d", "LEF32@48000", "-c", "2"])
            .arg(path)
            .arg(&converted)
            .status()
    } else {
        return Err(format!(
            "{}: only WAV files can be read without ffmpeg",
            path.display()
        ));
    };
    if !ran.is_ok_and(|status| status.success()) {
        return Err(format!("{}: couldn't be decoded", path.display()));
    }
    let bytes = std::fs::read(&converted).map_err(|error| error.to_string());
    let _ = std::fs::remove_file(&converted);
    read_wav(&bytes?).map_err(|error| format!("{}: {error}", path.display()))
}

/// The `clips` mixed for a video of `frames` frames at `fps`: interleaved
/// stereo at [`SAMPLE_RATE`], `sounds` being what `src` decodes to.
pub fn mix(
    clips: &[AudioClip],
    fps: f64,
    frames: u64,
    sound: impl Fn(&Path) -> Option<std::rc::Rc<Sound>>,
) -> Vec<f32> {
    let samples_at = |frame: u64| (frame as f64 / fps * f64::from(SAMPLE_RATE)).round() as usize;
    let total = samples_at(frames);
    let mut out = vec![0f32; total * 2];
    for clip in clips {
        let Some(sound) = sound(&clip.src) else {
            continue;
        };
        let source = &sound.0;
        let length = source.len() / 2;
        if length == 0 {
            continue;
        }
        let start = samples_at(clip.from);
        let skip = samples_at(clip.trim);
        let lasts = match clip.duration {
            Some(frames) => samples_at(frames),
            None if clip.looped => total.saturating_sub(start),
            None => length.saturating_sub(skip),
        };
        let fade_in = samples_at(clip.fade_in);
        let fade_out = samples_at(clip.fade_out);
        for i in 0..lasts.min(total.saturating_sub(start)) {
            let mut at = skip + i;
            if at >= length {
                if !clip.looped {
                    break;
                }
                at %= length;
            }
            let mut gain = clip.volume as f32;
            if let Some(volumes) = clip.volumes.as_deref().filter(|v| !v.is_empty()) {
                let at = i as f64 * fps / f64::from(SAMPLE_RATE);
                let (whole, part) = (at.floor() as usize, at.fract() as f32);
                let here = volumes[whole.min(volumes.len() - 1)];
                let next = volumes[(whole + 1).min(volumes.len() - 1)];
                gain *= here + (next - here) * part;
            }
            if i < fade_in {
                gain *= i as f32 / fade_in as f32;
            }
            if lasts - i <= fade_out {
                gain *= (lasts - i) as f32 / fade_out as f32;
            }
            out[(start + i) * 2] += source[at * 2] * gain;
            out[(start + i) * 2 + 1] += source[at * 2 + 1] * gain;
        }
    }
    // A mix too loud comes down whole: the shape of what's in it stays.
    let peak = out.iter().fold(0f32, |peak, sample| peak.max(sample.abs()));
    if peak > 1. {
        out.iter_mut().for_each(|sample| *sample /= peak);
    }
    out
}

/// Write interleaved stereo as a 24-bit WAV.
pub fn write_wav(path: &Path, samples: &[f32]) -> std::io::Result<()> {
    let size = samples.len() as u32 * 3;
    let mut bytes = Vec::with_capacity(44 + size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 6).to_le_bytes());
    bytes.extend_from_slice(&6u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&size.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1., 1.) * 8_388_607.) as i32;
        bytes.extend_from_slice(&value.to_le_bytes()[..3]);
    }
    std::fs::write(path, bytes)
}

/// The sounds `clips` name, mixed into a temporary WAV for an encoder; `None`
/// if there are none. The caller removes it.
pub fn mixed_file(clips: &[AudioClip], fps: f64, frames: u64) -> Result<Option<PathBuf>, String> {
    if clips.is_empty() {
        return Ok(None);
    }
    let mut sounds = std::collections::HashMap::new();
    for clip in clips {
        if !sounds.contains_key(&clip.src) {
            // A video's sound, if it has one; a sound file's, or why not.
            let sound = match clip.optional {
                true if !crate::clip::has_audio(&clip.src) => Sound::default(),
                _ => decode(&clip.src)?,
            };
            sounds.insert(clip.src.clone(), std::rc::Rc::new(sound));
        }
    }
    let mixed = mix(clips, fps, frames, |path| sounds.get(path).cloned());
    // A name of its own: renders in one process at once (a host's, a test
    // run's) each mix to a file the others don't remove.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "zgx-video-mix-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    write_wav(&path, &mixed).map_err(|error| error.to_string())?;
    Ok(Some(path))
}
