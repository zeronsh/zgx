//! A renderer's frames copied on the GPU into CoreVideo pixel buffers, for a
//! window to show as they are (`zgx::native_surface`) rather than read back
//! to bytes and uploaded again: what a video player in a window draws with.
use crate::GpuError;
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBuffer,
    CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferPool, CVPixelBufferPoolFlushFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferHeightKey, kCVPixelBufferIOSurfacePropertiesKey,
    kCVPixelBufferMetalCompatibilityKey, kCVPixelBufferPixelFormatTypeKey,
    kCVPixelBufferPoolAllocationThresholdKey, kCVPixelBufferWidthKey,
    kCVReturnWouldExceedAllocationThreshold,
};
use objc2_metal::{MTLPixelFormat, MTLTextureType};
use std::ptr::NonNull;

/// A frame in a CoreVideo pixel buffer, finished: made only here, by
/// [`crate::GpuRenderer::readback_surface`] or [`blank_surface`], and handed
/// to another thread as it may be.
#[derive(Clone)]
pub struct PixelBuffer(CFRetained<CVPixelBuffer>);
// SAFETY: CVPixelBuffer is a CoreFoundation object, retained and released
// atomically; its pixels are only read once a frame is handed over.
unsafe impl Send for PixelBuffer {}

impl PixelBuffer {
    /// A buffer the GPU has finished writing, never written again.
    pub(crate) fn new(buffer: CFRetained<CVPixelBuffer>) -> Self {
        Self(buffer)
    }
    /// It, for a scene to show as it is.
    pub fn native_surface(&self) -> Result<zgx::native_surface::NativeSurface, &'static str> {
        // SAFETY: Every `PixelBuffer` is IOSurface-backed, Metal-compatible
        // BGRA, premultiplied (the target's pixels), written in full before
        // it is made, and never written again: its pool hands it out anew
        // only once every holder lets go.
        unsafe { zgx::native_surface::NativeSurface::from_pixel_buffer(self.0.clone()) }
    }
    /// Its pixels as RGBA8 bytes, row after row (as `readback` gives them):
    /// read from the buffer now, locked for reading meanwhile.
    pub fn to_rgba(&self) -> Vec<u8> {
        let buffer = &self.0;
        let (width, height) = (
            CVPixelBufferGetWidth(buffer),
            CVPixelBufferGetHeight(buffer),
        );
        let mut out = Vec::with_capacity(width * height * 4);
        // SAFETY: Locked for reading while its rows are read: `height` rows
        // `bytes per row` apart, each at least `width` BGRA pixels.
        unsafe {
            if CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly) != 0 {
                return vec![0; width * height * 4];
            }
            let base = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
            let row = CVPixelBufferGetBytesPerRow(buffer);
            if !base.is_null() {
                for y in 0..height {
                    let line = std::slice::from_raw_parts(base.add(y * row), width * 4);
                    for pixel in line.as_chunks::<4>().0 {
                        out.extend([pixel[2], pixel[1], pixel[0], pixel[3]]);
                    }
                }
            }
            CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::ReadOnly);
        }
        out.resize(width * height * 4, 0);
        out
    }
}

/// How many of its buffers may be held at once (shown, or on their way):
/// past that a frame gets a buffer of its own, outside the pool, freed as
/// soon as no one holds it (an app holding frames it never reads).
const HELD: i32 = 4;
/// `kCVPixelFormatType_32BGRA`: as the target (`FORMAT`) stores its pixels.
const BGRA: u32 = u32::from_be_bytes(*b"BGRA");

/// The buffers frames are copied into: a pool of at most `HELD` IOSurface
/// pixel buffers of the target's size, and the cache that makes Metal
/// textures of them. Made at the first copy; one renderer's, as `Readback`.
#[derive(Default)]
pub struct SurfaceReadback {
    pool: Option<((u32, u32), CFRetained<CVPixelBufferPool>)>,
    cache: Option<CFRetained<CVMetalTextureCache>>,
    /// Buffers made outside the pool, its `HELD` all held.
    unpooled: usize,
}

