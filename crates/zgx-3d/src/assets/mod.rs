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
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};
use zgx_gpu::wgpu;

#[cfg(feature = "ktx2")]
pub(crate) mod ktx2;

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
    /// Asked for and refused on this renderer's device (it couldn't hold
    /// them, or their files didn't decode): not asked for again while the
    /// device lasts. Another device asks again (a new renderer, new loads).
    refused: HashSet<(u64, bool)>,
    /// Cleared as the renderer goes: what it asked for and hasn't taken is
    /// left undone.
    wanted: Arc<AtomicBool>,
    /// Whose its jobs are in the pool's queue (`Queue`).
    owner: u64,
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
            refused: HashSet::new(),
            wanted: Arc::new(AtomicBool::new(true)),
            owner: {
                static NEXT: AtomicU64 = AtomicU64::new(1);
                NEXT.fetch_add(1, Ordering::Relaxed)
            },
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
    /// Whether texture `key.0`, read as sRGB or not (`key.1`), is on its way,
    /// or was refused on this device (`refuse`): either way, not to ask for.
    pub(crate) fn coming(&self, key: (u64, bool)) -> bool {
        (!self.pending.is_empty() && self.pending.contains(&key))
            || (!self.refused.is_empty() && self.refused.contains(&key))
    }
    /// Texture `key.0`, read as sRGB or not, wasn't put on this device: not
    /// asked for again on it.
    pub(crate) fn refuse(&mut self, key: (u64, bool)) {
        self.refused.insert(key);
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
            // Small, and not to be transcoded (a Basis Universal file's
            // never is in a frame).
            || (texture.width() * texture.height() <= crate::texture::SMALL
                && !texture.transcodes())
            || zgx::image::whole_frames_on_this_thread()
        {
            return Wanted::Now;
        }
        self.pending.insert(key);
        let (texture, device, queue) = (texture.clone(), device.clone(), queue.clone());
        let (sent, wanted) = (self.sent.clone(), self.wanted.clone());
        let queued = pool().run(
            self.owner,
            Box::new(move |scratch: &mut Vec<u8>| {
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
            }),
        );
        // No thread to load it (the system made none): this frame does.
        if !queued {
            self.pending.remove(&key);
            return Wanted::Now;
        }
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
            match made {
                Some(texture) => {
                    views.insert(key, (texture.create_view(&Default::default()), 0));
                }
                None => {
                    self.refused.insert(key);
                }
            }
        }
        any
    }
}

/// `levels` of `texture` on the GPU, read as sRGB colours or as data: RGBA8
/// texels written in place (`texture_with_levels`), or a file's blocks as
/// they are (`texture_with_block_levels`) where `device` has their format.
/// Where the device hasn't the blocks' format (a world made after its
/// device asks for none: `zgx_gpu::request_view_features`), they're decoded
/// to RGBA8 first, so the picture is the same everywhere. `None` if they
/// don't decode.
pub(crate) fn upload_levels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &crate::Texture,
    srgb: bool,
    levels: &crate::texture::Levels,
) -> Option<wgpu::Texture> {
    let size = (texture.width(), texture.height());
    let bytes = levels.bytes();
    if let crate::texture::Levels::Blocks(format, _) = levels {
        let format = if srgb {
            format.add_srgb_suffix()
        } else {
            format.remove_srgb_suffix()
        };
        if device.features().contains(format.required_features()) {
            return Some(zgx_gpu::viewport::texture_with_block_levels(
                device, queue, size, format, &bytes, "3D map",
            ));
        }
        let Some(decoded) = decoded_blocks(format, size, &bytes) else {
            #[cfg(debug_assertions)]
            undecoded(format);
            return None;
        };
        let bytes: Vec<&[u8]> = decoded.iter().map(Vec::as_slice).collect();
        let format = if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        return Some(zgx_gpu::viewport::texture_with_levels(
            device, queue, size, format, &bytes, "3D map",
        ));
    }
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    Some(zgx_gpu::viewport::texture_with_levels(
        device, queue, size, format, &bytes, "3D map",
    ))
}

