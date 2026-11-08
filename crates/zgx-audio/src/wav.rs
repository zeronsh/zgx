//! WAV files, read to interleaved stereo, and taken to another rate. The
//! same reading as zgx-video's (which the design moves here, for both).

/// Interleaved stereo and its rate, from a WAV file's bytes: 8, 16, 24 and
/// 32-bit integers or 32 and 64-bit floats, mono (both sides) or more.
pub fn read(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
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
            return Ok((stereo, rate));
        }
        at += 8 + size + size % 2;
    }
    Err("a WAV without data".into())
}

/// Interleaved stereo at `from` frames a second taken to `to`, in straight
/// lines.
pub fn resample(stereo: &[f32], from: u32, to: u32) -> Vec<f32> {
    let frames = stereo.len() / 2;
    if from == to || from == 0 || frames == 0 {
        return stereo.to_vec();
    }
    let out = (frames as u64 * u64::from(to) / u64::from(from)) as usize;
    let mut taken = Vec::with_capacity(out * 2);
    for i in 0..out {
        let at = i as f64 * f64::from(from) / f64::from(to);
        let (whole, part) = (at as usize, at.fract() as f32);
        let next = (whole + 1).min(frames - 1);
        for channel in 0..2 {
            let a = stereo[whole * 2 + channel];
            let b = stereo[next * 2 + channel];
            taken.push(a + (b - a) * part);
        }
    }
    taken
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data.len() as u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes()); // PCM
        bytes.extend(1u16.to_le_bytes()); // mono
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * 2).to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        bytes
    }

    #[test]
    fn mono_sixteen_bit_reads_to_both_sides() {
        let (stereo, rate) = read(&wav(22_050, &[16384, -32768])).unwrap();
        assert_eq!(rate, 22_050);
        assert_eq!(stereo, [0.5, 0.5, -1., -1.]);
    }

    #[test]
    fn resampling_doubles_the_frames_in_straight_lines() {
        let taken = resample(&[0., 0., 1., 1.], 1, 2);
        assert_eq!(taken, [0., 0., 0.5, 0.5, 1., 1., 1., 1.]);
    }
}
