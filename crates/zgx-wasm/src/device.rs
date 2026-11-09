//! The session's own GPU: a wgpu device of its own on the adapter zgx draws
//! with, so what the guest does to it (a shader that never ends, a device
//! lost, a driver's reset of its context) is its session's alone.
//!
//! Its frames reach zgx's device as an external surface's do, in this
//! process: zgx makes a ring of textures it can share (Vulkan memory as an
//! opaque file descriptor on Linux, IOSurfaces on macOS, Direct3D 12 shared
//! handles on Windows), the guest's device imports them, draws into one,
//! waits for its GPU (the watchdog: too long and the session fails) and
//! presents it. Where nothing can be shared, the guest's frame is read back
//! and uploaded, as shared memory is.
use crate::{
    runtime::Log,
    shared::{Link, Presented},
};
use std::{
    io,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use zgx_gpu::wgpu;
use zgx_surface::{
    protocol::{Buffer, BufferDesc, DeviceIds},
    wgpu29,
};

/// Frame textures in a ring: one showing, one waiting, one drawn into.
pub(crate) const RING: u32 = 3;
/// Textures of the guest's own drawn into where nothing is shared, at most.
const OWN_TEXTURES: usize = 3;

/// The adapter zgx draws with: what the guest's device is matched to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HostGpu {
    backend: wgpu::Backend,
    vulkan: Option<DeviceIds>,
    metal: Option<DeviceIds>,
    luid: Option<u64>,
    vendor: u32,
    device: u32,
    name: String,
}
impl HostGpu {
    pub fn of(device: &wgpu::Device) -> Self {
        let info = device.adapter_info();
        Self {
            backend: info.backend,
            vulkan: wgpu29::device_ids_of(device),
            metal: wgpu29::iosurface_ids_of(device),
            luid: wgpu29::device_luid_of(device),
            vendor: info.vendor,
            device: info.device,
            name: info.name.clone(),
        }
    }
}

/// How the guest's frames reach zgx's device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sharing {
    Vulkan,
    IoSurface,
    Dxgi,
    /// Read back to the CPU and uploaded (GL, a driver that can't export).
    Readback,
}
/// What `device` (zgx's) can share with a device of the guest's.
pub(crate) fn sharing_of(device: &wgpu::Device) -> Sharing {
    if std::env::var_os("ZGX_WASM_READBACK").is_some_and(|v| v != "0") {
        Sharing::Readback
    } else if wgpu29::can_share(device) {
        Sharing::Vulkan
    } else if wgpu29::can_share_iosurface(device) {
        Sharing::IoSurface
    } else if wgpu29::device_luid_of(device).is_some() {
        Sharing::Dxgi
    } else {
        Sharing::Readback
    }
}

/// A ring of textures on zgx's `device` at `size`, cleared, and what the
/// guest's device imports them from.
pub(crate) fn export_ring(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sharing: Sharing,
    size: (u32, u32),
) -> io::Result<(Vec<wgpu::Texture>, Vec<Buffer>, Option<Cleared>)> {
    let (mut textures, mut buffers) = (Vec::new(), Vec::new());
    for index in 0..RING {
        let (texture, handle, desc) = match sharing {
            Sharing::Vulkan => {
                let (texture, handle, desc) = wgpu29::export(device, size.0, size.1)?;
                (texture, Some(handle), desc)
            }
            Sharing::Dxgi => {
                let (texture, handle, desc) = wgpu29::export_dxgi(device, size.0, size.1)?;
                (texture, Some(handle), desc)
            }
            Sharing::IoSurface => {
                let (texture, surface, desc) = wgpu29::export_iosurface(device, size.0, size.1)?;
                let BufferDesc::IoSurface { id, .. } = desc else {
                    return Err(io::Error::other("an IOSurface export of another kind"));
                };
                let desc = BufferDesc::IoSurface {
                    id,
                    surface: Some(surface),
                };
                (texture, None, desc)
            }
            Sharing::Readback => return Ok((Vec::new(), Vec::new(), None)),
        };
        textures.push(texture);
        buffers.push(Buffer {
            index,
            handle,
            desc,
        });
    }
    // Cleared before the guest has them: zgx's first use of each moves it
    // out of the undefined layout (which may discard) before the guest
    // draws, and an IOSurface's first frame is transparent.
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("zgx-wasm clear"),
    });
    for texture in &textures {
        let view = texture.create_view(&Default::default());
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("zgx-wasm clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    // Waited for on the guest's thread (`Cleared::wait`), never zgx's.
    let submission = queue.submit([encoder.finish()]);
    Ok((
        textures,
        buffers,
        Some(Cleared {
            device: device.clone(),
            submission,
        }),
    ))
}

/// zgx's clearing of a new ring, which the guest waits for before drawing
/// into it.
pub(crate) struct Cleared {
    device: wgpu::Device,
    submission: wgpu::SubmissionIndex,
}
impl Cleared {
    fn wait(self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.device.poll(wgpu::PollType::Wait {
                submission_index: Some(self.submission),
                timeout: Some(Duration::from_secs(2)),
            })
        }));
    }
}

