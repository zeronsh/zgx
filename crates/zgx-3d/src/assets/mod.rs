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
                let made = guarded(&texture, || {
                    put_on_gpu(&device, &queue, &texture, srgb, scratch)
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

/// `make`'s answer for `texture`, a panic in it caught: `None`, and the
/// texture marked failed (drawn as if it had no map, not tried again), as
/// a file that doesn't decode is. For decoding on any thread: a texture
/// thread's, or a frame's (`render::materials`).
pub(crate) fn guarded<T>(texture: &crate::Texture, make: impl FnOnce() -> Option<T>) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(make)).unwrap_or_else(|_| {
        texture.failed_to_decode();
        None
    })
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
    let rgba = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
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
        // Decoded a level at a time and written as it is (one level's
        // texels held, never all), within `MAX_DECODED`.
        let refused = refusal(format, size).or_else(|| {
            bytes
                .is_empty()
                .then(|| "a map of no levels: nothing to decode".to_owned())
        });
        let mut made = refused.is_none().then(|| {
            zgx_gpu::viewport::TextureLevels::new(device, size, rgba, bytes.len() as u32, "3D map")
        });
        let decoded = match (&mut made, refused) {
            (Some(made), None) => decoded_blocks(format, size, &bytes, |level, texels| {
                made.write(queue, level as u32, texels)
            }),
            (_, refused) => Err(refused.unwrap_or_default()),
        };
        if let Err(why) = decoded {
            #[cfg(debug_assertions)]
            said_once(&format!("{format:?}"), || {
                format!(
                    "a map of {format:?} blocks draws as if it had none: its device can't \
                     hold them, and decoded, {why}. Call zgx_3d::request_view_features() \
                     before Application::run (Luau: \"gpu\": {{ \"view_features\": true }} \
                     in zgx.json) for a device that holds them."
                )
            });
            #[cfg(not(debug_assertions))]
            let _ = why;
            return None;
        }
        return made.map(|made| made.finish(device));
    }
    Some(zgx_gpu::viewport::texture_with_levels(
        device, queue, size, rgba, &bytes, "3D map",
    ))
}

/// The most bytes a map's first level may hold decoded to RGBA8 for a
/// device without its blocks' format, as a PNG's pixels may
/// (`texture::MAX_FILE`): blocks grow as they're decoded (BC1's 8 times,
/// BC7's 4, 12 by 12 ASTC's about 57), so an 8192² BC1 file's 32 MiB of
/// blocks would be 256 MiB of texels. A larger one draws as if it had none
/// there (its blocks are put on a device that holds them as they are).
#[cfg(feature = "ktx2")]
pub(crate) const MAX_DECODED: usize = 64 << 20;

/// A block decoder of texture2ddecoder's: blocks, width and height (whole
/// blocks), and texels to write (BGRA in a word).
#[cfg(feature = "ktx2")]
type Decode = fn(&[u8], usize, usize, &mut [u32]) -> Result<(), &'static str>;
/// `format`'s decoder (`None` for ASTC's, which takes its block size too),
/// or `Err` where there's none.
#[cfg(feature = "ktx2")]
fn decoder(format: wgpu::TextureFormat) -> Result<Option<Decode>, ()> {
    use texture2ddecoder as d;
    use wgpu::{AstcChannel, TextureFormat as F};
    Ok(Some(match format.remove_srgb_suffix() {
        F::Bc1RgbaUnorm => d::decode_bc1a,
        F::Bc2RgbaUnorm => d::decode_bc2,
        F::Bc3RgbaUnorm => d::decode_bc3,
        F::Bc4RUnorm | F::Bc4RSnorm => d::decode_bc4,
        F::Bc5RgUnorm | F::Bc5RgSnorm => d::decode_bc5,
        F::Bc6hRgbUfloat => d::decode_bc6_unsigned,
        F::Bc6hRgbFloat => d::decode_bc6_signed,
        F::Bc7RgbaUnorm => d::decode_bc7,
        F::Etc2Rgb8Unorm => d::decode_etc2_rgb,
        F::Etc2Rgb8A1Unorm => d::decode_etc2_rgba1,
        F::Etc2Rgba8Unorm => d::decode_etc2_rgba8,
        F::EacR11Unorm => d::decode_eacr,
        F::EacR11Snorm => d::decode_eacr_signed,
        F::EacRg11Unorm => d::decode_eacrg,
        F::EacRg11Snorm => d::decode_eacrg_signed,
        F::Astc { channel, .. } if channel != AstcChannel::Hdr => return Ok(None),
        _ => return Err(()),
    }))
}
/// Why a map of `format`'s blocks, its first level `size`, wouldn't be
/// decoded for a device without the format (`None`: it would be): there's
/// no decoder, or its texels would be over `MAX_DECODED`.
fn refusal(format: wgpu::TextureFormat, size: (u32, u32)) -> Option<String> {
    #[cfg(feature = "ktx2")]
    {
        if decoder(format).is_err() {
            return Some(format!("there's no decoder for {format:?}"));
        }
        let bytes = size.0 as u64 * size.1 as u64 * 4;
        (bytes > MAX_DECODED as u64).then(|| {
            format!(
                "its first level, {} by {}, would be {} MiB of texels, over the {} MiB a \
                 map may hold (as a PNG's pixels may)",
                size.0,
                size.1,
                bytes >> 20,
                MAX_DECODED >> 20
            )
        })
    }
    #[cfg(not(feature = "ktx2"))]
    {
        let _ = size;
        Some(format!(
            "there's no decoder for {format:?} (zgx-3d's `ktx2` feature)"
        ))
    }
}

