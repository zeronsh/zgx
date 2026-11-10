//! Playing to the system's output: a [`Device`] runs the mixer, wrapped as
//! a [`Renderer`], on the system's audio thread. A host thread starts the
//! device with the first sound and stops it after a hold of silence, then
//! sleeps until the next sound: with nothing playing, nothing runs.
use crate::mixer::{Meter, Mixer, Timing};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// The format of a device's stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// Frames a second.
    pub rate: u32,
    /// Channels, interleaved: stereo is mixed, and copied or summed to
    /// these.
    pub channels: usize,
}

impl Format {
    /// It, if the mixer can run at it: what [`Config::check`] allows of a
    /// rate (1 kHz to 768 kHz) and of channels (1 to 64).
    ///
    /// [`Config::check`]: crate::Config::check
    fn check(self) -> Result<Format, String> {
        if (1_000..=768_000).contains(&self.rate) && (1..=64).contains(&self.channels) {
            Ok(self)
        } else {
            Err(format!(
                "the output asks for {} channels at {} Hz, which the mixer can't run at",
                self.channels, self.rate
            ))
        }
    }
}

/// The mixer as a device runs it, block by block on its own thread.
pub struct Renderer {
    mixer: Mixer,
    channels: usize,
    meter: Arc<Meter>,
}

impl Renderer {
    pub(crate) fn new(mixer: Mixer, channels: usize, meter: Arc<Meter>) -> Renderer {
        clock::prime();
        Renderer {
            mixer,
            channels: channels.max(1),
            meter,
        }
    }

    /// The format it mixes in.
    pub fn format(&self) -> Format {
        Format {
            rate: self.mixer.rate(),
            channels: self.channels,
        }
    }

    /// Applies what's been sent while no stream runs: gains and mutes take
    /// effect, plays find their voices. Returns whether anything would
    /// sound, so a device need start. Off the audio thread.
    pub fn idle(&mut self) -> bool {
        self.mixer.idle()
    }

    /// Mixes in `format` from now on. Allocates: call it off the audio
    /// thread, between streams.
    pub(crate) fn set_format(&mut self, format: Format) {
        self.mixer.set_rate(format.rate);
        self.channels = format.channels.max(1);
        self.meter
            .rate
            .store(u64::from(self.mixer.rate()), Ordering::Release);
    }

    /// Fills `out` (interleaved, `format().channels` wide) with the next
    /// frames. `timing`, when the device knows it, places the block on the
    /// device's clock, which is how the time a sound is heard is known.
    /// Never locks, allocates, frees or calls the system.
    pub fn render(&mut self, out: &mut [f32], timing: Option<Timing>) {
        let plays = self.meter.play_heard.load(Ordering::Relaxed);
        let began = clock::now();
        self.mixer.render(out, self.channels, timing);
        let ended = clock::now();
        if self.meter.play_heard.load(Ordering::Relaxed) != plays {
            self.meter
                .play_called
                .store(began.ticks(), Ordering::Relaxed);
        }
        let micros = ended.micros_since(began);
        self.meter.times[micros.min(self.meter.times.len() - 1)].fetch_add(1, Ordering::Relaxed);
    }
}

/// A clock read on the audio thread without a system call: on macOS
/// `mach_absolute_time` (whose ticks CoreAudio's timestamps count), kept
/// as ticks and made nanoseconds only when read out; elsewhere `Instant`.
pub(crate) mod clock {
    #[cfg(target_os = "macos")]
    mod mach {
        unsafe extern "C" {
            pub fn mach_absolute_time() -> u64;
            pub fn mach_timebase_info(info: *mut [u32; 2]) -> i32;
        }
        static TIMEBASE: std::sync::OnceLock<[u32; 2]> = std::sync::OnceLock::new();
        /// Nanoseconds to a tick, as a fraction; read once, before any
        /// stream runs.
        pub fn timebase() -> [u32; 2] {
            *TIMEBASE.get_or_init(|| {
                let mut info = [1, 1];
                // SAFETY: fills two u32s.
                unsafe { mach_timebase_info(&mut info) };
                [info[0], info[1].max(1)]
            })
        }
    }

