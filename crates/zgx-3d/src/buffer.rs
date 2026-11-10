//! GPU data an app owns (design-3d-aaa.md, 5.3): buffers and textures made
//! by `World::add_buffer` and `World::add_texture`, bound by compute passes
//! and read back without stalling a frame.
//!
//! A handle owns its data: the world holds it weakly, counts its bytes in
//! `World::memory` while it lives, and lets go of the device's copy when the
//! last clone drops (by the next frame). Its device copy is made with the
//! first frame drawn, and again, zeroed, on a new device.
//!
//! Textures are never written by a copy: on Metal a blit into a texture
//! opens 8 MB driver blocks. They're made renderable so wgpu clears them by
//! a render pass, and written by compute passes only.
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ops::Range,
    rc::{Rc, Weak},
    sync::{Arc, atomic::AtomicU8},
};
use zgx_gpu::wgpu;

/// What `World::set_memory_cap` starts at: 256 MiB.
pub const DEFAULT_MEMORY_CAP: u64 = 256 << 20;
/// What `World::set_readback_budget` starts at: 64 KiB a frame.
pub const DEFAULT_READBACK_BUDGET: u64 = 64 << 10;

/// A storage texture's texel format: those every tier A device can write
/// from a compute pass.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    /// One 32-bit float. Read and written by one pass (`Access::ReadWrite`).
    R32Float,
    /// One 32-bit unsigned integer. Read and written by one pass.
    R32Uint,
    /// One 32-bit signed integer. Read and written by one pass.
    R32Sint,
    /// Two 32-bit floats.
    Rg32Float,
    /// Four 8-bit channels, 0 to 1.
    Rgba8Unorm,
    /// Four 16-bit floats.
    Rgba16Float,
    /// Four 32-bit floats.
    Rgba32Float,
    /// Four 32-bit unsigned integers.
    Rgba32Uint,
}
impl Format {
    pub(crate) fn wgpu(self) -> wgpu::TextureFormat {
        use wgpu::TextureFormat as F;
        match self {
            Self::R32Float => F::R32Float,
            Self::R32Uint => F::R32Uint,
            Self::R32Sint => F::R32Sint,
            Self::Rg32Float => F::Rg32Float,
            Self::Rgba8Unorm => F::Rgba8Unorm,
            Self::Rgba16Float => F::Rgba16Float,
            Self::Rgba32Float => F::Rgba32Float,
            Self::Rgba32Uint => F::Rgba32Uint,
        }
    }
    /// Bytes a texel.
    pub fn texel(self) -> u32 {
        match self {
            Self::R32Float | Self::R32Uint | Self::R32Sint | Self::Rgba8Unorm => 4,
            Self::Rg32Float | Self::Rgba16Float => 8,
            Self::Rgba32Float | Self::Rgba32Uint => 16,
        }
    }
    /// Whether one pass may read and write it: WebGPU's baseline allows the
    /// one-channel 32-bit formats alone.
    pub fn read_write(self) -> bool {
        matches!(self, Self::R32Float | Self::R32Uint | Self::R32Sint)
    }
    pub(crate) fn from_naga(format: wgpu::naga::StorageFormat) -> Option<Self> {
        use wgpu::naga::StorageFormat as S;
        Some(match format {
            S::R32Float => Self::R32Float,
            S::R32Uint => Self::R32Uint,
            S::R32Sint => Self::R32Sint,
            S::Rg32Float => Self::Rg32Float,
            S::Rgba8Unorm => Self::Rgba8Unorm,
            S::Rgba16Float => Self::Rgba16Float,
            S::Rgba32Float => Self::Rgba32Float,
            S::Rgba32Uint => Self::Rgba32Uint,
            _ => return None,
        })
    }
}

