//! What time it is for what moves: the wall clock, unless the thread has a
//! [`VirtualClock`], whose host says what time it is.
//!
//! An interface whose time is a virtual clock's plays the same way however
//! fast its frames are made: a video made of it, one frame after another
//! offline, shows each animation, timer and shader at that frame's time. The
//! clock belongs to a thread, so a window can show the frames of a virtual
//! clock another thread runs.
//!
//! Animations read [`now`] where they'd read `Instant::now()`. Code measuring
//! itself (a slice of work, a profile, a script's time budget) goes on reading
//! the wall clock: its time isn't the interface's.
use crate::timer::Driver;
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

/// How many virtual clocks are installed, on any thread: none (every app that
/// makes no video), and [`now`] is this one load before the wall clock.
static INSTALLED: AtomicUsize = AtomicUsize::new(0);

struct Current {
    now: Instant,
    start: Instant,
    epoch: SystemTime,
    timers: Arc<Driver>,
}
thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
}

/// The time on this thread: its virtual clock's, else the wall clock's.
#[inline]
pub fn now() -> Instant {
    if INSTALLED.load(Ordering::Relaxed) != 0
        && let Some(now) = virtual_now()
    {
        return now;
    }
    Instant::now()
}
#[cold]
#[inline(never)]
fn virtual_now() -> Option<Instant> {
    CURRENT.with_borrow(|current| current.as_ref().map(|current| current.now))
}

/// How long ago `start` was, on this thread's time.
#[inline]
pub fn since(start: Instant) -> Duration {
    now().saturating_duration_since(start)
}

/// The date on this thread: its virtual clock's (its epoch and the time since
/// it started), else the system's.
pub fn system_now() -> SystemTime {
    if INSTALLED.load(Ordering::Relaxed) != 0 {
        let date = CURRENT.with_borrow(|current| {
            current
                .as_ref()
                .map(|current| current.epoch + (current.now - current.start))
        });
        if let Some(date) = date {
            return date;
        }
    }
    SystemTime::now()
}

/// Whether this thread's time is a virtual clock's.
pub fn is_virtual() -> bool {
    INSTALLED.load(Ordering::Relaxed) != 0 && virtual_now().is_some()
}

/// The timers of this thread's virtual clock, if it has one.
pub(crate) fn virtual_timers() -> Option<Arc<Driver>> {
    if INSTALLED.load(Ordering::Relaxed) == 0 {
        return None;
    }
    CURRENT.with_borrow(|current| current.as_ref().map(|current| current.timers.clone()))
}

/// This thread's time, as long as it lives, moved only by [`VirtualClock::advance`].
/// Sleeps on it ([`crate::timer::sleep`]) wake as it passes them, on this
/// thread. Not `Send`: it is this thread's.
pub struct VirtualClock {
    start: Instant,
    /// Tied to the thread.
    _thread: std::marker::PhantomData<*const ()>,
}

impl VirtualClock {
    /// Stands at its start, on the date `epoch`: 1 January 2000 UTC with
    /// [`VirtualClock::new`], so an interface showing the date is the same
    /// each time. Only one at a time, on a thread.
    pub fn with_epoch(epoch: SystemTime) -> Self {
        let start = Instant::now();
        let installed = CURRENT.with_borrow_mut(|current| {
            let free = current.is_none();
            if free {
                *current = Some(Current {
                    now: start,
                    start,
                    epoch,
                    timers: Driver::hosted_by_clock(),
                });
            }
            free
        });
        assert!(installed, "a thread has one virtual clock at a time");
        INSTALLED.fetch_add(1, Ordering::Relaxed);
        Self {
            start,
            _thread: std::marker::PhantomData,
        }
    }
    /// At its start, on 1 January 2000 UTC.
    pub fn new() -> Self {
        // 946684800: seconds from 1970 to 2000.
        Self::with_epoch(SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800))
    }
    /// The instant it started at.
    pub fn start(&self) -> Instant {
        self.start
    }
    /// How long it has run.
    pub fn elapsed(&self) -> Duration {
        self.now() - self.start
    }
    /// The time it stands at.
    pub fn now(&self) -> Instant {
        CURRENT.with_borrow(|current| current.as_ref().expect("installed").now)
    }
    /// The earliest sleep waiting on it, if any.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.timers().earliest()
    }
    /// Move to `elapsed` from its start (never back), waking each sleep due by
    /// then at its own time, in the order they fall due, and calling `settle`
    /// after each, for what they wake to run before later ones fall due.
    pub fn advance(&self, elapsed: Duration, mut settle: impl FnMut()) {
        let to = self.start + elapsed;
        let timers = self.timers();
        while let Some(due) = timers.earliest().filter(|due| *due <= to) {
            self.stand_at(due);
            timers.fire(self.now());
            settle();
        }
        self.stand_at(to);
    }
    fn stand_at(&self, to: Instant) {
        CURRENT.with_borrow_mut(|current| {
            let current = current.as_mut().expect("installed");
            current.now = current.now.max(to);
        });
    }
    fn timers(&self) -> Arc<Driver> {
        CURRENT.with_borrow(|current| current.as_ref().expect("installed").timers.clone())
    }
}
impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for VirtualClock {
    fn drop(&mut self) {
        // Its sleeps go with it: their tasks are dropped with what they served
        // or never wake, as a host's are when its loop ends.
        let current = CURRENT.with_borrow_mut(Option::take);
        INSTALLED.fetch_sub(1, Ordering::Relaxed);
        drop(current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn its_thread_keeps_its_time_and_others_the_wall_clock() {
        let clock = VirtualClock::new();
        let at = now();
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(now(), at, "it stands still");
        assert!(is_virtual());
        clock.advance(Duration::from_secs(90), || {});
        assert_eq!(now() - at, Duration::from_secs(90));
        assert_eq!(clock.elapsed(), Duration::from_secs(90));
        let date = system_now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(date, 946_684_800 + 90);
        std::thread::spawn(move || {
            assert!(!is_virtual(), "another thread has the wall clock");
        })
        .join()
        .unwrap();
        drop(clock);
        assert!(!is_virtual());
    }

    #[test]
    fn sleeps_wake_at_their_own_time_in_order() {
        let clock = VirtualClock::new();
        let log: Rc<RefCell<Vec<(&str, Duration)>>> = Rc::default();
        let start = clock.start();
        let executor = RefCell::new(crate::task::LocalExecutor::new());
        for (ms, name) in [(30, "late"), (10, "early"), (50, "outside")] {
            let log = log.clone();
            executor.borrow_mut().spawn(async move {
                crate::timer::sleep(Duration::from_millis(ms)).await;
                log.borrow_mut().push((name, now() - start));
            });
        }
        executor.borrow_mut().tick();
        assert_eq!(
            clock.next_deadline(),
            Some(start + Duration::from_millis(10))
        );
        std::thread::sleep(Duration::from_millis(40));
        executor.borrow_mut().tick();
        assert!(log.borrow().is_empty(), "the wall clock wakes none");
        clock.advance(Duration::from_millis(40), || {
            executor.borrow_mut().tick();
        });
        assert_eq!(
            *log.borrow(),
            [
                ("early", Duration::from_millis(10)),
                ("late", Duration::from_millis(30)),
            ]
        );
    }
}
