//! A composition's sound through the speakers as its player plays: the mix a
//! render would mux ([`crate::render::placed_audio`], [`crate::audio::mixed`]),
//! made on a thread of its own whenever the composition, its props or its
//! script change, and played from where the picture is. It keeps the clock the
//! frames keep: played, sought or looped, it starts again from the frame due;
//! stopped, its output stops too, so a player at rest does no work. macOS
//! plays it through an AudioQueue; elsewhere it's silent for now.
use crate::audio::SAMPLE_RATE;
use crate::render::{RenderOptions, placed_audio};
use serde_json::Value as Json;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// What the speakers play: a mix, where in it they are and whether they
/// play, shared with the output's own thread.
#[derive(Debug, Default)]
pub(crate) struct Tape {
    /// The mix, interleaved stereo at [`SAMPLE_RATE`]: `None` until it's
    /// made, and for a composition with no sound.
    pub(crate) samples: Option<Arc<[f32]>>,
    /// The sample frame (a left and a right) played next.
    pub(crate) at: usize,
    /// Whether it plays.
    pub(crate) playing: bool,
    /// Which mix it waits for: one made for a composition, props or script
    /// asked for before is dropped.
    asked: u64,
    /// A mix is being made (one at a time).
    mixing: bool,
    /// The latest mix asked for while one was being made: props dragged
    /// through many values make the mix of the last, not one for each.
    next: Option<Mixing>,
}

/// A mix to make: what a render's options say, so long, for the `asked`th
/// time asked.
#[derive(Debug)]
struct Mixing {
    options: RenderOptions,
    frames: u64,
    asked: u64,
}
impl Tape {
    /// Fills `out` (interleaved stereo) from where it is, with silence past
    /// its end or before its mix is made, and moves on as far: time passes
    /// either way, so a mix made while playing comes in where the picture is.
    pub(crate) fn fill(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let played = match &self.samples {
            Some(samples) => {
                let from = (self.at * 2).min(samples.len());
                let to = ((self.at + frames) * 2).min(samples.len());
                out[..to - from].copy_from_slice(&samples[from..to]);
                to - from
            }
            None => 0,
        };
        out[played..].fill(0.);
        self.at += frames;
    }
}

pub(crate) fn lock(tape: &Mutex<Tape>) -> MutexGuard<'_, Tape> {
    tape.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The speakers, as a player's thread drives them.
