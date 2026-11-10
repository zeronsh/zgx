//! The decode thread: one for every stream, filling each stream's ring
//! about half a second ahead. With no stream open it sleeps until one is.
use crate::decode::Decode;
use crate::ring::Writer;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

/// How often an open stream's ring is topped up: a fifth of what it holds.
const FILL: Duration = Duration::from_millis(100);

/// A stream the decode thread fills.
pub(crate) struct Feeding {
    pub decode: Box<dyn Decode>,
    pub writer: Writer,
    pub looping: bool,
}

struct Open {
    feeding: Feeding,
    /// Decoded and not yet written.
    pending: Vec<f32>,
    at: usize,
}

enum Order {
    Open(Feeding),
    /// Fill every stream now, then say so: an offline render waits for it.
    Fill(Sender<()>),
    Quit,
}

/// The decode thread, as the engine holds it.
pub(crate) struct Decoder {
    orders: Sender<Order>,
    thread: Option<JoinHandle<()>>,
    /// Streams the thread is filling.
    pub open: Arc<AtomicUsize>,
}

impl Decoder {
    pub fn new() -> Decoder {
        let (orders, inbox) = channel();
        let open = Arc::new(AtomicUsize::new(0));
        let counted = open.clone();
        let thread = std::thread::Builder::new()
            .name("zgx-audio-decode".into())
            .spawn(move || run(inbox, &counted))
            .expect("a thread to decode audio");
        Decoder {
            orders,
            thread: Some(thread),
            open,
        }
    }

    pub fn open(&self, feeding: Feeding) {
        let _ = self.orders.send(Order::Open(feeding));
    }

    /// Waits until every stream has been filled as far as it can be.
    pub fn fill(&self) {
        let (done, wait) = channel();
        if self.orders.send(Order::Fill(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.orders.send(Order::Quit);
        if let Some(thread) = self.thread.take() {
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
            Some(Order::Open(feeding)) => open.push(Open {
                feeding,
                pending: Vec::new(),
                at: 0,
            }),
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
    let mut decoded_since_rewind = true;
    loop {
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
            Ok(true) => decoded_since_rewind = true,
            // A loop opens its source again, unless that gave nothing.
            Ok(false) if stream.feeding.looping && decoded_since_rewind => {
                decoded_since_rewind = false;
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
}