/// `levels` of `format`'s blocks (`size` its first's), each decoded to RGBA8
/// texels, as stored (an sRGB file's still sRGB): for a device without the
/// format. `None` where a level doesn't decode.
#[cfg(feature = "ktx2")]
fn decoded_blocks(
    format: wgpu::TextureFormat,
    size: (u32, u32),
    levels: &[&[u8]],
) -> Option<Vec<Vec<u8>>> {
    use texture2ddecoder as d;
    use wgpu::{AstcChannel, TextureFormat as F};
    type Decode = fn(&[u8], usize, usize, &mut [u32]) -> Result<(), &'static str>;
    let (bw, bh) = format.block_dimensions();
    let decode: Option<Decode> = match format.remove_srgb_suffix() {
        F::Bc1RgbaUnorm => Some(d::decode_bc1a),
        F::Bc2RgbaUnorm => Some(d::decode_bc2),
        F::Bc3RgbaUnorm => Some(d::decode_bc3),
        F::Bc4RUnorm | F::Bc4RSnorm => Some(d::decode_bc4),
        F::Bc5RgUnorm | F::Bc5RgSnorm => Some(d::decode_bc5),
        F::Bc6hRgbUfloat => Some(d::decode_bc6_unsigned),
        F::Bc6hRgbFloat => Some(d::decode_bc6_signed),
        F::Bc7RgbaUnorm => Some(d::decode_bc7),
        F::Etc2Rgb8Unorm => Some(d::decode_etc2_rgb),
        F::Etc2Rgb8A1Unorm => Some(d::decode_etc2_rgba1),
        F::Etc2Rgba8Unorm => Some(d::decode_etc2_rgba8),
        F::EacR11Unorm => Some(d::decode_eacr),
        F::EacR11Snorm => Some(d::decode_eacr_signed),
        F::EacRg11Unorm => Some(d::decode_eacrg),
        F::EacRg11Snorm => Some(d::decode_eacrg_signed),
        F::Astc { channel, .. } if channel != AstcChannel::Hdr => None,
        _ => return None,
    };
    let mut out = Vec::with_capacity(levels.len());
    for (level, blocks) in levels.iter().enumerate() {
        let (w, h) = ((size.0 >> level).max(1), (size.1 >> level).max(1));
        // Whole blocks decoded, then the level's own texels kept.
        let (pw, ph) = (w.div_ceil(bw) * bw, h.div_ceil(bh) * bh);
        let mut texels = vec![0u32; (pw * ph) as usize];
        let decoded = match decode {
            Some(decode) => decode(blocks, pw as usize, ph as usize, &mut texels),
            None => d::decode_astc(
                blocks,
                pw as usize,
                ph as usize,
                bw as usize,
                bh as usize,
                &mut texels,
            ),
        };
        decoded.ok()?;
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                // BGRA in a word, as the decoder packs it.
                let [b, g, r, a] = texels[(y * pw + x) as usize].to_le_bytes();
                rgba.extend([r, g, b, a]);
            }
        }
        out.push(rgba);
    }
    Some(out)
}
#[cfg(not(feature = "ktx2"))]
fn decoded_blocks(_: wgpu::TextureFormat, _: (u32, u32), _: &[&[u8]]) -> Option<Vec<Vec<u8>>> {
    None
}
/// Said once a format, in debug builds: a map of `format`'s blocks didn't
/// decode for a device without the format, and draws as if it had none.
#[cfg(debug_assertions)]
#[cold]
fn undecoded(format: wgpu::TextureFormat) {
    static SAID: Mutex<Vec<wgpu::TextureFormat>> = Mutex::new(Vec::new());
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if !said.contains(&format) {
        said.push(format);
        eprintln!(
            "zgx-3d: a map of {format:?} blocks draws as if it had none: its device can't \
             hold them and they didn't decode. Call zgx_3d::request_view_features() before \
             Application::run (Luau: \"gpu\": {{ \"view_features\": true }} in zgx.json)."
        );
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
    let (width, height) = (texture.width(), texture.height());
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    // A file's own levels (a KTX2's): put on the GPU as they are.
    if let Some(levels) = texture.kept_levels(device.features()) {
        return upload_levels(device, queue, texture, srgb, &levels);
    }
    let image = texture.pixels()?;
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
/// memory (`put_on_gpu`'s scratch). Renderers take turns (`Queue`).
struct Pool {
    jobs: Mutex<Jobs>,
    ready: Condvar,
    /// Most threads it makes.
    limit: usize,
}
/// Most threads the pool makes.
const MAX_THREADS: usize = 8;
#[derive(Default)]
struct Jobs {
    queue: Queue,
    threads: usize,
    idle: usize,
}
type Job = Box<dyn FnOnce(&mut Vec<u8>) + Send>;
/// Jobs by whose they are (a renderer's `Loads`), taken in turn: one of
/// each in line before a second of any, so a world's one map doesn't wait
/// behind another's hundreds.
#[derive(Default)]
struct Queue(VecDeque<(u64, VecDeque<Job>)>);
impl Queue {
    fn push(&mut self, owner: u64, job: Job) {
        match self.0.iter_mut().find(|(o, _)| *o == owner) {
            Some((_, jobs)) => jobs.push_back(job),
            None => self.0.push_back((owner, VecDeque::from([job]))),
        }
    }
    fn pop(&mut self) -> Option<Job> {
        let (owner, mut jobs) = self.0.pop_front()?;
        let job = jobs.pop_front();
        if !jobs.is_empty() {
            self.0.push_back((owner, jobs));
        }
        job
    }
}
fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool::new(most_threads()))
}
fn most_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get().saturating_sub(2))
        .clamp(1, MAX_THREADS)
}
impl Pool {
    fn new(limit: usize) -> Self {
        Self {
            jobs: Mutex::new(Jobs::default()),
            ready: Condvar::new(),
            limit,
        }
    }
    /// `job`, of `owner`'s, queued for a thread: whether one will run it.
    /// None will if the pool has no thread and can't make one (the system
    /// refusing it): the job is dropped, for its owner to do itself.
    fn run(&'static self, owner: u64, job: Job) -> bool {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.idle > 0 {
            jobs.queue.push(owner, job);
            self.ready.notify_one();
            return true;
        }
        if jobs.threads < self.limit {
            let spawned = std::thread::Builder::new()
                .name("zgx-3d textures".into())
                .spawn(move || self.work());
            if spawned.is_ok() {
                jobs.threads += 1;
            }
        }
        if jobs.threads == 0 {
            return false;
        }
        jobs.queue.push(owner, job);
        true
    }
    fn work(&self) {
        let mut scratch = Vec::new();
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(job) = jobs.queue.pop() {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A job noting `tag` in `log` as it runs.
    fn noting(log: &Arc<Mutex<Vec<u32>>>, tag: u32) -> Job {
        let log = log.clone();
        Box::new(move |_| log.lock().unwrap().push(tag))
    }

    #[cfg(feature = "ktx2")]
    #[test]
    fn a_map_of_blocks_the_device_cant_hold_is_decoded_for_it() {
        // wgpu's no-op device asks for no compressed textures: a BC7 KTX2
        // map is decoded to RGBA8 for it (no blit, no panic), and the
        // texture isn't marked failed (another device may hold its blocks).
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::NOOP;
        descriptor.backend_options.noop.enable = true;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster_free(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) = pollster_free(adapter.request_device(&Default::default())).unwrap();
        assert!(
            !device
                .features()
                .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        );
        let file = ktx2::write(4, 4, 146, &[vec![0; 16], vec![0; 16], vec![0; 16]]);
        let texture = crate::Texture::decode(&file).unwrap();
        let levels = texture.levels(wgpu::Features::empty()).expect("its blocks");
        assert!(matches!(levels, crate::texture::Levels::Blocks(..)));
        let made = upload_levels(&device, &queue, &texture, true, &levels).expect("decoded");
        assert_eq!(made.format(), wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(made.mip_level_count(), 3);
        assert!(!texture.failed());
    }

    #[test]
    fn a_map_refused_on_one_device_is_asked_for_on_the_next() {
        // Refused on a renderer's device: not asked for again there. A new
        // device (its renderer's loads made anew) asks again.
        let mut loads = Loads::default();
        let key = (7, true);
        assert!(!loads.coming(key));
        loads.refuse(key);
        assert!(loads.coming(key), "not asked for again on this device");
        assert!(!Loads::default().coming(key), "asked for on the next");
    }

    /// `future`, of wgpu's no-op backend, which never waits.
    #[cfg(feature = "ktx2")]
    fn pollster_free<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match std::pin::pin!(future).poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("the no-op backend waited"),
        }
    }

    #[test]
    fn renderers_take_turns_in_the_queue() {
        // Five of one renderer's, then one of another's: the other's goes
        // second, not sixth.
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut queue = Queue::default();
        for i in 0..5 {
            queue.push(1, noting(&log, 10 + i));
        }
        queue.push(2, noting(&log, 20));
        queue.push(1, noting(&log, 15));
        while let Some(job) = queue.pop() {
            job(&mut Vec::new());
        }
        assert_eq!(*log.lock().unwrap(), [10, 20, 11, 12, 13, 14, 15]);
    }

    #[test]
    fn a_pool_with_no_thread_runs_nothing_and_says_so() {
        // No thread can be made (here, none allowed): the job isn't kept
        // waiting for one, and its owner is told to do it itself.
        let none: &'static Pool = Box::leak(Box::new(Pool::new(0)));
        let log = Arc::new(Mutex::new(Vec::new()));
        assert!(!none.run(1, noting(&log, 1)));
        assert_eq!(none.jobs.lock().unwrap().queue.pop().map(|_| ()), None);
        // With one, it runs.
        let one: &'static Pool = Box::leak(Box::new(Pool::new(1)));
        assert!(one.run(1, noting(&log, 2)));
        let started = std::time::Instant::now();
        while log.lock().unwrap().is_empty() {
            assert!(started.elapsed().as_secs() < 10, "it ran");
            std::thread::yield_now();
        }
        assert_eq!(*log.lock().unwrap(), [2]);
    }
}
