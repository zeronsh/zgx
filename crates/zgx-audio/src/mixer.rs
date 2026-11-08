//! The mix, made on the audio thread: the voices playing and the stream a
//! synth fills, added into the buffer the system hands over. What it needs
//! comes as commands through a [`ring`]; what it lets go of (a sound that
//! ended, a stream replaced) goes back the same way, to be freed off the
//! audio thread. Rendering never allocates, frees, locks or calls a script.
use crate::ring::{Reader, Writer, ring};
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

/// Decoded sound: interleaved stereo at its controller's rate.
#[derive(Clone, Debug)]
pub struct Sound(pub(crate) Arc<[f32]>);

impl Sound {
    /// Interleaved stereo, already at its controller's rate.
    pub fn from_stereo(samples: Vec<f32>) -> Self {
        Self(samples.into())
    }
    /// Its length, in frames.
    pub fn frames(&self) -> usize {
        self.0.len() / 2
    }
}

/// What a synth's stream is told each time it fills a buffer.
#[derive(Clone, Copy, Debug)]
pub struct Info {
    /// Frames a second.
    pub rate: u32,
    /// The frame this buffer starts at, counted from the stream's start.
    pub frame: u64,
}

/// Fills the output, on the audio thread: `out` is interleaved stereo, zeroed,
/// and the voices are added over what it writes. It must not allocate, lock
/// or block: its state comes in by atomics or a [`ring`](crate::ring).
pub trait Stream: Send + 'static {
    fn fill(&mut self, out: &mut [f32], info: Info);
}
impl<F: FnMut(&mut [f32], Info) + Send + 'static> Stream for F {
    fn fill(&mut self, out: &mut [f32], info: Info) {
        self(out, info)
    }
}

/// A voice playing, to change or stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Voice(u64);

/// How a sound plays.
#[derive(Clone, Copy, Debug)]
pub struct Play {
    /// Its gain, 1 as recorded.
    pub volume: f32,
    /// -1 all left, 0 both, 1 all right.
    pub pan: f32,
    /// Starts again at its end until stopped.
    pub looped: bool,
    /// Starts this many seconds after the command arrives, to the frame.
    pub delay: f64,
    /// Starts at this time on the mix's clock ([`Controller::now`]), to the
    /// frame, over `delay`: sounds scheduled on one clock keep their spacing
    /// however their commands are spaced. One whose time has passed when it
    /// arrives starts at once, and is counted late ([`Reading::late`]).
    pub at: Option<f64>,
}
impl Default for Play {
    fn default() -> Self {
        Self {
            volume: 1.,
            pan: 0.,
            looped: false,
            delay: 0.,
            at: None,
        }
    }
}

pub(crate) enum Command {
    Play {
        voice: Voice,
        sound: Arc<[f32]>,
        gain: [f32; 2],
        looped: bool,
        start: Start,
    },
    Set {
        voice: Voice,
        gain: [f32; 2],
    },
    Stop {
        voice: Voice,
    },
    Stream(Option<Box<dyn Stream>>),
}

/// When a sound starts: frames after its command is taken, or at a frame
/// of the mix's clock.
#[derive(Clone, Copy)]
pub(crate) enum Start {
    After(u64),
    At(u64),
}

