//! The decode thread: one for every stream, filling each stream's ring
//! about half a second ahead. With no stream open it sleeps until one is.
//!
//! Where there are no threads (a browser), or for an engine whose output
//! the app pulls ([`Audio::pulled`](crate::Audio::pulled)), the same
//! filling runs inline instead, as the mixer is asked for blocks.
use crate::decode::Decode;
use crate::ring::Writer;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// How often an open stream's ring is topped up: a fifth of what it holds.
const FILL: Duration = Duration::from_millis(100);

/// Packets decoded for a stream in one round, at most: no source holds
/// the thread, whatever it gives, and the others' turns and a quit come
/// round.
const MOST_PACKETS: usize = 256;

/// Packets in a row that give no samples, past which a source has ended.
const MOST_EMPTY: usize = 4_096;

/// How long dropping the engine waits for the decode thread to quit: past
/// it, a source stuck in its decoder is left to finish on its own.
const QUIT: Duration = Duration::from_millis(500);

/// A stream the decode thread fills.
pub(crate) struct Feeding {
    pub decode: Box<dyn Decode>,
    pub writer: Writer,
    pub looping: bool,
    /// Frames decoded and dropped before any is written: where a play
    /// starts partway. Through rewinds, for a loop.
    pub skip: u64,
}

struct Open {
    feeding: Feeding,
    /// Decoded and not yet written.
    pending: Vec<f32>,
    at: usize,
    /// Whether any samples came since it was last opened again: a loop of
    /// a source that gives none ends rather than opening it for ever.
    since_rewind: bool,
    /// Packets in a row that gave nothing.
    empty: usize,
}

impl Open {
    fn new(feeding: Feeding) -> Open {
        Open {
            feeding,
            pending: Vec::new(),
            at: 0,
            since_rewind: true,
            empty: 0,
        }
    }
}

enum Order {
    Open(Feeding),
    /// Fill every stream now, then say so: an offline render waits for it.
    Fill(Sender<()>),
    Quit,
}

/// The decode thread, as the engine holds it, or its streams filled
/// inline.
pub(crate) struct Decoder {
    kind: Kind,
    /// Streams the thread is filling.
    pub open: Arc<AtomicUsize>,
}

enum Kind {
    Thread {
        orders: Sender<Order>,
        thread: Option<JoinHandle<()>>,
        /// Closed when the thread returns.
        quit: Receiver<()>,
    },
    /// No thread: each `fill` tops the streams up on the caller's.
    Inline(Mutex<Vec<Open>>),
}

impl Decoder {
    /// A decode thread, or where there are none (wasm32), streams filled
    /// inline.
    pub fn new() -> Decoder {
        if cfg!(target_arch = "wasm32") {
            return Decoder::inline();
        }
        let (orders, inbox) = channel();
        let open = Arc::new(AtomicUsize::new(0));
        let counted = open.clone();
        let (quitting, quit) = channel::<()>();
        let thread = std::thread::Builder::new()
            .name("zgx-audio-decode".into())
            .spawn(move || {
                run(inbox, &counted);
                drop(quitting);
            })
            .expect("a thread to decode audio");
        Decoder {
            kind: Kind::Thread {
                orders,
                thread: Some(thread),
                quit,
            },
            open,
        }
    }

