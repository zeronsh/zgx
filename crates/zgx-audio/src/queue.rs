//! A fixed-size queue from one thread to one other: pushing and popping
//! never lock, allocate or wait, so the audio callback can do either.
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Keeps the two ends' counters on cache lines of their own, so a push
/// doesn't evict the line the other thread is reading.
#[repr(align(128))]
struct Padded(AtomicUsize);

struct Ring<T> {
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    mask: usize,
    /// The next slot to pop, written by the consumer alone.
    head: Padded,
    /// The next slot to push, written by the producer alone.
    tail: Padded,
}

// SAFETY: a slot is written by the producer before `tail` is released past
// it, and read by the consumer only after acquiring that `tail`, then
// handed back by releasing `head`: no slot is touched by both at once.
unsafe impl<T: Send> Sync for Ring<T> {}
unsafe impl<T: Send> Send for Ring<T> {}

impl<T> Drop for Ring<T> {
    fn drop(&mut self) {
        let (head, tail) = (*self.head.0.get_mut(), *self.tail.0.get_mut());
        for at in head..tail {
            // SAFETY: slots in head..tail were written and not yet read.
            unsafe { self.slots[at & self.mask].get_mut().assume_init_drop() };
        }
    }
}

/// The pushing end.
pub(crate) struct Producer<T>(Arc<Ring<T>>);
/// The popping end.
pub(crate) struct Consumer<T>(Arc<Ring<T>>);

/// A queue of at least `capacity` items (rounded up to a power of two).
pub(crate) fn queue<T>(capacity: usize) -> (Producer<T>, Consumer<T>) {
    let capacity = capacity.max(2).next_power_of_two();
    let ring = Arc::new(Ring {
        slots: (0..capacity)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect(),
        mask: capacity - 1,
        head: Padded(AtomicUsize::new(0)),
        tail: Padded(AtomicUsize::new(0)),
    });
    (Producer(ring.clone()), Consumer(ring))
}

impl<T> Producer<T> {
    /// Adds `item`, or gives it back when the queue is full.
    pub fn push(&mut self, item: T) -> Result<(), T> {
        let ring = &*self.0;
        let tail = ring.tail.0.load(Ordering::Relaxed);
        if tail.wrapping_sub(ring.head.0.load(Ordering::Acquire)) > ring.mask {
            return Err(item);
        }
        // SAFETY: the slot at `tail` is outside head..tail, so the consumer
        // isn't reading it; only this producer writes.
        unsafe { (*ring.slots[tail & ring.mask].get()).write(item) };
        ring.tail.0.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }
}

impl<T> Consumer<T> {
    /// The oldest item, if any.
    pub fn pop(&mut self) -> Option<T> {
        let ring = &*self.0;
        let head = ring.head.0.load(Ordering::Relaxed);
        if head == ring.tail.0.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: `head` is inside head..tail: written and released by the
        // producer, and not yet handed back.
        let item = unsafe { (*ring.slots[head & ring.mask].get()).assume_init_read() };
        ring.head.0.store(head.wrapping_add(1), Ordering::Release);
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_come_out_in_order_and_a_full_queue_gives_them_back() {
        let (mut tx, mut rx) = queue(3);
        for i in 0..4 {
            tx.push(i).unwrap();
        }
        assert_eq!(tx.push(9), Err(9));
        assert_eq!(rx.pop(), Some(0));
        tx.push(4).unwrap();
        assert_eq!(
            (1..5).map(|_| rx.pop().unwrap()).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        assert_eq!(rx.pop(), None);
    }

    #[test]
    fn items_left_in_a_dropped_queue_are_dropped() {
        let item = Arc::new(());
        let (mut tx, rx) = queue(4);
        tx.push(item.clone()).unwrap();
        tx.push(item.clone()).unwrap();
        drop((tx, rx));
        assert_eq!(Arc::strong_count(&item), 1);
    }

    #[test]
    fn two_threads_pass_every_item_once() {
        let (mut tx, mut rx) = queue::<u32>(64);
        let sender = std::thread::spawn(move || {
            for i in 0..100_000 {
                let mut item = i;
                while let Err(back) = tx.push(item) {
                    item = back;
                    std::hint::spin_loop();
                }
            }
        });
        let mut next = 0;
        while next < 100_000 {
            if let Some(item) = rx.pop() {
                assert_eq!(item, next);
                next += 1;
            }
        }
        sender.join().unwrap();
    }
}