pub(crate) enum Garbage {
    // Held only to be dropped where allocation is allowed.
    Sound(#[allow(dead_code)] Arc<[f32]>),
    Stream(#[allow(dead_code)] Box<dyn Stream>),
}

/// The left and right gains of `volume` at `pan`: both at full in the middle,
/// the far side falling along a quarter cosine as it pans away.
fn gain(volume: f32, pan: f32) -> [f32; 2] {
    let pan = pan.clamp(-1., 1.);
    let side = |away: f32| (away.max(0.) * std::f32::consts::FRAC_PI_2).cos();
    let volume = volume.max(0.);
    [volume * side(pan), volume * side(-pan)]
}

/// Voices a mix holds at once; a sound played past them is dropped.
pub const VOICES: usize = 64;
/// Commands waiting at once (a frame's worth of plays and changes).
const COMMANDS: usize = 1024;

/// What the audio thread did, for a host to read.
#[derive(Default)]
pub struct Stats {
    /// The mix's clock: frames rendered since the first command.
    frames: AtomicU64,
    started: AtomicU64,
    late: AtomicU64,
    peak: AtomicU32,
    /// Commands the controller has queued, and the mixer has taken: the
    /// same when none waits.
    sent: AtomicU64,
    taken: AtomicU64,
    /// Frames rendered since anything sounded (no voice, no stream).
    silent: AtomicU64,
    /// Frames of silence after which the device may close.
    close_after: AtomicU64,
}

/// What a device's thread reads to know when it may close, and whether a
/// command waits for it (to open again).
#[derive(Clone)]
pub struct Watch(Arc<Stats>);

impl Watch {
    /// A command is queued that the mixer hasn't taken.
    pub fn pending(&self) -> bool {
        self.0.sent.load(Ordering::SeqCst) != self.0.taken.load(Ordering::SeqCst)
    }
    /// Silent long enough to close, with nothing waiting.
    pub fn idle(&self) -> bool {
        self.0.silent.load(Ordering::Relaxed) >= self.0.close_after.load(Ordering::Relaxed)
            && !self.pending()
    }
}
/// A reading of [`Stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// The clock: frames rendered since the first command.
    pub frames: u64,
    /// Voices that began to sound.
    pub started: u64,
    /// Sounds whose time ([`Play::at`]) had passed when they arrived.
    pub late: u64,
    /// The loudest sample since the last reading.
    pub peak: f32,
}

struct Playing {
    voice: Voice,
    sound: Arc<[f32]>,
    /// The next frame of it.
    at: usize,
    delay: u64,
    looped: bool,
    gain: [f32; 2],
    target: [f32; 2],
    /// Fading to silence, then gone.
    stopping: bool,
    /// Has sounded.
    started: bool,
}

/// The audio thread's side.
pub struct Mixer {
    rate: u32,
    commands: Reader<Command>,
    garbage: Writer<Garbage>,
    stats: Arc<Stats>,
    voices: Vec<Playing>,
    stream: Option<Box<dyn Stream>>,
    stream_frame: u64,
    /// A command has come: the clock runs.
    begun: bool,
}

/// The side that sends it sounds, and frees what it lets go of.
pub struct Controller {
    rate: u32,
    commands: Writer<Command>,
    garbage: Reader<Garbage>,
    stats: Arc<Stats>,
    next: u64,
}

/// A controller and the mixer it drives, at `rate` frames a second. A device
/// runs the mixer on its audio thread; a test or an offline render calls
/// [`Mixer::render`] itself.
pub fn pair(rate: u32) -> (Controller, Mixer) {
    let (commands, from_controller) = ring(COMMANDS);
    // Room for every voice and every command's leftovers: never full.
    let (to_controller, garbage) = ring(VOICES + COMMANDS);
    let stats = Arc::new(Stats::default());
    stats
        .close_after
        .store(u64::from(rate) * 3, Ordering::Relaxed);
    (
        Controller {
            rate,
            commands,
            garbage,
            stats: stats.clone(),
            next: 0,
        },
        Mixer {
            rate,
            commands: from_controller,
            garbage: to_controller,
            stats,
            voices: Vec::with_capacity(VOICES),
            stream: None,
            stream_frame: 0,
            begun: false,
        },
    )
}

impl Controller {
    /// Frames a second.
    pub fn rate(&self) -> u32 {
        self.rate
    }
    fn send(&mut self, command: Command) {
        self.collect();
        // Full only if the audio thread has stopped taking: dropped then.
        if self.commands.push(command).is_ok() {
            self.stats.sent.fetch_add(1, Ordering::SeqCst);
        }
    }
    /// What a device's thread watches.
    pub fn watch(&self) -> Watch {
        Watch(self.stats.clone())
    }
    /// The device may close after `seconds` of silence (3 to begin with).
    pub fn close_after(&mut self, seconds: f64) {
        let frames = (seconds.max(0.) * f64::from(self.rate)) as u64;
        self.stats.close_after.store(frames, Ordering::Relaxed);
    }
    /// Frees what the audio thread let go of.
    pub fn collect(&mut self) {
        while self.garbage.pop().is_some() {}
    }
    /// Plays `sound`.
    pub fn play(&mut self, sound: &Sound, play: Play) -> Voice {
        self.next += 1;
        let voice = Voice(self.next);
        self.send(Command::Play {
            voice,
            sound: sound.0.clone(),
            gain: gain(play.volume, play.pan),
            looped: play.looped,
            start: match play.at {
                Some(at) => Start::At((at.max(0.) * f64::from(self.rate)).round() as u64),
                None => Start::After((play.delay.max(0.) * f64::from(self.rate)).round() as u64),
            },
        });
        voice
    }
    /// The mix's clock, in seconds: where the next buffer begins. It starts
    /// at the first command and stands still while the device is closed. A sound scheduled at `now()` plus
    /// a buffer or more ([`Play::at`]) starts on its frame.
    pub fn now(&self) -> f64 {
        self.stats.frames.load(Ordering::Acquire) as f64 / f64::from(self.rate)
    }
    /// Moves `voice` to `volume` and `pan`, smoothly over the next buffer.
    pub fn set(&mut self, voice: Voice, volume: f32, pan: f32) {
        self.send(Command::Set {
            voice,
            gain: gain(volume, pan),
        });
    }
    /// Fades `voice` out over a few milliseconds, and ends it.
    pub fn stop(&mut self, voice: Voice) {
        self.send(Command::Stop { voice });
    }
    /// `stream` fills the output from now on (`None`: silence under the
    /// voices), in place of the last one.
    pub fn stream(&mut self, stream: Option<Box<dyn Stream>>) {
        self.send(Command::Stream(stream));
    }
    /// What the audio thread has done; the peak starts again.
    pub fn reading(&self) -> Reading {
        Reading {
            frames: self.stats.frames.load(Ordering::Relaxed),
            started: self.stats.started.load(Ordering::Relaxed),
            late: self.stats.late.load(Ordering::Relaxed),
            peak: f32::from_bits(self.stats.peak.swap(0, Ordering::Relaxed)),
        }
    }
}

impl Mixer {
    fn take_commands(&mut self) {
        while let Some(command) = self.commands.pop() {
            self.stats.taken.fetch_add(1, Ordering::SeqCst);
            self.begun = true;
            match command {
                Command::Play {
                    voice,
                    sound,
                    gain,
                    looped,
                    start,
                } => {
                    let delay = match start {
                        Start::After(frames) => frames,
                        Start::At(frame) => {
                            let clock = self.stats.frames.load(Ordering::Relaxed);
                            if frame < clock {
                                self.stats.late.fetch_add(1, Ordering::Relaxed);
                            }
                            frame.saturating_sub(clock)
                        }
                    };
                    if self.voices.len() == VOICES || sound.len() < 2 {
                        let _ = self.garbage.push(Garbage::Sound(sound));
                        continue;
                    }
                    self.voices.push(Playing {
                        voice,
                        sound,
                        at: 0,
                        delay,
                        looped,
                        gain,
                        target: gain,
                        stopping: false,
                        started: false,
                    });
                }
                Command::Set { voice, gain } => {
                    if let Some(playing) = self.voices.iter_mut().find(|p| p.voice == voice) {
                        playing.target = gain;
                    }
                }
                Command::Stop { voice } => {
                    if let Some(playing) = self.voices.iter_mut().find(|p| p.voice == voice) {
                        playing.stopping = true;
                        playing.target = [0.; 2];
                    }
                }
                Command::Stream(stream) => {
                    if let Some(old) = std::mem::replace(&mut self.stream, stream) {
                        let _ = self.garbage.push(Garbage::Stream(old));
                    }
                    self.stream_frame = 0;
                }
            }
        }
    }