/// A buffer to make (`World::add_buffer`).
#[derive(Clone, Debug)]
pub struct BufferDesc {
    pub(crate) size: u64,
    pub(crate) contents: Option<Vec<u8>>,
}
impl BufferDesc {
    /// Room for `count` values of `T`, zeroed.
    pub fn storage<T: bytemuck::Pod>(count: usize) -> Self {
        Self::bytes((count * std::mem::size_of::<T>()) as u64)
    }
    /// `size` bytes, zeroed (rounded up to a multiple of 4).
    pub fn bytes(size: u64) -> Self {
        Self {
            size: size.next_multiple_of(4),
            contents: None,
        }
    }
    /// Holding `values` (padded with zeros to a multiple of 4 bytes).
    pub fn from_slice<T: bytemuck::Pod>(values: &[T]) -> Self {
        let mut contents = bytemuck::cast_slice::<T, u8>(values).to_vec();
        contents.resize(contents.len().next_multiple_of(4), 0);
        Self {
            size: contents.len() as u64,
            contents: Some(contents),
        }
    }
}

/// A texture to make (`World::add_texture`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureDesc {
    pub(crate) size: (u32, u32),
    pub(crate) format: Format,
}
impl TextureDesc {
    /// `width` by `height` texels of `format`, zeroed, for compute passes to
    /// write and read and materials to sample.
    pub fn storage(width: u32, height: u32, format: Format) -> Self {
        Self {
            size: (width, height),
            format,
        }
    }
    pub(crate) fn bytes(&self) -> u64 {
        u64::from(self.size.0) * u64::from(self.size.1) * u64::from(self.format.texel())
    }
}

/// Why GPU data couldn't be made.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuDataError {
    /// It would take the app's data past the world's cap
    /// (`World::set_memory_cap`).
    OverBudget {
        /// Its bytes.
        asked: u64,
        /// The app's live bytes already.
        used: u64,
        /// The cap.
        cap: u64,
    },
    /// No bytes, or more than a device of tier A is sure to hold.
    Size(u64),
}
impl std::fmt::Display for GpuDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverBudget { asked, used, cap } => write!(
                f,
                "{asked} bytes more would take the app's GPU data past its cap ({used} of {cap} used)"
            ),
            Self::Size(size) => write!(f, "a buffer or texture of {size} bytes can't be made"),
        }
    }
}
impl std::error::Error for GpuDataError {}

/// Why a write was refused.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    /// It runs past the buffer's end.
    OutOfRange,
    /// Its offset or length isn't a multiple of 4 bytes.
    Unaligned,
}
impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OutOfRange => "the write runs past the buffer's end",
            Self::Unaligned => "a write's offset and length are multiples of 4 bytes",
        })
    }
}
impl std::error::Error for WriteError {}

/// Why a readback has no bytes.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The range runs past the buffer's end.
    OutOfRange,
    /// The world was dropped first.
    Dropped,
    /// The view moved to another device before it was read.
    DeviceChanged,
}
impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OutOfRange => "the read runs past the buffer's end",
            Self::Dropped => "the world was dropped before the read",
            Self::DeviceChanged => "the view moved to another device before the read",
        })
    }
}
impl std::error::Error for ReadError {}

/// What the world and its handles share: the view to wake, writes waiting
/// for a frame, and reads waiting for the device.
pub(crate) struct Link {
    pub(crate) view: zgx::gpu_view::GpuView,
    pub(crate) writes: RefCell<Vec<Weak<BufferShared>>>,
    pub(crate) reads: RefCell<VecDeque<ReadJob>>,
    next_id: Cell<u64>,
}
impl Link {
    pub(crate) fn new(view: zgx::gpu_view::GpuView) -> Self {
        Self {
            view,
            writes: RefCell::default(),
            reads: RefCell::default(),
            next_id: Cell::new(1),
        }
    }
    pub(crate) fn id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }
}

