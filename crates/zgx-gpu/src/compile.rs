//! Custom shaders' pipelines, compiled off the frame: on one thread, a
//! shader at a time, as a script declares them (`warm_fill_shader`) or a
//! frame first meets them. A window's frame that meets one still compiling
//! draws without it and goes on; the thread wakes the host when it is done
//! (`zgx::image::wake_waiting`), which draws again. A compile takes 70 ms
//! with Metal's shader cache warm, 360 ms the first time after a build: a
//! frame waiting for it froze the window that long.

use std::sync::{Arc, Mutex, OnceLock, Weak, atomic::AtomicU64, atomic::Ordering};

/// What a compile makes, once it has: `get` is `None` until then.
pub(crate) type Slot<T> = Arc<OnceLock<T>>;

type Job = Box<dyn FnOnce() + Send>;

/// Run `make` on the compile thread; its result lands in the slot returned,
/// and the host is woken for any frame that went without it.
pub(crate) fn spawn<T: Send + Sync + 'static>(
    make: impl FnOnce() -> T + Send + 'static,
) -> Slot<T> {
    static WORKER: OnceLock<Mutex<std::sync::mpsc::Sender<Job>>> = OnceLock::new();
    let slot: Slot<T> = Arc::default();
    let filled = slot.clone();
    let job: Job = Box::new(move || {
        let _ = filled.set(make());
        zgx::image::wake_waiting();
    });
    let sender = WORKER.get_or_init(|| {
        let (send, receive) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("zgx shader compile".into())
            .spawn(move || {
                for job in receive {
                    job();
                }
            })
            .expect("a shader compiling thread");
        Mutex::new(send)
    });
    let _ = sender.lock().unwrap_or_else(|p| p.into_inner()).send(job);
    slot
}

/// A compile started or done, in a renderer's cache of pipelines by shader.
pub(crate) enum Compiled<T> {
    Pending(Slot<Option<T>>),
    /// `None`: it did not compile (the box fills with its colour).
    Done(Option<T>),
}

impl<T> Compiled<T> {
    /// Whether it can draw, moving a finished compile's pipeline in; `wait`
    /// blocks for one still compiling (an offscreen renderer, whose frames
    /// have no deadline and nothing to wake). `None` while it compiles.
    pub(crate) fn ready(&mut self, wait: bool) -> Option<bool> {
        if let Self::Pending(slot) = self {
            let compiled = match wait {
                true => slot.wait().is_some(),
                false => slot.get()?.is_some(),
            };
            // The compile thread lets go of the slot once it has filled it:
            // the pipeline moves in then (until then, `get` reads it there).
            match Arc::get_mut(slot).and_then(OnceLock::take) {
                Some(pipeline) => *self = Self::Done(pipeline),
                None => return Some(compiled),
            }
        }
        Some(self.get().is_some())
    }

    /// The pipeline, once compiled.
    pub(crate) fn get(&self) -> Option<&T> {
        match self {
            Self::Done(pipeline) => pipeline.as_ref(),
            Self::Pending(slot) => slot.get()?.as_ref(),
        }
    }
}

/// Fill shaders a script declared (`ui.shader`), for each renderer to start
/// compiling before anything draws them.
static DECLARED: Mutex<Vec<Weak<zgx::decoration::FillShader>>> = Mutex::new(Vec::new());
/// How many `DECLARED` has had: a renderer that has seen as many has
/// nothing new to start.
static DECLARED_COUNT: AtomicU64 = AtomicU64::new(0);

/// Start compiling `shader`'s fill in every renderer, as it is declared,
/// so the frame that first draws it finds it compiled: a status ring shown
/// the moment work starts appears with it, not a frame that froze for its
/// compile. Renderer plumbing for the script host.
pub fn warm_fill_shader(shader: &Arc<zgx::decoration::FillShader>) {
    let mut declared = DECLARED.lock().unwrap_or_else(|p| p.into_inner());
    declared.retain(|shader| shader.strong_count() > 0);
    declared.push(Arc::downgrade(shader));
    DECLARED_COUNT.fetch_add(1, Ordering::Release);
}

/// Whether `shader` is one declared (`warm_fill_shader`) and still held.
#[cfg(test)]
pub(crate) fn is_declared(shader: &Arc<zgx::decoration::FillShader>) -> bool {
    let declared = DECLARED.lock().unwrap_or_else(|p| p.into_inner());
    declared.iter().any(|held| {
        held.upgrade()
            .is_some_and(|held| Arc::ptr_eq(&held, shader))
    })
}

/// How many fill shaders scripts have declared (`declared_since`).
#[inline]
pub(crate) fn declared() -> u64 {
    DECLARED_COUNT.load(Ordering::Acquire)
}

/// The fill shaders declared, if more were since `seen` (updated).
pub(crate) fn declared_since(seen: &mut u64) -> Option<Vec<Arc<zgx::decoration::FillShader>>> {
    let count = declared();
    if count == *seen {
        return None;
    }
    *seen = count;
    let declared = DECLARED.lock().unwrap_or_else(|p| p.into_inner());
    Some(declared.iter().filter_map(Weak::upgrade).collect())
}