pub(crate) struct Speaker {
    tape: Arc<Mutex<Tape>>,
    /// The output, made the first time it plays.
    #[cfg(target_os = "macos")]
    queue: Option<queue::Queue>,
    /// There's no output here (said once): it stays silent.
    #[cfg(target_os = "macos")]
    failed: bool,
    /// Its output runs. (The tape may have stopped already: a player stopped
    /// stops it at once, while its thread may still be drawing.)
    #[cfg(target_os = "macos")]
    running: bool,
}
impl Speaker {
    pub fn new(tape: Arc<Mutex<Tape>>) -> Self {
        Self {
            tape,
            #[cfg(target_os = "macos")]
            queue: None,
            #[cfg(target_os = "macos")]
            failed: false,
            #[cfg(target_os = "macos")]
            running: false,
        }
    }
    /// Make the mix of composition `id` of the script at `path`, with `props`
    /// over its defaults, `frames` long, on a thread of its own: the tape
    /// takes it when it's made, unless another was asked for since. `keep`
    /// plays the mix it has until then (the same composition, built again);
    /// else it's silent until then.
    pub fn mix(&self, path: &Path, id: &str, props: &Json, frames: u64, keep: bool) {
        let first = {
            let mut tape = lock(&self.tape);
            tape.asked += 1;
            if !keep {
                tape.samples = None;
            }
            let mixing = Mixing {
                options: RenderOptions {
                    script: path.to_owned(),
                    composition: Some(id.to_owned()),
                    props: props.clone(),
                    ..Default::default()
                },
                frames,
                asked: tape.asked,
            };
            // One being made: this one's made once it's done.
            if std::mem::replace(&mut tape.mixing, true) {
                tape.next = Some(mixing);
                return;
            }
            mixing
        };
        let tape = self.tape.clone();
        let made = std::thread::Builder::new()
            .name("zgx-video sound".into())
            .spawn(move || {
                let mut mixing = Some(first);
                while let Some(Mixing {
                    options,
                    frames,
                    asked,
                }) = mixing
                {
                    let mixed = placed_audio(&options)
                        .and_then(|(clips, fps)| crate::audio::mixed(&clips, fps, frames));
                    let mut tape = lock(&tape);
                    if tape.asked == asked {
                        match mixed {
                            Ok(samples) => tape.samples = samples.map(Arc::from),
                            Err(error) => eprintln!("zgx-video: no sound: {error}"),
                        }
                    }
                    mixing = tape.next.take();
                    tape.mixing = mixing.is_some();
                }
            });
        if let Err(error) = made {
            lock(&self.tape).mixing = false;
            eprintln!("zgx-video: no sound: {error}");
        }
    }
    /// Stop, and forget the mix: sound is off.
    pub fn clear(&mut self) {
        self.stop();
        let mut tape = lock(&self.tape);
        tape.asked += 1;
        tape.samples = None;
        tape.next = None;
    }
    /// Play from `seconds` into the composition.
    pub fn play_at(&mut self, seconds: f64) {
        self.stop();
        {
            let mut tape = lock(&self.tape);
            tape.at = (seconds.max(0.) * f64::from(SAMPLE_RATE)).round() as usize;
            tape.playing = true;
        }
        #[cfg(target_os = "macos")]
        self.start();
    }
    #[cfg(target_os = "macos")]
    fn start(&mut self) {
        if self.failed {
            return;
        }
        if self.queue.is_none() {
            match queue::Queue::new(self.tape.clone()) {
                Ok(queue) => self.queue = Some(queue),
                Err(error) => {
                    self.failed = true;
                    eprintln!("zgx-video: no sound: {error}");
                    return;
                }
            }
        }
        if let Some(queue) = &mut self.queue {
            match queue.start() {
                Ok(()) => self.running = true,
                Err(error) => {
                    self.failed = true;
                    eprintln!("zgx-video: no sound: {error}");
                }
            }
        }
    }
    /// Stop playing. The output stops at once, and the tape stays where it
    /// stopped.
    pub fn stop(&mut self) {
        // (The tape before its queue: a buffer handed back as the queue stops
        // is then not queued again.)
        lock(&self.tape).playing = false;
        #[cfg(target_os = "macos")]
        if std::mem::take(&mut self.running)
            && let Some(queue) = &mut self.queue
        {
            queue.stop();
        }
    }
}

