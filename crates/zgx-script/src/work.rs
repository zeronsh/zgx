//! Work off the script's thread (a picture rendered, decoded or a font
//! read) on a few threads shared by every script: made as work comes, up
//! to `MOST`, and gone once idle. A script that asks for thousands at once
//! waits its turn instead of starting a thread for each, which the system
//! would refuse (and a refused thread panics).
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex, OnceLock},
    time::Duration,
};

/// The most threads at once.
const MOST: usize = 4;
/// How long an idle thread waits for more before it goes: idle windows do
/// no work.
const IDLE: Duration = Duration::from_secs(2);

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    idle: usize,
    threads: usize,
}

struct Pool {
    queue: Mutex<Queue>,
    ready: Condvar,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        queue: Mutex::new(Queue::default()),
        ready: Condvar::new(),
    })
}

/// Runs `job` on one of the shared threads, after those asked before it.
/// With no thread to run it (the system refused one, and none runs), it is
/// dropped: what it would tell is told it stopped.
pub(crate) fn run(job: impl FnOnce() + Send + 'static) {
    let pool = pool();
    let mut queue = pool.queue.lock().unwrap_or_else(|p| p.into_inner());
    queue.jobs.push_back(Box::new(job));
    if queue.idle > 0 {
        pool.ready.notify_one();
        return;
    }
    if queue.threads < MOST {
        let started = std::thread::Builder::new()
            .name("zgx-script work".into())
            .spawn(work);
        match started {
            Ok(_) => queue.threads += 1,
            // None to run it: dropped.
            Err(_) if queue.threads == 0 => drop(queue.jobs.pop_back()),
            Err(_) => {}
        }
    }
}

fn work() {
    let pool = pool();
    let mut queue = pool.queue.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        let Some(job) = queue.jobs.pop_front() else {
            queue.idle += 1;
            let (back, waited) = pool
                .ready
                .wait_timeout(queue, IDLE)
                .unwrap_or_else(|p| p.into_inner());
            queue = back;
            queue.idle -= 1;
            if waited.timed_out() && queue.jobs.is_empty() {
                queue.threads -= 1;
                return;
            }
            continue;
        };
        drop(queue);
        // A job that panics is its own failure (its promise told it
        // stopped), not the next one's.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
        queue = pool.queue.lock().unwrap_or_else(|p| p.into_inner());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicUsize, atomic::Ordering, mpsc};

    #[test]
    fn many_jobs_run_on_a_few_threads_and_a_panic_stops_only_its_own() {
        let (tell, heard) = mpsc::channel();
        let threads = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let ran = Arc::new(AtomicUsize::new(0));
        run(|| panic!("this job's own failure"));
        for _ in 0..200 {
            let (tell, threads, ran) = (tell.clone(), threads.clone(), ran.clone());
            run(move || {
                threads.lock().unwrap().insert(std::thread::current().id());
                ran.fetch_add(1, Ordering::SeqCst);
                tell.send(()).unwrap();
            });
        }
        for _ in 0..200 {
            heard
                .recv_timeout(Duration::from_secs(10))
                .expect("each runs");
        }
        assert_eq!(ran.load(Ordering::SeqCst), 200);
        assert!(threads.lock().unwrap().len() <= MOST);
    }
}