    /// Fills `out`, interleaved stereo, with the next of the mix.
    pub fn render(&mut self, out: &mut [f32]) {
        self.take_commands();
        out.fill(0.);
        let frames = out.len() / 2;
        if let Some(stream) = &mut self.stream {
            stream.fill(
                out,
                Info {
                    rate: self.rate,
                    frame: self.stream_frame,
                },
            );
            self.stream_frame += frames as u64;
        }
        // A change of gain, or a stop, moves at most a whole gain in about
        // 5 ms: no click.
        let step = 1. / (self.rate as f32 / 200.).max(1.);
        let mut i = 0;
        while i < self.voices.len() {
            let playing = &mut self.voices[i];
            let skip = playing.delay.min(frames as u64) as usize;
            playing.delay -= skip as u64;
            if skip < frames && !playing.started {
                playing.started = true;
                self.stats.started.fetch_add(1, Ordering::Relaxed);
            }
            let length = playing.sound.len() / 2;
            let mut ended = false;
            for frame in skip..frames {
                if playing.at >= length {
                    if playing.looped {
                        playing.at = 0;
                    } else {
                        ended = true;
                        break;
                    }
                }
                for channel in 0..2 {
                    let towards = playing.target[channel] - playing.gain[channel];
                    playing.gain[channel] += towards.clamp(-step, step);
                    out[frame * 2 + channel] +=
                        playing.sound[playing.at * 2 + channel] * playing.gain[channel];
                }
                playing.at += 1;
                if playing.stopping && playing.gain == [0.; 2] {
                    ended = true;
                    break;
                }
            }
            if ended {
                let gone = self.voices.swap_remove(i);
                let _ = self.garbage.push(Garbage::Sound(gone.sound));
            } else {
                i += 1;
            }
        }
        if self.voices.is_empty() && self.stream.is_none() {
            self.stats
                .silent
                .fetch_add(frames as u64, Ordering::Relaxed);
        } else {
            self.stats.silent.store(0, Ordering::Relaxed);
        }
        let peak = out.iter().fold(0f32, |peak, s| peak.max(s.abs()));
        self.stats.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        // The clock starts at the first command, not when the device
        // opened: a script reading it before its first sound (0) gets a time
        // that holds however long the device and that sound take to open.
        if self.begun {
            self.stats
                .frames
                .fetch_add(frames as u64, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ones(frames: usize) -> Sound {
        Sound::from_stereo(vec![1.; frames * 2])
    }

    #[test]
    fn a_sound_plays_once_at_its_volume_and_ends() {
        let (mut control, mut mixer) = pair(48_000);
        control.play(
            &ones(100),
            Play {
                volume: 0.5,
                ..Play::default()
            },
        );
        let mut out = vec![0.; 256];
        mixer.render(&mut out);
        assert!(out[..200].iter().all(|s| *s == 0.5));
        assert!(out[200..].iter().all(|s| *s == 0.), "it ended at its end");
        assert!(mixer.voices.is_empty());
        let reading = control.reading();
        assert_eq!((reading.frames, reading.started), (128, 1));
        assert_eq!(reading.peak, 0.5);
    }

    #[test]
    fn pan_keeps_the_near_side_whole_and_lowers_the_far_side() {
        assert_eq!(gain(1., 0.), [1., 1.]);
        let [left, right] = gain(1., -1.);
        assert_eq!(left, 1.);
        assert!(right.abs() < 1e-6, "all left");
        let [left, right] = gain(0.8, 0.5);
        assert!((left - 0.8 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(right, 0.8);
    }

    #[test]
    fn a_delay_starts_it_on_its_frame_across_buffers() {
        let (mut control, mut mixer) = pair(1000);
        control.play(
            &ones(10),
            Play {
                delay: 0.1,
                ..Play::default()
            },
        );
        let mut first = vec![0.; 128];
        mixer.render(&mut first);
        assert!(first.iter().all(|s| *s == 0.), "64 frames of its 100 wait");
        let mut second = vec![0.; 128];
        mixer.render(&mut second);
        let start = second.iter().position(|s| *s != 0.).unwrap() / 2;
        assert_eq!(start, 100 - 64);
        assert_eq!(second.iter().filter(|s| **s != 0.).count(), 20);
    }

    #[test]
    fn sounds_scheduled_on_the_clock_start_on_their_frames_however_they_arrive() {
        let (mut control, mut mixer) = pair(48_000);
        let click = Sound::from_stereo(vec![1., 1.]);
        let mut out = vec![0.; 512 * 2];
        let mut heard = Vec::new();
        let mut buffer = 0;
        let mut render = |mixer: &mut Mixer, heard: &mut Vec<u64>| {
            mixer.render(&mut out);
            for (i, frame) in out.as_chunks::<2>().0.iter().enumerate() {
                if frame[0] != 0. {
                    heard.push(buffer * 512 + i as u64);
                }
            }
            buffer += 1;
        };
        render(&mut mixer, &mut heard);
        let t = control.now() + 0.05;
        assert_eq!(t, 0.05, "no command yet: the clock hasn't started");
        // Each command a few buffers after the last: their own arrival
        // would space them unevenly.
        for (i, wait) in [0, 3, 7].into_iter().enumerate() {
            for _ in 0..wait {
                render(&mut mixer, &mut heard);
            }
            let at = Some(t + i as f64 * 0.2);
            control.play(
                &click,
                Play {
                    at,
                    ..Play::default()
                },
            );
        }
        while mixer.voices.len() + heard.len() < 3 || !mixer.voices.is_empty() {
            render(&mut mixer, &mut heard);
        }
        // The clock began with the buffer that took the first command, the
        // second rendered.
        let first = 512 + 2400;
        assert_eq!(heard, [first, first + 9600, first + 19_200]);
        assert_eq!(control.reading().late, 0);

        // One whose time has passed starts at once, counted late.
        control.play(
            &click,
            Play {
                at: Some(0.),
                ..Play::default()
            },
        );
        let clock = (control.now() * 48_000.).round() as u64;
        render(&mut mixer, &mut heard);
        assert_eq!(heard.last(), Some(&(512 + clock)));
        assert_eq!(control.reading().late, 1);
    }

    #[test]
    fn a_loop_wraps_and_a_stop_fades_it_out() {
        let (mut control, mut mixer) = pair(1000);
        let voice = control.play(
            &Sound::from_stereo((0..8).map(|i| (i / 2) as f32).collect()),
            Play {
                looped: true,
                ..Play::default()
            },
        );
        let mut out = vec![0.; 16];
        mixer.render(&mut out);
        assert_eq!(&out[..8], &out[8..], "the four frames again");
        control.stop(voice);
        let mut fading = vec![0.; 32];
        mixer.render(&mut fading);
        assert!(mixer.voices.is_empty(), "ended within its 5 frames' fade");
        assert!(fading[10..].iter().all(|s| *s == 0.));
    }

    #[test]
    fn the_stream_fills_under_the_voices_and_counts_its_frames() {
        let (mut control, mut mixer) = pair(1000);
        control.stream(Some(Box::new(|out: &mut [f32], info: Info| {
            out.fill(info.frame as f32);
        })));
        control.play(&ones(1), Play::default());
        let mut out = vec![0.; 8];
        mixer.render(&mut out);
        assert_eq!(out, [1., 1., 0., 0., 0., 0., 0., 0.]);
        mixer.render(&mut out);
        assert!(
            out.iter().all(|s| *s == 4.),
            "its second buffer starts at frame 4"
        );
    }

    #[test]
    fn it_is_idle_after_its_silence_and_not_while_a_command_waits() {
        let (mut control, mut mixer) = pair(1000);
        control.close_after(0.1);
        let watch = control.watch();
        let mut out = vec![0.; 100];
        mixer.render(&mut out);
        assert!(!watch.idle(), "50 frames of the 100");
        mixer.render(&mut out);
        assert!(watch.idle());
        control.play(&ones(60), Play::default());
        assert!(watch.pending() && !watch.idle(), "a sound waits for it");
        mixer.render(&mut out);
        assert!(!watch.pending() && !watch.idle());
        mixer.render(&mut out);
        assert!(!watch.idle(), "it ended in this buffer");
        mixer.render(&mut out);
        assert!(watch.idle());
    }

    #[test]
    fn what_it_lets_go_of_comes_back_to_be_freed() {
        let (mut control, mut mixer) = pair(1000);
        let sound = ones(1);
        control.play(&sound, Play::default());
        mixer.render(&mut [0.; 8]);
        assert_eq!(
            Arc::strong_count(&sound.0),
            2,
            "the ended voice's, queued back"
        );
        control.collect();
        assert_eq!(Arc::strong_count(&sound.0), 1);
    }
}