    /// Streams filled on the caller's thread, by [`fill`](Self::fill).
    pub fn inline() -> Decoder {
        Decoder {
            kind: Kind::Inline(Mutex::new(Vec::new())),
            open: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn open(&self, feeding: Feeding) {
        match &self.kind {
            Kind::Thread { orders, .. } => {
                let _ = orders.send(Order::Open(feeding));
            }
            Kind::Inline(open) => {
                let mut open = open.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                open.push(Open::new(feeding));
                self.open.store(open.len(), Ordering::Release);
            }
        }
    }

    /// Fills every stream as far as it can be: waits for the thread, or
    /// does it here.
    pub fn fill(&self) {
        match &self.kind {
            Kind::Thread { orders, .. } => {
                let (done, wait) = channel();
                if orders.send(Order::Fill(done)).is_ok() {
                    let _ = wait.recv();
                }
            }
            Kind::Inline(open) => {
                let mut open = open.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                open.retain_mut(fill);
                self.open.store(open.len(), Ordering::Release);
            }
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let Kind::Thread {
            orders,
            thread,
            quit,
        } = &mut self.kind
        else {
            return;
        };
        let _ = orders.send(Order::Quit);
        // Joined once it has quit; a source stuck in its decoder isn't
        // waited on past `QUIT`: the thread is left to end by itself.
        if let Err(RecvTimeoutError::Disconnected) = quit.recv_timeout(QUIT)
            && let Some(thread) = thread.take()
        {
            let _ = thread.join();
        }
    }
}

fn run(orders: Receiver<Order>, count: &AtomicUsize) {
    let mut open: Vec<Open> = Vec::new();
    loop {
        let order = if open.is_empty() {
            // Nothing to fill: asleep until a stream opens.
            match orders.recv() {
                Ok(order) => Some(order),
                Err(_) => return,
            }
        } else {
            match orders.recv_timeout(FILL) {
                Ok(order) => Some(order),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        let mut asked = None;
        match order {
            Some(Order::Open(feeding)) => open.push(Open::new(feeding)),
            Some(Order::Fill(done)) => asked = Some(done),
            Some(Order::Quit) => return,
            None => {}
        }
        open.retain_mut(fill);
        count.store(open.len(), Ordering::Release);
        if let Some(done) = asked {
            let _ = done.send(());
        }
    }
}

/// Fills `stream`'s ring. Returns whether to keep it: not once it has
/// ended, failed, or its voice is gone.
fn fill(stream: &mut Open) -> bool {
    if stream.feeding.writer.abandoned() {
        return false;
    }
    for _ in 0..MOST_PACKETS {
        if stream.at < stream.pending.len() {
            stream.at += stream.feeding.writer.write(&stream.pending[stream.at..]);
            if stream.at < stream.pending.len() {
                // The ring is full: more next time.
                return true;
            }
        }
        stream.pending.clear();
        stream.at = 0;
        match stream.feeding.decode.next(&mut stream.pending) {
            Ok(true) if !stream.pending.is_empty() => {
                stream.since_rewind = true;
                stream.empty = 0;
                if stream.feeding.skip > 0 {
                    let channels = stream.feeding.decode.channels().max(1);
                    let frames = (stream.pending.len() / channels) as u64;
                    let dropped = frames.min(stream.feeding.skip);
                    stream.feeding.skip -= dropped;
                    stream.at = dropped as usize * channels;
                }
            }
            // A packet of nothing (Vorbis's first is): the next, unless
            // there have been too many.
            Ok(true) => {
                stream.empty += 1;
                if stream.empty >= MOST_EMPTY {
                    stream.feeding.writer.finish();
                    return false;
                }
            }
            // A loop opens its source again, unless that gave nothing.
            Ok(false) if stream.feeding.looping && stream.since_rewind => {
                stream.since_rewind = false;
                if stream.feeding.decode.rewind().is_err() {
                    stream.feeding.writer.finish();
                    return false;
                }
            }
            Ok(false) | Err(_) => {
                stream.feeding.writer.finish();
                return false;
            }
        }
    }
    // More next round.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::ring::ring;
    use std::time::Instant;

    /// A source whose packets give nothing: `then_end` of them, and then its
    /// end (or never, for `usize::MAX`). Each packet takes `slow`.
    struct Empty {
        packets: usize,
        then_end: usize,
        slow: Duration,
    }
    impl Decode for Empty {
        fn channels(&self) -> usize {
            2
        }
        fn rate(&self) -> u32 {
            48_000
        }
        fn next(&mut self, _: &mut Vec<f32>) -> Result<bool, Error> {
            std::thread::sleep(self.slow);
            self.packets += 1;
            Ok(self.packets <= self.then_end)
        }
        fn rewind(&mut self) -> Result<(), Error> {
            self.packets = 0;
            Ok(())
        }
    }

    fn open(then_end: usize, looping: bool) -> (Open, crate::ring::Reader) {
        let (writer, reader) = ring(4096);
        let decode = Box::new(Empty {
            packets: 0,
            then_end,
            slow: Duration::ZERO,
        });
        (
            Open::new(Feeding {
                decode,
                writer,
                looping,
                skip: 0,
            }),
            reader,
        )
    }

    /// `fill` on another thread, given `within` to come back.
    fn filled(mut stream: Open, within: Duration) -> Option<(bool, Open)> {
        let (done, wait) = channel();
        std::thread::spawn(move || {
            let kept = fill(&mut stream);
            let _ = done.send((kept, stream));
        });
        wait.recv_timeout(within).ok()
    }

    #[test]
    fn a_loop_of_a_source_that_gives_nothing_ends_rather_than_spinning() {
        // A file whose one audio packet decodes to no samples (lewton's
        // first): the loop opens it again, finds nothing again, and ends.
        let (stream, reader) = open(1, true);
        let (kept, _) = filled(stream, Duration::from_secs(2)).expect("fill came back");
        assert!(!kept && reader.finished());
    }

    #[test]
    fn a_source_of_empty_packets_holds_the_thread_a_bounded_time_and_then_ends() {
        let (mut stream, reader) = open(usize::MAX, false);
        let mut rounds = 0;
        loop {
            let (kept, back) = filled(stream, Duration::from_secs(2)).expect("fill came back");
            stream = back;
            rounds += 1;
            if !kept {
                break;
            }
            assert!(rounds < 1_000, "never ended");
        }
        assert!(rounds > 1, "ended in its first round");
        assert!(reader.finished());
    }

    #[test]
    fn dropping_the_decode_thread_doesn_t_wait_on_a_stuck_source() {
        let decoder = Decoder::new();
        let (writer, _reader) = ring(4096);
        decoder.open(Feeding {
            decode: Box::new(Empty {
                packets: 0,
                then_end: usize::MAX,
                slow: Duration::from_secs(30),
            }),
            writer,
            looping: false,
            skip: 0,
        });
        std::thread::sleep(Duration::from_millis(50));
        let began = Instant::now();
        drop(decoder);
        assert!(
            began.elapsed() < Duration::from_secs(3),
            "waited {:?}",
            began.elapsed()
        );
    }
}
