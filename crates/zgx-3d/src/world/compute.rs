//! A world's GPU data and compute passes (design-3d-aaa.md, 5.3): the
//! `World` calls of `crate::buffer` and `crate::compute`.
use super::World;
use crate::{
    BufferDesc, Compute, ComputeBudget, ComputeError, ComputeId, ComputeStatus, GpuBuffer,
    GpuDataError, GpuTexture, TextureDesc,
    buffer::{BufferShared, TextureShared},
    compute::{Data, UNSETTLED},
};
use std::{cell::RefCell, rc::Rc};

impl World {
    /// Its GPU data, made with the first call needing it.
    fn data<T>(&self, f: impl FnOnce(&mut Data) -> T) -> T {
        let mut inner = self.inner.borrow_mut();
        let view = self.view.clone();
        let data = inner.data.get_or_insert_with(|| Box::new(Data::new(view)));
        let out = f(data);
        let wants = data.wants_frames();
        inner.unsettle(UNSETTLED, wants);
        out
    }

    /// A buffer on the device for compute passes to bind, and to read back:
    /// its bytes count against the world's cap (`set_memory_cap`) while the
    /// handle (or a pass binding it) lives.
    pub fn add_buffer(&self, desc: BufferDesc) -> Result<GpuBuffer, GpuDataError> {
        let most = self
            .caps()
            .map_or(wgpu_limits().max_buffer_size, |caps| caps.max_buffer);
        if desc.size == 0 || desc.size > most {
            return Err(GpuDataError::Size(desc.size));
        }
        self.data(|data| {
            data.room(desc.size)?;
            let shared = Rc::new(BufferShared {
                id: data.link.id(),
                size: desc.size,
                contents: RefCell::new(desc.contents),
                writes: RefCell::default(),
                gpu: RefCell::default(),
                refused: Default::default(),
                link: data.link.clone(),
            });
            data.add_buffer(&shared);
            Ok(GpuBuffer(shared))
        })
    }

    /// A texture on the device for compute passes to write and read and
    /// materials to sample, zeroed: counted as `add_buffer`'s are.
    pub fn add_texture(&self, desc: TextureDesc) -> Result<GpuTexture, GpuDataError> {
        let caps = self.caps();
        if caps.is_some_and(|caps| !caps.compute || caps.storage_textures == 0) {
            return Err(GpuDataError::Unsupported);
        }
        let most = caps.map_or(wgpu_limits().max_texture_dimension_2d, |caps| {
            caps.max_texture
        });
        let (width, height) = desc.size;
        if width == 0 || height == 0 || width > most || height > most {
            return Err(GpuDataError::Size(desc.bytes()));
        }
        self.data(|data| {
            data.room(desc.bytes())?;
            let shared = Rc::new(TextureShared {
                id: data.link.id(),
                desc,
                gpu: RefCell::default(),
                refused: Default::default(),
                link: data.link.clone(),
            });
            data.add_texture(&shared);
            Ok(GpuTexture(shared))
        })
    }

    /// The most the app's buffers and textures may hold, in bytes (256 MiB
    /// to begin with): `add_buffer` and `add_texture` refuse more. Lowering
    /// it lets go of nothing.
    pub fn set_memory_cap(&self, bytes: u64) {
        self.data(|data| data.cap = bytes);
    }

    /// The most readbacks copy a frame, in bytes (64 KiB to begin with, 64
    /// MiB at most): larger reads take more frames, smaller ones share one.
    ///
    /// Reads queue up to 64 frames of it (4 MiB to begin with): one asked
    /// for past that is refused (`ReadError::Busy`), unless nothing else is
    /// queued. A read whose `Readback` is dropped is let go.
    pub fn set_readback_budget(&self, bytes: u64) {
        self.data(|data| {
            data.budget = bytes.clamp(4, MOST_READBACK);
            data.link.queue_cap.set(data.budget.saturating_mul(64));
        });
    }

    /// A frame's compute passes' GPU times charged as the device's timestamps
    /// would charge them, for tests to judge budgets without timing a GPU.
    #[doc(hidden)]
    pub fn time_compute_frame(&self, times: &[(ComputeId, f32)]) {
        self.data(|data| data.charge_frame(times));
    }

