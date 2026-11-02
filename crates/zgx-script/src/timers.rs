//! `ui.every(ms, fn)` and `ui.after(ms, fn)`: timers on zgx's own clock
//! that wake the app only when they fire (no frames between), owned like
//! `ui.effect`: by the component or builder that made them, else the script,
//! so a reload or an unmount stops them.
//!
//! `ui.after(0, fn)` runs `fn` on the next frame. Under [`crate::testing`],
//! timers keep the test's time: they fire as its frames advance it, never on
//! the wall clock in the middle of a step.
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    future::{Future, poll_fn},
    pin::Pin,
    rc::Rc,
    task::{Poll, Waker},
    time::Duration,
};

type Pending = Pin<Box<dyn Future<Output = ()>>>;

/// Where timers run: the host view's tasks, once it mounts (a script makes
/// its timers while it loads, before that; they wait here).
#[derive(Default)]
pub(crate) struct Timers {
    tasks: RefCell<Option<zgx::compose::Tasks>>,
    pending: RefCell<Vec<Pending>>,
    clocks: Rc<Clocks>,
}
/// What timers wait on, read as they first run (after the view mounts).
#[derive(Default)]
struct Clocks {
    /// The window's frames, for `ui.after(0)`.
    frames: RefCell<Option<zgx::frame::FrameClock>>,
    /// A test's time, in place of the wall clock.
    test: RefCell<Option<Rc<TestClock>>>,
}
impl Timers {
    pub fn set_frames(&self, frames: zgx::frame::FrameClock) {
        *self.clocks.frames.borrow_mut() = Some(frames);
    }
    /// Keep the test's time from now on: see [`TestClock`].
    pub fn use_test_clock(&self) {
        self.clocks.test.borrow_mut().get_or_insert_default();
    }
    /// Move the test's time to `to`, firing each timer due by then at its
    /// own time, in order, `settle` running what each wakes.
    pub fn run_test_timers(&self, to: Duration, settle: impl FnMut()) {
        let test = self.clocks.test.borrow();
        let Some(clock) = test.as_deref() else {
            return;
        };
        if clock.sleeping.borrow().is_empty() {
            clock.now.set(clock.now.get().max(to));
            return;
        }
        clock.run(to, settle);
    }
    pub fn set_tasks(&self, tasks: zgx::compose::Tasks) {
        for task in self.pending.take() {
            tasks.spawn(task);
        }
        *self.tasks.borrow_mut() = Some(tasks);
    }
    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + 'static) {
        match &*self.tasks.borrow() {
            Some(tasks) => tasks.spawn(task),
            None => self.pending.borrow_mut().push(Box::pin(task)),
        }
    }
    /// Call `fire` after `period`, then every `period` while `repeat`, until
    /// the returned guard drops or `fire` says stop (returns false).
    pub fn start(
        &self,
        period: Duration,
        repeat: bool,
        mut fire: impl FnMut() -> bool + 'static,
    ) -> Stop {
        let alive = Rc::new(Cell::new(true));
        let running = alive.clone();
        let clocks = self.clocks.clone();
        self.spawn(async move {
            let frames = clocks.frames.borrow().clone().filter(|_| period.is_zero());
            let test = clocks.test.borrow().clone();
            loop {
                match (&frames, &test) {
                    (Some(frames), _) => drop(frames.next().await),
                    (None, Some(test)) => test.sleep(period).await,
                    (None, None) => zgx::timer::sleep(period.max(Duration::from_millis(1))).await,
                }
                if !running.get() || !fire() || !repeat {
                    return;
                }
            }
        });
        Stop(alive)
    }
}
/// Stops its timer when dropped (or `stop`ped).
pub(crate) struct Stop(Rc<Cell<bool>>);
impl Stop {
    pub fn handle(&self) -> Rc<Cell<bool>> {
        self.0.clone()
    }
}
impl Drop for Stop {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// A test's time: [`crate::testing::TestApp`]'s frames advance it, and the
/// timers due by then fire in order, each at its own time.
#[derive(Default)]
pub(crate) struct TestClock {
    now: Cell<Duration>,
    sleeping: RefCell<BTreeMap<(Duration, u64), Waker>>,
    next: Cell<u64>,
}
impl TestClock {
    #[cold]
    #[inline(never)]
    fn run(&self, to: Duration, mut settle: impl FnMut()) {
        while let Some(due) = self.next_due(to) {
            self.advance(due);
            settle();
        }
        self.advance(to);
    }
    /// The earliest sleep due by `to`, if any.
    fn next_due(&self, to: Duration) -> Option<Duration> {
        let sleeping = self.sleeping.borrow();
        let (&(due, _), _) = sleeping.first_key_value()?;
        (due <= to).then_some(due)
    }
    /// Move to `to` (never back), waking the sleeps due by then.
    fn advance(&self, to: Duration) {
        self.now.set(self.now.get().max(to));
        let now = self.now.get();
        let mut due = Vec::new();
        {
            let mut sleeping = self.sleeping.borrow_mut();
            while sleeping
                .first_key_value()
                .is_some_and(|(&(at, _), _)| at <= now)
            {
                due.push(sleeping.pop_first().unwrap().1);
            }
        }
        due.into_iter().for_each(Waker::wake);
    }
    fn sleep(&self, period: Duration) -> impl Future<Output = ()> + '_ {
        let due = self.now.get() + period;
        let key = (due, self.next.get());
        self.next.set(key.1 + 1);
        poll_fn(move |cx| {
            if self.now.get() >= due {
                self.sleeping.borrow_mut().remove(&key);
                return Poll::Ready(());
            }
            self.sleeping.borrow_mut().insert(key, cx.waker().clone());
            Poll::Pending
        })
    }
}