/// `levels` of `format`'s blocks (`size` its first's, whole blocks), each
/// decoded to RGBA8 texels, as stored (an sRGB file's still sRGB), and
/// handed to `each` with its index before the next is decoded: for a device
/// without the format. All in one buffer of the first level's texels, so a
/// map holds no more than that decoded (`MAX_DECODED`). `Err`, saying why,
/// where it's over that (`refusal`), a level doesn't decode, or the decoder
/// panics (caught here: on the frame's thread too, a hostile file is a map
/// that draws as if it had none).
#[cfg(feature = "ktx2")]
fn decoded_blocks(
    format: wgpu::TextureFormat,
    size: (u32, u32),
    levels: &[&[u8]],
    mut each: impl FnMut(usize, &[u8]),
) -> Result<(), String> {
    if let Some(why) = refusal(format, size) {
        return Err(why);
    }
    let decode = decoder(format).map_err(|()| "no decoder")?;
    let (bw, bh) = format.block_dimensions();
    let first = size.0 as usize * size.1 as usize;
    let mut texels = vec![0u32; first];
    for (level, blocks) in levels.iter().enumerate() {
        let (w, h) = ((size.0 >> level).max(1), (size.1 >> level).max(1));
        // Whole blocks decoded: no more texels than the first level's,
        // itself whole blocks (each side rounded up to a multiple of a
        // block's is no more than the first's, a multiple).
        let (pw, ph) = (w.div_ceil(bw) * bw, h.div_ceil(bh) * bh);
        let padded = texels
            .get_mut(..(pw * ph) as usize)
            .ok_or_else(|| format!("level {level} is larger than the first"))?;
        let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match decode {
            Some(decode) => decode(blocks, pw as usize, ph as usize, padded),
            None => texture2ddecoder::decode_astc(
                blocks,
                pw as usize,
                ph as usize,
                bw as usize,
                bh as usize,
                padded,
            ),
        }));
        decoded
            .map_err(|_| format!("the decoder panicked on level {level}"))?
            .map_err(|e| format!("level {level} didn't decode ({e})"))?;
        // Its own texels: rows closed up to its width (each moves back, so
        // in order none is overwritten before it's moved), and BGRA in a
        // word turned to RGBA bytes in place.
        let (w, h, pw) = (w as usize, h as usize, pw as usize);
        if pw != w {
            for y in 1..h {
                texels.copy_within(y * pw..y * pw + w, y * w);
            }
        }
        let own = &mut texels[..w * h];
        for texel in own.iter_mut() {
            let [b, g, r, a] = texel.to_le_bytes();
            *texel = u32::from_ne_bytes([r, g, b, a]);
        }
        each(level, bytemuck::cast_slice(own));
    }
    Ok(())
}
#[cfg(not(feature = "ktx2"))]
fn decoded_blocks(
    format: wgpu::TextureFormat,
    size: (u32, u32),
    _: &[&[u8]],
    _: impl FnMut(usize, &[u8]),
) -> Result<(), String> {
    Err(refusal(format, size).unwrap_or_default())
}
/// Said once for each `kind`, in debug builds: why a map draws as if it had
/// none (`message`, made only the first time).
#[cfg(debug_assertions)]
#[cold]
pub(crate) fn said_once(kind: &str, message: impl FnOnce() -> String) {
    static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut said = SAID.lock().unwrap_or_else(|e| e.into_inner());
    if !said.iter().any(|k| k == kind) {
        said.push(kind.to_owned());
        eprintln!("zgx-3d: {}", message());
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
        let (device, queue) = noop_device();
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

    /// Bytes allocated on each thread, and their most since `peak_during`
    /// began: a test's own, whatever others run beside it.
    #[cfg(feature = "ktx2")]
    mod counting {
        use std::{
            alloc::{GlobalAlloc, Layout, System},
            cell::Cell,
        };
        thread_local! {
            static NOW: Cell<isize> = const { Cell::new(0) };
            static PEAK: Cell<isize> = const { Cell::new(0) };
        }
        fn add(bytes: isize) {
            let _ = NOW.try_with(|now| {
                now.set(now.get() + bytes);
                let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now.get())));
            });
        }
        pub(super) struct Counting;
        // SAFETY: The system's allocator, its sizes counted.
        unsafe impl GlobalAlloc for Counting {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                let at = unsafe { System.alloc(layout) };
                if !at.is_null() {
                    add(layout.size() as isize);
                }
                at
            }
            unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
                let at = unsafe { System.alloc_zeroed(layout) };
                if !at.is_null() {
                    add(layout.size() as isize);
                }
                at
            }
            unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
                unsafe { System.dealloc(at, layout) };
                add(-(layout.size() as isize));
            }
            unsafe fn realloc(&self, at: *mut u8, layout: Layout, size: usize) -> *mut u8 {
                let moved = unsafe { System.realloc(at, layout, size) };
                if !moved.is_null() {
                    add(size as isize - layout.size() as isize);
                }
                moved
            }
        }
        /// The most bytes `f` held at once on this thread, beyond what it
        /// started with.
        pub(super) fn peak_during(f: impl FnOnce()) -> usize {
            let start = NOW.with(Cell::get);
            PEAK.with(|peak| peak.set(start));
            f();
            (PEAK.with(Cell::get) - start).max(0) as usize
        }
    }
    #[cfg(feature = "ktx2")]
    #[global_allocator]
    static COUNTING: counting::Counting = counting::Counting;

    /// A map's levels' blocks, `size` its first's, each block `block`.
    #[cfg(feature = "ktx2")]
    fn levels_of(format: wgpu::TextureFormat, size: (u32, u32), block: &[u8]) -> Vec<Vec<u8>> {
        let (bw, bh) = format.block_dimensions();
        let count = 32 - size.0.max(size.1).leading_zeros();
        (0..count)
            .map(|level| {
                let (w, h) = ((size.0 >> level).max(1), (size.1 >> level).max(1));
                block.repeat((w.div_ceil(bw) * h.div_ceil(bh)) as usize)
            })
            .collect()
    }

    #[cfg(feature = "ktx2")]
    #[test]
    fn a_decoded_map_holds_one_level_of_texels_at_once() {
        // 1024² of red BC1 blocks, every level, decoded for a device without
        // BC: the most it holds at once is the first level's texels (4 MiB),
        // not a padded copy beside them nor every level's together.
        let format = wgpu::TextureFormat::Bc1RgbaUnormSrgb;
        let size = (1024, 1024);
        let levels = levels_of(format, size, &[0x00, 0xf8, 0x00, 0xf8, 0, 0, 0, 0]);
        let levels: Vec<&[u8]> = levels.iter().map(Vec::as_slice).collect();
        let mut seen = Vec::new();
        let peak = counting::peak_during(|| {
            decoded_blocks(format, size, &levels, |level, texels| {
                let (w, h) = ((size.0 >> level).max(1), (size.1 >> level).max(1));
                assert_eq!(texels.len(), (w * h * 4) as usize, "level {level}");
                assert!(
                    texels.chunks(4).all(|t| t == [255, 0, 0, 255]),
                    "level {level}"
                );
                seen.push(level);
            })
            .expect("decoded");
        });
        assert_eq!(seen, (0..levels.len()).collect::<Vec<_>>());
        let first = 1024 * 1024 * 4;
        assert!(
            peak <= first + (64 << 10),
            "held {peak} bytes at once, the first level {first}"
        );
    }

    #[cfg(feature = "ktx2")]
    #[test]
    fn a_map_over_the_decoded_bound_is_refused_before_its_texels_are_made() {
        // 8192 by 4096 of BC1 blocks (16 MiB) would be 128 MiB of texels:
        // over `MAX_DECODED`, refused unread, nothing allocated for it.
        let format = wgpu::TextureFormat::Bc1RgbaUnorm;
        let size = (8192, 4096);
        assert!(size.0 as usize * size.1 as usize * 4 > MAX_DECODED);
        let first = vec![0u8; (size.0 / 4 * size.1 / 4 * 8) as usize];
        let mut called = false;
        let peak = counting::peak_during(|| {
            let decoded = decoded_blocks(format, size, &[&first], |_, _| called = true);
            let why = decoded.unwrap_err();
            assert!(
                why.contains("8192 by 4096") && why.contains("128 MiB") && why.contains("64 MiB"),
                "{why}"
            );
        });
        assert!(!called);
        assert!(peak < 4 << 10, "allocated {peak} bytes for a refused map");
        // Through the upload a frame or a texture thread makes, on a device
        // without BC (wgpu's no-op one): no texture, and the texture isn't
        // marked failed (a device that holds BC takes its blocks as they are).
        let file = ktx2::write(size.0, size.1, 131, &[first]);
        let texture = crate::Texture::decode(&file).unwrap();
        let levels = texture.levels(wgpu::Features::empty()).expect("its blocks");
        let (device, queue) = noop_device();
        assert!(upload_levels(&device, &queue, &texture, false, &levels).is_none());
        assert!(!texture.failed());
    }

    #[cfg(feature = "ktx2-basis")]
    #[test]
    fn a_basis_map_too_large_for_its_device_as_rgba8_is_refused_there_unread() {
        // 8192² of UASTC (64 MiB of blocks) would be 256 MiB of RGBA8 for a
        // device without BC or ASTC: refused before the transcoder makes
        // any of it, and not failed (a device with BC7 takes 64 MiB).
        let file = ktx2::write(8192, 8192, 0, &[vec![0; 64 << 20]]);
        let texture = crate::Texture::decode_owned(file).unwrap();
        let peak = counting::peak_during(|| {
            assert!(texture.kept_levels(wgpu::Features::empty()).is_none());
            assert!(texture.pixels().is_none());
        });
        assert!(peak < 64 << 10, "allocated {peak} bytes for a refused map");
        assert!(!texture.failed());
    }

    /// Every format texture2ddecoder decodes for us: ASTC's every LDR
    /// block size, then BC1 to BC7, ETC2 and EAC.
    #[cfg(feature = "ktx2")]
    fn decoded_formats() -> Vec<wgpu::TextureFormat> {
        use wgpu::{AstcBlock as B, AstcChannel, TextureFormat as F};
        let astc = [
            B::B4x4,
            B::B5x4,
            B::B5x5,
            B::B6x5,
            B::B6x6,
            B::B8x5,
            B::B8x6,
            B::B8x8,
            B::B10x5,
            B::B10x6,
            B::B10x8,
            B::B10x10,
            B::B12x10,
            B::B12x12,
        ]
        .map(|block| F::Astc {
            block,
            channel: AstcChannel::Unorm,
        });
        let others = [
            F::Bc1RgbaUnorm,
            F::Bc2RgbaUnorm,
            F::Bc3RgbaUnorm,
            F::Bc4RUnorm,
            F::Bc4RSnorm,
            F::Bc5RgUnorm,
            F::Bc5RgSnorm,
            F::Bc6hRgbUfloat,
            F::Bc6hRgbFloat,
            F::Bc7RgbaUnorm,
            F::Etc2Rgb8Unorm,
            F::Etc2Rgb8A1Unorm,
            F::Etc2Rgba8Unorm,
            F::EacR11Unorm,
            F::EacR11Snorm,
            F::EacRg11Unorm,
            F::EacRg11Snorm,
        ];
        astc.into_iter().chain(others).collect()
    }

    /// Panics in texture2ddecoder, caught (`decoded_blocks`), aren't said:
    /// a test that makes thousands of them on purpose isn't drowned in
    /// them. Others' are, as before.
    #[cfg(feature = "ktx2")]
    fn decoder_panics_unsaid() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let said = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if !info
                    .location()
                    .is_some_and(|at| at.file().contains("texture2ddecoder"))
                {
                    said(info);
                }
            }));
        });
    }

    /// One block of `format`, decoded alone: how it went.
    #[cfg(feature = "ktx2")]
    fn one_block(format: wgpu::TextureFormat, block: &[u8]) -> Result<(), String> {
        let size = format.block_dimensions();
        decoded_blocks(format, size, &[block], |level, texels| {
            // In bounds: the level's own texels, no more, no fewer.
            assert_eq!((level, texels.len()), (0, (size.0 * size.1 * 4) as usize));
        })
    }

    #[cfg(feature = "ktx2")]
    #[test]
    fn random_blocks_of_every_decoded_format_decode_or_are_refused_never_panic_out() {
        // Seeded random blocks, each decoded alone (a panic stops a map's
        // decoding at its block, so blocks one at a time reach them all):
        // 300,000 of every ASTC block size's (reserved block modes and
        // encodings among them) and of BC7's, 20,000 of every other
        // format's. Each decodes, its texels in bounds, or is refused; a
        // panic in the decoder is caught (`decoded_blocks`), never out of it.
        decoder_panics_unsaid();
        // A thread a format, each its own seed.
        let found: Vec<_> = std::thread::scope(|scope| {
            let runs: Vec<_> = decoded_formats()
                .into_iter()
                .enumerate()
                .map(|(i, format)| {
                    scope.spawn(move || {
                        let mut state = 0x9e37_79b9_7f4a_7c15_u64 ^ (i as u64 + 1) << 32;
                        let mut random = move || {
                            state ^= state << 13;
                            state ^= state >> 7;
                            state ^= state << 17;
                            state
                        };
                        let count = match format {
                            wgpu::TextureFormat::Astc { .. }
                            | wgpu::TextureFormat::Bc7RgbaUnorm => 300_000,
                            _ => 20_000,
                        };
                        let bytes = format.block_copy_size(None).unwrap() as usize;
                        let mut block = vec![0u8; bytes];
                        let (mut refused, mut panicked, mut first) = (0, 0, None);
                        for _ in 0..count {
                            block.chunks_mut(8).for_each(|c| {
                                let r = random().to_le_bytes();
                                c.copy_from_slice(&r[..c.len()]);
                            });
                            if let Err(why) = one_block(format, &block) {
                                refused += 1;
                                if why.contains("panicked") {
                                    panicked += 1;
                                    first.get_or_insert_with(|| block.clone());
                                }
                            }
                        }
                        (format, count, refused, panicked, first)
                    })
                })
                .collect();
            runs.into_iter().map(|run| run.join().unwrap()).collect()
        });
        // Said, for the record: how many each refused and how many of those
        // panicked, and a block of each format that did (kept in
        // `BLOCKS_THAT_PANICKED`).
        for (format, count, refused, panicked, first) in found {
            eprintln!("{format:?}: {refused} of {count} refused, {panicked} of them panicked");
            if let Some(block) = first {
                eprintln!("  panicked: {block:?}");
            }
        }
    }

    /// A block of each format that panicked texture2ddecoder 0.1.2 (as
    /// pinned), found by `random_blocks_of_every_decoded_format…`: each
    /// a map's whole first level, ASTC's every block size and BC6H's.
    #[cfg(feature = "ktx2")]
    const BLOCKS_THAT_PANICKED: [(wgpu::AstcBlock, [u8; 16]); 14] = {
        use wgpu::AstcBlock as B;
        [
            (
                B::B4x4,
                [
                    236, 74, 197, 161, 224, 22, 235, 44, 121, 251, 123, 78, 206, 29, 16, 151,
                ],
            ),
            (
                B::B5x4,
                [
                    87, 191, 139, 82, 243, 69, 225, 228, 233, 157, 249, 90, 170, 83, 61, 238,
                ],
            ),
            (
                B::B5x5,
                [
                    88, 177, 156, 218, 182, 182, 177, 17, 58, 222, 90, 58, 99, 175, 84, 97,
                ],
            ),
            (
                B::B6x5,
                [
                    91, 237, 164, 81, 109, 220, 231, 177, 65, 146, 21, 91, 154, 93, 148, 15,
                ],
            ),
            (
                B::B6x6,
                [
                    246, 161, 140, 2, 189, 135, 118, 76, 53, 5, 245, 69, 157, 186, 254, 214,
                ],
            ),
            (
                B::B8x5,
                [
                    219, 91, 64, 190, 20, 196, 13, 0, 172, 77, 9, 151, 217, 85, 8, 35,
                ],
            ),
            (
                B::B8x6,
                [
                    216, 100, 60, 88, 230, 231, 248, 52, 17, 42, 44, 67, 164, 10, 36, 97,
                ],
            ),
            (
                B::B8x8,
                [
                    87, 86, 158, 92, 139, 231, 237, 61, 59, 31, 14, 153, 240, 251, 31, 234,
                ],
            ),
            (
                B::B10x5,
                [
                    181, 203, 163, 145, 127, 153, 198, 229, 98, 193, 192, 253, 148, 71, 190, 69,
                ],
            ),
            (
                B::B10x6,
                [
                    70, 26, 142, 81, 93, 234, 162, 236, 242, 87, 7, 228, 42, 5, 82, 149,
                ],
            ),
            (
                B::B10x8,
                [
                    201, 166, 14, 95, 49, 126, 212, 140, 196, 41, 72, 121, 251, 36, 226, 225,
                ],
            ),
            (
                B::B10x10,
                [
                    199, 38, 63, 65, 10, 59, 67, 217, 74, 9, 56, 236, 147, 202, 201, 208,
                ],
            ),
            (
                B::B12x10,
                [
                    210, 27, 100, 253, 196, 186, 166, 187, 101, 103, 40, 96, 230, 132, 98, 222,
                ],
            ),
            (
                B::B12x12,
                [
                    233, 69, 31, 218, 11, 93, 12, 204, 34, 33, 131, 224, 138, 57, 43, 191,
                ],
            ),
        ]
    };
    #[cfg(feature = "ktx2")]
    const BC6H_BLOCKS_THAT_PANICKED: [(wgpu::TextureFormat, [u8; 16]); 2] = [
        (
            wgpu::TextureFormat::Bc6hRgbUfloat,
            [
                47, 217, 231, 213, 41, 42, 93, 36, 93, 189, 37, 126, 139, 41, 114, 193,
            ],
        ),
        (
            wgpu::TextureFormat::Bc6hRgbFloat,
            [
                79, 39, 208, 142, 99, 184, 202, 111, 193, 180, 175, 144, 113, 182, 226, 176,
            ],
        ),
    ];

    #[cfg(feature = "ktx2")]
    #[test]
    fn blocks_that_panicked_the_decoder_are_refused_saying_so() {
        // Regression cases: each is caught and refused, saying the decoder
        // panicked. (Should a newer decoder refuse or decode one, its case
        // here changes with it.)
        decoder_panics_unsaid();
        let astc = BLOCKS_THAT_PANICKED.map(|(block, bytes)| {
            let format = wgpu::TextureFormat::Astc {
                block,
                channel: wgpu::AstcChannel::UnormSrgb,
            };
            (format, bytes)
        });
        for (format, block) in astc.into_iter().chain(BC6H_BLOCKS_THAT_PANICKED) {
            let why = one_block(format, &block).unwrap_err();
            assert!(why.contains("panicked"), "{format:?}: {why}");
        }
    }

    #[test]
    fn a_decoder_that_panics_is_a_map_that_doesnt_decode() {
        // On any thread (a frame's: `render::materials::decoded`): `None`,
        // the texture marked failed, the panic not out of it.
        let texture = crate::Texture::from_rgba8(1, 1, vec![0; 4]).unwrap();
        assert_eq!(guarded(&texture, || Some(1)), Some(1));
        assert!(!texture.failed());
        assert_eq!(
            guarded(&texture, || -> Option<u32> { panic!("a hostile file") }),
            None
        );
        assert!(texture.failed());
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

    /// A device of wgpu's no-op backend: no compressed textures.
    #[cfg(feature = "ktx2")]
    fn noop_device() -> (wgpu::Device, wgpu::Queue) {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::NOOP;
        descriptor.backend_options.noop.enable = true;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster_free(instance.request_adapter(&Default::default())).unwrap();
        pollster_free(adapter.request_device(&Default::default())).unwrap()
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
