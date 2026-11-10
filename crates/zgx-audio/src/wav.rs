//! WAV files: PCM of 8, 16, 24 or 32 bits and 32- or 64-bit float, read
//! at their own rate and channels; written as 24-bit PCM or 32-bit float.
use crate::Error;

/// A WAV file's samples, interleaved, as floats from -1 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Wav {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub rate: u32,
}

/// The samples of a WAV file's bytes.
pub fn read(bytes: &[u8]) -> Result<Wav, Error> {
    let bad = |what: &str| Error::Format(what.to_string());
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a WAV file"));
    }
    let mut at = 12;
    let mut format: Option<(u16, usize, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        // A size past the end (a file still being written says 0xFFFFFFFF)
        // means the rest of it.
        let body = &bytes[at + 8..(at + 8).saturating_add(size).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            let word = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
            let mut tag = word(0);
            // Extensible: its real tag leads its sub-format.
            if tag == 0xFFFE && body.len() >= 26 {
                tag = word(24);
            }
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            format = Some((tag, usize::from(word(2)), rate, word(14)));
        } else if id == b"data" {
            let (tag, channels, rate, bits) =
                format.ok_or_else(|| bad("WAV data before its format"))?;
            if channels == 0 || rate == 0 {
                return Err(bad("a WAV of no channels or no rate"));
            }
            let mut samples: Vec<f32> = match (tag, bits) {
                (1, 8) => body.iter().map(|&b| (f32::from(b) - 128.) / 128.).collect(),
                (1, 16) => body
                    .as_chunks()
                    .0
                    .iter()
                    .map(|b| f32::from(i16::from_le_bytes(*b)) / 32_768.)
                    .collect(),
                (1, 24) => body
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.)
                    .collect(),
                (1, 32) => body
                    .as_chunks()
                    .0
                    .iter()
                    .map(|b| i32::from_le_bytes(*b) as f32 / 2_147_483_648.)
                    .collect(),
                (3, 32) => body
                    .as_chunks()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect(),
                (3, 64) => body
                    .as_chunks()
                    .0
                    .iter()
                    .map(|b| f64::from_le_bytes(*b) as f32)
                    .collect(),
                _ => {
                    return Err(Error::Format(format!(
                        "a WAV of {bits}-bit samples in format {tag}"
                    )));
                }
            };
            // A last frame cut short is dropped.
            samples.truncate(samples.len() / channels * channels);
            return Ok(Wav {
                samples,
                channels,
                rate,
            });
        }
        at = at.saturating_add(8 + size + size % 2);
    }
    Err(bad("a WAV without data"))
}

/// How samples are stored when written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// 24-bit integers, clamped to -1..1: what most tools read.
    Pcm24,
    /// 32-bit floats, exactly as mixed.
    Float32,
}

/// The bytes of a WAV file holding `samples`.
pub fn write(samples: &[f32], channels: usize, rate: u32, encoding: Encoding) -> Vec<u8> {
    let (tag, width) = match encoding {
        Encoding::Pcm24 => (1u16, 3usize),
        Encoding::Float32 => (3, 4),
    };
    let size = (samples.len() * width) as u32;
    let mut bytes = Vec::with_capacity(44 + size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size + size % 2).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&tag.to_le_bytes());
    bytes.extend_from_slice(&(channels as u16).to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * (channels * width) as u32).to_le_bytes());
    bytes.extend_from_slice(&((channels * width) as u16).to_le_bytes());
    bytes.extend_from_slice(&((width * 8) as u16).to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&size.to_le_bytes());
    for &sample in samples {
        match encoding {
            Encoding::Pcm24 => {
                let value = (f64::from(sample.clamp(-1., 1.)) * 8_388_607.).round() as i32;
                bytes.extend_from_slice(&value.to_le_bytes()[..3]);
            }
            Encoding::Float32 => bytes.extend_from_slice(&sample.to_le_bytes()),
        }
    }
    if size % 2 == 1 {
        bytes.push(0);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV of `data` with a format chunk saying `tag`, `channels`, `bits`.
    fn wav(tag: u16, channels: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
        // A chunk before the format, as some tools write: skipped.
        bytes.extend_from_slice(b"LIST\x03\0\0\0abc\0");
        bytes.extend_from_slice(b"fmt \x10\0\0\0");
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&44_100u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 6]);
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn every_pcm_width_and_float_reads_as_the_same_values() {
        let cases: [(u16, u16, Vec<u8>); 6] = [
            (1, 8, vec![0, 128, 192]),
            (
                1,
                16,
                [i16::MIN, 0, 16_384]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
            (
                1,
                24,
                [-8_388_608i32, 0, 4_194_304]
                    .iter()
                    .flat_map(|v| v.to_le_bytes()[..3].to_vec())
                    .collect(),
            ),
            (
                1,
                32,
                [i32::MIN, 0, 1 << 30]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
            (
                3,
                32,
                [-1f32, 0., 0.5]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
            (
                3,
                64,
                [-1f64, 0., 0.5]
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect(),
            ),
        ];
        for (tag, bits, data) in cases {
            let read = read(&wav(tag, 1, bits, &data)).unwrap();
            assert_eq!(
                read,
                Wav {
                    samples: vec![-1., 0., 0.5],
                    channels: 1,
                    rate: 44_100
                },
                "{bits}-bit, format {tag}"
            );
        }
    }

    #[test]
    fn written_files_read_back() {
        let samples = [0.25, -0.5, 1.0, -1.0, 0.0, 0.125];
        let float = read(&write(&samples, 2, 48_000, Encoding::Float32)).unwrap();
        assert_eq!(
            float,
            Wav {
                samples: samples.to_vec(),
                channels: 2,
                rate: 48_000
            }
        );
        let pcm = read(&write(&samples, 2, 48_000, Encoding::Pcm24)).unwrap();
        for (a, b) in pcm.samples.iter().zip(samples) {
            assert!((a - b).abs() < 2e-7, "{a} {b}");
        }
    }

    #[test]
    fn unreadable_files_say_why() {
        assert!(read(b"RIFX").is_err());
        assert!(
            read(&wav(2, 1, 4, &[0; 4]))
                .unwrap_err()
                .to_string()
                .contains("4-bit samples in format 2")
        );
        let mut cut = wav(1, 2, 16, &[0; 6]);
        assert_eq!(
            read(&cut).unwrap().samples.len(),
            2,
            "a frame cut short is dropped"
        );
        cut.truncate(20);
        assert!(read(&cut).is_err());
    }
}
