//! zgx's text shaping and measuring, with no renderer: the fonts, shaped
//! text and prepared lines zgx-gpu draws from, for a host that lays text
//! out but never draws it (a headless service measuring as a window does).
//! zgx-gpu re-exports all of it.
#![warn(missing_docs)]
use cosmic_text::FontSystem;
use std::{cell::RefCell, rc::Rc};
use zgx::scene::Scene;

#[cfg(target_os = "macos")]
mod core_text_glyphs;
#[doc(hidden)]
pub mod font_cache;
pub mod prepared;
pub mod text;
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use core_text_glyphs::core_text_glyph;
pub use font_cache::set_desktop_sans_serif;

/// Shapes using exactly the same font selection and line metrics as the renderer.
pub fn measure_text(
    fonts: &mut FontSystem,
    text: &str,
    size: f32,
    width: Option<f32>,
) -> (f32, f32) {
    text::ShapedText::new(fonts, text, size, width).size()
}

/// Text shaping and measurement without a GPU: what a renderer installs on
/// a scene (`GpuRenderer::install_text`), for laying out what is never drawn
/// (measuring a scene's update, layout and damage work on its own).
pub struct TextServices {
    fonts: Rc<RefCell<FontSystem>>,
    cache: Rc<RefCell<prepared::TextCache>>,
}
impl Default for TextServices {
    fn default() -> Self {
        Self::new()
    }
}
impl TextServices {
    /// The system's fonts (from the font cache) and an empty text cache.
    pub fn new() -> Self {
        Self {
            fonts: Rc::new(RefCell::new(font_cache::system_fonts())),
            cache: Default::default(),
        }
    }
    /// The font system it shapes and measures text with (a renderer's
    /// `text_system`): register bundled fonts here.
    pub fn text_system(&self) -> Rc<RefCell<FontSystem>> {
        self.fonts.clone()
    }
    /// The text it has prepared (a renderer's `text_cache`).
    pub fn text_cache(&self) -> Rc<RefCell<prepared::TextCache>> {
        self.cache.clone()
    }
    /// Shape and measure `scene`'s text with these, as a renderer would.
    /// With no renderer to trim the words it shapes as it draws, the text
    /// cache sweeps them as they grow (`TextCache`); what it holds prepared
    /// stays until [`TextServices::release_unused`] or its budget.
    pub fn install(&self, scene: &mut Scene) {
        install_text(scene, &self.fonts, &self.cache);
    }
    /// Let go of all the text it holds prepared and every word it shaped:
    /// a headless host returns to its baseline at once (after a batch).
    /// Text still shown is shaped again when it is next laid out.
    pub fn release_unused(&self) {
        self.cache.borrow_mut().clear();
        self.fonts.borrow_mut().shape_run_cache.clear();
    }
}

/// `fonts` and `cache` as text services a scene whose host installed none
/// takes (`Scene::set_default_text_services`): a renderer's, so a scene it
/// draws without `install_text` is measured as it is drawn, not by zgx's
/// estimate (a row's label narrower than its glyphs, its last one cut).
/// Held weakly: once they go, scenes keep what they have.
#[doc(hidden)]
pub fn default_text_services(
    fonts: &Rc<RefCell<FontSystem>>,
    cache: &Rc<RefCell<prepared::TextCache>>,
) -> Rc<zgx::scene::DefaultTextServices> {
    let (fonts, cache) = (Rc::downgrade(fonts), Rc::downgrade(cache));
    Rc::new(move |scene: &mut Scene| {
        let (Some(fonts), Some(cache)) = (fonts.upgrade(), cache.upgrade()) else {
            return false;
        };
        install_text(scene, &fonts, &cache);
        true
    })
}
/// Install text services sharing `shared_fonts` and `shared_cache` on `scene`.
#[doc(hidden)]
pub fn install_text(
    scene: &mut Scene,
    shared_fonts: &Rc<RefCell<FontSystem>>,
    shared_cache: &Rc<RefCell<prepared::TextCache>>,
) {
    let fonts = shared_fonts.clone();
    scene.set_text_measurer(move |text: &str, size: f32, max: Option<f32>| {
        let _span = zgx::profile::nested(zgx::profile::Phase::Text);
        crate::measure_text(&mut fonts.borrow_mut(), text, size, max)
    });
    let fonts = shared_fonts.clone();
    scene.set_font_text_shaper(
        move |text: &str,
              size: f32,
              width: Option<f32>,
              font: &zgx::text_layout::FontStyle|
              -> Box<dyn zgx::text_layout::TextLayout> {
            let _span = zgx::profile::nested(zgx::profile::Phase::Text);
            Box::new(crate::text::ShapedText::with_font(
                &mut fonts.borrow_mut(),
                text,
                size,
                width,
                font,
            ))
        },
    );
    let fonts = shared_fonts.clone();
    scene.set_rich_text_shaper(move |rich, width| {
        let _span = zgx::profile::nested(zgx::profile::Phase::Text);
        Box::new(crate::text::ShapedText::with_runs(
            &mut fonts.borrow_mut(),
            rich,
            width,
        ))
    });
    // Prepared text: shaped once, then measured at any width by line
    // breaking alone; the renderer draws from the same lines.
    let (fonts, cache) = (shared_fonts.clone(), shared_cache.clone());
    scene.set_rich_text_measurer(move |rich, width| {
        let _span = zgx::profile::nested(zgx::profile::Phase::Text);
        cache
            .borrow_mut()
            .measure(&mut fonts.borrow_mut(), rich, width)
    });
    // Plain text nodes, from the same prepared text.
    let (fonts, cache) = (shared_fonts.clone(), shared_cache.clone());
    scene.set_font_text_measurer(move |text, size, width, font| {
        let _span = zgx::profile::nested(zgx::profile::Phase::Text);
        cache
            .borrow_mut()
            .measure_plain(&mut fonts.borrow_mut(), text, size, font, width)
    });
    // Wrapping plain text, with the widths its lines break the same at: a
    // window resized a few points serves them without breaking them again.
    let (fonts, cache) = (shared_fonts.clone(), shared_cache.clone());
    scene.set_font_text_reach_measurer(move |text, size, width, font| {
        let _span = zgx::profile::nested(zgx::profile::Phase::Text);
        cache
            .borrow_mut()
            .measure_plain_reach(&mut fonts.borrow_mut(), text, size, font, width)
    });
    // Plain text's min-content size, from the words it was shaped into.
    let (fonts, cache) = (shared_fonts.clone(), shared_cache.clone());
    scene.set_font_min_content_measurer(move |text, size, font| {
        let _span = zgx::profile::nested(zgx::profile::Phase::Text);
        cache
            .borrow_mut()
            .min_content_plain(&mut fonts.borrow_mut(), text, size, font)
    });
    // Rich text's, likewise.
    let (fonts, cache) = (shared_fonts.clone(), shared_cache.clone());
    scene.set_rich_min_content_measurer(move |rich| {
        let _span = zgx::profile::nested(zgx::profile::Phase::Text);
        cache
            .borrow_mut()
            .min_content(&mut fonts.borrow_mut(), rich)
    });
    // Apps measuring what they have not mounted keep no glyphs for it.
    let (fonts, cache) = (shared_fonts.clone(), shared_cache.clone());
    scene.set_detached_rich_text_measurer(move |rich, width| {
        let _span = zgx::profile::nested(zgx::profile::Phase::Text);
        cache
            .borrow_mut()
            .measure_detached(&mut fonts.borrow_mut(), rich, width)
    });
}
