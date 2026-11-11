//! The system's default output through cpal: CoreAudio on macOS, WASAPI
//! (shared mode) on Windows, ALSA on Linux.
use crate::device::{Device, Format, Renderer};
use crate::mixer::Timing;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

/// What a stop does with cpal's stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stopping {
    /// Pauses it, keeping it (and its threads) to resume. On macOS this is
    /// the slower restart (about 40 ms against 9) and leaves three more
    /// threads parked, so it isn't the default.
    Pause,
    /// Drops it: nothing of it is left, and the next start builds a new one.
    #[default]
    Close,
}

/// The frames asked of a stream's block: about 5 ms at 48 kHz, where the
/// device allows it.
const BLOCK: u32 = 256;

/// The default output through cpal.
pub struct Cpal {
    host: Option<cpal::Host>,
    stopping: Stopping,
    stream: Option<cpal::Stream>,
    paused: bool,
    device: Option<(cpal::Device, cpal::SupportedStreamConfig)>,
    back: (Sender<Box<Renderer>>, Receiver<Box<Renderer>>),
    lost: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
}

impl Default for Cpal {
    fn default() -> Self {
        Cpal::new()
    }
}

impl Cpal {
    /// Touches nothing of the system until the engine first starts it.
    pub fn new() -> Cpal {
        Cpal {
            host: None,
            stopping: Stopping::default(),
            stream: None,
            paused: false,
            device: None,
            back: channel(),
            lost: Arc::new(AtomicBool::new(false)),
            underruns: Arc::new(AtomicU64::new(0)),
        }
    }

    /// What a stop does with the stream.
    pub fn stopping(mut self, stopping: Stopping) -> Cpal {
        self.stopping = stopping;
        self
    }
}

/// The renderer inside cpal's callback; sent back when the callback is
/// dropped with its stream.
struct Held {
    renderer: Option<Box<Renderer>>,
    back: Sender<Box<Renderer>>,
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Some(renderer) = self.renderer.take() {
            let _ = self.back.send(renderer);
        }
    }
}

/// Whether the mixer's floats can be written as `format`: every PCM
/// format cpal has, not DSD's one-bit streams.
fn writable(format: cpal::SampleFormat) -> bool {
    use cpal::SampleFormat::*;
    matches!(
        format,
        I8 | I16 | I24 | I32 | I64 | U8 | U16 | U24 | U32 | U64 | F32 | F64
    )
}

fn unwritable(format: cpal::SampleFormat) -> String {
    format!("the device's samples are {format}, which zgx-audio can't write")
}

/// The largest float below 1: 1 itself is a step past the largest 24-bit
/// sample, which wraps to the most negative.
const BELOW_ONE: f32 = 1. - f32::EPSILON / 2.;

/// The mixer's floats as `T`, kept to full scale. Not a number is silence
/// (in an unsigned format, 0 is the most negative sample).
fn write<T: cpal::Sample + cpal::FromSample<f32>>(floats: &[f32], out: &mut [T]) {
    for (out, &sample) in out.iter_mut().zip(floats) {
        let sample = if sample.is_nan() { 0. } else { sample };
        *out = T::from_sample(sample.clamp(-1., BELOW_ONE));
    }
}

/// `floats` written into `data` from sample `at` on, as its format's
/// samples: one loop, chosen once for the buffer.
fn write_into(data: &mut cpal::Data, at: usize, floats: &[f32]) {
    use cpal::SampleFormat::*;
    fn into<T: cpal::SizedSample + cpal::FromSample<f32>>(
        data: &mut cpal::Data,
        at: usize,
        floats: &[f32],
    ) {
        if let Some(out) = data.as_slice_mut::<T>() {
            write(floats, &mut out[at..at + floats.len()]);
        }
    }
    match data.sample_format() {
        I8 => into::<i8>(data, at, floats),
        I16 => into::<i16>(data, at, floats),
        I24 => into::<cpal::I24>(data, at, floats),
        I32 => into::<i32>(data, at, floats),
        I64 => into::<i64>(data, at, floats),
        U8 => into::<u8>(data, at, floats),
        U16 => into::<u16>(data, at, floats),
        U24 => into::<cpal::U24>(data, at, floats),
        U32 => into::<u32>(data, at, floats),
        U64 => into::<u64>(data, at, floats),
        F64 => into::<f64>(data, at, floats),
        // Floats are mixed in place; DSD is refused before a stream.
        _ => {}
    }
}

