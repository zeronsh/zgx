//! Sound for zgx, in a crate of its own: an app that doesn't depend on it
//! links none of it, and one that does opens no device until it plays.
//!
//! An [`Engine`] plays [`Sound`]s read from files, once or looped, each a
//! [`Voice`] with a volume and a pan, and runs a [`Stream`] a synth fills, on
//! the system's audio thread. The mix ([`mixer::Mixer`]) takes what it needs
//! by lock-free queues and gives back what it lets go of, so the audio thread
//! never allocates, locks or runs a script. `docs/audio-design.md` is the
//! design; this is its prototype, with output on macOS, iOS, Windows and Linux.
pub mod mixer;
pub mod ring;
#[cfg(feature = "script")]
pub mod script;
pub mod wav;

mod backend;

pub use mixer::{Info, Play, Reading, Sound, Stream, Voice};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// The system's output, playing a mix.
pub struct Engine {
    control: mixer::Controller,
    sounds: HashMap<PathBuf, Sound>,
    // Dropped last: stops the audio thread before the queues go.
    output: backend::Output,
}

impl Engine {
    /// Opens the default output and starts it, silent.
    pub fn open() -> Result<Self, String> {
        let device = backend::Device::open()?;
        let (control, mixer) = mixer::pair(device.rate());
        let output = device.start(mixer, control.watch())?;
        Ok(Self {
            control,
            sounds: HashMap::new(),
            output,
        })
    }
    /// Frames a second, the device's.
    pub fn rate(&self) -> u32 {
        self.control.rate()
    }
    /// The sound in `path`, read once and kept: a WAV file, or on macOS
    /// anything the system decodes (MP3, AAC, ALAC, AIFF, CAF, FLAC).
    pub fn load(&mut self, path: &Path) -> Result<Sound, String> {
        if let Some(sound) = self.sounds.get(path) {
            return Ok(sound.clone());
        }
        let (stereo, rate) = decode(path)?;
        let sound = Sound::from_stereo(wav::resample(&stereo, rate, self.rate()));
        self.sounds.insert(path.to_owned(), sound.clone());
        Ok(sound)
    }
    /// Interleaved stereo at `rate`, as a sound to play.
    pub fn sound(&self, stereo: &[f32], rate: u32) -> Sound {
        Sound::from_stereo(wav::resample(stereo, rate, self.rate()))
    }
    /// Plays `sound`.
    pub fn play(&mut self, sound: &Sound, play: Play) -> Voice {
        let voice = self.control.play(sound, play);
        self.output.wake();
        voice
    }
    /// Moves `voice` to `volume` and `pan`.
    pub fn set(&mut self, voice: Voice, volume: f32, pan: f32) {
        self.control.set(voice, volume, pan);
        self.output.wake();
    }
    /// Fades `voice` out and ends it.
    pub fn stop(&mut self, voice: Voice) {
        self.control.stop(voice);
        self.output.wake();
    }
    /// `stream` fills the output from now on, under the voices.
    pub fn stream(&mut self, stream: impl Stream) {
        self.control.stream(Some(Box::new(stream)));
        self.output.wake();
    }
    /// Silence under the voices again.
    pub fn end_stream(&mut self) {
        self.control.stream(None);
        self.output.wake();
    }
    /// The device closes after `seconds` of silence (3 to begin with), and
    /// opens again at the next sound.
    pub fn close_after(&mut self, seconds: f64) {
        self.control.close_after(seconds);
    }
    /// The mix's clock, in seconds: where the next buffer begins. Sounds
    /// scheduled on it ([`Play::at`]) start on their frames; schedule a
    /// little ahead of it (a buffer, a few milliseconds).
    pub fn now(&self) -> f64 {
        self.control.now()
    }
    /// Whether the device is open and playing (not closed after silence).
    pub fn is_running(&self) -> bool {
        self.output.is_running()
    }
    /// The device it plays on (an `AudioObjectID`), for tests.
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn device(&self) -> Option<u32> {
        self.output.device()
    }
    /// What the audio thread has done since it started.
    pub fn reading(&mut self) -> Reading {
        self.control.collect();
        self.control.reading()
    }
}

/// Interleaved stereo and its rate, from the file at `path`.
fn decode(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.starts_with(b"RIFF") {
        return wav::read(&bytes).map_err(|error| format!("{}: {error}", path.display()));
    }
    #[cfg(target_os = "macos")]
    {
        // The prototype converts through the system's tool; the design reads
        // with ExtAudioFile on Apple systems and Symphonia elsewhere.
        let converted = std::env::temp_dir().join(format!("zgx-audio-{}.wav", std::process::id()));
        let ran = std::process::Command::new("/usr/bin/afconvert")
            .args(["-f", "WAVE", "-d", "LEF32", "-c", "2"])
            .arg(path)
            .arg(&converted)
            .status();
        let read = std::fs::read(&converted);
        let _ = std::fs::remove_file(&converted);
        if ran.is_ok_and(|status| status.success())
            && let Ok(bytes) = read
        {
            return wav::read(&bytes).map_err(|error| format!("{}: {error}", path.display()));
        }
    }
    Err(format!(
        "{}: only WAV files can be read here",
        path.display()
    ))
}
