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
        let built = match supported.sample_format() {
            cpal::SampleFormat::I16 => {
                // Mixed in floats into a buffer made here, converted in place.
                let mut held = held;
                let mut floats = vec![0f32; 8192 * format.channels];
                device.build_output_stream(
                    config,
                    move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
                        let renderer = held.renderer.as_mut().expect("held while streaming");
                        let stamp = info.timestamp();
                        let mut timing = Some(Timing {
                            asked: nanos(stamp.callback),
                            heard: nanos(stamp.playback),
                        });
                        for chunk in data.chunks_mut(floats.len()) {
                            let floats = &mut floats[..chunk.len()];
                            renderer.render(floats, timing.take());
                            for (out, &sample) in chunk.iter_mut().zip(floats.iter()) {
                                *out = (sample.clamp(-1., 1.) * 32767.) as i16;
                            }
                        }
                    },
                    errors,
                    None,
                )
            }
            _ => {
                let mut held = held;
                device.build_output_stream(
                    config,
                    move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                        let renderer = held.renderer.as_mut().expect("held while streaming");
                        let stamp = info.timestamp();
                        renderer.render(
                            data,
                            Some(Timing {
                                asked: nanos(stamp.callback),
                                heard: nanos(stamp.playback),
                            }),
                        );
                    },
                    errors,
                    None,
                )
            }
        };
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
}