/// `device.poll`, waiting up to `timeout`: wgpu panics on what it can't
/// tell as a timeout (a lost device among them), which here is an error.
pub(crate) fn wait(device: &wgpu::Device, timeout: Duration) -> Result<(), WaitError> {
    let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(timeout),
        })
    }));
    match polled {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(wgpu::PollError::Timeout)) => Err(WaitError::Timeout),
        Ok(Err(error)) => Err(WaitError::Failed(error.to_string())),
        Err(_) => Err(WaitError::Failed("the device failed (lost?)".into())),
    }
}
pub(crate) enum WaitError {
    Timeout,
    Failed(String),
}

/// The guest's device and queue.
pub(crate) struct Device {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// `Features::TIMESTAMP_QUERY`.
    pub timestamps: bool,
    pub srgb_views: bool,
    host: HostGpu,
    lost: Arc<Mutex<Option<String>>>,
}

/// The guests' wgpu instance, one for the process.
fn instance() -> &'static wgpu::Instance {
    static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
    INSTANCE
        .get_or_init(|| wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle()))
}

/// A device of the guest's own on the adapter `host` names (by its Vulkan
/// UUIDs, Metal registry id or Direct3D LUID; else its vendor, device and
/// name), its errors outside any scope going to `log`.
fn open(host: &HostGpu, log: &Log) -> Result<Device, String> {
    let backends = wgpu::Backends::from(host.backend);
    let adapters = pollster::block_on(instance().enumerate_adapters(backends));
    let same_info = |a: &wgpu::Adapter| {
        let info = a.get_info();
        info.vendor == host.vendor && info.device == host.device && info.name == host.name
    };
    let fallback = adapters
        .iter()
        .position(same_info)
        .or((!adapters.is_empty()).then_some(0));
    let matched = match (host.vulkan, host.metal, host.luid) {
        (Some(ids), _, _) | (None, Some(ids), _) => wgpu29::adapter_for(adapters.clone(), &ids),
        (None, None, Some(luid)) => wgpu29::adapter_for_luid(adapters.clone(), luid),
        _ => None,
    };
    let adapter = matched
        .or_else(|| fallback.map(|at| adapters[at].clone()))
        .ok_or_else(|| format!("no adapter like zgx's ({})", host.name))?;
    let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    let required_features = if timestamps {
        wgpu::Features::TIMESTAMP_QUERY
    } else {
        wgpu::Features::empty()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("zgx-wasm guest"),
        required_features,
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .map_err(|e| format!("its GPU device: {e}"))?;
    let errors = log.clone();
    device.on_uncaptured_error(Arc::new(move |error| {
        errors.line(format!("gpu error: {error}"));
    }));
    let lost: Arc<Mutex<Option<String>>> = Arc::default();
    let losing = lost.clone();
    device.set_device_lost_callback(move |reason, message| {
        if reason != wgpu::DeviceLostReason::Destroyed {
            *losing.lock().unwrap() = Some(format!("its GPU device was lost: {message}"));
        }
    });
    let srgb_views = adapter
        .get_downlevel_capabilities()
        .flags
        .contains(wgpu::DownlevelFlags::VIEW_FORMATS);
    Ok(Device {
        device,
        queue,
        timestamps,
        srgb_views,
        host: host.clone(),
        lost,
    })
}

/// Keep `device` (and whatever else `keep` holds of it) until its GPU work
/// is done, on a thread of its own, then drop them: wgpu's queue waits a
/// few seconds as it drops and then panics, and destroying resources the
/// GPU still uses is undefined. A guest's work that never ends keeps it
/// until the driver loses the device. Once the process exits, a device is
/// left to the driver's teardown, never destroyed beside it (NVIDIA's
/// crashes so).
pub(crate) fn reap<T: Send + 'static>(device: wgpu::Device, keep: T) {
    exit::hook();
    let spawned = std::thread::Builder::new()
        .name("zgx-wasm reaper".into())
        .spawn(move || {
            let since = std::time::Instant::now();
            loop {
                let Some(_busy) = exit::busy() else {
                    std::mem::forget((keep, device));
                    return;
                };
                match wait(&device, Duration::from_millis(100)) {
                    Err(WaitError::Timeout) => continue,
                    _ => break,
                }
            }
            if std::env::var_os("ZGX_WASM_LOG").is_some_and(|v| v != "0") {
                eprintln!(
                    "zgx-wasm: a device let go after its work, {:.2} s later",
                    since.elapsed().as_secs_f64()
                );
            }
            let Some(_busy) = exit::busy() else {
                std::mem::forget((keep, device));
                return;
            };
            drop(keep);
            drop(device);
        });
    if let Err(error) = spawned {
        eprintln!("zgx-wasm: no thread to release a GPU device: {error}");
    }
}

