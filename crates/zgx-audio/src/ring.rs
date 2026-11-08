//! A queue from one thread to one other, of a fixed size made up front:
//! neither side allocates, locks or waits, so the audio thread can take from
//! it and give back to it.
use std::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Shared<T> {
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    /// The next slot the reader takes (counts up forever; wraps by mask).
    head: AtomicUsize,
    /// The next slot the writer fills.
    tail: AtomicUsize,
}
// One writer and one reader, each touching only slots the other has handed
// over (through `head` and `tail`, with release and acquire).
unsafe impl<T: Send> Sync for Shared<T> {}

impl<T> Drop for Shared<T> {
    fn drop(&mut self) {
        let (head, tail) = (*self.head.get_mut(), *self.tail.get_mut());
        let mask = self.slots.len() - 1;
        for at in head..tail {
            unsafe { self.slots[at & mask].get_mut().assume_init_drop() };
        }
    }
}

/// The writing end.
pub struct Writer<T>(Arc<Shared<T>>);
/// The reading end.
pub struct Reader<T>(Arc<Shared<T>>);

/// A queue of at least `capacity` items (rounded up to a power of two).
pub fn ring<T: Send>(capacity: usize) -> (Writer<T>, Reader<T>) {
    let size = capacity.max(2).next_power_of_two();
    let shared = Arc::new(Shared {
        slots: (0..size)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect(),
        head: AtomicUsize::new(0),
        tail: AtomicUsize::new(0),
    });
    (Writer(shared.clone()), Reader(shared))
}

impl<T: Send> Writer<T> {
    /// Adds `item`, or gives it back if the queue is full.
    pub fn push(&mut self, item: T) -> Result<(), T> {
        let shared = &*self.0;
        let tail = shared.tail.load(Ordering::Relaxed);
        if tail - shared.head.load(Ordering::Acquire) == shared.slots.len() {
            return Err(item);
        }
        let slot = &shared.slots[tail & (shared.slots.len() - 1)];
        unsafe { (*slot.get()).write(item) };
        shared.tail.store(tail + 1, Ordering::Release);
        Ok(())
    }
}

impl<T: Send> Reader<T> {
    /// The oldest item, if any.
    pub fn pop(&mut self) -> Option<T> {
        let shared = &*self.0;
        let head = shared.head.load(Ordering::Relaxed);
        if head == shared.tail.load(Ordering::Acquire) {
            return None;
        }
        let slot = &shared.slots[head & (shared.slots.len() - 1)];
        let item = unsafe { (*slot.get()).assume_init_read() };
        shared.head.store(head + 1, Ordering::Release);
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_come_out_in_order_and_a_full_queue_refuses() {
        let (mut writer, mut reader) = ring(3);
        for i in 0..4 {
            writer.push(i).unwrap();
        }
        assert_eq!(writer.push(9), Err(9), "four slots, all full");
        assert_eq!(reader.pop(), Some(0));
        writer.push(4).unwrap();
        let rest: Vec<_> = std::iter::from_fn(|| reader.pop()).collect();
        assert_eq!(rest, [1, 2, 3, 4]);
    }

    #[test]
    fn across_threads_nothing_is_lost_or_doubled() {
        let (mut writer, mut reader) = ring::<u64>(64);
        let sender = std::thread::spawn(move || {
            for i in 0..100_000 {
                let mut item = i;
                while let Err(back) = writer.push(item) {
                    item = back;
                    std::hint::spin_loop();
                }
            }
        });
        let mut next = 0;
        while next < 100_000 {
            if let Some(item) = reader.pop() {
                assert_eq!(item, next);
                next += 1;
            }
        }
        sender.join().unwrap();
    }

    #[test]
    fn what_is_left_is_dropped_with_it() {
        let item = Arc::new(());
        let (mut writer, reader) = ring(4);
        writer.push(item.clone()).unwrap();
        drop((writer, reader));
        assert_eq!(Arc::strong_count(&item), 1);
    }
}
