//! The process's compile worker for the app's pipelines and shader modules,
//! made off the frame: one thread however many passes, materials and
//! devices (compute passes', particles', custom materials'). Sandboxed
//! code has a worker of its own (`compute::compile_sandboxed`), started by
//! `on_worker` too.
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

/// Work for the compile worker: pipelines and shader modules made off the
/// frame.
pub(crate) type Job = Box<dyn FnOnce() + Send>;

/// Compile workers started in this process: one, ever.
static WORKERS: AtomicU64 = AtomicU64::new(0);

/// `job` run on the process's one compile worker, started with the first
/// job and parked on its queue while there's none: however many passes and
/// devices, one thread. Its error if the worker can't be started.
pub(crate) fn compile_off_the_frame(job: impl FnOnce() + Send + 'static) -> Result<(), String> {
    static QUEUE: Mutex<Option<std::sync::mpsc::Sender<Job>>> = Mutex::new(None);
    on_worker(&QUEUE, Box::new(job))
}

pub(crate) fn on_worker(
    queue: &Mutex<Option<std::sync::mpsc::Sender<Job>>>,
    job: Job,
) -> Result<(), String> {
    let mut queue = queue.lock().unwrap_or_else(|p| p.into_inner());
    if queue.is_none() {
        let (send, receive) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("zgx-3d compile".into())
            .spawn(move || {
                for job in receive {
                    job();
                }
            })
            .map_err(|error| error.to_string())?;
        WORKERS.fetch_add(1, Ordering::Relaxed);
        *queue = Some(send);
    }
    queue
        .as_ref()
        .unwrap()
        .send(job)
        .map_err(|_| "the compile worker stopped".to_owned())
}

/// Compile workers this process has started: one for the app's, one for
/// sandboxed code, at most. For tests.
#[doc(hidden)]
pub fn compile_threads() -> u64 {
    WORKERS.load(Ordering::Relaxed)
}