    /// Bytes written to its buffers waiting for a frame. For tests.
    #[doc(hidden)]
    pub fn writes_queued(&self) -> u64 {
        self.inner.borrow().data.as_ref().map_or(0, |data| {
            data.link
                .writes
                .borrow()
                .iter()
                .filter_map(std::rc::Weak::upgrade)
                .map(|b| {
                    b.writes
                        .borrow()
                        .iter()
                        .map(|(_, w)| w.len() as u64)
                        .sum::<u64>()
                })
                .sum()
        })
    }

    /// Reads queued and their bytes. For tests.
    #[doc(hidden)]
    pub fn readbacks_queued(&self) -> (usize, u64) {
        self.inner.borrow().data.as_ref().map_or((0, 0), |data| {
            let reads = data.link.reads.borrow();
            (reads.len(), reads.iter().map(|job| job.bytes).sum())
        })
    }

    /// Add a compute pass, checked now: its WGSL parsed and validated (errors
    /// at the shader's own lines), its bindings against the shader's, its
    /// workgroups against the limits every tier A device has, and its order
    /// against the passes at its point. `Unsupported` where the view's
    /// device (once a frame has been drawn) runs no compute; on a device
    /// found to run none later, its status says so. It's compiled off the
    /// frame and runs from the first frame after.
    pub fn add_compute(&self, compute: Compute) -> Result<ComputeId, ComputeError> {
        if self.caps().is_some_and(|caps| !caps.compute) {
            return Err(ComputeError::Unsupported);
        }
        let timed = self.caps().map(|caps| caps.timestamps);
        let id = self.data(|data| data.add_compute(compute, timed))?;
        self.view.invalidate();
        Ok(id)
    }

    /// How much work its compute passes may give the device: invocations a
    /// frame, and GPU milliseconds a pass where the device has timestamps
    /// (`ComputeBudget`). A pass already over the invocations stays, and
    /// waits for frames with room.
    ///
    /// Times that aren't a positive number are the default's.
    pub fn set_compute_budget(&self, budget: ComputeBudget) {
        let default = ComputeBudget::default();
        let ms = |ms: f32, default: f32| {
            if ms.is_finite() && ms > 0. {
                ms
            } else {
                default
            }
        };
        let budget = ComputeBudget {
            gpu_ms: ms(budget.gpu_ms, default.gpu_ms),
            sandboxed_gpu_ms: ms(budget.sandboxed_gpu_ms, default.sandboxed_gpu_ms),
            ..budget
        };
        self.data(|data| data.limits = budget);
    }

    /// What its compute passes may give the device (`set_compute_budget`).
    pub fn compute_budget(&self) -> ComputeBudget {
        self.inner
            .borrow()
            .data
            .as_ref()
            .map_or_else(ComputeBudget::default, |data| data.limits)
    }

    /// Take a compute pass out: what it bound is let go with it, unless the
    /// app holds it too.
    pub fn remove_compute(&self, id: ComputeId) {
        self.data(|data| data.remove_compute(id));
    }

    /// Enable or disable a compute pass: disabled, it doesn't run and keeps
    /// no frames coming.
    pub fn set_compute_enabled(&self, id: ComputeId, on: bool) {
        if self.data(|data| data.set_enabled(id, on)) && on {
            self.view.invalidate();
        }
    }

    /// Run an on-demand compute pass once more, in the next frame (which this
    /// asks for).
    pub fn run_compute(&self, id: ComputeId) {
        if self.data(|data| data.demand(id)) {
            self.view.invalidate();
        }
    }

    /// Where a compute pass is on the device; `None` if it was removed.
    pub fn compute_status(&self, id: ComputeId) -> Option<ComputeStatus> {
        self.inner
            .borrow()
            .data
            .as_ref()
            .and_then(|data| data.status(id))
    }
}

/// The most a frame's readbacks copy, whatever budget is asked: 64 MiB.
const MOST_READBACK: u64 = 64 << 20;

/// The limits GPU data is made within: WebGPU's defaults, which every tier
/// A device meets.
fn wgpu_limits() -> zgx_gpu::wgpu::Limits {
    zgx_gpu::wgpu::Limits::default()
}