#[cfg(target_os = "macos")]
mod queue {
    //! An AudioQueue playing a tape: three buffers of [`FRAMES`] sample
    //! frames, each filled again on the queue's own thread as it's played.
    use super::{SAMPLE_RATE, Tape, lock};
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};
    use std::sync::{Arc, Mutex};

    /// Sample frames a buffer holds: 21 ms at 48 kHz, a sound's start or
    /// stop no later than that.
    const FRAMES: u32 = 1024;
    /// A left and a right, as f32s.
    const FRAME_BYTES: u32 = 8;
    const BUFFERS: usize = 3;

    #[repr(C)]
    struct StreamDescription {
        sample_rate: f64,
        format: u32,
        flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels: u32,
        bits_per_channel: u32,
        reserved: u32,
    }
    #[repr(C)]
    struct Buffer {
        capacity: u32,
        data: *mut c_void,
        size: u32,
        user_data: *mut c_void,
        descriptions_capacity: u32,
        descriptions: *mut c_void,
        descriptions_count: u32,
    }
    // CoreAudioTypes.h's AudioStreamBasicDescription is 40 bytes and
    // AudioQueue.h's AudioQueueBuffer 56.
    const _: () = assert!(size_of::<StreamDescription>() == 40 && size_of::<Buffer>() == 56);
    /// kAudioFormatLinearPCM.
    const LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
    /// kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked: interleaved f32s.
    const FLOAT_PACKED: u32 = 1 | 8;
    type QueueRef = *mut c_void;
    type Refill = unsafe extern "C" fn(*mut c_void, QueueRef, *mut Buffer);
    // SAFETY: AudioToolbox's declarations (AudioQueue.h): OSStatus is i32,
    // Boolean u8, UInt32 u32, an AudioQueueRef and a CFRunLoopRef or
    // CFStringRef pointers, AudioQueueTimeStamp's and the packet
    // descriptions' only by pointer, and the output callback's signature is
    // `Refill`'s.
    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn AudioQueueNewOutput(
            format: *const StreamDescription,
            callback: Refill,
            user: *mut c_void,
            run_loop: *const c_void,
            run_loop_mode: *const c_void,
            flags: u32,
            queue: *mut QueueRef,
        ) -> i32;
        fn AudioQueueAllocateBuffer(queue: QueueRef, bytes: u32, buffer: *mut *mut Buffer) -> i32;
        fn AudioQueueEnqueueBuffer(
            queue: QueueRef,
            buffer: *mut Buffer,
            descriptions: u32,
            packet_descriptions: *const c_void,
        ) -> i32;
        fn AudioQueueStart(queue: QueueRef, at: *const c_void) -> i32;
        fn AudioQueueStop(queue: QueueRef, immediate: u8) -> i32;
        fn AudioQueueDispose(queue: QueueRef, immediate: u8) -> i32;
    }

    /// The queue's thread, as a buffer has played: filled again and queued,
    /// unless the tape has stopped.
    unsafe extern "C" fn refill(user: *mut c_void, queue: QueueRef, buffer: *mut Buffer) {
        // SAFETY: `user` is the tape the queue's `Queue` holds, alive until
        // the queue is disposed, which waits for this; `buffer` is one of the
        // queue's, handed back, so queued nowhere.
        let (tape, buffer) = unsafe { (&*(user as *const Mutex<Tape>), &mut *buffer) };
        let mut tape = lock(tape);
        if tape.playing {
            // SAFETY: as above.
            unsafe { fill(&mut tape, queue, buffer) };
        }
    }

    /// Fill `buffer` from `tape`, and queue it.
    ///
    /// # Safety
    ///
    /// `buffer` is one of `queue`'s, not queued.
    unsafe fn fill(tape: &mut Tape, queue: QueueRef, buffer: &mut Buffer) {
        // SAFETY: the buffer holds `capacity` bytes, allocated as a whole
        // number of sample frames, aligned for f32s.
        let out = unsafe {
            std::slice::from_raw_parts_mut(buffer.data.cast::<f32>(), buffer.capacity as usize / 4)
        };
        tape.fill(out);
        buffer.size = buffer.capacity;
        // SAFETY: a buffer of the queue's, not queued (the caller's promise).
        unsafe { AudioQueueEnqueueBuffer(queue, buffer, 0, null()) };
    }

    pub(super) struct Queue {
        queue: QueueRef,
        buffers: [*mut Buffer; BUFFERS],
        /// What its callback reads, kept for as long as the queue is.
        tape: Arc<Mutex<Tape>>,
    }
    impl Queue {
        pub fn new(tape: Arc<Mutex<Tape>>) -> Result<Self, String> {
            let format = StreamDescription {
                sample_rate: f64::from(SAMPLE_RATE),
                format: LINEAR_PCM,
                flags: FLOAT_PACKED,
                bytes_per_packet: FRAME_BYTES,
                frames_per_packet: 1,
                bytes_per_frame: FRAME_BYTES,
                channels: 2,
                bits_per_channel: 32,
                reserved: 0,
            };
            let mut queue = null_mut();
            // SAFETY: with no run loop, `refill` is called on the queue's own
            // thread, `user` being the tape this keeps until it disposes of
            // the queue.
            let status = unsafe {
                AudioQueueNewOutput(
                    &format,
                    refill,
                    Arc::as_ptr(&tape) as *mut c_void,
                    null(),
                    null(),
                    0,
                    &mut queue,
                )
            };
            if status != 0 {
                return Err(format!("no audio output (AudioQueueNewOutput: {status})"));
            }
            let mut buffers = [null_mut(); BUFFERS];
            for buffer in &mut buffers {
                // SAFETY: a queue just made; writes the new buffer's pointer.
                let status =
                    unsafe { AudioQueueAllocateBuffer(queue, FRAMES * FRAME_BYTES, buffer) };
                if status != 0 {
                    // SAFETY: the queue just made, never started.
                    unsafe { AudioQueueDispose(queue, 1) };
                    return Err(format!(
                        "no audio output (AudioQueueAllocateBuffer: {status})"
                    ));
                }
            }
            Ok(Self {
                queue,
                buffers,
                tape,
            })
        }
        /// Play from where the tape is, which plays: each buffer filled and
        /// queued, then the queue started.
        pub fn start(&mut self) -> Result<(), String> {
            {
                let mut tape = lock(&self.tape);
                for &buffer in &self.buffers {
                    // SAFETY: the queue is stopped (made, or stopped at once),
                    // so none of its buffers is queued.
                    unsafe { fill(&mut tape, self.queue, &mut *buffer) };
                }
            }
            // SAFETY: a queue of ours; no time starts it as soon as it can.
            let status = unsafe { AudioQueueStart(self.queue, null()) };
            if status != 0 {
                return Err(format!("no audio output (AudioQueueStart: {status})"));
            }
            Ok(())
        }
        /// Stop at once: what's queued is dropped, and the tape (stopped
        /// first) has none queued again.
        pub fn stop(&mut self) {
            // SAFETY: a queue of ours; stopping at once is synchronous, so it
            // returns with none of its buffers queued.
            unsafe { AudioQueueStop(self.queue, 1) };
        }
    }
    impl Drop for Queue {
        fn drop(&mut self) {
            // SAFETY: a queue of ours, disposed of at once: its callback is
            // done before the tape it reads is let go.
            unsafe { AudioQueueDispose(self.queue, 1) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(samples: &[f32], at: usize) -> Tape {
        Tape {
            samples: Some(samples.into()),
            at,
            playing: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_tape_plays_from_where_it_is_and_moves_on() {
        let mut tape = tape(&[1., 2., 3., 4., 5., 6.], 1);
        let mut out = [9.; 4];
        tape.fill(&mut out);
        assert_eq!(out, [3., 4., 5., 6.]);
        assert_eq!(tape.at, 3);
    }

    #[test]
    fn past_its_end_a_tape_is_silent_and_time_still_passes() {
        let mut tape = tape(&[1., 2., 3., 4.], 1);
        let mut out = [9.; 6];
        tape.fill(&mut out);
        assert_eq!(out, [3., 4., 0., 0., 0., 0.]);
        assert_eq!(tape.at, 4);
        tape.fill(&mut out);
        assert_eq!(out, [0.; 6]);
        assert_eq!(tape.at, 7);
    }

    #[test]
    fn a_tape_with_no_mix_yet_is_silent_and_keeps_time() {
        let mut tape = Tape {
            at: 10,
            playing: true,
            ..Default::default()
        };
        let mut out = [9.; 4];
        tape.fill(&mut out);
        assert_eq!(out, [0.; 4]);
        // Its mix, made now, comes in where the picture is.
        assert_eq!(tape.at, 12);
    }
}