/// Attributes of an IOSurface-backed BGRA buffer Metal can use.
fn buffer_attributes(width: u32, height: u32) -> CFRetained<CFDictionary<CFString, CFType>> {
    let (w, h, format) = (
        CFNumber::new_i32(width as i32),
        CFNumber::new_i32(height as i32),
        CFNumber::new_i64(i64::from(BGRA)),
    );
    let surface = CFDictionary::<CFString, CFType>::empty();
    // SAFETY: CoreVideo's own constant keys, read only.
    let keys: [&CFString; 5] = unsafe {
        [
            kCVPixelBufferWidthKey,
            kCVPixelBufferHeightKey,
            kCVPixelBufferPixelFormatTypeKey,
            kCVPixelBufferIOSurfacePropertiesKey,
            kCVPixelBufferMetalCompatibilityKey,
        ]
    };
    let values: [&CFType; 5] = [&w, &h, &format, &surface, CFBoolean::new(true)];
    CFDictionary::from_slices(&keys, &values)
}

impl SurfaceReadback {
    /// Gives back the buffers no one holds (at rest, all but the one shown).
    /// The pool first, then CoreVideo's texture cache: the cache keeps a
    /// texture of each buffer the pool still has, and the buffer with it,
    /// so flushed only before the pool (after each frame) it kept them.
    pub fn release_unused(&self) {
        if let Some((_, pool)) = &self.pool {
            pool.flush(CVPixelBufferPoolFlushFlags::ExcessBuffers);
        }
        self.flush();
    }
    /// How many buffers it made outside its pool, all of the pool's held.
    #[doc(hidden)]
    pub fn unpooled(&self) -> usize {
        self.unpooled
    }
    /// A free buffer of this size: the pool's, or one of its own while the
    /// pool's `HELD` are all held.
    pub(crate) fn buffer(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<CFRetained<CVPixelBuffer>, GpuError> {
        if self
            .pool
            .as_ref()
            .is_none_or(|(size, _)| *size != (width, height))
        {
            let attributes = buffer_attributes(width, height);
            let mut pool = std::ptr::null_mut();
            // SAFETY: Attributes as CoreVideo documents for pixel buffers; the
            // output is a writable local.
            let status = unsafe {
                CVPixelBufferPool::create(
                    None,
                    None,
                    Some(attributes.as_opaque()),
                    NonNull::from(&mut pool),
                )
            };
            let pool = NonNull::new(pool)
                .filter(|_| status == 0)
                .ok_or_else(|| GpuError(format!("CVPixelBufferPoolCreate failed: {status}")))?;
            // SAFETY: A successful Create returns a +1 reference, adopted here.
            self.pool = Some(((width, height), unsafe { CFRetained::from_raw(pool) }));
        }
        let (_, pool) = self.pool.as_ref().expect("made above");
        let held = CFNumber::new_i32(HELD);
        // SAFETY: CoreVideo's own constant key, read only.
        let key: &CFString = unsafe { kCVPixelBufferPoolAllocationThresholdKey };
        let aux = CFDictionary::<CFString, CFNumber>::from_slices(&[key], &[&held]);
        let mut buffer = std::ptr::null_mut();
        // SAFETY: A live pool, its threshold as CoreVideo documents; the
        // output is a writable local.
        let status = unsafe {
            CVPixelBufferPool::create_pixel_buffer_with_aux_attributes(
                None,
                pool,
                Some(aux.as_opaque()),
                NonNull::from(&mut buffer),
            )
        };
        if status == kCVReturnWouldExceedAllocationThreshold {
            self.unpooled += 1;
            return create(width, height);
        }
        let buffer = NonNull::new(buffer)
            .filter(|_| status == 0)
            .ok_or_else(|| GpuError(format!("no pixel buffer from the pool: {status}")))?;
        // SAFETY: A successful Create returns a +1 reference, adopted here.
        Ok(unsafe { CFRetained::from_raw(buffer) })
    }
    /// A buffer holding `rgba` (premultiplied, `width` by `height`): a frame
    /// drawn as bytes (blurred, of several samples) for a window to show.
    pub fn upload(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<PixelBuffer, GpuError> {
        let (w, h) = (width as usize, height as usize);
        if rgba.len() != w * h * 4 {
            return Err(GpuError(
                "pixels of the wrong size for a pixel buffer".into(),
            ));
        }
        let buffer = self.buffer(width, height)?;
        // SAFETY: Locked for writing while its rows are written: `height`
        // rows `bytes per row` apart, each at least `width` BGRA pixels. No
        // one else holds a buffer just made or handed out free.
        unsafe {
            if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags(0)) != 0 {
                return Err(GpuError("pixel buffer didn't lock".into()));
            }
            let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
            let row = CVPixelBufferGetBytesPerRow(&buffer);
            if !base.is_null() {
                for (y, source) in rgba.chunks_exact(w * 4).enumerate() {
                    let line = std::slice::from_raw_parts_mut(base.add(y * row), w * 4);
                    for (to, from) in line
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(source.as_chunks::<4>().0)
                    {
                        *to = [from[2], from[1], from[0], from[3]];
                    }
                }
            }
            CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags(0));
        }
        Ok(PixelBuffer(buffer))
    }
    /// `buffer` as a texture of `device` a copy can write: with the
    /// CoreVideo wrapper, to keep until the copy completes.
    pub(crate) fn texture(
        &mut self,
        device: &wgpu::Device,
        buffer: &CVPixelBuffer,
        (width, height): (u32, u32),
    ) -> Result<(wgpu::Texture, CFRetained<CVMetalTexture>), GpuError> {
        if self.cache.is_none() {
            // SAFETY: The raw device is only handed to CoreVideo, which
            // retains it; nothing destroys it.
            let metal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or_else(|| GpuError("CoreVideo surfaces require a Metal device".into()))?;
            let mut cache = std::ptr::null_mut();
            // SAFETY: No attribute dictionaries; the output is a writable local.
            let status = unsafe {
                CVMetalTextureCache::create(
                    None,
                    None,
                    metal.raw_device(),
                    None,
                    NonNull::from(&mut cache),
                )
            };
            let cache = NonNull::new(cache)
                .filter(|_| status == 0)
                .ok_or_else(|| GpuError(format!("CVMetalTextureCacheCreate failed: {status}")))?;
            // SAFETY: A successful Create returns a +1 reference, adopted here.
            self.cache = Some(unsafe { CFRetained::from_raw(cache) });
        }
        let cache = self.cache.as_ref().expect("made above");
        let mut wrapper = std::ptr::null_mut();
        // SAFETY: A live BGRA pixel buffer of this size, plane 0, no
        // attributes; the output is a writable local.
        let status = unsafe {
            CVMetalTextureCache::create_texture_from_image(
                None,
                cache,
                buffer,
                None,
                MTLPixelFormat::BGRA8Unorm,
                width as usize,
                height as usize,
                0,
                NonNull::from(&mut wrapper),
            )
        };
        let wrapper = NonNull::new(wrapper)
            .filter(|_| status == 0)
            .ok_or_else(|| GpuError(format!("CoreVideo texture failed: {status}")))?;
        // SAFETY: A successful Create returns a +1 reference, adopted here.
        let wrapper = unsafe { CFRetained::from_raw(wrapper) };
        let metal = CVMetalTextureGetTexture(&wrapper)
            .ok_or_else(|| GpuError("CoreVideo texture has no Metal texture".into()))?;
        // SAFETY: CoreVideo made this texture on this device, BGRA8 and this
        // size, one level and layer; the wrapper and buffer outlive the copy.
        let texture = unsafe {
            let raw = wgpu::hal::metal::Device::texture_from_raw(
                metal,
                crate::FORMAT,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent {
                    width,
                    height,
                    depth: 1,
                },
            );
            device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                raw,
                &wgpu::TextureDescriptor {
                    label: Some("frame pixel buffer"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::FORMAT,
                    usage: wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
            )
        };
        Ok((texture, wrapper))
    }
    /// Releases textures CoreVideo keeps for buffers given back.
    pub(crate) fn flush(&self) {
        if let Some(cache) = &self.cache {
            cache.flush(0);
        }
    }
}

/// A black pixel buffer of one pixel, IOSurface-backed: what a surface view
/// shows before its first frame.
pub fn blank_surface() -> Result<PixelBuffer, GpuError> {
    // A new IOSurface's bytes are zero: transparent black.
    create(1, 1).map(PixelBuffer)
}

/// A pixel buffer of its own (no pool's), IOSurface-backed BGRA.
fn create(width: u32, height: u32) -> Result<CFRetained<CVPixelBuffer>, GpuError> {
    let attributes = buffer_attributes(width, height);
    let mut buffer = std::ptr::null_mut();
    // SAFETY: BGRA, attributes as for the pool's; the output is a writable
    // local.
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            width as usize,
            height as usize,
            BGRA,
            Some(attributes.as_opaque()),
            NonNull::from(&mut buffer),
        )
    };
    let buffer = NonNull::new(buffer)
        .filter(|_| status == 0)
        .ok_or_else(|| GpuError(format!("CVPixelBufferCreate failed: {status}")))?;
    // SAFETY: A successful Create returns a +1 reference, adopted here.
    Ok(unsafe { CFRetained::from_raw(buffer) })
}