/// The process exiting: reapers stop calling the driver, and the exit waits
/// for those inside it.
mod exit {
    use std::sync::{
        Once,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    static EXITING: AtomicBool = AtomicBool::new(false);
    static BUSY: AtomicUsize = AtomicUsize::new(0);

    /// A reaper inside the driver, until dropped; `None` once exiting.
    pub struct Busy(());
    impl Drop for Busy {
        fn drop(&mut self) {
            BUSY.fetch_sub(1, Ordering::SeqCst);
        }
    }
    pub fn busy() -> Option<Busy> {
        BUSY.fetch_add(1, Ordering::SeqCst);
        if EXITING.load(Ordering::SeqCst) {
            BUSY.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Busy(()))
    }
    /// Run before the driver's own exit handlers: registered after it loaded.
    pub fn hook() {
        static HOOKED: Once = Once::new();
        #[cfg(unix)]
        HOOKED.call_once(|| {
            extern "C" fn exiting() {
                EXITING.store(true, Ordering::SeqCst);
                let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
                while BUSY.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < until {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            // Safety: a plain function, safe at exit (atomics and a sleep).
            unsafe { libc::atexit(exiting) };
        });
        // Windows ends the other threads before its libraries' teardown.
        #[cfg(not(unix))]
        HOOKED.call_once(|| {});
    }
}

/// Where the guest draws its frames.
enum Targets {
    /// Nothing yet: no device, or no textures from zgx.
    None,
    /// zgx's shared textures, imported, by index.
    Shared {
        generation: u32,
        textures: Vec<wgpu::Texture>,
        free: Vec<u32>,
    },
    /// Its own, read back after each frame.
    Own {
        all: Vec<wgpu::Texture>,
        free: Vec<wgpu::Texture>,
    },
}
/// The frame being drawn.
enum Current {
    Shared {
        generation: u32,
        index: u32,
        texture: wgpu::Texture,
    },
    Own(wgpu::Texture),
}

/// What a submission may ask of the GPU: a frame's worth, not a stall.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorkCaps {
    pub draws: u64,
    pub workgroups: u64,
}

/// The session's GPU: its device, its frames, what the guest holds.
pub(crate) struct Gpu {
    pub(crate) device: Option<Device>,
    pub(crate) max_bytes: u64,
    pub(crate) max_objects: u32,
    pub(crate) bytes: u64,
    pub(crate) objects: u32,
    /// How long a frame's GPU work may take before the session fails.
    watchdog: Duration,
    /// Loop iterations a shader invocation may run (`loops`; 0: unbounded).
    pub(crate) loop_budget: u32,
    /// When the call into the guest began: its GPU work, every
    /// submission's together, finishes within the watchdog of it.
    pub(crate) since: std::time::Instant,
    pub(crate) caps: WorkCaps,
    /// Since the last submission: draws and workgroups recorded, and
    /// whether a cap was passed (the submission is refused).
    pub(crate) draws: u64,
    pub(crate) workgroups: u64,
    pub(crate) over: Option<String>,
    size: (u32, u32),
    pub(crate) in_frame: bool,
    current: Option<Current>,
    targets: Targets,
    /// The readback's buffer, kept between frames.
    staging: Option<wgpu::Buffer>,
}

/// Errors captured on the guest's thread while it is alive.
pub(crate) struct Scope {
    guards: Vec<wgpu::ErrorScopeGuard>,
}
impl Scope {
    pub fn push(device: &wgpu::Device) -> Self {
        Self {
            guards: [
                wgpu::ErrorFilter::Validation,
                wgpu::ErrorFilter::OutOfMemory,
                wgpu::ErrorFilter::Internal,
            ]
            .into_iter()
            .map(|filter| device.push_error_scope(filter))
            .collect(),
        }
    }
    pub fn pop(mut self) -> Vec<String> {
        let mut errors = Vec::new();
        while let Some(guard) = self.guards.pop() {
            if let Some(error) = pollster::block_on(guard.pop()) {
                errors.push(error.to_string());
            }
        }
        errors
    }
}

impl Gpu {
    pub fn new(
        max_bytes: u64,
        max_objects: u32,
        watchdog: Duration,
        caps: WorkCaps,
        loop_budget: u32,
    ) -> Self {
        Self {
            loop_budget,
            since: std::time::Instant::now(),
            device: None,
            max_bytes,
            max_objects,
            bytes: 0,
            objects: 0,
            watchdog,
            caps,
            draws: 0,
            workgroups: 0,
            over: None,
            size: (0, 0),
            in_frame: false,
            current: None,
            targets: Targets::None,
            staging: None,
        }
    }
    /// zgx draws on `host`: a device of the guest's own on that adapter.
    /// Whether one it had was replaced (every object made before is dead:
    /// `gpu-reset`).
    pub fn set_host(&mut self, host: &HostGpu, log: &Log) -> Result<bool, String> {
        if self.device.as_ref().is_some_and(|d| d.host == *host) {
            return Ok(false);
        }
        let device = open(host, log)?;
        let old = self.device.replace(device);
        self.targets = Targets::None;
        self.current = None;
        self.staging = None;
        let reset = old.is_some();
        if let Some(old) = old {
            reap(old.device.clone(), old);
        }
        Ok(reset)
    }
    /// zgx's textures for its frames (none: it shares nothing, so the guest
    /// draws into its own, read back).
    pub fn set_buffers(
        &mut self,
        generation: u32,
        buffers: Vec<Buffer>,
        cleared: Option<Cleared>,
    ) -> Result<(), String> {
        if let Some(cleared) = cleared {
            cleared.wait();
        }
        let Some(device) = &self.device else {
            return Err("textures before a device".into());
        };
        if buffers.is_empty() {
            if !matches!(self.targets, Targets::Own { .. }) {
                self.targets = Targets::Own {
                    all: Vec::new(),
                    free: Vec::new(),
                };
            }
            return Ok(());
        }
        let mut textures = Vec::new();
        for buffer in buffers {
            // Safety: zgx made them on the adapter this device was opened
            // on (`open` matched it), as the external surfaces' client does.
            let texture = unsafe { wgpu29::import(&device.device, buffer) };
            match texture {
                Ok(texture) => textures.push(texture),
                Err(error) => {
                    self.targets = Targets::Own {
                        all: Vec::new(),
                        free: Vec::new(),
                    };
                    return Err(format!(
                        "its frames can't be shared ({error}): read back instead"
                    ));
                }
            }
        }
        self.targets = Targets::Shared {
            generation,
            free: (0..textures.len() as u32).rev().collect(),
            textures,
        };
        Ok(())
    }
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = (width, height);
        let size = self.size;
        if let Targets::Own { all, free } = &mut self.targets {
            free.retain(|t| (t.width(), t.height()) == size);
            all.retain(|t| (t.width(), t.height()) == size || !free.contains(t));
        }
    }
    /// Errors the guest's call leaves uncaught, captured from now.
    pub fn scope(&self) -> Option<Scope> {
        self.device.as_ref().map(|d| Scope::push(&d.device))
    }
    /// Why the device is gone, if it is.
    pub fn lost(&self) -> Option<String> {
        self.device
            .as_ref()
            .and_then(|d| d.lost.lock().unwrap().clone())
    }
    pub fn begin_frame(&mut self) {
        self.in_frame = true;
        self.current = None;
    }
    /// The frame drawn, once its GPU work is done: what to present, or why
    /// the session fails (the watchdog, a lost device).
    pub fn end_frame(&mut self) -> Result<Option<Presented>, String> {
        self.in_frame = false;
        let Some(current) = self.current.take() else {
            return Ok(None);
        };
        self.settle()?;
        match current {
            Current::Shared {
                generation, index, ..
            } => Ok(Some(Presented::Shared { generation, index })),
            Current::Own(texture) => {
                let read = self.read_back(&texture);
                if let Targets::Own { free, .. } = &mut self.targets {
                    free.push(texture);
                }
                let (pixels, width, height) = read?;
                Ok(Some(Presented::Pixels {
                    pixels,
                    width,
                    height,
                    bgra: true,
                }))
            }
        }
    }
    /// Wait for the device's work, up to the watchdog.
    pub(crate) fn settle(&self) -> Result<(), String> {
        let Some(device) = &self.device else {
            return Ok(());
        };
        if let Some(lost) = self.lost() {
            return Err(lost);
        }
        let left = self.watchdog.saturating_sub(self.since.elapsed());
        let waited = wait(&device.device, left);
        if let Some(lost) = self.lost() {
            return Err(lost);
        }
        match waited {
            Ok(()) => Ok(()),
            Err(WaitError::Timeout) => Err(format!(
                "its GPU work for a call ran longer than {:.1} s: stopped",
                self.watchdog.as_secs_f64()
            )),
            Err(WaitError::Failed(why)) => Err(format!("its GPU device failed: {why}")),
        }
    }
    /// A frame replaced unseen: free to draw into again.
    pub fn take_back(&mut self, frame: Option<Presented>) {
        if let Some(Presented::Shared { generation, index }) = frame {
            self.released(generation, index);
        }
    }
    fn released(&mut self, generation: u32, index: u32) {
        if let Targets::Shared {
            generation: ours,
            free,
            ..
        } = &mut self.targets
            && *ours == generation
            && !free.contains(&index)
        {
            free.push(index);
        }
    }
    /// The texture to draw this frame into (the same for each call in a
    /// frame).
    pub fn current_texture(&mut self, link: &Link) -> Result<wgpu::Texture, String> {
        if !self.in_frame {
            return Err("current-texture: only inside `frame`".into());
        }
        if let Some(current) = &self.current {
            return Ok(match current {
                Current::Shared { texture, .. } => texture.clone(),
                Current::Own(texture) => texture.clone(),
            });
        }
        for (generation, index) in link.released() {
            self.released(generation, index);
        }
        let Some(device) = &self.device else {
            return Err("current-texture: no GPU device".into());
        };
        let (width, height) = self.size;
        let current = match &mut self.targets {
            Targets::None => return Err("current-texture: no frame texture from zgx yet".into()),
            Targets::Shared {
                generation,
                textures,
                free,
            } => {
                let index = free
                    .pop()
                    .ok_or("current-texture: every frame texture is in use")?;
                Current::Shared {
                    generation: *generation,
                    index,
                    texture: textures[index as usize].clone(),
                }
            }
            Targets::Own { all, free } => {
                if width == 0 || height == 0 {
                    return Err("current-texture: the element has no size".into());
                }
                let texture = match free.pop() {
                    Some(texture) => texture,
                    None if all.len() < OWN_TEXTURES => {
                        let srgb = [wgpu::TextureFormat::Bgra8UnormSrgb];
                        let texture = device.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("zgx-wasm frame"),
                            size: wgpu::Extent3d {
                                width,
                                height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Bgra8Unorm,
                            usage: wgpu29::USAGE,
                            view_formats: if device.srgb_views { &srgb } else { &[] },
                        });
                        all.push(texture.clone());
                        texture
                    }
                    None => return Err("current-texture: every frame texture is in use".into()),
                };
                Current::Own(texture)
            }
        };
        let texture = match &current {
            Current::Shared { texture, .. } | Current::Own(texture) => texture.clone(),
        };
        self.current = Some(current);
        Ok(texture)
    }
    /// `texture`'s pixels (BGRA rows, tightly packed), once the GPU is done.
    fn read_back(&mut self, texture: &wgpu::Texture) -> Result<(Vec<u8>, u32, u32), String> {
        let device = self.device.as_ref().ok_or("no GPU device")?;
        let (width, height) = (texture.width(), texture.height());
        let row = width * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let size = u64::from(padded) * u64::from(height);
        if self.staging.as_ref().is_none_or(|b| b.size() < size) {
            self.staging = Some(device.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("zgx-wasm readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
        }
        let staging = self.staging.as_ref().expect("made above");
        let mut encoder = device.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        device.queue.submit([encoder.finish()]);
        let slice = staging.slice(..size);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |mapped| {
            let _ = tx.send(mapped);
        });
        self.settle()?;
        match rx.try_recv() {
            Ok(Ok(())) => {}
            _ => return Err("its frame couldn't be read back".into()),
        }
        let mut pixels = Vec::with_capacity((row * height) as usize);
        {
            let mapped = slice.get_mapped_range();
            for y in 0..height as usize {
                let at = y * padded as usize;
                pixels.extend_from_slice(&mapped[at..at + row as usize]);
            }
        }
        staging.unmap();
        Ok((pixels, width, height))
    }
}