pub(crate) struct BufferShared {
    pub(crate) id: u64,
    pub(crate) size: u64,
    /// What it starts holding, until a device copy is made.
    pub(crate) contents: RefCell<Option<Vec<u8>>>,
    /// Writes since the last frame: offset and bytes.
    pub(crate) writes: RefCell<Vec<(u64, Vec<u8>)>>,
    /// Its device copy, and the device generation it was made on.
    pub(crate) gpu: RefCell<Option<(u64, wgpu::Buffer)>>,
    pub(crate) link: Rc<Link>,
}

/// A buffer of the app's on the device (`World::add_buffer`): bound by
/// name in compute passes, written from the CPU, read back a few frames
/// later. Clones share it; it's let go when the last drops.
#[derive(Clone)]
pub struct GpuBuffer(pub(crate) Rc<BufferShared>);
impl std::fmt::Debug for GpuBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuBuffer")
            .field("id", &self.0.id)
            .field("size", &self.0.size)
            .finish()
    }
}
impl PartialEq for GpuBuffer {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl GpuBuffer {
    /// Its bytes.
    pub fn size(&self) -> u64 {
        self.0.size
    }
    /// Write `values` at byte `offset`, before the next frame's passes run;
    /// it asks for one.
    pub fn write<T: bytemuck::Pod>(&self, offset: u64, values: &[T]) -> Result<(), WriteError> {
        let bytes: &[u8] = bytemuck::cast_slice(values);
        let len = bytes.len() as u64;
        if !offset.is_multiple_of(4) || !len.is_multiple_of(4) {
            return Err(WriteError::Unaligned);
        }
        if offset.checked_add(len).is_none_or(|end| end > self.0.size) {
            return Err(WriteError::OutOfRange);
        }
        if len == 0 {
            return Ok(());
        }
        let mut writes = self.0.writes.borrow_mut();
        if writes.is_empty() {
            self.0.link.writes.borrow_mut().push(Rc::downgrade(&self.0));
        }
        writes.push((offset, bytes.to_vec()));
        self.0.link.view.invalidate();
        Ok(())
    }
    /// Its bytes in `range` as the GPU has them after a frame's passes: a
    /// readback resolving 1 to 3 frames later, the world drawing until it
    /// does. Reads go in the order asked, within the world's budget a frame
    /// (`World::set_readback_budget`); none stalls a frame.
    pub fn read(&self, range: Range<u64>) -> Readback {
        if range.start > range.end || range.end > self.0.size {
            return Readback::failed(ReadError::OutOfRange);
        }
        let start = range.start & !3;
        let end = range.end.next_multiple_of(4);
        let (readback, state) = Readback::pending();
        let job = ReadJob {
            state,
            source: Source::Buffer(Rc::downgrade(&self.0)),
            trim: (range.start - start) as usize..(range.end - start) as usize,
            total: end - start,
            from: start,
            stage: Stage::Queued,
            copied: 0,
            staging: None,
            generation: 0,
        };
        self.0.link.reads.borrow_mut().push_back(job);
        self.0.link.view.invalidate();
        readback
    }
}

pub(crate) struct TextureShared {
    pub(crate) id: u64,
    pub(crate) desc: TextureDesc,
    pub(crate) gpu: RefCell<Option<(u64, wgpu::Texture, wgpu::TextureView)>>,
    pub(crate) link: Rc<Link>,
}

/// A texture of the app's on the device (`World::add_texture`): written by
/// compute passes, sampled by them and by materials, read back a few frames
/// later. Clones share it; it's let go when the last drops.
#[derive(Clone)]
pub struct GpuTexture(pub(crate) Rc<TextureShared>);
impl std::fmt::Debug for GpuTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuTexture")
            .field("id", &self.0.id)
            .field("size", &self.0.desc.size)
            .field("format", &self.0.desc.format)
            .finish()
    }
}
impl PartialEq for GpuTexture {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl GpuTexture {
    /// Its width and height in texels.
    pub fn size(&self) -> (u32, u32) {
        self.0.desc.size
    }
    /// Its texel format.
    pub fn format(&self) -> Format {
        self.0.desc.format
    }
    /// Its texels as the GPU has them after a frame's passes, rows top to
    /// bottom, packed: a readback as [`GpuBuffer::read`]'s, taken a band of
    /// rows at a time within the budget.
    pub fn read(&self) -> Readback {
        let (width, height) = self.0.desc.size;
        let row = u64::from(width) * u64::from(self.0.desc.format.texel());
        let (readback, state) = Readback::pending();
        let job = ReadJob {
            state,
            source: Source::Texture(Rc::downgrade(&self.0)),
            trim: 0..(row * u64::from(height)) as usize,
            total: u64::from(height),
            from: 0,
            stage: Stage::Queued,
            copied: 0,
            staging: None,
            generation: 0,
        };
        self.0.link.reads.borrow_mut().push_back(job);
        self.0.link.view.invalidate();
        readback
    }
}

/// Bytes on their way back from the device: poll `try_take` a frame, or
/// await it.
#[must_use = "a readback does nothing to the data; take its bytes"]
pub struct Readback(Rc<ReadState>);
#[derive(Default)]
pub(crate) struct ReadState {
    result: RefCell<Option<Result<Vec<u8>, ReadError>>>,
    waker: RefCell<Option<std::task::Waker>>,
    done: Cell<bool>,
}
impl ReadState {
    /// Whether it was resolved (its result may have been taken since).
    pub(crate) fn done(&self) -> bool {
        self.done.get()
    }
    pub(crate) fn resolve(&self, result: Result<Vec<u8>, ReadError>) {
        self.done.set(true);
        *self.result.borrow_mut() = Some(result);
        if let Some(waker) = self.waker.borrow_mut().take() {
            waker.wake();
        }
    }
}
impl Readback {
    fn pending() -> (Self, Rc<ReadState>) {
        let state = Rc::new(ReadState::default());
        (Self(state.clone()), state)
    }
    fn failed(error: ReadError) -> Self {
        let state = ReadState::default();
        *state.result.borrow_mut() = Some(Err(error));
        Self(Rc::new(state))
    }
    /// Whether its bytes (or why there are none) have come.
    pub fn is_ready(&self) -> bool {
        self.0.result.borrow().is_some()
    }
    /// Its bytes, once they've come (taken: the next call has none).
    pub fn try_take(&self) -> Option<Result<Vec<u8>, ReadError>> {
        self.0.result.borrow_mut().take()
    }
}
impl std::fmt::Debug for Readback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Readback")
            .field("ready", &self.is_ready())
            .finish()
    }
}
impl std::future::Future for Readback {
    type Output = Result<Vec<u8>, ReadError>;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.0.result.borrow_mut().take() {
            Some(result) => std::task::Poll::Ready(result),
            None => {
                *self.0.waker.borrow_mut() = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        }
    }
}

pub(crate) enum Source {
    Buffer(Weak<BufferShared>),
    Texture(Weak<TextureShared>),
}

/// Where a read is.
#[derive(Clone, Debug)]
pub(crate) enum Stage {
    /// Waiting its turn in the budget.
    Queued,
    /// Copied whole into its staging buffer in the frame of this index:
    /// mapped at the start of a later one.
    Copied(u64),
    /// Being mapped: the device sets it to 1 when mapped, 2 when that
    /// failed.
    Mapping(Arc<AtomicU8>),
}

/// A read on its way: copied a part a frame into a staging buffer, then
/// mapped and resolved.
pub(crate) struct ReadJob {
    pub(crate) state: Rc<ReadState>,
    pub(crate) source: Source,
    /// The bytes asked for within what's copied.
    pub(crate) trim: Range<usize>,
    /// From where, and how much in all: bytes of a buffer (aligned to 4),
    /// rows of a texture.
    pub(crate) from: u64,
    pub(crate) total: u64,
    pub(crate) copied: u64,
    pub(crate) stage: Stage,
    pub(crate) staging: Option<wgpu::Buffer>,
    /// The device generation it was copied on.
    pub(crate) generation: u64,
}
