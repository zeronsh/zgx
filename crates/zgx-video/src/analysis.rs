//! What a sound holds at a moment, for a video to draw: its spectrum as bars,
//! its loudness, its waveform. Each is a function of the sound and the time
//! alone, so any frame gives the same bars however it was reached.
use crate::audio::{SAMPLE_RATE, Sound};

/// The samples an analysis looks at: 2048 at 48 kHz, about 43 ms.
const WINDOW: usize = 2048;

/// The mono sample at `index` of `sound`, silence past its ends.
fn mono(sound: &Sound, index: isize) -> f32 {
    if index < 0 {
        return 0.;
    }
    let at = index as usize * 2;
    match sound.0.get(at..at + 2) {
        Some(pair) => (pair[0] + pair[1]) * 0.5,
        None => 0.,
    }
}

/// An in-place radix-2 FFT of `re` and `im` (their length a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2. * std::f32::consts::PI / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (sin, cos) = (angle * k as f32).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cos - im[b] * sin, re[b] * sin + im[b] * cos);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// How bars read loudness.
#[derive(Clone, Copy, Debug)]
pub struct Bars {
    /// How many.
    pub count: usize,
    /// The lowest and highest frequency shown, in hertz (40, 16000).
    pub low: f32,
    /// See `low`.
    pub high: f32,
    /// The quietest level that shows, in decibels (-70): it and below read 0.
    pub floor_db: f32,
}
impl Default for Bars {
    fn default() -> Self {
        Self {
            count: 32,
            low: 40.,
            high: 16_000.,
            floor_db: -70.,
        }
    }
}

/// `sound`'s spectrum at `seconds`, as `bars.count` values from 0 to 1, in
/// bands spaced as hearing spaces pitch (logarithmically).
pub fn bars(sound: &Sound, seconds: f64, bars: Bars) -> Vec<f32> {
    let center = (seconds * f64::from(SAMPLE_RATE)).round() as isize;
    let mut re = vec![0f32; WINDOW];
    let mut im = vec![0f32; WINDOW];
    for (i, value) in re.iter_mut().enumerate() {
        // A Hann window, so a band's edges don't spill into its neighbours.
        let hann = 0.5 - 0.5 * (2. * std::f32::consts::PI * i as f32 / (WINDOW - 1) as f32).cos();
        *value = mono(sound, center - (WINDOW / 2) as isize + i as isize) * hann;
    }
    fft(&mut re, &mut im);
    let bin_hz = SAMPLE_RATE as f32 / WINDOW as f32;
    let count = bars.count.max(1);
    let ratio = (bars.high / bars.low).max(1.0001);
    (0..count)
        .map(|bar| {
            let from = bars.low * ratio.powf(bar as f32 / count as f32);
            let to = bars.low * ratio.powf((bar + 1) as f32 / count as f32);
            let (first, last) = (
                (from / bin_hz).floor() as usize,
                (to / bin_hz).ceil() as usize,
            );
            let last = last.clamp(first + 1, WINDOW / 2);
            // The band's strongest bin, its amplitude as a sine of 1 would read 1.
            let peak = (first..last)
                .map(|bin| (re[bin] * re[bin] + im[bin] * im[bin]).sqrt())
                .fold(0f32, f32::max)
                * 4.
                / WINDOW as f32;
            let db = 20. * peak.max(1e-9).log10();
            ((db - bars.floor_db) / -bars.floor_db).clamp(0., 1.)
        })
        .collect()
}

/// `sound`'s loudness at `seconds`, from 0 (silence, or `floor_db` and below)
/// to 1 (a full-scale sine), over the window around it.
pub fn level(sound: &Sound, seconds: f64, floor_db: f32) -> f32 {
    let center = (seconds * f64::from(SAMPLE_RATE)).round() as isize;
    let sum: f32 = (0..WINDOW as isize)
        .map(|i| mono(sound, center - (WINDOW / 2) as isize + i).powi(2))
        .sum();
    // A full-scale sine's mean square is a half.
    let db = 10. * (sum / WINDOW as f32 * 2.).max(1e-12).log10();
    ((db - floor_db) / -floor_db).clamp(0., 1.)
}

/// `sound`'s waveform from `seconds` for `span` seconds, as `count` peaks
/// from -1 to 1: each the sample of largest magnitude in its share, signed.
pub fn waveform(sound: &Sound, seconds: f64, span: f64, count: usize) -> Vec<f32> {
    let start = (seconds * f64::from(SAMPLE_RATE)).round() as isize;
    let length = (span * f64::from(SAMPLE_RATE)).round().max(1.) as isize;
    let count = count.max(1) as isize;
    (0..count)
        .map(|i| {
            let (from, to) = (start + length * i / count, start + length * (i + 1) / count);
            (from..to.max(from + 1))
                .map(|at| mono(sound, at))
                .fold(0f32, |peak, sample| {
                    if sample.abs() > peak.abs() {
                        sample
                    } else {
                        peak
                    }
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sine(hz: f32, amplitude: f32, seconds: f32) -> Sound {
        let samples = (seconds * SAMPLE_RATE as f32) as usize;
        Sound(
            (0..samples)
                .flat_map(|i| {
                    let v = (2. * std::f32::consts::PI * hz * i as f32 / SAMPLE_RATE as f32).sin()
                        * amplitude;
                    [v, v]
                })
                .collect(),
        )
    }
    #[test]
    fn a_tone_lights_its_own_band() {
        let tone = sine(1000., 1., 1.);
        let shown = bars(&tone, 0.5, Bars::default());
        let loudest = (0..shown.len())
            .max_by(|a, b| shown[*a].total_cmp(&shown[*b]))
            .unwrap();
        // 1 kHz in 32 log bands from 40 Hz to 16 kHz: band 17.
        let band = ((1000f32 / 40.).ln() / (16_000f32 / 40.).ln() * 32.) as usize;
        assert_eq!(loudest, band);
        assert!(shown[band] > 0.95, "{}", shown[band]);
        assert!(shown[3] < 0.3, "far from it: {}", shown[3]);
    }
    #[test]
    fn loudness_follows_amplitude_and_silence_is_zero() {
        assert!((level(&sine(440., 1., 1.), 0.5, -70.) - 1.).abs() < 0.02);
        let half = level(&sine(440., 0.5, 1.), 0.5, -70.);
        assert!((half - (1. - 6.02 / 70.)).abs() < 0.02, "{half}");
        assert_eq!(level(&sine(440., 1., 1.), 5., -70.), 0., "past its end");
    }
    #[test]
    fn a_waveform_holds_each_shares_peak() {
        let tone = sine(10., 0.8, 1.);
        let peaks = waveform(&tone, 0., 0.1, 4);
        // A quarter of a 10 Hz cycle each: rising to +0.8 by the first's end.
        assert!(
            (peaks[0] - 0.8).abs() < 0.01 && (peaks[2] + 0.8).abs() < 0.01,
            "{peaks:?}"
        );
    }
}
