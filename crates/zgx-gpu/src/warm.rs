//! The pipelines a window's first frames draw with, made on threads of their
//! own while the host builds its first window (`GpuContext::prepare_in_background`),
//! instead of on the main thread as those frames first meet them.
//!
//! A launch's first frames draw through wgpu while their textures upload
//! (its quads, and a backdrop blur's passes where the interface has one),
//! then, on macOS, encode with Metal directly (`metal_frame`), whose own
//! quads and blur are other pipelines again. Each pipeline is a trip to
//! Metal's compiler service: on an M5 Max launching Zeron, the main thread
//! spent 0.3 to 0.9 s making them, between the window and the first frame
//! that took a key. Made here, in parallel with the host's own work (and
//! with each other: the compiler service runs several at once), they are
//! taken ready, or waited for if one is not done yet, which is never longer
//! than making it then.
//!
//! Only a context prepared in the background (a host's launch) on macOS
//! warms: a context made in place (tests, offscreen renderers, the
//! instruction gate's scenes) and other platforms make their pipelines as
//! their frames meet them, as they always have.

use std::cell::RefCell;
use std::thread::JoinHandle;

/// A thing being made on a thread of its own, taken once by whoever needs it.
pub(crate) struct Warming<T>(RefCell<Option<(&'static str, JoinHandle<T>)>>);

impl<T: Send + 'static> Warming<T> {
    /// `make` started on a thread named for `name`; nothing if no thread
    /// could be had (it is then made as it is met, as without warming).
    /// With `ZGX_PROFILE`, a mark as it is made (`profile::mark`).
    pub(crate) fn start(name: &'static str, make: impl FnOnce() -> T + Send + 'static) -> Self {
        let made = move || {
            let made = make();
            zgx::profile::mark(&format!("warm {name} made"));
            made
        };
        Self(RefCell::new(
            std::thread::Builder::new()
                .name(format!("zgx warm {name}"))
                .spawn(made)
                .ok()
                .map(|handle| (name, handle)),
        ))
    }

    /// What was made, waited for if it is still being made; `None` if it was
    /// taken already, never started, or its thread panicked. With
    /// `ZGX_PROFILE`, a frame that waits says so.
    pub(crate) fn take(&self) -> Option<T> {
        let (name, handle) = self.0.borrow_mut().take()?;
        if !handle.is_finished() {
            zgx::profile::mark(&format!("waiting for warm {name}"));
        }
        handle.join().ok()
    }
}

impl<T> Default for Warming<T> {
    fn default() -> Self {
        Self(RefCell::new(None))
    }
}

/// The quad pipelines every wgpu frame draws with (`QuadPipelines`), for
/// the target's format: over and replacing, full and basic.
pub(crate) struct Quads {
    pub(crate) shader: wgpu::ShaderModule,
    pub(crate) full: wgpu::RenderPipeline,
    pub(crate) full_clear: wgpu::RenderPipeline,
    pub(crate) basic: wgpu::RenderPipeline,
    pub(crate) basic_clear: wgpu::RenderPipeline,
}

impl Quads {
    pub(crate) fn make(device: &wgpu::Device, layout: &wgpu::PipelineLayout) -> Self {
        let shader = crate::quad_shader(device);
        let pipeline =
            |blend, basic| crate::quad_pipeline(device, &shader, layout, blend, None, basic);
        Self {
            full: pipeline(Some(crate::OVER), false),
            full_clear: pipeline(None, false),
            basic: pipeline(Some(crate::OVER), true),
            basic_clear: pipeline(None, true),
            shader,
        }
    }
}

/// What a launch's context warms: each started as the device is made.
#[derive(Default)]
pub(crate) struct Warm {
    pub(crate) quads: Warming<Quads>,
    pub(crate) blur: Warming<crate::BlurPipelines>,
    #[cfg(target_os = "macos")]
    pub(crate) metal: Warming<Option<crate::metal_frame::Parts>>,
}

impl Warm {
    /// Start making them, each on a thread of its own.
    pub(crate) fn start(device: &wgpu::Device, quad_layout: &wgpu::PipelineLayout) -> Self {
        let (d, layout) = (device.clone(), quad_layout.clone());
        let quads = Warming::start("quads", move || Quads::make(&d, &layout));
        let d = device.clone();
        let blur = Warming::start("blur", move || crate::BlurPipelines::new(&d));
        #[cfg(target_os = "macos")]
        let metal = {
            let d = device.clone();
            Warming::start("metal", move || {
                crate::metal_frame::Parts::make_with_blur(&d).ok()
            })
        };
        Self {
            quads,
            blur,
            #[cfg(target_os = "macos")]
            metal,
        }
    }
}