fn nanos(instant: cpal::StreamInstant) -> u64 {
    instant.as_nanos().min(u128::from(u64::MAX)) as u64
}

impl Device for Cpal {
    fn format(&mut self) -> Result<Format, String> {
        let host = self.host.get_or_insert_with(cpal::default_host);
        let device = host
            .default_output_device()
            .ok_or("no default output device")?;
        let config = device.default_output_config().map_err(|e| e.to_string())?;
        if !writable(config.sample_format()) {
            return Err(unwritable(config.sample_format()));
        }
        let format = Format {
            rate: config.sample_rate(),
            channels: usize::from(config.channels()),
        };
        self.device = Some((device, config));
        Ok(format)
    }

    fn start(
        &mut self,
        renderer: Box<Renderer>,
        format: Format,
    ) -> Result<(), (String, Box<Renderer>)> {
        let Some((device, supported)) = self.device.take() else {
            return Err(("no device chosen".into(), renderer));
        };
        let buffer_size = match supported.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&BLOCK) => {
                cpal::BufferSize::Fixed(BLOCK)
            }
            _ => cpal::BufferSize::Default,
        };
        let config = cpal::StreamConfig {
            channels: format.channels as u16,
            sample_rate: format.rate,
            buffer_size,
        };
        let (lost, underruns) = (self.lost.clone(), self.underruns.clone());
        let errors = move |error: cpal::Error| match error.kind() {
            cpal::ErrorKind::Xrun => {
                underruns.fetch_add(1, Ordering::Relaxed);
            }
            // The default output's unit follows the system's default by
            // itself; anything else ends the stream.
            cpal::ErrorKind::DeviceChanged => {}
            _ => lost.store(true, Ordering::Relaxed),
        };
        let held = Held {
            renderer: Some(renderer),
            back: self.back.0.clone(),
        };
        let channels = format.channels;
        let format_of = supported.sample_format();
        if !writable(format_of) {
            // Refused by `format`, which chose the device.
            drop(held);
            let renderer = self.back.1.recv().expect("the renderer, given back");
            return Err((unwritable(format_of), renderer));
        }
        // One callback for every format, built once (not one a sample
        // type): floats mixed straight into a float device's buffer, and
        // into one of their own for the rest, converted after.
        let mut held = held;
        let mut floats = vec![0f32; 8192 * channels.max(1)];
        let built = device.build_output_stream_raw(
            config,
            format_of,
            move |data: &mut cpal::Data, info: &cpal::OutputCallbackInfo| {
                let renderer = held.renderer.as_mut().expect("held while streaming");
                let stamp = info.timestamp();
                let mut timing = Some(Timing {
                    asked: nanos(stamp.callback),
                    heard: nanos(stamp.playback),
                });
                if let Some(out) = data.as_slice_mut::<f32>() {
                    renderer.render(out, timing);
                    return;
                }
                let mut at = 0;
                while at < data.len() {
                    let n = (data.len() - at).min(floats.len());
                    let floats = &mut floats[..n];
                    renderer.render(floats, timing.take());
                    write_into(data, at, floats);
                    at += n;
                }
            },
            errors,
            None,
        );
        let stream = match built {
            Ok(stream) => stream,
            Err(error) => {
                // The callback, and the renderer in it, dropped with the
                // failed stream: it's on its way back.
                let renderer = self
                    .back
                    .1
                    .try_recv()
                    .map_err(|_| "a renderer lost with its stream".to_string());
                return match renderer {
                    Ok(renderer) => Err((error.to_string(), renderer)),
                    Err(why) => panic!("{why}: {error}"),
                };
            }
        };
        if let Err(error) = stream.play() {
            drop(stream);
            let renderer = self
                .back
                .1
                .recv()
                .expect("the stream gives its renderer back");
            return Err((error.to_string(), renderer));
        }
        self.stream = Some(stream);
        self.paused = false;
        self.lost.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn stop(&mut self) -> Option<Box<Renderer>> {
        match self.stopping {
            Stopping::Pause => {
                if let Some(stream) = &self.stream
                    && stream.pause().is_ok()
                {
                    self.paused = true;
                    return None;
                }
                self.close()
            }
            Stopping::Close => self.close(),
        }
    }

    fn resume(&mut self) -> bool {
        if !self.paused {
            return false;
        }
        match &self.stream {
            Some(stream) if stream.play().is_ok() => {
                self.paused = false;
                true
            }
            _ => false,
        }
    }

    fn close(&mut self) -> Option<Box<Renderer>> {
        self.paused = false;
        drop(self.stream.take()?);
        self.back.1.recv().ok()
    }

    fn lost(&mut self) -> bool {
        self.lost.swap(false, Ordering::Relaxed)
    }

    fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn making_it_touches_nothing_of_the_system() {
        // No host, no device, no stream until the engine first starts it:
        // an app that never plays a sound never loads the system's audio.
        let cpal = Cpal::new();
        assert!(cpal.host.is_none() && cpal.device.is_none() && cpal.stream.is_none());
    }

    #[test]
    fn every_pcm_format_a_device_can_ask_for_is_written_at_full_scale() {
        use cpal::SampleFormat::*;
        for format in [I8, I16, I24, I32, I64, U8, U16, U24, U32, U64, F32, F64] {
            assert!(writable(format), "{format}");
        }
        assert!(!writable(DsdU8) && unwritable(DsdU8).contains("can't write"));
        let floats = [0., 1., -1., 0.5, 2., f32::NAN];
        let mut i32s = [0i32; 6];
        write(&floats, &mut i32s);
        assert_eq!((i32s[0], i32s[2], i32s[5]), (0, i32::MIN, 0));
        assert!(i32s[1] > i32::MAX - 256 && i32s[4] == i32s[1]);
        assert!((i32s[3] - (1 << 30)).abs() <= 1);
        let mut u16s = [0u16; 6];
        write(&floats, &mut u16s);
        assert_eq!(&u16s[..3], &[32_768, u16::MAX, 0], "silence in the middle");
        assert_eq!(u16s[5], 32_768, "not a number is silence, not the floor");
        use cpal::Sample;
        let mut i24s = [cpal::I24::EQUILIBRIUM; 3];
        write(&floats[..3], &mut i24s);
        assert_eq!(i24s.map(|s| s.inner()), [0, (1 << 23) - 1, -(1 << 23)]);
        let mut i16s = [0i16; 3];
        write(&floats[..3], &mut i16s);
        assert_eq!(i16s, [0, i16::MAX, i16::MIN]);
    }

    #[test]
    fn one_callback_writes_any_format_s_buffer_a_piece_at_a_time() {
        // A device's buffer in three formats, written in two pieces as the
        // callback does when its float buffer is smaller.
        let floats = [0., 0.5, -1., 1., 0.25, -0.25];
        let mut i16s = [0i16; 6];
        let mut data =
            unsafe { cpal::Data::from_parts(i16s.as_mut_ptr().cast(), 6, cpal::SampleFormat::I16) };
        write_into(&mut data, 0, &floats[..4]);
        write_into(&mut data, 4, &floats[4..]);
        assert_eq!(i16s, [0, 16_384, i16::MIN, i16::MAX, 8_192, -8_192]);
        let mut u8s = [0u8; 6];
        let mut data =
            unsafe { cpal::Data::from_parts(u8s.as_mut_ptr().cast(), 6, cpal::SampleFormat::U8) };
        write_into(&mut data, 0, &floats);
        assert_eq!(u8s[0], 128, "silence in the middle");
        let mut f64s = [0f64; 6];
        let mut data =
            unsafe { cpal::Data::from_parts(f64s.as_mut_ptr().cast(), 6, cpal::SampleFormat::F64) };
        write_into(&mut data, 2, &floats[..4]);
        assert_eq!(f64s, [0., 0., 0., 0.5, -1., f64::from(BELOW_ONE)]);
    }
}
