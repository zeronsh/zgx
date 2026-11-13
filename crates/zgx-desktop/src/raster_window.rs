//! A window drawn by the software raster and shown through softbuffer:
//! where the only GPU is a software one (lavapipe, llvmpipe, WARP), which
//! draws a frame in some 1.5 to 9 ms of CPU where the raster takes 0.2 to
//! 0.5, at a third of the memory. 1x windows only: one at a higher scale,
//! or that comes to one, draws on the GPU.
use crate::{presentation::Presentation, raster::Raster};
use std::{error::Error, num::NonZeroU32, sync::Arc};
use winit::window::Window;
use zgx::scene::{Color, Rect, Scene};

/// Which renderer a window draws with (`WindowOptions::renderer`, else
/// `ZGX_RENDERER`): the GPU's (the default), the raster, or the raster
/// where the only adapter is a software one.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RendererChoice {
    #[default]
    Gpu,
    Raster,
    Auto,
}
impl RendererChoice {
    /// `ZGX_RENDERER`: `raster` or `auto`; anything else (or nothing) is
    /// the GPU's.
    pub(crate) fn from_env() -> Self {
        match zgx::env::var_os("ZGX_RENDERER")
            .as_deref()
            .and_then(|value| value.to_str())
        {
            Some("raster") => Self::Raster,
            Some("auto") => Self::Auto,
            _ => Self::Gpu,
        }
    }
}
/// Whether a window can draw on the raster: opaque (the raster's pixels
/// have no alpha) and at 1x.
pub(crate) fn can_draw(transparent: bool, scale: f64) -> bool {
    !transparent && scale == 1.
}
/// Whether an adapter draws on the CPU: by its type, or by the names of
/// the software drivers that report another.
pub(crate) fn software_adapter(name: &str, device_type: zgx_gpu::wgpu::DeviceType) -> bool {
    let name = name.to_lowercase();
    device_type == zgx_gpu::wgpu::DeviceType::Cpu
        || [
            "llvmpipe",
            "lavapipe",
            "softpipe",
            "swiftshader",
            "basic render driver",
        ]
        .iter()
        .any(|driver| name.contains(driver))
}

/// The raster, the text services its scene is measured with, and the
/// window surface its frames are copied to.
pub(crate) struct RasterWindow {
    raster: Raster,
    services: zgx_gpu::TextServices,
    surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
    presentation: Presentation,
    size: (usize, usize),
}
impl RasterWindow {
    /// One for `window`, its text set up with `fonts` and `language`.
    pub(crate) fn new(
        window: &Arc<Window>,
        fonts: &[zgx_gpu::text::FontData],
        language: Option<&str>,
    ) -> Result<Self, Box<dyn Error>> {
        let context = softbuffer::Context::new(window.clone())?;
        let surface = softbuffer::Surface::new(&context, window.clone())?;
        let services = zgx_gpu::TextServices::new();
        for font in fonts {
            font.install(&mut services.text_system().borrow_mut());
        }
        if let Some(language) = language {
            services
                .text_system()
                .borrow_mut()
                .set_language(Some(language));
        }
        let mut raster = Raster::new(1, 1);
        raster.share_text(services.text_system(), services.text_cache());
        let mut made = Self {
            raster,
            services,
            surface,
            presentation: Presentation::default(),
            size: (0, 0),
        };
        let size = window.inner_size();
        made.resize(size.width, size.height);
        Ok(made)
    }
    /// Measure and shape `scene`'s text with the services it draws with.
    pub(crate) fn install_text(&self, scene: &mut Scene) {
        self.services.install(scene);
    }
    /// The app's fonts or text language changed (`WindowFactory::replace_font`,
    /// `set_text_language`): its text shapes and draws again with `fonts`,
    /// as in `language`.
    pub(crate) fn fonts_replaced(
        &mut self,
        fonts: &[zgx_gpu::text::FontData],
        language: Option<&str>,
    ) {
        {
            let system = self.services.text_system();
            let mut system = system.borrow_mut();
            for font in fonts {
                font.replace_in(&mut system);
            }
            system.set_language(language);
        }
        self.services.text_cache().borrow_mut().clear();
        self.raster.fonts_replaced();
    }
    /// The colour its frames clear to.
    pub(crate) fn set_background(&mut self, color: Color) {
        self.raster
            .set_background(u32::from_be_bytes([0, color.0, color.1, color.2]));
    }
    /// The window's new size in pixels: its next frame draws whole. Whether
    /// it changed (a surface that can't take it keeps the old one, said).
    pub(crate) fn resize(&mut self, width: u32, height: u32) -> bool {
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return false;
        };
        if self.size == (width as usize, height as usize) {
            return false;
        }
        if let Err(error) = self.surface.resize(w, h) {
            eprintln!("zgx: the window's surface kept its size ({error})");
            return false;
        }
        self.raster.resize(width as usize, height as usize);
        self.size = (width as usize, height as usize);
        true
    }
    /// Draw `damage` of `scene` (and what scrolled), and show what changed.
    pub(crate) fn render(&mut self, scene: &Scene, damage: &[Rect]) -> Result<(), Box<dyn Error>> {
        if self.size.0 == 0 || self.size.1 == 0 {
            return Ok(());
        }
        self.raster.render(scene, damage);
        let mut buffer = self.surface.buffer_mut()?;
        // A surface buffer of another size (a resize the window system has
        // yet to apply): drawn whole at the next frame.
        if buffer.len() != self.raster.pixels.len() {
            return Ok(());
        }
        let age = buffer.age();
        let shown = self.presentation.copy(
            &self.raster.pixels,
            &mut buffer,
            self.size,
            age,
            self.raster.changed(),
        );
        zgx_gpu::present_log::log(|| format!("raster present rects={}", shown.len()));
        if shown.is_empty() {
            buffer.present()?;
        } else {
            buffer.present_with_damage(&shown)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_software_drivers_are_known_by_type_or_name() {
        use zgx_gpu::wgpu::DeviceType::*;
        assert!(software_adapter("anything", Cpu));
        assert!(software_adapter("llvmpipe (LLVM 19.1.7, 256 bits)", Other));
        assert!(software_adapter("Microsoft Basic Render Driver", Other));
        assert!(!software_adapter("Apple M5 Max", IntegratedGpu));
        assert!(!software_adapter("NVIDIA GeForce RTX 4090", DiscreteGpu));
    }

    #[test]
    fn only_opaque_1x_windows_draw_on_the_raster() {
        assert!(can_draw(false, 1.));
        assert!(!can_draw(true, 1.));
        assert!(!can_draw(false, 2.));
        assert!(!can_draw(false, 1.25));
    }
}