    #[derive(Clone, Copy)]
    pub struct Now(
        #[cfg(target_os = "macos")] u64,
        #[cfg(not(target_os = "macos"))] std::time::Instant,
    );

    pub fn now() -> Now {
        // SAFETY: reads the page the kernel shares with every process.
        #[cfg(target_os = "macos")]
        return Now(unsafe { mach::mach_absolute_time() });
        #[cfg(not(target_os = "macos"))]
        return Now(std::time::Instant::now());
    }

    /// Ready to read before the first stream: the timebase is asked once.
    pub fn prime() {
        #[cfg(target_os = "macos")]
        mach::timebase();
    }

    impl Now {
        /// The raw ticks (macOS; 0 elsewhere), for [`nanos`].
        pub fn ticks(self) -> u64 {
            #[cfg(target_os = "macos")]
            return self.0;
            #[cfg(not(target_os = "macos"))]
            return 0;
        }

        pub fn micros_since(self, earlier: Now) -> usize {
            #[cfg(target_os = "macos")]
            {
                let [numer, denom] = mach::timebase();
                (self.0.saturating_sub(earlier.0) * u64::from(numer) / u64::from(denom) / 1000)
                    as usize
            }
            #[cfg(not(target_os = "macos"))]
            {
                self.0.duration_since(earlier.0).as_micros() as usize
            }
        }
    }

    /// `ticks` as nanoseconds of the host's clock.
    pub fn nanos(ticks: u64) -> u64 {
        #[cfg(target_os = "macos")]
        {
            let [numer, denom] = mach::timebase();
            (u128::from(ticks) * u128::from(numer) / u128::from(denom)) as u64
        }
        #[cfg(not(target_os = "macos"))]
        {
            ticks
        }
    }
}

/// An output: the system's default device through some API. zgx-audio's
/// own is `Cpal`, with the `device` feature; another goes in
/// [`Audio::with_device`](crate::Audio::with_device).
///
/// Every method is called from the engine's host thread, never the audio
/// thread, and never two at once.
pub trait Device: Send + 'static {
    /// The format a stream on the default output would have now.
    fn format(&mut self) -> Result<Format, String>;

    /// Starts a stream in `format`, `renderer` filling each block. On
    /// failure, gives the renderer back.
    fn start(
        &mut self,
        renderer: Box<Renderer>,
        format: Format,
    ) -> Result<(), (String, Box<Renderer>)>;

    /// Stops the stream. A device that can pause may keep the renderer to
    /// [`resume`](Device::resume) with (and returns `None`); one that can't
    /// gives it back.
    fn stop(&mut self) -> Option<Box<Renderer>>;

    /// Starts again what a [`stop`](Device::stop) kept. `false` if it kept
    /// nothing: then [`start`](Device::start) is called.
    fn resume(&mut self) -> bool {
        false
    }

    /// Ends the stream for good and gives the renderer back.
    fn close(&mut self) -> Option<Box<Renderer>>;

    /// Whether the stream was lost (its device gone, or invalidated by a
    /// change of format) since this was last asked: it is then closed and
    /// started again on the default output, keeping every voice.
    fn lost(&mut self) -> bool {
        false
    }

    /// Underruns the system has reported.
    fn underruns(&self) -> u64 {
        0
    }
}

pub(crate) enum Message {
    /// A command was sent while the device was stopped.
    Wake,
    /// Start now, and hold for a while even if nothing plays.
    Prewarm,
    Quit,
}

/// How often a running host looks for silence (and a lost stream).
const CHECK: Duration = Duration::from_millis(100);

