//! A ring of samples from one thread to one other, copied in and out in
//! runs: the decode thread writes a stream's frames ahead, the mixer reads
//! them as it plays. Neither end locks, allocates or waits.
use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[repr(align(128))]
struct Padded(AtomicUsize);

struct Ring {
    samples: Box<[UnsafeCell<f32>]>,
    /// Samples read and written so far, each by its own end alone.
    read: Padded,
    written: Padded,
    /// Set by the writer after its last sample: what's left is the end.
    finished: AtomicBool,
}

// SAFETY: the writer touches only samples in written..read + capacity, the
// reader only read..written; each publishes its count with Release after
// touching them and reads the other's with Acquire.
unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

pub(crate) struct Writer(Arc<Ring>);
pub(crate) struct Reader(Arc<Ring>);

/// A ring of `capacity` samples.
pub(crate) fn ring(capacity: usize) -> (Writer, Reader) {
    let ring = Arc::new(Ring {
        samples: (0..capacity.max(1)).map(|_| UnsafeCell::new(0.)).collect(),
        read: Padded(AtomicUsize::new(0)),
        written: Padded(AtomicUsize::new(0)),
        finished: AtomicBool::new(false),
    });
    (Writer(ring.clone()), Reader(ring))
}

impl Writer {
    /// Room for this many more samples.
    pub fn space(&self) -> usize {
        let ring = &*self.0;
        ring.samples.len()
            - (ring.written.0.load(Ordering::Relaxed) - ring.read.0.load(Ordering::Acquire))
    }

    /// Writes as many of `samples` as fit; returns how many.
    pub fn write(&mut self, samples: &[f32]) -> usize {
        let ring = &*self.0;
        let written = ring.written.0.load(Ordering::Relaxed);
        let count = samples.len().min(self.space());
        let capacity = ring.samples.len();
        for (i, &sample) in samples[..count].iter().enumerate() {
            // SAFETY: these slots are outside read..written: the reader
            // isn't reading them.
            unsafe { *ring.samples[(written + i) % capacity].get() = sample };
        }
        ring.written.0.store(written + count, Ordering::Release);
        count
    }

    /// Says nothing more will be written.
    pub fn finish(&self) {
        self.0.finished.store(true, Ordering::Release);
    }

    /// Whether the reader is gone (its stream ended or was stopped).
    pub fn abandoned(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }
}

impl Reader {
    /// Samples ready to read.
    pub fn ready(&self) -> usize {
        let ring = &*self.0;
        ring.written.0.load(Ordering::Acquire) - ring.read.0.load(Ordering::Relaxed)
    }

    /// Reads up to `out.len()` samples into `out`; returns how many.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        let ring = &*self.0;
        let read = ring.read.0.load(Ordering::Relaxed);
        let count = out.len().min(self.ready());
        let capacity = ring.samples.len();
        for (i, slot) in out[..count].iter_mut().enumerate() {
            // SAFETY: these slots are inside read..written: written and
            // published by the writer, not yet given back.
            *slot = unsafe { *ring.samples[(read + i) % capacity].get() };
        }
        ring.read.0.store(read + count, Ordering::Release);
        count
    }

    /// Whether the writer has finished: what's in the ring is the last.
    pub fn finished(&self) -> bool {
        self.0.finished.load(Ordering::Acquire)
    }

    /// Whether the writer has finished and everything it wrote was read.
    #[cfg(test)]
    pub fn ended(&self) -> bool {
        self.0.finished.load(Ordering::Acquire) && self.ready() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_wrap_round_and_come_out_in_order() {
        let (mut writer, mut reader) = ring(5);
        assert_eq!(writer.write(&[1., 2., 3.]), 3);
        let mut out = [0.; 2];
        assert_eq!(reader.read(&mut out), 2);
        assert_eq!(out, [1., 2.]);
        assert_eq!(writer.write(&[4., 5., 6., 7., 8.]), 4);
        let mut out = [0.; 8];
        assert_eq!(reader.read(&mut out), 5);
        assert_eq!(&out[..5], &[3., 4., 5., 6., 7.]);
        assert!(!reader.ended());
        writer.finish();
        assert!(reader.ended());
    }

    #[test]
    fn two_threads_pass_every_sample_once() {
        let (mut writer, mut reader) = ring(97);
        let sender = std::thread::spawn(move || {
            let samples: Vec<f32> = (0..50_000).map(|i| i as f32).collect();
            let mut at = 0;
            while at < samples.len() {
                at += writer.write(&samples[at..(at + 13).min(samples.len())]);
            }
            writer.finish();
        });
        let mut next = 0;
        let mut out = [0f32; 31];
        while !reader.ended() {
            let got = reader.read(&mut out);
            for &sample in &out[..got] {
                assert_eq!(sample, next as f32);
                next += 1;
            }
        }
        sender.join().unwrap();
        assert_eq!(next, 50_000);
    }
}
