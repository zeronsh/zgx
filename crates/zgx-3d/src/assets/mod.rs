//! Textures loaded off the frame: decoded, their smaller levels made and
//! put on the GPU by threads of their own, while frames draw what has
//! arrived (`Loads`).
//!
//! A mesh whose map hasn't arrived draws as if it had none: its material's
//! own colours (a base colour of white times its colour, a flat normal…).
//! Its world keeps drawing frames while any is on its way, and settles once
//! the last has arrived. A frame drawn whole (a video's,
//! `zgx::image::whole_frames_on_this_thread`) waits for its maps instead,
//! as do maps whose pixels a texture holds and maps of
//! `texture::SMALL` pixels or fewer: those cost less than a frame's wait for them.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use zgx_gpu::wgpu;

/// Where a map is (`Loads::request`'s answer).
pub(crate) enum Wanted {
    /// It must be put on the GPU before this frame draws.
    Now,
    /// It's on its way.
    Coming,
}

/// A renderer's maps on their way: asked for of the pool's threads, each
/// arriving on the GPU as a texture of its device.
pub(crate) struct Loads {
    sent: mpsc::Sender<Arrived>,
    arrived: mpsc::Receiver<Arrived>,
    /// Asked for, not yet arrived, by texture and whether it's read as sRGB.
    pending: HashSet<(u64, bool)>,
    /// Cleared as the renderer goes: what it asked for and hasn't taken is
    /// left undone.
    wanted: Arc<AtomicBool>,
}
/// A map put on the GPU (`None`: its file didn't decode).
type Arrived = ((u64, bool), Option<wgpu::Texture>);
impl Default for Loads {
    fn default() -> Self {
        let (sent, arrived) = mpsc::channel();
        Self {
            sent,
            arrived,
            pending: HashSet::new(),
            wanted: Arc::new(AtomicBool::new(true)),
        }
    }
}
impl Drop for Loads {
    fn drop(&mut self) {
        self.wanted.store(false, Ordering::Relaxed);
    }
}
impl Loads {
    /// Whether any map is on its way.
    pub(crate) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    /// Whether texture `key.0`, read as sRGB or not (`key.1`), is on its way.
    pub(crate) fn coming(&self, key: (u64, bool)) -> bool {
        !self.pending.is_empty() && self.pending.contains(&key)
    }
    /// `texture` read as sRGB or not, wanted on `device`: asked for of the
    /// pool if it's worth waiting for (`Coming`), else for the frame to put
    /// on the GPU itself (`Now`).
    pub(crate) fn request(
        &mut self,
        (device, queue): (&wgpu::Device, &wgpu::Queue),
        texture: &crate::Texture,
        srgb: bool,
    ) -> Wanted {
        let key = (texture.id(), srgb);
        if self.pending.contains(&key) {
            return Wanted::Coming;
        }
        if texture.holds_pixels()
            || texture.width() * texture.height() <= crate::texture::SMALL
            || zgx::image::whole_frames_on_this_thread()
        {
            return Wanted::Now;
        }
        self.pending.insert(key);
        let (texture, device, queue) = (texture.clone(), device.clone(), queue.clone());
        let (sent, wanted) = (self.sent.clone(), self.wanted.clone());
        pool().run(Box::new(move |scratch: &mut Vec<u8>| {
            // A renderer gone, or remade: no one to take it.
            if !wanted.load(Ordering::Relaxed) {
                return;
            }
            // One whose decoder panics is lost alone, as one that fails;
            // either way it's answered, so nothing waits on it.
            let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                put_on_gpu(&device, &queue, &texture, srgb, scratch)
            }));
            let made = made.unwrap_or_else(|_| {
                texture.failed_to_decode();
                None
            });
            let _ = sent.send((key, made));
        }));
        Wanted::Coming
    }
    /// Maps that have arrived, as views into `views` (by texture and sRGB;
    /// the frame they were last drawn, 0). Whether any did: bindings made
    /// with stand-ins for them must be made again.
    pub(crate) fn receive(
        &mut self,
        views: &mut HashMap<(u64, bool), (wgpu::TextureView, u64)>,
    ) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        let mut any = false;
        while let Ok((key, made)) = self.arrived.try_recv() {
            self.pending.remove(&key);
            any = true;
            if let Some(texture) = made {
                views.insert(key, (texture.create_view(&Default::default()), 0));
            }
        }
        any
    }
}

/// `texture` and its smaller levels on the GPU, made in `scratch` (kept by
/// the thread from texture to texture: one block reused, not one a texture
/// left in the footprint). `None` if its file doesn't decode.
fn put_on_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &crate::Texture,
    srgb: bool,
    scratch: &mut Vec<u8>,
) -> Option<wgpu::Texture> {
    let image = texture.pixels()?;
    let (width, height) = (texture.width(), texture.height());
    let mut sizes = vec![(width, height)];
    while let Some(&(w, h)) = sizes.last().filter(|&&(w, h)| w > 1 || h > 1) {
        sizes.push(((w / 2).max(1), (h / 2).max(1)));
    }
    let bytes = |(w, h): (u32, u32)| w as usize * h as usize * 4;
    scratch.clear();
    scratch.resize(sizes[1..].iter().copied().map(bytes).sum(), 0);
    // Each level from the one above: the file's pixels, then the last made.
    let mut rest = &mut scratch[..];
    let mut above: &[u8] = image.pixels();
    let mut levels: Vec<&[u8]> = vec![image.pixels()];
    for pair in sizes.windows(2) {
        let (level, after) = std::mem::take(&mut rest).split_at_mut(bytes(pair[1]));
        crate::texture::halve(pair[0], above, level);
        above = level;
        levels.push(above);
        rest = after;
    }
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let made = zgx_gpu::viewport::texture_with_levels(
        device,
        queue,
        (width, height),
        format,
        &levels,
        "3D map",
    );
    texture.uploaded();
    Some(made)
}

/// Threads loading maps for every renderer: as many as the machine's cores
/// less two (the frame's thread and the system's), at least 1 and at most
/// `MAX_THREADS`, made as work comes and parked on a condition while there
/// is none (no wakeups at rest). Each keeps one map's levels' worth of
/// memory (`put_on_gpu`'s scratch).
struct Pool {
    jobs: Mutex<Jobs>,
    ready: Condvar,
}
/// Most threads the pool makes.
const MAX_THREADS: usize = 8;
#[derive(Default)]
struct Jobs {
    queue: VecDeque<Job>,
    threads: usize,
    idle: usize,
}
type Job = Box<dyn FnOnce(&mut Vec<u8>) + Send>;
fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        jobs: Mutex::new(Jobs::default()),
        ready: Condvar::new(),
    })
}
fn most_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get().saturating_sub(2))
        .clamp(1, MAX_THREADS)
}
impl Pool {
    fn run(&'static self, job: Job) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.queue.push_back(job);
        if jobs.idle > 0 {
            self.ready.notify_one();
        } else if jobs.threads < most_threads() {
            let spawned = std::thread::Builder::new()
                .name("zgx-3d textures".into())
                .spawn(move || self.work());
            if spawned.is_ok() {
                jobs.threads += 1;
            }
        }
    }
    fn work(&self) {
        let mut scratch = Vec::new();
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(job) = jobs.queue.pop_front() {
                drop(jobs);
                job(&mut scratch);
                jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
                continue;
            }
            jobs.idle += 1;
            jobs = self.ready.wait(jobs).unwrap_or_else(|e| e.into_inner());
            jobs.idle -= 1;
        }
    }
}