/// The host thread: owns the device, starts and stops it.
pub(crate) fn host(
    mut device: Box<dyn Device>,
    renderer: Box<Renderer>,
    messages: Receiver<Message>,
    meter: Arc<Meter>,
    hold: Duration,
) {
    let mut renderer = Some(renderer);
    let mut quit = false;
    while !quit {
        // Stopped: asleep until a command comes. One that makes no sound
        // (a gain, a mute) is applied here, without the device.
        match messages.recv() {
            Ok(Message::Wake) => {
                if let Some(held) = renderer.as_mut()
                    && !held.idle()
                {
                    continue;
                }
            }
            Ok(Message::Prewarm) => {}
            Ok(Message::Quit) | Err(_) => break,
        }
        'start: loop {
            let began = Instant::now();
            let resumed = renderer.is_none() && device.resume();
            if !resumed {
                let mut held = match renderer.take().or_else(|| device.close()) {
                    Some(held) => held,
                    None => break 'start,
                };
                // A format the mixer can't run at (a rate or a channel count
                // a config couldn't have) is the device failing, not a
                // stream started at it.
                let format = match device.format().and_then(|format| format.check()) {
                    Ok(format) => format,
                    Err(why) => {
                        meter.failures.fetch_add(1, Ordering::Relaxed);
                        meter.set_error(why);
                        renderer = Some(held);
                        break 'start;
                    }
                };
                held.set_format(format);
                if let Err((why, back)) = device.start(held, format) {
                    meter.failures.fetch_add(1, Ordering::Relaxed);
                    meter.set_error(why);
                    renderer = Some(back);
                    break 'start;
                }
            }
            meter
                .start_micros
                .store(began.elapsed().as_micros() as u64, Ordering::Relaxed);
            meter.starts.fetch_add(1, Ordering::Relaxed);
            meter.running.store(true, Ordering::SeqCst);
            // Held from here as if something had just been heard.
            let mut floor = meter.now.load(Ordering::Acquire);
            let mut told = meter.ended.load(Ordering::Acquire);
            loop {
                match messages.recv_timeout(CHECK) {
                    Ok(Message::Quit) | Err(RecvTimeoutError::Disconnected) => {
                        quit = true;
                        break;
                    }
                    Ok(Message::Prewarm) => floor = meter.now.load(Ordering::Acquire),
                    Ok(Message::Wake) | Err(RecvTimeoutError::Timeout) => {}
                }
                meter.underruns.store(device.underruns(), Ordering::Relaxed);
                let ended = meter.ended.load(Ordering::Acquire);
                if ended != told {
                    told = ended;
                    meter.notify();
                }
                if device.lost() {
                    meter.losses.fetch_add(1, Ordering::Relaxed);
                    renderer = device.close();
                    meter.running.store(false, Ordering::SeqCst);
                    continue 'start;
                }
                let rate = meter.rate.load(Ordering::Acquire).max(1);
                let now = meter.now.load(Ordering::Acquire);
                let heard = meter.audible.load(Ordering::Acquire).max(floor);
                let silent =
                    Duration::from_secs_f64(now.saturating_sub(heard) as f64 / rate as f64);
                if silent >= hold && !meter.pending() {
                    if let Some(back) = device.stop() {
                        renderer = Some(back);
                    }
                    meter.running.store(false, Ordering::SeqCst);
                    meter.stops.fetch_add(1, Ordering::Relaxed);
                    // Voices that ended in its last blocks.
                    if meter.ended.load(Ordering::Acquire) != told {
                        meter.notify();
                    }
                    // A command sent as it stopped, which saw it running and
                    // sent no wake: applied, and started again if it sounds.
                    if meter.pending() && renderer.as_mut().is_none_or(|held| held.idle()) {
                        continue 'start;
                    }
                    break;
                }
            }
            break;
        }
    }
    meter.running.store(false, Ordering::SeqCst);
    // The stream ends here, on this thread; the renderer comes back to be
    // dropped here too.
    drop(device.close());
    drop(renderer);
}

/// The host's handle, as [`Audio`](crate::Audio) holds it.
pub(crate) struct Host {
    pub messages: Sender<Message>,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.messages.send(Message::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
