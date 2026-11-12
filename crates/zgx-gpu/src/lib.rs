//! Damage-scissored retained GPU renderer for Vulkan (Linux) and Metal (macOS).
//! The output is premultiplied BGRA8 (read back as RGBA8). Text uses cosmic-text shaping and a bounded
//! GPU glyph atlas; stable text never reshapes on paint or compositor updates.
#![warn(missing_docs)]
use cosmic_text::{Buffer, CacheKey, FontSystem, Metrics, SwashCache, SwashContent};
use rustc_hash::{FxHashMap, FxHashSet};
use std::{cell::RefCell, fmt, rc::Rc, sync::Arc};
use zgx::scene::{Color, NodeId, NodeKind, Rect, Scene};
// Text shaping and measuring, in a crate of their own (zgx-text) that a host
// with no renderer links alone; here as they always were.
pub use zgx_text::{TextServices, measure_text, prepared, text};
use zgx_text::{default_text_services, font_cache, install_text};

mod bands;
mod compile;
#[cfg(all(windows, feature = "composition"))]
mod composition;
#[cfg(d3d11)]
mod d3d11_frame;
#[cfg(windows)]
mod dx12_fill;
#[cfg(windows)]
mod dxgi;
#[cfg(windows)]
mod dxgi_present;
mod effects;
#[cfg(windows)]
mod fxc_child;
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub mod iosurface;
mod passes;
#[cfg(d3d11)]
mod view_host;
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use zgx_text::core_text_glyph;
#[cfg(target_os = "macos")]
mod layer_present;
#[cfg(target_os = "macos")]
mod loops;
#[cfg(target_os = "macos")]
mod plane_drawables;
/// A caret Core Animation draws over the frames (`GpuRenderer::set_caret`).
#[cfg(target_os = "macos")]
pub use layer_present::Caret;
#[cfg(target_os = "macos")]
mod metal_frame;
#[cfg(target_os = "macos")]
mod native_surface;
#[cfg(target_os = "macos")]
mod surfaces;
#[cfg(target_os = "macos")]
pub use surfaces::{PixelBuffer, SurfaceReadback, blank_surface};
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod phase_watch;
mod planes;
pub use planes::plane_size;
pub mod present_log;
mod surface;
#[cfg(target_os = "macos")]
mod trial_energy;
mod upload;
pub use passes::check as check_layer_inputs;
pub mod viewport;
pub use compile::warm_fill_shader;
pub use surface::PresentationStatus;
/// The wgpu this renderer draws with, for GPU views (`viewport`): an app
/// drawing one uses it, not a wgpu of its own, whose types would not be
/// these. A new major version of wgpu is a new major version of zgx-gpu.
pub use wgpu;
pub use zgx_text::set_desktop_sans_serif;

/// The corner radius of find's marks behind text.
const MARK_RADIUS: f32 = 2.;
const MAX_ATLAS: u32 = 2048;
const INITIAL_ATLAS: u32 = 512;
/// An atlas that fills again within this many frames of its last recycle
/// doubles instead (see `render_flat`).
const ATLAS_THRASH_FRAMES: u64 = 600;
/// How long after its last resize a macOS layer keeps a third drawable.
#[cfg(target_os = "macos")]
const WIDE_DRAWABLES: std::time::Duration = std::time::Duration::from_secs(2);
/// SVG rasters up to this size (physical pixels) share the glyph atlas.
const ATLAS_IMAGE_MAX: u32 = 128;
/// Write straight-alpha `pixels` premultiplied (as `upload_image` does) into
/// the interior of a bordered atlas cell `width` + 2 texels wide.
fn premultiply_into_cell(cell: &mut [u8], pixels: &[u8], width: u32) {
    let row = width as usize * 4;
    for (y, source) in pixels.chunks_exact(row).enumerate() {
        let start = ((y + 1) * (width as usize + 2) + 1) * 4;
        for (texel, pixel) in cell[start..start + row]
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(source.as_chunks::<4>().0)
        {
            let alpha = u16::from(pixel[3]);
            for channel in 0..3 {
                texel[channel] = ((u16::from(pixel[channel]) * alpha + 127) / 255) as u8;
            }
            texel[3] = pixel[3];
        }
    }
}
/// The retained target's format: BGRA, as window surfaces are on macOS and
/// X11, so presenting can be a plain copy (see `copy_present`). Readback
/// converts to RGBA.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;
/// The backdrop blur atlas's format (see `atlas_texture`).
const BLUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Why rendering failed, in words.
#[derive(Debug)]
pub struct GpuError(pub String);
impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for GpuError {}
/// What one frame's rendering did, for tests and diagnostics.
#[derive(Default, Debug, Clone, Copy)]
pub struct GpuStats {
    /// Draw calls encoded.
    pub draw_calls: usize,
    /// Encoded render passes, including repainted isolated layers; excludes presentation.
    pub render_passes: usize,
    /// Backdrop filter passes (three per batch of filters blurred together), included in render_passes.
    pub blur_passes: usize,
    /// Quad instances drawn.
    pub instances: usize,
    /// Text nodes shaped (not reused from the cache).
    pub shaped_nodes: usize,
    /// Glyphs uploaded to the atlas.
    pub glyph_uploads: usize,
    /// Images uploaded to textures.
    pub image_uploads: usize,
    /// Native GPU imports/conversions, never CPU image uploads.
    pub native_surface_imports: usize,
    /// Native surfaces converted to a drawable format.
    pub native_surface_conversions: usize,
    /// Canvases and decorations rasterized.
    pub canvas_rasterizations: usize,
    /// Quads canvases drew instead of a raster (`canvas_quads`).
    pub canvas_quads: usize,
    /// SVGs rasterized.
    pub svg_rasterizations: usize,
    /// Times the instance geometry was rebuilt.
    pub geometry_rebuilds: usize,
    /// Paint items patched into the window as kept, in a frame that drew
    /// the rest as they were (see `render_kept`); 0 for a frame drawn from
    /// the scene's walk.
    pub patched_items: usize,
    /// Vertex buffers allocated or grown.
    pub vertex_buffer_allocations: usize,
    /// Physical pixels repainted.
    pub damaged_pixels: u64,
    /// Cached layers repainted.
    pub layer_repaints: usize,
    /// Copies of the target taken for blended and backdrop-reading layers.
    pub backdrop_copies: usize,
    /// Of those, the ones repainted only where the window's damage reaches
    /// them (`prepare_layer`'s `shown`).
    pub layer_partial_repaints: usize,
    /// Cached layers composited without repainting.
    pub layer_cache_hits: usize,
    /// Layer textures allocated.
    pub layer_texture_allocations: usize,
    /// Scrolled regions shifted by copying instead of redrawn.
    pub scroll_copies: usize,
    /// Layer effect passes (content blurs and masks drawn anew; at most two
    /// a frame however many), included in render_passes.
    pub effect_passes: usize,
    /// Damage regions the window's frame drew, once nearby damage is joined
    /// (`merge_damage`): each drawn over by every batch it touches.
    pub regions: usize,
}
/// Retained allocation counters for diagnostics; excludes driver/font database allocations.
#[derive(Debug, Clone, Copy)]
pub struct DebugCacheStats {
    /// Text nodes whose shaping is cached.
    pub shaped_nodes: usize,
    /// Bytes of cached shaping.
    pub shaped_bytes: usize,
    /// Glyph quads cached with it.
    pub cached_quads: usize,
    /// Glyphs in the atlas.
    pub atlas_entries: usize,
    /// The atlas texture's bytes.
    pub atlas_bytes: usize,
    /// Glyph images in the rasterizer's cache.
    pub swash_images: usize,
    /// Glyph outlines in the rasterizer's cache.
    pub swash_outlines: usize,
    /// Image textures.
    pub image_textures: usize,
    /// Their bytes.
    pub image_bytes: usize,
    /// Image textures held softer than drawn, for the budget.
    pub images_softer: usize,
    /// Bytes of rasterized canvases and decorations.
    pub canvas_raster_bytes: usize,
    /// Bytes of rasterized SVGs.
    pub svg_raster_bytes: usize,
    /// The vertex buffer's bytes (all of the ring's, `vertex_buffers`).
    pub vertex_buffer_bytes: usize,
    /// Vertex buffers held: one with a staging copy; up to four with
    /// vertices written in place, a ring of buffers in flight.
    pub vertex_buffers: usize,
    /// Cached layer textures.
    pub layer_textures: usize,
    /// Layers that went coarser for their budget, and are still layers.
    pub layers_coarser: usize,
    /// Their bytes.
    pub layer_bytes: usize,
    /// Pipelines of an app's own shaders (fills, layer shaders and their
    /// blends) held.
    pub shader_pipelines: usize,
    /// Frames after which the native surface cache polled and flushed, to
    /// let go of surfaces no longer shown (`SurfaceCache::collect`): only in
    /// the second and a half after one was let go (macOS).
    pub surface_collections: u64,
}
const SHAPE_BUDGET: usize = 8 * 1024 * 1024;
/// Shaping frames a cached word survives without being used.
const SHAPE_RUN_AGES: u64 = 2;
const VERTEX_BUDGET: usize = 32 * 1024 * 1024;
/// Image textures a renderer holds at once, each with its own bind group (a
/// view in Direct3D 12's fixed-size heap; see `GpuContext::create`).
const MAX_IMAGES: usize = 16_384;
/// The bytes images' own textures may hold (`upload_image`).
const IMAGE_TEXTURES: usize = 64 * 1024 * 1024;
/// What textures of images no node shows may hold until the window goes idle:
/// a few windows' worth of pictures to scroll back to.
const IMAGES_GONE_BYTES: usize = 32 * 1024 * 1024;
const IMAGES_GONE_COUNT: usize = 256;
/// An image's texture: its bind group, bytes, texture and the size it
/// holds (`fitted_size`; (0, 0): all of it).
type ImageTexture = (wgpu::BindGroup, usize, Option<wgpu::Texture>, (u32, u32));
/// A kept file's pixels aren't decoded yet (`ImageData::pixels_at`).
const IMAGE_PENDING: &str = "an image's pixels are decoding";
/// The size in `images` of a texture on its way out (`images_outgoing`): smaller
/// than anything asked for, so it never fits.
const IMAGE_OUTGOING: (u32, u32) = (1, 1);
/// The longest side a texture is cut to past the budget (`upload_image_at`):
/// however full it is, a picture shows, and this little goes over it.
const IMAGE_LEAST_SIDE: u32 = 64;
/// `size` by `scale`, its shape kept, at most `image`'s own size.
fn scaled_size(size: (u32, u32), image: &zgx::image::ImageData, scale: f32) -> (u32, u32) {
    let side = |v: u32, full: u32| ((v as f32 * scale).ceil() as u32).clamp(1, full.max(1));
    (side(size.0, image.width()), side(size.1, image.height()))
}
use zgx::image::{
    fitted_size, premultiply,
    resample::{halve, shrink},
};
/// Why an image can have no texture: a frame goes on without it.
const IMAGE_MISSES: [&str; 2] = [
    "visible images exceed the budget of 16,384",
    "image exceeds GPU texture dimensions",
];
const GLYPH_TOO_LARGE: &str = "glyph font size exceeds atlas raster limit: not drawn";
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Quad {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    fade: [f32; 4],
    options: [f32; 4],
    shape: [f32; 4],
    border: [f32; 4],
    /// `x`: the index plus one of the clip region the quad lies in (its
    /// ancestors' rounded clips and edge fades, see `Region`), 0 for none.
    mask: [f32; 4],
}
const NO_MASK: [f32; 4] = [0.; 4];
/// A clip region quads share (`Region` in draw.wgsl): up to two rounded
/// boxes (x, y, width, height; a negative width for none) with their
/// corner radii, and a box (x0, y0, x1, y1) fading over bands at its edges
/// (top, right, bottom, left). In the target's logical units.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Region {
    round0: [f32; 4],
    radii0: [f32; 4],
    round1: [f32; 4],
    radii1: [f32; 4],
    fade_box: [f32; 4],
    fade: [f32; 4],
    /// Colour filters: an affine map of straight RGB, rows R, G, B, on
    /// where `tint[0]` is 1. `tint[1]` and `tint[2]`: the rounded boxes'
    /// feathers (`RoundRect::feather`), in lanes the record has anyway.
    color: [[f32; 4]; 3],
    tint: [f32; 4],
}
impl Region {
    /// A shader fill's uniforms as a record of the region table (`time`
    /// first, its `params` in the four lanes after; see `fill_source`).
    fn uniforms(uniforms: zgx::decoration::ShaderUniforms) -> Self {
        let [a, b, c, d] = uniforms.params;
        Self {
            round0: [uniforms.time, 0., 0., 0.],
            radii0: a,
            round1: b,
            radii1: c,
            fade_box: d,
            fade: [0.; 4],
            color: [[0.; 4]; 3],
            tint: [0.; 4],
        }
    }
    /// A layer shader's: its uniforms, and in lanes a fill's doesn't read,
    /// the box's corner `radii`, how far it paints and reads past the box
    /// (`LayerShader::bleed`), where the layer's pixels lie from the box's
    /// top-left, what it can read of the backdrop from there, and its
    /// texture's `frame` (`layer_frame`).
    fn layer_uniforms(
        layer: &zgx::scene::LayerShader,
        radii: [f32; 4],
        painted: [f32; 4],
        readable: [f32; 4],
        frame: [f32; 4],
    ) -> Self {
        Self {
            fade: radii,
            color: [painted, readable, frame],
            tint: [layer.bleed, 0., 0., 0.],
            ..Self::uniforms(layer.uniforms)
        }
    }
    /// Whether it leaves every pixel of `bounds` as it is (`interior`).
    fn covers(&self, bounds: Rect, scale: f32) -> bool {
        Interior::of(self, scale).holds(bounds)
    }
    /// `item`'s, if it lies in one.
    fn of(item: &zgx::scene::PaintItem<'_>) -> Option<Self> {
        if item.mask().is_none()
            && item.rounded().iter().all(Option::is_none)
            && item.color().is_none()
        {
            return None;
        }
        let round = |clip: Option<zgx::scene::RoundRect>| match clip {
            Some(clip) => (
                [clip.rect.x, clip.rect.y, clip.rect.width, clip.rect.height],
                clip.radii,
            ),
            None => ([0., 0., -1., -1.], [0.; 4]),
        };
        let feather =
            |clip: Option<zgx::scene::RoundRect>| clip.map_or(0., |clip| clip.feather.points());
        let ((round0, radii0), (round1, radii1)) =
            (round(item.rounded()[0]), round(item.rounded()[1]));
        let (fade_box, fade) = item.mask().map_or(([0.; 4], [0.; 4]), |(rect, bands)| {
            (
                [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height],
                bands,
            )
        });
        Some(Self {
            round0,
            radii0,
            round1,
            radii1,
            fade_box,
            fade,
            color: item.color().unwrap_or_default(),
            tint: [
                if item.color().is_some() { 1. } else { 0. },
                feather(item.rounded()[0]),
                feather(item.rounded()[1]),
                0.,
            ],
        })
    }
}
/// The index (plus one) `region` has in `regions`, added if new; 0 for none.
/// Neighbouring items mostly share one (a list in a rounded card).
/// Two bands of a region where it leaves every pixel as it is, as edges
/// (left, top, right, bottom): with no colour filter, every pixel centre
/// fully inside each rounded box, and inside every fade band (draw.wgsl's
/// `region` is exactly 1 there). Clear of its corner squares a box's
/// distance is its nearest edge's, and coverage is 1 half a device pixel
/// inside it: at every pixel centre within the device pixels the box spans
/// whole, even an edge it shares. So the bands are the box across, between
/// its corners' squares, and the box down; a feathered box (or one whose
/// corners meet) keeps only the part inside its largest corner and feather.
#[derive(Clone, Copy)]
struct Interior([[f32; 4]; 2]);
impl Interior {
    const NONE: Self = Self([[f32::MAX, f32::MAX, f32::MIN, f32::MIN]; 2]);
    fn of(region: &Region, scale: f32) -> Self {
        // (Slack past the edges, so rounding never reads a band's last step.)
        const SLACK: f32 = 1e-3;
        // A colour filter changes every pixel; an outer fade (`tint[3]`, a
        // nested fade's, rare) is left to the shader everywhere.
        if region.tint[0] != 0. || region.tint[3] != 0. {
            return Self::NONE;
        }
        let all = [f32::MIN, f32::MIN, f32::MAX, f32::MAX];
        let rounded = |round: [f32; 4], radii: [f32; 4], feather: f32| {
            let [x0, y0, width, height] = round;
            if width < 0. {
                return [all; 2];
            }
            let (x1, y1) = (x0 + width, y0 + height);
            let corner = radii.into_iter().fold(0f32, f32::max);
            if feather > 0. || corner > width.min(height) * 0.5 {
                let margin = corner + (0.5 / scale).max(feather) + SLACK;
                return [[x0 + margin, y0 + margin, x1 - margin, y1 - margin]; 2];
            }
            let [top_left, top_right, bottom_right, bottom_left] = radii;
            let whole = [
                (x0 * scale).ceil() / scale,
                (y0 * scale).ceil() / scale,
                (x1 * scale).floor() / scale,
                (y1 * scale).floor() / scale,
            ];
            [
                [
                    whole[0],
                    whole[1].max(y0 + top_left.max(top_right)),
                    whole[2],
                    whole[3].min(y1 - bottom_left.max(bottom_right)),
                ],
                [
                    whole[0].max(x0 + top_left.max(bottom_left)),
                    whole[1],
                    whole[2].min(x1 - top_right.max(bottom_right)),
                    whole[3],
                ],
            ]
        };
        let meet = |a: [f32; 4], b: [f32; 4]| {
            [
                a[0].max(b[0]),
                a[1].max(b[1]),
                a[2].min(b[2]),
                a[3].min(b[3]),
            ]
        };
        let [top, right, bottom, left] = region.fade;
        let [x0, y0, x1, y1] = region.fade_box;
        let faded = [
            if left > 0. {
                x0 + left + SLACK
            } else {
                f32::MIN
            },
            if top > 0. { y0 + top + SLACK } else { f32::MIN },
            if right > 0. {
                x1 - right - SLACK
            } else {
                f32::MAX
            },
            if bottom > 0. {
                y1 - bottom - SLACK
            } else {
                f32::MAX
            },
        ];
        let first = rounded(region.round0, region.radii0, region.tint[1]);
        let second = rounded(region.round1, region.radii1, region.tint[2]);
        Self([0, 1].map(|band| meet(meet(first[band], second[band]), faded)))
    }
    #[inline]
    fn holds(&self, bounds: Rect) -> bool {
        let (x1, y1) = (bounds.x + bounds.width, bounds.y + bounds.height);
        self.0.iter().any(|band| {
            bounds.x >= band[0] && bounds.y >= band[1] && x1 <= band[2] && y1 <= band[3]
        })
    }
}
/// Marks `quads` as drawn in clip region `region` (`mask.x`), or without it
/// where it changes none of their pixels (its index in `mask.w`): not in
/// perspective, which `quad_rect` bounds only in part, nor blurred.
#[inline(always)]
fn mark_region(
    interiors: &mut Vec<Option<Interior>>,
    regions: &[Region],
    region: f32,
    quads: &mut [Quad],
    bounds: Rect,
    blurred: bool,
    scale: f32,
) {
    let at = region as usize - 1;
    if interiors.len() <= at {
        interiors.resize(at + 1, None);
    }
    let interior = *interiors[at].get_or_insert_with(|| Interior::of(&regions[at], scale));
    let flat = |quad: Option<&Quad>| quad.is_some_and(|quad| quad.options[2] < 1.5);
    if blurred || !flat(quads.first()) || !flat(quads.last()) {
        for quad in quads {
            quad.mask[0] = region;
        }
    } else if interior.holds(bounds) {
        for quad in quads {
            quad.mask[3] = region;
        }
    } else {
        // Partly inside (a row whose border runs past its card): each quad
        // by its own rectangle.
        for quad in quads {
            let lane = if interior.holds(quad_rect(quad)) {
                3
            } else {
                0
            };
            quad.mask[lane] = region;
        }
    }
}
/// `item`'s region's index (`region_index`). Under two fades (a faded label
/// in a faded scroll) the outer one is a record of its own, which the
/// item's names in `tint[3]` (draw.wgsl's `region` multiplies it in).
#[inline(always)]
fn item_region(regions: &mut Vec<Region>, item: &zgx::scene::PaintItem<'_>) -> f32 {
    let mut region = Region::of(item);
    if let Some(region) = &mut region
        && let Some(outer) = item.outer_mask()
    {
        region.tint[3] = outer_fade(regions, outer);
    }
    region_index(regions, region)
}
/// An outer fade's record (`item_region`), out of line: nested fades are rare.
#[cold]
#[inline(never)]
fn outer_fade(regions: &mut Vec<Region>, (rect, bands): zgx::scene::FadeMask) -> f32 {
    let none = [0., 0., -1., -1.];
    region_index(
        regions,
        Some(Region {
            round0: none,
            radii0: [0.; 4],
            round1: none,
            radii1: [0.; 4],
            fade_box: [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height],
            fade: bands,
            color: [[0.; 4]; 3],
            tint: [0.; 4],
        }),
    )
}
fn region_index(regions: &mut Vec<Region>, region: Option<Region>) -> f32 {
    let Some(region) = region else {
        return 0.;
    };
    // (By bits, which compare as whole words: equal bits shade alike.)
    let bits = |region: &Region| *bytemuck::cast_ref::<Region, [u32; 40]>(region);
    let wanted = bits(&region);
    if let Some(at) = regions.iter().rev().take(8).position(|r| bits(r) == wanted) {
        return (regions.len() - at) as f32;
    }
    regions.push(region);
    regions.len() as f32
}
/// How a quad is drawn, where drawing is split (see `push_draw`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shading {
    /// Corners, borders, shadows, fades, masks or transforms: `shade`.
    Full,
    /// Colour, or colour times a texture sample: `basic`.
    Basic,
    /// An opaque flat colour: `basic` without blending, which reads every
    /// pixel it writes. Same pixels.
    Opaque,
    /// A shader fill (`FillShader`, by id): its pipeline, `shade` for
    /// whatever else the draw holds.
    Fill(u64),
    /// A cached layer composited through its layer shader
    /// (`zgx::scene::LayerShader`, by shader id): its pipeline, `shade` for
    /// the rest. Never batched with other shadings.
    Filter(u64),
    /// A cached layer blended with the backdrop (`zgx::scene::BlendMode`,
    /// its mode in `mask.z`): blend.wgsl's pipeline, reading a copy of the
    /// target under it, and replacing the target's pixels. Drawn alone, at
    /// the start of a pass (see `render_flat`).
    Blend,
    /// `Blend` through a layer shader (by shader id).
    FilterBlend(u64),
    /// An image drawn with an effect (`ImageEffect`): image_effects.wgsl's
    /// pipeline, compiled when one first draws, `shade` for whatever else
    /// the draw holds.
    ImageEffect,
    /// A canvas's polyline strokes (polyline.wgsl): their own pipeline,
    /// reading their points, never joined with another shading's quads.
    Polyline,
    /// `shade`, keeping what's drawn where it's drawn over and nothing
    /// else (source-in): a gradient text's gradient over its glyphs, in
    /// its layer (`gradient_text_quads`).
    SourceIn,
    /// `shade` behind what's drawn (destination-over): a gradient text's
    /// box, marks, selection and shadows under its filled glyphs.
    Behind,
}
fn shading(quad: &Quad, scale: f32) -> Shading {
    // A panel with square corners and no border only antialiases its edges:
    // on whole device pixels, coverage is exactly what rasterizing gives.
    let whole = |value: f32| {
        let device = value * scale;
        (device - device.round()).abs() < 1e-3
    };
    let [x, y, width, height] = quad.rect;
    let square = quad.shape[3] > 0.5
        && quad.shape[0] <= 0.
        && quad.shape[1] <= 0.
        && quad.options[3] < 0.5
        && quad.rect == quad.fade
        && [x, y, x + width, y + height].into_iter().all(whole);
    // The basic shader computes what the full one does in the same order,
    // bit for bit, except for the coverage of a translucent square panel (an
    // extra divide and multiply): those stay full, so either shader draws the
    // same bytes.
    // (Drawn without the region it lies in, `mask.w`: full still, batched
    // as it was. Tested first, as a region's quads are.)
    let plain = quad.options[0] == 0.
        && quad.options[2] < 0.5
        && quad.shape[2] <= 0.
        && (quad.shape[3] <= 0.5 || (square && quad.color[3] >= 1.))
        && quad.mask[0] <= 0.
        && quad.mask[3] <= 0.;
    if !plain {
        Shading::Full
    } else if quad.options[1] <= 0.5 && quad.color[3] >= 1. {
        Shading::Opaque
    } else {
        Shading::Basic
    }
}
/// The quad pipelines, each compiled on first use: a software rasterizer
/// compiles a pipeline into megabytes of code, and plain interfaces never
/// need the full shader there.
struct QuadPipelines {
    device: wgpu::Device,
    /// Their module (draw.wgsl), made with the first pipeline: a window
    /// whose frames Metal encodes directly (metal_frame.rs) needs none.
    shader: std::cell::OnceCell<wgpu::ShaderModule>,
    layout: wgpu::PipelineLayout,
    full: std::cell::OnceCell<wgpu::RenderPipeline>,
    full_clear: std::cell::OnceCell<wgpu::RenderPipeline>,
    basic: std::cell::OnceCell<wgpu::RenderPipeline>,
    basic_clear: std::cell::OnceCell<wgpu::RenderPipeline>,
    /// `Shading::SourceIn`'s and `Shading::Behind`'s: gradient text's.
    source_in: std::cell::OnceCell<wgpu::RenderPipeline>,
    behind: std::cell::OnceCell<wgpu::RenderPipeline>,
    /// The quad pipelines' group layouts (texture, viewport), for the
    /// polyline layout.
    texture_layout: wgpu::BindGroupLayout,
    viewport_layout: wgpu::BindGroupLayout,
    polylines: std::cell::OnceCell<Polylines>,
    /// Whether canvases' filled areas draw as quads (`areas_on_gpu`): their
    /// shader is in the polylines' module then.
    areas: bool,
}
/// The polyline strokes' module (draw.wgsl and polyline.wgsl), its layout
/// (the quads' groups, and their points as group 2) and its pipeline into
/// the retained target: made when a frame first draws one.
struct Polylines {
    module: wgpu::ShaderModule,
    points_layout: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    pipeline: wgpu::RenderPipeline,
}
impl QuadPipelines {
    /// The pipeline drawing quads of `shading`.
    fn get(&self, shading: Shading) -> &wgpu::RenderPipeline {
        let (cell, basic, blend) = match shading {
            Shading::Full
            | Shading::Fill(_)
            | Shading::Filter(_)
            | Shading::Blend
            | Shading::FilterBlend(_)
            | Shading::ImageEffect => (&self.full, false, Some(OVER)),
            Shading::Basic => (&self.basic, true, Some(OVER)),
            Shading::Opaque => (&self.basic_clear, true, None),
            Shading::Polyline => return &self.polylines().pipeline,
            Shading::SourceIn => (&self.source_in, false, Some(SOURCE_IN)),
            Shading::Behind => (&self.behind, false, Some(BEHIND)),
        };
        cell.get_or_init(|| {
            quad_pipeline(
                &self.device,
                self.shader(),
                &self.layout,
                blend,
                None,
                basic,
            )
        })
    }
    fn shader(&self) -> &wgpu::ShaderModule {
        self.shader.get_or_init(|| {
            self.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("quads"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("draw.wgsl").into()),
                })
        })
    }
    fn polylines(&self) -> &Polylines {
        self.polylines.get_or_init(|| {
            let device = &self.device;
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("polylines"),
                source: wgpu::ShaderSource::Wgsl(polyline_source(self.areas).into()),
            });
            let points_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("polyline points"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("polylines"),
                bind_group_layouts: &[
                    Some(&self.texture_layout),
                    Some(&self.viewport_layout),
                    Some(&points_layout),
                ],
                immediate_size: 0,
            });
            let pipeline = polyline_pipeline(device, &module, &layout, None, false);
            Polylines {
                module,
                points_layout,
                layout,
                pipeline,
            }
        })
    }
    /// The pipeline clearing a region to the background quad.
    fn clear(&self, basic: bool) -> &wgpu::RenderPipeline {
        if basic {
            self.get(Shading::Opaque)
        } else {
            self.full_clear.get_or_init(|| {
                quad_pipeline(&self.device, self.shader(), &self.layout, None, None, false)
            })
        }
    }
}
/// Damage regions merge when their union adds at most half the smaller's
/// area and this many device pixels (see `merge_damage`): each region costs
/// its draws again, worth more GPU time than shading a 128×128 square.
const NEAR_PIXELS: f32 = 16_384.;
/// Damaged regions of at least this many device pixels draw opaque quads
/// without blending (see `encode_draws`).
const SPLIT_PIXELS: f32 = 32_768.;
/// Split a draw's quads into runs of one shading, in paint order, when
/// `split`. The full shader's size (the registers it holds) slows every pixel
/// it shades, on a GPU as on a software rasterizer: plain quads (backgrounds,
/// glyphs, bars) draw with the basic one.
/// A canvas's polyline strokes (when `polylines`) draw with their
/// own pipeline: their runs are draws of their own however the rest is.
#[inline(always)]
fn push_draw(
    draws: &mut Vec<Draw>,
    quads: &[Quad],
    draw: Draw,
    split: bool,
    polylines: bool,
    scale: f32,
    glyphs: Option<Shading>,
) {
    let split = split
        && draw.blur.is_none()
        && !matches!(draw.source, TextureSource::Layer(_))
        && !matches!(
            draw.shading,
            Shading::Fill(_)
                | Shading::Filter(_)
                | Shading::Blend
                | Shading::FilterBlend(_)
                | Shading::ImageEffect
        );
    if !(split || polylines) || draw.end <= draw.start {
        draws.push(draw);
        return;
    }
    // Quads of one shading, known already (a text's glyphs): one run.
    if let Some(shading) = glyphs.filter(|_| split && !polylines) {
        draws.push(Draw { shading, ..draw });
        return;
    }
    // A polyline stroke's quad holds its point count in `mask.z`.
    let kind_of = |quad: &Quad| match polylines && quad.mask[2] > 0. {
        true => Shading::Polyline,
        false if split => shading(quad, scale),
        false => draw.shading,
    };
    // Each run ends where the shading changes, so runs alternate shadings.
    let mut start = draw.start;
    while start < draw.end {
        let kind = kind_of(&quads[start as usize]);
        let mut end = start + 1;
        while end < draw.end && kind_of(&quads[end as usize]) == kind {
            end += 1;
        }
        // One run (the usual item): the draw's own bounds.
        let bounds = match (start, end) == (draw.start, draw.end) {
            true => draw.bounds,
            false => quad_bounds(&quads[start as usize..end as usize]),
        };
        draws.push(Draw {
            start,
            end,
            bounds,
            shading: kind,
            // A backdrop filter goes under the first run alone.
            blur: draw.blur.clone().filter(|_| start == draw.start),
            ..draw.clone()
        });
        start = end;
    }
}
fn texel_at(texture: &wgpu::Texture, x: u32, y: u32) -> wgpu::TexelCopyTextureInfo<'_> {
    wgpu::TexelCopyTextureInfo {
        texture,
        mip_level: 0,
        origin: wgpu::Origin3d { x, y, z: 0 },
        aspect: wgpu::TextureAspect::All,
    }
}
/// What keeps a scroll move from being a copy of the pixels drawn
/// (`copy_blocker`), for devtools' note on the frame (`zgx inspect frames`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum CopyBlocker {
    /// Nothing of the content paints in the clip.
    NoContent,
    /// Something not of the content paints between its items.
    Between(NodeId),
    /// A content item reads what lies under it or the window (a backdrop blur,
    /// a blend, a shader reading either), or its shader's inputs run on as
    /// frames draw.
    Reads(NodeId),
    /// A dithered picture: its dots lie on the screen's grid.
    Dots(NodeId),
    /// A layer under a fade whose bands can't draw as one with it
    /// (`bands::fades_apart`).
    FilteredUnderFade(NodeId),
    /// The view lies under two fades around it.
    TwoFades,
    /// A content item fades with this mask (`fade_edges`): the move copies
    /// within its interior, if the mask holds still (`scroll_copies`).
    Fade(NodeId, zgx::scene::FadeMask),
    /// A node paints over the content and does not repaint this frame.
    Over(NodeId),
    /// What lies under the content shows through and does not repaint.
    Under(NodeId),
}
impl CopyBlocker {
    /// The note, its nodes as `zgx inspect tree` numbers them.
    fn describe(self, scene: &Scene) -> String {
        let node =
            |id: NodeId| format!("@{} ({})", id.number(), zgx::inspect::kind_name(scene, id));
        match self {
            Self::NoContent => "none of its content paints in its view".into(),
            Self::Between(id) => format!("{} paints between its content's items", node(id)),
            Self::Reads(id) => format!(
                "{} in it reads what is under it or the window, or runs passes each frame",
                node(id)
            ),
            Self::Dots(id) => format!(
                "{} in it is a dithered picture, its dots on the screen's grid",
                node(id)
            ),
            Self::FilteredUnderFade(id) => format!(
                "{} in it is a blurred, masked or shaded layer under a fade",
                node(id)
            ),
            Self::TwoFades => "it lies under two fades around it".into(),
            Self::Fade(id, _) => format!(
                "{} in it fades with bands cut short as it scrolls (`fade_overflow`) or not \
                 its view's own",
                node(id)
            ),
            Self::Over(id) => format!(
                "{} draws over it, so moving its pixels would move that too (keep what draws \
                 over a scroll view outside its box)",
                node(id)
            ),
            Self::Under(id) => format!(
                "{} shows under it (give the scroll view an opaque fill of its own)",
                node(id)
            ),
        }
    }
}
/// Which paint items are a node's subtree's, asked in paint order, without a
/// walk up the tree for each: an item's parent is the item before it or one
/// of that one's ancestors (a walk paints a node before what it holds, and
/// its border pass after), so the nodes from the subtree's root down to the
/// item before are kept, and an item whose parent is among them is within.
/// Any other is walked up, as `Scene::is_within` does. (A transcript's rows
/// lie 16 nodes under its scroll view: a walk for each of 300 items, every
/// frame it scrolls.)
#[derive(Default)]
struct Within(Vec<NodeId>, #[cfg(test)] usize);
impl Within {
    /// Whether `id` is `root` or within it (`Scene::is_within`).
    fn holds(&mut self, scene: &Scene, id: NodeId, root: NodeId) -> bool {
        if id == root {
            self.0.clear();
            self.0.push(id);
            return true;
        }
        if let Some(parent) = scene.parent(id) {
            while let Some(&last) = self.0.last() {
                if last == parent {
                    self.0.push(id);
                    return true;
                }
                self.0.pop();
            }
        }
        // Walked up: the nodes on the way, root first, are kept if within.
        #[cfg(test)]
        {
            self.1 += 1;
        }
        let mut at = id;
        loop {
            self.0.push(at);
            if at == root {
                self.0.reverse();
                return true;
            }
            match scene.parent(at) {
                Some(parent) => at = parent,
                None => {
                    self.0.clear();
                    return false;
                }
            }
        }
    }
}
/// Whether moving `scroll`'s pixels reproduces the frame (`None`), and what
/// keeps it from it otherwise: its content is one contiguous run of paint
/// items none of which reads what lies under it; beneath it the clip shows a
/// single opaque rectangle (or the window background); above it nothing
/// paints in the clip outside this frame's damage. What paints above or
/// shows beneath it and repaints where it is still had its pixels in the
/// frame copied: `carried` gets where the copy puts them, to repaint.
/// Masks, fades, layers and filters of the content's own move with it, and
/// after a move of whole pixels draw what they drew (`cut_fades` for those
/// that don't). A fade around the content is no obstacle: `fade` gets the
/// mask, for the move to copy within its interior (`scroll_copies`).
fn copy_blocker(
    scene: &Scene,
    scroll: &zgx::scene::ScrollMove,
    damage: &[Rect],
    scale: f32,
    carried: &mut Vec<Rect>,
    fade: &mut Option<zgx::scene::FadeMask>,
) -> Option<CopyBlocker> {
    let clip = scroll.clip;
    let items = scene.layer_items_within(None, Some(&[clip]));
    let content = |item: &zgx::scene::PaintItem<'_>| scene.is_within(item.id, scroll.node);
    let Some(first) = items.iter().position(content) else {
        return Some(CopyBlocker::NoContent);
    };
    let last = items.iter().rposition(content).expect("found above");
    // What the content has from around it, which holds still as it moves:
    // the extras its own node was given. Any other an item has is its own.
    let Some(around) = items[first..=last].iter().find(|i| i.id == scroll.node) else {
        return Some(CopyBlocker::NoContent);
    };
    if around.outer_mask().is_some() {
        return Some(CopyBlocker::TwoFades);
    }
    *fade = around.mask();
    let paints = |item: &zgx::scene::PaintItem<'_>| -> Option<Rect> {
        let bounds = match item.kind {
            // A layer: all it holds, turned and scaled as it is.
            _ if item.isolated => scene.layer_ink(item)?,
            NodeKind::Container(_) if item.effects.blur_radius <= 0. => return None,
            // Clear paints nothing: an overlay scrollbar's track away from
            // the pointer, over every list it scrolls, or a clear box.
            NodeKind::Rect(zgx::scene::Color(_, _, _, 0)) => return None,
            NodeKind::Quad(quad) | NodeKind::Panel { quad, .. }
                if quad.fill.3 == 0
                    && !quad.has_border()
                    && quad.shadows().is_empty()
                    && quad.decoration.is_none()
                    && quad.outline.is_none() =>
            {
                return None;
            }
            NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } => quad.paint_bounds(item.bounds),
            _ => item.bounds,
        };
        if item.effects.opacity <= 0. {
            return None;
        }
        bounds.intersection(item.clip?)?.intersection(clip)
    };
    let repainted = |rect: Rect| {
        damage.iter().any(|d| {
            d.x <= rect.x
                && d.y <= rect.y
                && d.x + d.width >= rect.x + rect.width
                && d.y + d.height >= rect.y + rect.height
        })
    };
    let mut looked: Option<*const zgx::scene::PaintExtras> = None;
    let mut within = Within::default();
    for item in &items[first..=last] {
        if !within.holds(scene, item.id, scroll.node) {
            return Some(CopyBlocker::Between(item.id));
        }
        // What reads the pixels under it reads the view's, which hold still
        // where the content moves over them.
        if item.effects.blur_radius > 0. || (item.isolated && reads_under(scene, item.id)) {
            return Some(CopyBlocker::Reads(item.id));
        }
        // Dots on the screen's grid stay put as the picture slides under them.
        if let NodeKind::Image(image) = item.kind
            && image.effect() != zgx::image::ImageEffect::Plain
        {
            return Some(CopyBlocker::Dots(item.id));
        }
        // Under a fade, such a layer has its whole run fade item by item
        // where it paints whole, and a copy's bands are walked without it.
        if item.mask().is_some() && !bands::fades_apart(scene, item) {
            return Some(CopyBlocker::FilteredUnderFade(item.id));
        }
        // A loop a layer plays isn't in the frames, and its layer goes where
        // its node is whatever a frame walks (`sync_loops`): nothing of it
        // to carry. But only a walk finds the view's edge cutting it, or
        // something over it, for which the layer would show too much: all it
        // can paint repaints, for the walk to reach it.
        #[cfg(target_os = "macos")]
        if scene.baked_loops().iter().any(|(node, _)| *node == item.id) {
            let ink = scene.layer_ink(item).unwrap_or(item.bounds);
            carried.extend(ink.union(item.bounds).intersection(clip));
        }
        // (A subtree shares its extras: looked at once.)
        if let Some(extras) = &item.extras
            && !looked.is_some_and(|at| std::ptr::eq(at, &**extras))
        {
            looked = Some(&**extras);
            if item.mask() != around.mask() {
                cut_fades(scene, item, around.mask(), scroll, carried);
            }
        }
    }
    let carry = |rect: Rect| {
        Rect::new(
            rect.x + scroll.dx,
            rect.y + scroll.dy,
            rect.width,
            rect.height,
        )
        .intersection(clip)
    };
    for item in &items[last + 1..] {
        let Some(rect) = paints(item) else {
            continue;
        };
        // All it can put on screen: a box's paint (its shadows and outline),
        // or for text, an image or a canvas the ink the scene keeps for it,
        // which may reach past its box (a glyph's overhang, a stroke).
        let ink = match item.kind {
            NodeKind::Quad(_) | NodeKind::Panel { .. } => rect,
            _ => scene
                .layer_ink(item)
                .and_then(|ink| ink.intersection(item.clip?))
                .and_then(|ink| ink.intersection(clip))
                .map_or(rect, |ink| ink.union(rect)),
        };
        if !repainted(ink) {
            // Something that stays put over the view (a composer floating
            // on a transcript): the copy moves the view's pixels, its among
            // them, so it repaints where it is, as the copy left what was
            // under it, and where the copy put it. Only a blur reads past
            // its own box, so a blurred one still repaints the whole view.
            if item.isolated || item.effects.blur_radius > 0. {
                return Some(CopyBlocker::Over(item.id));
            }
            carried.push(ink);
        }
        carried.extend(carry(ink));
    }
    // Content opaque across the whole clip (a list's rows) hides whatever
    // lies under it, before the move and after.
    let opaque = |item: &&zgx::scene::PaintItem<'_>| {
        matches!(item.kind, NodeKind::Quad(quad) | NodeKind::Panel { quad, .. }
            if quad.fill.3 == 255 && quad.radius <= 0. && quad.shadow.is_none() && quad.decoration.is_none())
            && item.effects.opacity >= 1.
            && item.effects.edge_fade <= 0.
            // Under no mask, rounded clip or colour filter of the content's
            // own, which would show what lies under it.
            && !item.isolated
            && item.extras == around.extras
    };
    if spans_cover(
        clip,
        items[first..=last].iter().filter(opaque).filter_map(paints),
        scale,
    ) {
        return None;
    }
    for item in items[..first].iter().rev() {
        let Some(rect) = paints(item) else {
            continue;
        };
        if let NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } = item.kind
            && quad.fill.3 == 255
            && quad.radius <= 0.
            && quad.border_width <= 0.
            && quad.shadow.is_none()
            && quad.decoration.is_none()
            && item.effects.opacity >= 1.
            && item.effects.edge_fade <= 0.
            && item.mask().is_none()
            && !item.isolated
            && rect == clip
        {
            return None;
        }
        // One colour across the whole clip, however see-through (a card's
        // tint on the window's glass): the same under every pixel before
        // the move and after, so the copy carries it unchanged.
        if let Some(corners) = uniform_across(item, clip) {
            for corner in corners {
                carried.push(corner);
                carried.extend(carry(corner));
            }
            continue;
        }
        if !repainted(rect) {
            return Some(CopyBlocker::Under(item.id));
        }
        carried.extend(carry(rect));
    }
    None
}
/// Whether layer `id` draws from outside itself: a blend, or anywhere in it a
/// backdrop filter or a shader reading the backdrop or the window, or whose
/// inputs run on as frames draw.
fn reads_under(scene: &Scene, id: NodeId) -> bool {
    scene.blend(id) != zgx::scene::BlendMode::Normal
        || scene
            .layer_shader(id)
            .is_some_and(zgx::scene::LayerShader::reads_backdrop)
        || scene.reads_window_within(id)
}
/// A fade in scroll content draws what it drew after the move only where its
/// band's edge moved with it. Its box comes from its clip, so an edge the
/// view's own clip cuts (the content's foot past the view's, a band on a node
/// that clips nothing) stays at the view's edge as the content moves: `carried`
/// gets what lies under such a fade, to repaint. `around` is the fade the
/// content has from around it, which `scroll_copies` keeps.
#[cold]
#[inline(never)]
fn cut_fades(
    scene: &Scene,
    item: &zgx::scene::PaintItem<'_>,
    around: Option<zgx::scene::FadeMask>,
    scroll: &zgx::scene::ScrollMove,
    carried: &mut Vec<Rect>,
) {
    // The largest of the insets taken off the fades between it and the view:
    // the clips the boxes came from reach at least that far out, no further
    // than the view's edge if cut.
    let mut insets = [0_f32; 4];
    let mut at = scene.parent(item.id);
    while let Some(node) = at {
        let style = scene.style(node);
        let [top, bottom] = style.fade_insets;
        let [left, right] = style.fade_insets_x;
        for (most, inset) in insets.iter_mut().zip([top, right, bottom, left]) {
            if inset.is_finite() {
                *most = most.max(inset);
            }
        }
        if node == scroll.node {
            break;
        }
        at = scene.parent(node);
    }
    let view = scroll.clip;
    let (view_right, view_bottom) = (view.x + view.width, view.y + view.height);
    for (rect, [top, right, bottom, left]) in [item.mask(), item.outer_mask()].into_iter().flatten()
    {
        if Some((rect, [top, right, bottom, left])) == around {
            continue;
        }
        let clip = Rect::new(
            rect.x - insets[3],
            rect.y - insets[0],
            rect.width + insets[3] + insets[1],
            rect.height + insets[0] + insets[2],
        );
        let (clip_right, clip_bottom) = (clip.x + clip.width, clip.y + clip.height);
        // Across the move nothing about a band changes; along it, its edge
        // lies inside the view before the move and after. An inset comes off
        // what's left of the clip, so with one either edge cut moves the band.
        let (dx, dy) = (scroll.dx, scroll.dy);
        let [cut_top, cut_bottom, cut_left, cut_right] = [
            clip.y.min(clip.y - dy) <= view.y,
            clip_bottom.max(clip_bottom - dy) >= view_bottom,
            clip.x.min(clip.x - dx) <= view.x,
            clip_right.max(clip_right - dx) >= view_right,
        ];
        let cut = (dy != 0.
            && ((top > 0. && cut_top)
                || (bottom > 0. && cut_bottom)
                || ((top > 0. || bottom > 0.)
                    && insets[0] + insets[2] > 0.
                    && (cut_top || cut_bottom))))
            || (dx != 0.
                && ((left > 0. && cut_left)
                    || (right > 0. && cut_right)
                    || ((left > 0. || right > 0.)
                        && insets[1] + insets[3] > 0.
                        && (cut_left || cut_right))));
        if cut {
            carried.extend(clip.intersection(view));
        }
    }
}
/// Whether `item` paints one colour over all of `clip` but its rounded
/// corners: a plain fill whose box holds the clip inside its border, with any
/// shadow under it at full strength there and nothing over its content. It
/// gives the corners' squares within the clip, which the copy can't carry
/// and so repaint (where they are and where the copy puts them).
fn uniform_across(item: &zgx::scene::PaintItem<'_>, clip: Rect) -> Option<Vec<Rect>> {
    let (NodeKind::Quad(quad) | NodeKind::Panel { quad, .. }) = item.kind else {
        return None;
    };
    let holds = |outer: Rect, inner: Rect| {
        const E: f32 = 1e-3;
        inner.x >= outer.x - E
            && inner.y >= outer.y - E
            && inner.x + inner.width <= outer.x + outer.width + E
            && inner.y + inner.height <= outer.y + outer.height + E
    };
    let inset = |r: Rect, by: f32| {
        Rect::new(
            r.x + by,
            r.y + by,
            (r.width - 2. * by).max(0.),
            (r.height - 2. * by).max(0.),
        )
    };
    if quad.decoration.is_some()
        || quad.border_over
        || quad.outline.is_some_and(|outline| outline.visible())
        || item.isolated
        || item.mask().is_some()
        || item.outer_mask().is_some()
        || item.effects.edge_fade > 0.
        || item.effects.blur_radius > 0.
        || !item.clip.is_some_and(|own| holds(own, clip))
    {
        return None;
    }
    let b = item.bounds;
    let border = if quad.border_width > 0. && quad.border_color.3 > 0 {
        quad.border_width
    } else {
        0.
    };
    if !holds(inset(b, border), clip) {
        return None;
    }
    let r = quad.radius.max(0.);
    let shadows_full = quad.shadows().iter().all(|shadow| {
        let grown = b.expand(shadow.spread.max(0.));
        let moved = Rect::new(
            grown.x + shadow.offset.x,
            grown.y + shadow.offset.y,
            grown.width,
            grown.height,
        );
        let full = inset(moved, shadow.blur_radius.max(0.) * 3. + r);
        holds(full, clip)
    });
    if !shadows_full {
        return None;
    }
    // The rounded corners' squares the clip reaches.
    let side = r + border;
    Some(
        [
            Rect::new(b.x, b.y, side, side),
            Rect::new(b.x + b.width - side, b.y, side, side),
            Rect::new(b.x, b.y + b.height - side, side, side),
            Rect::new(b.x + b.width - side, b.y + b.height - side, side, side),
        ]
        .into_iter()
        .filter(|_| r > 0.)
        .filter_map(|corner| corner.intersection(clip))
        .collect(),
    )
}
/// Whether rectangles spanning `clip` side to side (or top to bottom) cover
/// it, meeting on whole device pixels: no seam lets anything beneath show.
fn spans_cover(clip: Rect, rects: impl Iterator<Item = Rect>, scale: f32) -> bool {
    let whole = |value: f32| ((value * scale) - (value * scale).round()).abs() < 1e-3;
    let (mut rows, mut columns) = (Vec::new(), Vec::new());
    for r in rects {
        let (right, bottom) = (r.x + r.width, r.y + r.height);
        if r.x <= clip.x && right >= clip.x + clip.width && whole(r.y) && whole(bottom) {
            rows.push((r.y, bottom));
        }
        if r.y <= clip.y && bottom >= clip.y + clip.height && whole(r.x) && whole(right) {
            columns.push((r.x, right));
        }
    }
    let covered = |mut spans: Vec<(f32, f32)>, start: f32, end: f32| {
        spans.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut reach = start;
        for (from, to) in spans {
            if from > reach {
                break;
            }
            reach = reach.max(to);
        }
        reach >= end
    };
    covered(rows, clip.y, clip.y + clip.height) || covered(columns, clip.x, clip.x + clip.width)
}
/// Positioned glyphs of laid-out runs; rich text keeps each glyph's run.
/// Its colour is read when quads are made, from the retained rich text.
#[inline]
fn glyphs_of<'a>(
    glyphs: Vec<Glyph>,
    runs: impl Iterator<Item = cosmic_text::LayoutRun<'a>>,
    starts: &[usize],
    rich: Option<&zgx::rich_text::RichText>,
    scale: f32,
) -> Vec<Glyph> {
    match rich {
        // Glyphs come mostly in their run's order: the run of the last one
        // is tried first, and searched for (`run_at`) only when it isn't
        // this one's (runs cover the text in order, `RichText::new`).
        Some(rich) => {
            let mut current = 0;
            styled_glyphs(glyphs, runs, scale, |line, glyph| {
                let byte = starts[line] + glyph.start;
                if !rich.runs()[current].range.contains(&byte) {
                    current = text::run_at(rich, byte);
                }
                current
            })
        }
        // Every plain buffer here uses text::attrs, without colour spans.
        None => styled_glyphs(glyphs, runs, scale, |_, _| 0),
    }
}
/// The scale below which text is drawn for a low-density screen: glyphs on
/// whole device pixels (`styled_glyphs`) and their coverage firmed
/// (`FIRM_COVERAGE`). At 2x and over, where a pixel is too small to see,
/// glyphs keep their subpixel places and outlines' plain coverage.
const SHARP_TEXT_BELOW: f32 = 2.;
/// A glyph quad's `options.y` below `SHARP_TEXT_BELOW`: a mask whose coverage
/// `draw.wgsl` firms by its colour (`firm`).
const FIRM_GLYPH: f32 = 1.25;
/// A glyph's `options.y`: straight alpha, firmed below `SHARP_TEXT_BELOW` if a
/// mask (a colour glyph's own colours stay as they are).
fn glyph_texture(color: bool, scale: f32) -> f32 {
    if !color && scale < SHARP_TEXT_BELOW {
        FIRM_GLYPH
    } else {
        1.
    }
}
/// `glyphs_of`, each glyph's run from `style` (given its line).
fn styled_glyphs<'a>(
    mut glyphs: Vec<Glyph>,
    runs: impl Iterator<Item = cosmic_text::LayoutRun<'a>>,
    scale: f32,
    mut style: impl FnMut(usize, &cosmic_text::LayoutGlyph) -> usize,
) -> Vec<Glyph> {
    for run in runs {
        // Most texts are one line: one allocation at its size.
        glyphs.reserve(run.glyphs.len());
        // Below 2x, the line's start on a whole pixel, and each glyph on one
        // counted from there: a line that only moves keeps its spacing.
        let sharp = scale < SHARP_TEXT_BELOW;
        let line_start = match run.glyphs.first() {
            Some(glyph) if sharp => (glyph.x + glyph.font_size * glyph.x_offset) * scale,
            _ => 0.,
        };
        for glyph in run.glyphs {
            // Below 2x, each glyph on the nearest whole device pixel (its
            // subpixel bin zero), as Windows' own text is: a stem a quarter
            // pixel off lands on two columns at part strength, which reads
            // as blur on a ~96 dpi screen. Its line keeps its layout.
            let p = if sharp {
                let exact = (glyph.x + glyph.font_size * glyph.x_offset) * scale;
                let placed = line_start.round() + (exact - line_start).round();
                glyph.physical((placed - exact, 0.), scale)
            } else {
                glyph.physical((0., 0.), scale)
            };
            let run_index = style(run.line_i, glyph);
            glyphs.push(Glyph {
                key: p.cache_key,
                run: run_index,
                x: p.x as f32,
                // Each line's baseline on a whole device pixel too.
                y: p.y as f32 + (run.line_y * scale).round(),
            });
        }
    }
    glyphs
}
/// A pixel rectangle: x, y, width, height.
type Scissor = (u32, u32, u32, u32);
/// A scroll copy's destination and the offset of its source.
type ScrollCopy = (Scissor, (i32, i32));
/// For each damage region, the index of the topmost quad that paints every
/// pixel of the region opaque: a plain solid rectangle (no texture, radius,
/// border, shadow, fade, mask or transform) at full alpha, unclipped over the
/// region's scissor. Nothing drawn before it, including the clear, can show.
/// 0 when no quad qualifies.
fn occluded_from(
    quads: &[Quad],
    draws: &[Draw],
    regions: &[(Rect, Scissor)],
    scale: f32,
) -> Vec<u32> {
    let covers = |outer: Rect, inner: Rect| {
        outer.x <= inner.x
            && outer.y <= inner.y
            && outer.x + outer.width >= inner.x + inner.width
            && outer.y + outer.height >= inner.y + inner.height
    };
    let plain = |q: &Quad| {
        q.color[3] >= 1.
            && q.options == [0.; 4]
            && q.shape[0] <= 0.
            && q.shape[1] <= 0.
            && q.shape[2] <= 0.
            && q.mask == NO_MASK
    };
    regions
        .iter()
        .map(|(_, (x, y, w, h))| {
            // The pixels the scissor lets through, in logical units.
            let pixels = Rect::new(
                *x as f32 / scale,
                *y as f32 / scale,
                *w as f32 / scale,
                *h as f32 / scale,
            );
            for draw in draws.iter().rev() {
                if draw.source != TextureSource::Atlas
                    || draw.blur.is_some()
                    || !covers(draw.bounds, pixels)
                    || !covers(draw.clip, pixels)
                {
                    continue;
                }
                for index in (draw.start..draw.end).rev() {
                    let q = &quads[index as usize];
                    let rect = Rect::new(q.rect[0], q.rect[1], q.rect[2], q.rect[3]);
                    if plain(q) && covers(rect, pixels) {
                        return index;
                    }
                }
            }
            0
        })
        .collect()
}
#[derive(Clone)]
struct Glyph {
    key: CacheKey,
    /// The rich run it was shaped from, including paint-only changes.
    run: usize,
    x: f32,
    y: f32,
}
/// A text's cached quad, in its own place: its box, atlas cell, colour and
/// paint (corner radius, glyph flag, rounded-background flag). The rest of
/// the GPU `Quad` is the same for every one of a text's and is made where it
/// is placed (`render_flat`): half the bytes cached, copied and read.
#[derive(Clone, Copy)]
struct TextQuad {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    paint: [f32; 4],
}
/// A rounded span background's quads among a text's (`shape.w`, untextured)
/// rounded on their own boxes rather than the text's: draw.wgsl rounds a
/// quad's `fade` box, which `place_text_quads` gives as the text's. Each
/// line's piece of a wrapped span is a quad of its own, so each rounds on
/// its own ends (CSS's `box-decoration-break: clone`). Out of line: texts
/// without one never come here.
#[cold]
#[inline(never)]
fn round_own_boxes(quads: &mut [Quad]) {
    for quad in quads {
        if quad.shape[3] > 0.5 && quad.options[1] == 0. {
            quad.fade = quad.rect;
        }
    }
}
/// A gradient text's quads from `start`, its glyphs from `glyphs` on, put
/// in the order its own layer draws them (`Scene::set_text_gradient`): its
/// glyphs (white, a mask), its `gradient` kept where they are
/// (`Shading::SourceIn`), its box, marks, selection and shadows behind them
/// (`Shading::Behind`), then its `selected` glyphs over all in the
/// selected-text colour. Where each run after the glyphs starts.
#[cold]
#[inline(never)]
fn gradient_text_quads(
    quads: &mut Vec<Quad>,
    (start, glyphs): (u32, usize),
    gradient: Quad,
    selected: Vec<Quad>,
) -> [u32; 3] {
    let under: Vec<Quad> = quads.drain(start as usize..glyphs).collect();
    let filled = quads.len() as u32;
    quads.push(gradient);
    let behind = quads.len() as u32;
    quads.extend(under);
    let over = quads.len() as u32;
    quads.extend(selected);
    [filled, behind, over]
}
/// `gradient_text_quads`' runs as `draw`'s draws, the gradient from `image`.
#[cold]
#[inline(never)]
fn push_gradient_text(
    draws: &mut Vec<Draw>,
    quads: &[Quad],
    draw: &Draw,
    [filled, behind, over]: [u32; 3],
    image: u64,
) {
    let runs = [
        (draw.start, filled, Shading::Full, TextureSource::Atlas),
        (
            filled,
            behind,
            Shading::SourceIn,
            TextureSource::Image(image),
        ),
        (behind, over, Shading::Behind, TextureSource::Atlas),
        (over, draw.end, Shading::Full, TextureSource::Atlas),
    ];
    for (start, end, shading, source) in runs {
        if end > start {
            draws.push(Draw {
                shading,
                blur: None,
                start,
                end,
                clip: draw.clip,
                bounds: quad_bounds(&quads[start as usize..end as usize]),
                source,
            });
        }
    }
}
/// A kept text's quads at `x`, `y`, each coloured by `color`.
fn place_text_quads(
    quads: &mut Vec<Quad>,
    source: &[TextQuad],
    x: f32,
    y: f32,
    fade: [f32; 4],
    edge_fade: f32,
    mut color: impl FnMut(&TextQuad) -> [f32; 4],
) {
    quads.extend(source.iter().map(|cached| Quad {
        rect: [
            cached.rect[0] + x,
            cached.rect[1] + y,
            cached.rect[2],
            cached.rect[3],
        ],
        uv: cached.uv,
        color: color(cached),
        fade,
        options: [edge_fade, cached.paint[1], 0., 0.],
        shape: [cached.paint[0], 0., 0., cached.paint[3]],
        border: [0.; 4],
        mask: NO_MASK,
    }));
}
/// A shadowed text's clip: `clip` (its ancestors') over its box and where
/// its shadows paint, each its box moved by the shadow's offset and grown by
/// three sigmas (as the scene counts their damage). Its own clip, `own`,
/// goes to `own_clip`: its glyphs stay where they were clipped (`truncate`,
/// `lines`), only its shadows reach past its box. A text with none: `own`.
#[cold]
#[inline(never)]
fn shadowed(
    scene: &Scene,
    id: NodeId,
    bounds: Rect,
    clip: Rect,
    own: Rect,
    own_clip: &mut Option<Rect>,
) -> Rect {
    let shadows = scene.text_shadows(id);
    if shadows.is_empty() {
        return own;
    }
    *own_clip = Some(own);
    let reach = shadows.iter().fold(bounds, |all, shadow| {
        let blur = shadow.blur.max(0.) * 3.;
        all.union(Rect::new(
            bounds.x + shadow.offset.x - blur,
            bounds.y + shadow.offset.y - blur,
            bounds.width + 2. * blur,
            bounds.height + 2. * blur,
        ))
    });
    clip.intersection(reach).unwrap_or_default()
}
/// A text shadow's blur as `blurred_glyph` keys it: its sigma in eighths of
/// a device pixel, at most `MAX_SHADOW_SIGMA`; 0 is sharp.
fn shadow_sigma(blur: f32, scale: f32) -> u32 {
    if blur.is_nan() || blur <= 0. {
        return 0;
    }
    ((blur * scale).min(MAX_SHADOW_SIGMA) * 8.).round() as u32
}
/// A glyph's coverage blurred (`blur_coverage`): `alpha` holds `texels`, each
/// `size / texels` device pixels square, the glyph `reach` pixels in.
struct BlurredCoverage {
    alpha: Vec<u8>,
    texels: (u32, u32),
    size: (u32, u32),
    reach: u32,
}
/// `coverage` (`width` by `height`, 0 to 255) blurred by a Gaussian of
/// `sigma` pixels: averaged down by a power of two first where sigma would
/// span more than four texels, so its taps stay few however wide it is.
fn blur_coverage(coverage: &[f32], (width, height): (u32, u32), sigma: f32) -> BlurredCoverage {
    let mut step = 1u32;
    while sigma / step as f32 > 4. && step < 64 {
        step *= 2;
    }
    let reach = ((sigma * 3.).ceil() as u32).div_ceil(step) * step;
    let size = (
        (width + 2 * reach).div_ceil(step) * step,
        (height + 2 * reach).div_ceil(step) * step,
    );
    let (tw, th) = ((size.0 / step) as usize, (size.1 / step) as usize);
    let mut texels = vec![0f32; tw * th];
    let area = (step * step) as f32;
    for (row, line) in coverage.chunks_exact(width as usize).enumerate() {
        let ty = (row + reach as usize) / step as usize;
        for (column, &value) in line.iter().enumerate() {
            texels[ty * tw + (column + reach as usize) / step as usize] += value / area;
        }
    }
    let s = sigma / step as f32;
    let radius = (s * 3.).ceil() as usize;
    let mut kernel: Vec<f32> = (0..=2 * radius)
        .map(|i| (-((i as f32 - radius as f32).powi(2)) / (2. * s * s)).exp())
        .collect();
    let total: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= total);
    let pass = |from: &[f32],
                to: &mut [f32],
                (along, lines): (usize, usize),
                at: fn(usize, usize, usize) -> usize,
                w: usize| {
        for line in 0..lines {
            for i in 0..along {
                let mut sum = 0.;
                for (k, weight) in kernel.iter().enumerate() {
                    let j = i + k;
                    if j >= radius && j - radius < along {
                        sum += from[at(line, j - radius, w)] * weight;
                    }
                }
                to[at(line, i, w)] = sum;
            }
        }
    };
    let mut across = vec![0f32; tw * th];
    pass(&texels, &mut across, (tw, th), |row, x, w| row * w + x, tw);
    pass(
        &across,
        &mut texels,
        (th, tw),
        |column, y, w| y * w + column,
        tw,
    );
    BlurredCoverage {
        alpha: texels
            .iter()
            .map(|&a| a.round().clamp(0., 255.) as u8)
            .collect(),
        texels: (tw as u32, th as u32),
        size,
        reach,
    }
}
// Keep the former glyph charge so compaction cannot retain more entries.
#[allow(dead_code)]
struct GlyphCharge {
    key: CacheKey,
    color: Option<zgx::scene::Color>,
    run: usize,
    x: f32,
    y: f32,
}
struct Shaped {
    rich: Option<Arc<zgx::rich_text::RichText>>,
    text_options: zgx::text_layout::TextOptions,
    decorations: Vec<text::RichDecoration>,
    font: zgx::text_layout::FontStyle,
    text: Arc<str>,
    size: f32,
    width: f32,
    height: f32,
    /// Device pixels per logical pixel the glyphs were placed at: a layer's
    /// raster scale differs from the window's.
    scale: f32,
    glyphs: Vec<Glyph>,
    quads: Vec<TextQuad>,
    /// Whether `quads` hold colour glyphs (`COLOR_GLYPH`).
    colored: bool,
    atlas_epoch: u64,
    last_used: u64,
    /// The prepared text these glyphs were laid out from, kept while drawn.
    prepared: Option<u64>,
}
impl Shaped {
    fn bytes(&self) -> usize {
        self.rich
            .as_ref()
            .map_or(self.text.len(), |r| r.storage_bytes())
            + self.font.storage_bytes()
            + self.decorations.capacity() * std::mem::size_of::<text::RichDecoration>()
            + self.glyphs.capacity() * std::mem::size_of::<GlyphCharge>()
            // (Charged as the GPU quads they are placed as: the cache keeps
            // as many shapes as when it held those.)
            + self.quads.capacity() * std::mem::size_of::<Quad>()
    }
}
/// The atlas's key: cosmic-text's, losslessly, with its scalar fields
/// packed into words so it hashes and compares in a few operations.
/// Four-byte aligned, its table's buckets are the size of those holding
/// cosmic-text's key.
#[derive(Clone, Copy, PartialEq, Eq)]
struct AtlasKey {
    font: cosmic_text::fontdb::ID,
    words: [u32; 4],
}
impl From<CacheKey> for AtlasKey {
    fn from(key: CacheKey) -> Self {
        // Bins take 2 bits each (4 variants), the glyph, weight and optical
        // size 16 each: 52 bits.
        let glyph = (key.x_bin as u64)
            | ((key.y_bin as u64) << 2)
            | (u64::from(key.glyph_id) << 4)
            | (u64::from(key.font_weight.0) << 20)
            | (u64::from(key.font_optical) << 36);
        Self {
            font: key.font_id,
            words: [
                key.font_size_bits,
                key.flags.bits(),
                glyph as u32,
                (glyph >> 32) as u32,
            ],
        }
    }
}
impl std::hash::Hash for AtlasKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.font, state);
        state.write_u64(u64::from(self.words[0]) | (u64::from(self.words[1]) << 32));
        state.write_u64(u64::from(self.words[2]) | (u64::from(self.words[3]) << 32));
    }
}
#[derive(Clone, Copy)]
struct AtlasEntry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    /// A colour glyph's (an emoji's): its texels its own colours, which the
    /// text's colour doesn't tint (`COLOR_GLYPH`).
    color: bool,
}
/// `TextQuad::paint[2]` of a colour glyph's quad (`AtlasEntry::color`):
/// placed white, with its text's alpha, so `basic`'s colour times texel
/// leaves its colours as they are.
const COLOR_GLYPH: f32 = 1.;
/// A glyph's coverage blurred for a text shadow (`blurred_glyph`): `texels`
/// in the atlas drawn over `size` device pixels, a wide blur at a power of
/// two fewer (sampled back up bilinearly, as `effects` does).
#[derive(Clone, Copy)]
struct BlurredGlyph {
    x: u32,
    y: u32,
    texels: (u32, u32),
    size: (u32, u32),
    left: i32,
    top: i32,
}
/// Blurred text shadows' glyphs and each text's quads of them, out of the
/// way of text without one (`GpuRenderer::blurred`).
#[derive(Default)]
struct BlurredShadows {
    /// Per glyph and sigma (eighths of a device pixel); given up with the
    /// atlas.
    glyphs: FxHashMap<(AtlasKey, u32), Option<BlurredGlyph>>,
    texts: FxHashMap<NodeId, BlurredText>,
}
/// A text's blurred shadows' quads, as `place_text_quads` takes them, per
/// sigma: placed while its shape, the atlas and the scale hold.
struct BlurredText {
    atlas_epoch: u64,
    scale: f32,
    sigmas: Vec<(u32, Vec<TextQuad>)>,
}
/// The widest text shadow blur, in device pixels (as `effects`' widest).
const MAX_SHADOW_SIGMA: f32 = 128.;
struct LayerCache {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    bounds: Rect,
    /// What it painted, in the scene's points (`Scene::layer_bounds`).
    content: Rect,
    revision: u64,
    root_origin: (f32, f32),
    bytes: usize,
    /// The content's extent in texture coordinates.
    uv: [f32; 2],
    /// Its raster scale over the device's, as steps of √2 (`raster_step`).
    step: i32,
    /// That scale's bits: a step's, or a zoom's exactly (`prepare_layer`).
    factor: u32,
    /// At a zoom's scale exactly: composited with its corner on a pixel,
    /// texel for pixel.
    exact: bool,
    /// What placing it at its scale keeps (`Placement`): `None` for one
    /// never placed (a fade band's), as `PLAIN`. Boxed, so that every
    /// layer's entry is as small as before placing was.
    placement: Option<Box<Placement>>,
    /// Promoted by a local transform rather than isolation: kept a while
    /// once back at identity (`LAYER_PARK_FRAMES`).
    transformed: bool,
    /// The frame (`GpuRenderer::layer_frame`) it was last a layer in.
    used: u64,
    /// No longer a layer, kept for its pixels.
    parked: bool,
    /// Drawn without something on its way (a picture decoding, a shader
    /// fill compiling: `images_awaited`): not its content, so drawn again
    /// rather than reused, whatever its revision says.
    incomplete: bool,
    /// Where in the window all of it showed when last drawn (`shown`),
    /// while every frame since that damaged it also drew it: its pixels
    /// then differ from its content's only where the window's damage
    /// says, and it can repaint just there. A frame damaging it without
    /// drawing it (hidden, culled) ends that.
    watched: Option<Rect>,
}
impl LayerCache {
    /// The bytes its content takes of its texture (`bytes`: all it holds).
    fn live_bytes(&self) -> usize {
        let (w, h) = (self.texture.width() as f32, self.texture.height() as f32);
        (w * self.uv[0]).round() as usize * (h * self.uv[1]).round() as usize * 4
    }
}
/// What a layer's raster keeps of where and how it was placed
/// (`LayerCache::placement`).
struct Placement {
    /// Drawn in part: what showed of it, magnified past what fits whole
    /// (`prepare_layer`).
    cropped: bool,
    /// Pixels its largest glyph takes per unit of raster scale, once a
    /// magnified raster found one too large for the atlas (0 until then):
    /// it rasterizes no finer than that glyph fits.
    glyphs: f32,
    /// Where it landed in its target (`Placed::grid`) and how magnified,
    /// in the last frame it was prepared in (the scene's flush serial
    /// first), and in the one before (`prepare_layer`): unchanged since
    /// then, it is still; changed, it moves, and keeps its raster.
    seen: Seen,
    /// Where its texel grid lies from its box's corner, within a texel:
    /// kept as it is while it moves, never worked out again from where its
    /// texels landed (`prepare_layer`).
    phase: (f32, f32),
}
/// A layer's `Placement` before it has one.
const PLACEMENT: Placement = Placement {
    cropped: false,
    glyphs: 0.,
    seen: Seen {
        frame: 0,
        magnification: 1.,
        before: 1.,
        grids: None,
    },
    phase: (0., 0.),
};
/// `PLACEMENT`, to read.
static PLAIN: Placement = PLACEMENT;
impl LayerCache {
    /// What placing it keeps.
    #[inline]
    fn placement(&self) -> &Placement {
        self.placement.as_deref().unwrap_or(&PLAIN)
    }
    /// What placing it keeps, to change.
    #[inline(always)]
    fn placement_mut(&mut self) -> &mut Placement {
        if self.placement.is_none() {
            self.place();
        }
        self.placement.as_deref_mut().expect("placed above")
    }
    /// A `Placement` of its own, for a layer that had none.
    #[cold]
    #[inline(never)]
    fn place(&mut self) {
        self.placement = Some(Box::new(PLACEMENT));
    }
}
/// `LayerCache::seen`: what a layer was in the frame last prepared, and in
/// the frame before that. Its grids only for a layer that reads them (one
/// magnified), so that no other carries them from frame to frame.
struct Seen {
    frame: u64,
    magnification: f32,
    before: f32,
    grids: Option<Box<SeenGrids>>,
}
/// `Seen`'s grids.
struct SeenGrids {
    now: Option<zgx::affine::Affine>,
    before: Option<zgx::affine::Affine>,
}
/// How a layer of the window's own shows there (`GpuRenderer::shown`).
#[derive(Clone, Copy)]
struct Shown<'a> {
    /// The window's damage this frame.
    damage: &'a [Rect],
    /// Its content onto the window.
    map: zgx::affine::Affine,
    /// What of the window it can show in.
    clip: Rect,
}
impl Shown<'_> {
    /// Where `content` shows in the window, if all of it does.
    #[inline(always)]
    fn window(&self, content: Rect) -> Option<Rect> {
        let window = self.map.bounds(content);
        within(self.clip, window).then_some(window)
    }
}
/// What says where a layer lands in its target, worked out only for a layer
/// that reads it (`Placing::placed`): every layer of every frame has one,
/// and few are magnified.
#[derive(Clone, Copy)]
struct Placing<'a> {
    item: &'a zgx::scene::PaintItem<'a>,
    /// Where the target's pixels start, in its points.
    origin: (f32, f32),
    scale: f32,
    /// What of the target can show.
    shows: Shows<'a>,
}
/// What of a layer's target can show: known, or (inside a layer that didn't
/// work out where it lands) worked out only by a layer that reads it.
#[derive(Clone, Copy)]
enum Shows<'a> {
    Known(Rect),
    /// Of the layer `id`, drawn in `bounds`, placed by `parent`.
    Inside {
        parent: &'a Placing<'a>,
        scene: &'a Scene,
        id: NodeId,
        bounds: Rect,
    },
}
/// Whether something reaches past layer `id`'s pixels (a blur, a mask, a
/// shader, a drop shadow): what of it shows is then not what bounds it.
fn reaches_past(scene: &Scene, id: NodeId) -> bool {
    scene.filter_blur(id) > 0.
        || scene.mask(id).is_some()
        || scene.layer_shader(id).is_some()
        || scene.drop_shadow(id).is_some()
}
impl Placing<'_> {
    /// `None` in perspective.
    #[cold]
    #[inline(never)]
    fn placed(&self) -> Option<Placed> {
        use zgx::affine::Affine;
        let map = match self.item.projective() {
            Some(map) => map.affine()?,
            None => Affine::IDENTITY,
        };
        let grid = map
            .then(Affine::translation(-self.origin.0, -self.origin.1))
            .then(Affine::scale(self.scale, self.scale));
        let target = match self.shows {
            Shows::Known(shows) => shows,
            Shows::Inside {
                parent,
                scene,
                id,
                bounds,
            } => (parent.placed())
                .and_then(|placed| placed.visible)
                .filter(|_| !reaches_past(scene, id))
                .and_then(|visible| visible.intersection(bounds))
                .unwrap_or(bounds),
        };
        let shows = match self.item.clip {
            Some(clip) => clip.intersection(target),
            None => Some(target),
        };
        let visible = shows.and_then(|shows| Some(map.inverse()?.bounds(shows)));
        Some(Placed { grid, visible })
    }
}
/// `prepare_layer`'s plan for a layer that reads where it lands: where it
/// does (`read`: magnified or drawn in part), whether it moved since the frame
/// before, the scale its raster was kept at, and what it is seen as now. Out
/// of line: few layers are magnified, and every layer is prepared each frame.
#[cold]
#[inline(never)]
fn plan_placed(
    placing: &Placing<'_>,
    cached: Option<&LayerCache>,
    magnification: f32,
    read: bool,
    serial: u64,
) -> (Option<Placed>, bool, Option<f32>, Seen) {
    let placed = read.then(|| placing.placed()).flatten();
    let grid = placed.map(|placed| placed.grid);
    let held = cached.map(|cache| {
        // What it was in the frame before this one (a frame may prepare it
        // twice: `layer_shifted`).
        let seen = &cache.placement().seen;
        let grids = seen.grids.as_deref();
        let (before_grid, before) = match seen.frame == serial {
            true => (grids.and_then(|grids| grids.before), seen.before),
            false => (grids.and_then(|grids| grids.now), seen.magnification),
        };
        let moving = match (grid, before_grid) {
            (Some(grid), Some(was)) => grid != was,
            _ => before != magnification,
        };
        (moving, f32::from_bits(cache.factor), before_grid, before)
    });
    let moving = held.is_some_and(|(moving, ..)| moving);
    let before_grid = held.map_or(grid, |(.., was, _)| was);
    let seen = Seen {
        frame: serial,
        magnification,
        before: held.map_or(magnification, |(.., before)| before),
        grids: (grid.is_some() || before_grid.is_some()).then(|| {
            Box::new(SeenGrids {
                now: grid,
                before: before_grid,
            })
        }),
    };
    (placed, moving, held.map(|(_, kept, ..)| kept), seen)
}
/// Where a layer's texels land in the target it composites into, and what
/// of it can show there (`Placing::placed`).
#[derive(Clone, Copy)]
struct Placed {
    /// Its content's points onto the target's pixels.
    grid: zgx::affine::Affine,
    /// What of its content can show there, in its own points.
    visible: Option<Rect>,
}
impl Placed {
    /// `grid`, where it lands each texel of a raster at `scale` on a pixel
    /// of its own: scaled alike both ways by as much, not turned or skewed.
    fn aligned(&self, scale: f32) -> Option<zgx::affine::Affine> {
        let grid = self.grid;
        let near = |v: f32| (v.abs() - scale).abs() <= scale * 1e-4;
        let level = |v: f32| v.abs() <= scale * 1e-6;
        (near(grid.a) && near(grid.d) && level(grid.b) && level(grid.c)).then_some(grid)
    }
}
/// Whether `inner` lies within `outer`.
fn within(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}
/// A layer's repaint, counted in the frame's `stats`.
/// The plain upload path's vertex buffer as made, and as given back at rest.
const VERTICES_AT_REST: usize = 4096;
fn add_layer_stats(stats: &mut GpuStats, result: &GpuStats) {
    stats.draw_calls += result.draw_calls;
    stats.render_passes += result.render_passes;
    stats.blur_passes += result.blur_passes;
    stats.backdrop_copies += result.backdrop_copies;
    stats.effect_passes += result.effect_passes;
    stats.instances += result.instances;
    stats.shaped_nodes += result.shaped_nodes;
    stats.glyph_uploads += result.glyph_uploads;
    stats.image_uploads += result.image_uploads;
    stats.canvas_rasterizations += result.canvas_rasterizations;
    stats.svg_rasterizations += result.svg_rasterizations;
    stats.geometry_rebuilds += result.geometry_rebuilds;
    stats.vertex_buffer_allocations += result.vertex_buffer_allocations;
    stats.layer_repaints += 1;
    stats.damaged_pixels += result.damaged_pixels;
}
/// Frames a transformed layer's texture outlives its transform, so an
/// animation passing through identity finds its pixels again instead of
/// repainting into a new allocation. The scene keeps counting the node's
/// layer revision meanwhile, so the pixels are known current.
const LAYER_PARK_FRAMES: u64 = 60;
/// The least the isolated layers' textures may take together (`layer_budget`).
const LAYER_BUDGET: usize = 64 * 1024 * 1024;
/// A layer's first try: no room of its own, only what the budget leaves (a
/// retry passes what is left).
const LAYER_ROOM: usize = usize::MAX;
const MB: f32 = 1024. * 1024.;
/// The most pixels a magnified layer's largest glyph rasterizes at: the
/// atlas's limit, with room for rounding (`prepare_layer`).
const GLYPH_ROOM: f32 = MAX_ATLAS as f32 * 0.98;
/// Raster scales a shrunk layer renders at, in √2 steps from the device's
/// (a magnified one renders at its own scale, `prepare_layer`, these only
/// where that does not fit): a sixteenth to four times. Below, what's
/// scaled down that far is a sliver of what it was laid out as.
const RASTER_STEPS: std::ops::RangeInclusive<i32> = -8..=4;
/// The raster step for content magnified `magnification` times: the
/// nearest √2 step, kept while within three quarters of a step of
/// `current`, so a scale animation re-renders a layer once per step it
/// crosses rather than every frame, and never back and forth at a boundary.
fn raster_step(magnification: f32, current: Option<i32>) -> i32 {
    if !(magnification.is_finite() && magnification > 0.) {
        return current.unwrap_or(0);
    }
    let level = 2. * magnification.log2();
    let step = match current {
        Some(step) if (level - step as f32).abs() < 0.75 => step,
        _ => level.round() as i32,
    };
    step.clamp(*RASTER_STEPS.start(), *RASTER_STEPS.end())
}
/// Step `step`'s raster scale, √2 to the `step`: whole steps of two exact
/// (`SQRT_2.powi(2)` is a ULP under 2, and a layer magnified 2 times then
/// read its texels a hair from their centres, which some GPUs' sampling
/// rounds into the next texel).
fn step_scale(step: i32) -> f32 {
    let twos = 2_f32.powi(step.div_euclid(2));
    match step.rem_euclid(2) {
        0 => twos,
        _ => twos * std::f32::consts::SQRT_2,
    }
}
/// A whole window's paint as `render_flat` made it, kept to patch where
/// the scene repaints (`render_kept`): its quads and draws, and where each
/// item's lie.
struct Kept {
    quads: Vec<Quad>,
    draws: Vec<Draw>,
    clips: Vec<Region>,
    items: Vec<KeptItem>,
    /// Each node's first item (a border pass follows it).
    by_node: KeptIndex,
    /// The scene's last flush it was brought up to date with.
    flushes: u64,
    /// What it was drawn for: the scene's nodes and identity, the atlas
    /// its glyphs lie in, and the target.
    revision: u64,
    identity: u64,
    atlas_epoch: u64,
    viewport: Rect,
    scale: f32,
    background: Color,
    /// Whether a frame has patched it yet.
    patched: bool,
    /// What its quads' patches wrote, for a vertex buffer still holding
    /// them to take only that (`MappedRing::write_kept`).
    upload: upload::KeptQuads,
    /// How many of its quads lie in paint order (as kept, or spliced):
    /// texts outgrowing their room move past them, to its end, where a
    /// subtree's quads no longer lie together.
    ordered: u32,
}
/// Each node's first item in a kept frame (`Kept::by_node`): a node's slot
/// number indexes a dense table while the frame's nodes are numbered close
/// together, a hash map otherwise. The item itself witnesses a slot's full
/// identity (number and generation), so a dense table can keep stale
/// entries after a splice: a node no longer there fails the item's bounds
/// or its identity check.
enum KeptIndex {
    Dense(Vec<u32>),
    Sparse(FxHashMap<NodeId, u32>),
}
impl KeptIndex {
    fn new(items: &[KeptItem]) -> Self {
        let span = items
            .iter()
            .map(|item| item.id.number() as usize + 1)
            .max()
            .unwrap_or(0);
        // A node has at most an ordinary item and a border item: six bytes
        // an item fit within the map's twelve-byte entry a node.
        let room = items.len().saturating_add(items.len() / 2);
        if span <= room {
            let mut slots = vec![0; room];
            for (index, item) in items.iter().enumerate().rev() {
                slots[item.id.number() as usize] = index as u32;
            }
            Self::Dense(slots)
        } else {
            let mut map = FxHashMap::default();
            Self::fill(&mut map, items);
            Self::Sparse(map)
        }
    }
    /// The distinct nodes among `items`: a node's second item is its border
    /// pass, at the end of its subtree.
    fn nodes(items: &[KeptItem]) -> usize {
        let borders = items
            .iter()
            .enumerate()
            .filter(|&(at, item)| {
                let end = item.end as usize;
                end > at + 1 && items[end - 1].id == item.id
            })
            .count();
        items.len() - borders
    }
    fn fill(map: &mut FxHashMap<NodeId, u32>, items: &[KeptItem]) {
        for (index, item) in items.iter().enumerate().rev() {
            map.insert(item.id, index as u32);
        }
    }
    #[inline]
    fn get(&self, id: NodeId, items: &[KeptItem]) -> Option<usize> {
        match self {
            Self::Dense(slots) => {
                let index = *slots.get(id.number() as usize)? as usize;
                (items.get(index)?.id == id).then_some(index)
            }
            Self::Sparse(map) => map.get(&id).map(|&index| index as usize),
        }
    }
    /// Brought up to date with `items` after splices (`renumbered`: all
    /// of them moved), `replaced` the nodes they took out (kept only for a
    /// sparse index).
    fn patch(
        &mut self,
        items: &[KeptItem],
        renumbered: bool,
        replaced: Vec<NodeId>,
        splices: &[(usize, usize, NodeId)],
    ) {
        match self {
            Self::Dense(slots) => {
                let span = if renumbered {
                    items.iter().map(|item| item.id.number() as usize + 1).max()
                } else {
                    splices
                        .iter()
                        .flat_map(|&(first, end, _)| &items[first..end])
                        .map(|item| item.id.number() as usize + 1)
                        .max()
                }
                .unwrap_or(0);
                if span > slots.len() {
                    let room = items.len().saturating_add(items.len() / 2);
                    if span > room {
                        let nodes = Self::nodes(items);
                        // The dense table goes first, and the map is made
                        // once at its size: growing it would hold two.
                        *self = Self::Sparse(FxHashMap::default());
                        let Self::Sparse(map) = self else {
                            unreachable!()
                        };
                        map.reserve(nodes);
                        Self::fill(map, items);
                        return;
                    }
                    slots.reserve_exact(room - slots.len());
                    slots.resize(room, 0);
                }
                if renumbered {
                    for (index, item) in items.iter().enumerate().rev() {
                        slots[item.id.number() as usize] = index as u32;
                    }
                } else {
                    for &(first, end, _) in splices {
                        for index in (first..end).rev() {
                            slots[items[index].id.number() as usize] = index as u32;
                        }
                    }
                }
            }
            Self::Sparse(map) => {
                if renumbered {
                    map.clear();
                    Self::fill(map, items);
                } else {
                    for id in replaced {
                        map.remove(&id);
                    }
                    for &(first, end, _) in splices {
                        for index in (first..end).rev() {
                            map.insert(items[index].id, index as u32);
                        }
                    }
                }
            }
        }
    }
}
/// What `render_flat` makes: a frame (`FRAME`, or the kept frame when
/// `draw_made`), or items' quads and draws alone, with where each
/// item's lie (`PATCH`: no frame begins, nothing is drawn), to patch a kept
/// frame with or, `record`ing, to keep. Frames drawn as usual run a loop
/// with nothing of keeping in it, so what it calls inlines as before.
const FRAME: u8 = 0;
const PATCH: u8 = 2;
/// One paint item's place in a `Kept`.
struct KeptItem {
    id: NodeId,
    inherited: zgx::scene::Inherited,
    /// Its quads: `len` of them, from `start`, in room for `room`. A text's
    /// value grows into spare quads after its own, kept empty (zero-sized,
    /// drawing nothing): a batch of draws in turn spans them.
    start: u32,
    len: u32,
    room: u32,
    /// Its draws: those of its quads' shadings, in turn.
    draws: std::ops::Range<u32>,
    /// A plain box's clip and compositing, to patch a move of it alone.
    boxed: Option<KeptBox>,
    /// The item after its subtree's (its own, its descendants' and its
    /// border pass): the run a splice replaces.
    end: u32,
}
/// Where each of `items`' subtrees ends (`KeptItem::end`), numbered from
/// `base`: items in paint order, each node's descendants after it and its
/// border pass after them.
fn subtree_ends(scene: &Scene, items: &mut [KeptItem], base: u32) {
    let mut open: Vec<(NodeId, usize)> = Vec::new();
    for at in 0..items.len() {
        let id = items[at].id;
        let after = base + at as u32;
        if open.iter().any(|&(node, _)| node == id) {
            // Its border pass closes it.
            while let Some((node, first)) = open.pop() {
                items[first].end = after + u32::from(node == id);
                if node == id {
                    break;
                }
            }
            items[at].end = after + 1;
            continue;
        }
        let parent = scene.parent(id);
        while let Some(&(node, first)) = open.last()
            && Some(node) != parent
        {
            items[first].end = after;
            open.pop();
        }
        open.push((id, at));
    }
    let after = base + items.len() as u32;
    for (_, first) in open {
        items[first].end = after;
    }
}
/// Paint that changes without the scene marking its node: a native
/// surface shows each frame its source gives it.
fn changes_unmarked(kind: &NodeKind) -> bool {
    #[cfg(target_os = "macos")]
    if matches!(kind, NodeKind::NativeSurface(_)) {
        return true;
    }
    let _ = kind;
    false
}
/// The share of the window a frame's damage covers from which it is drawn
/// from the window as kept (`render_kept`): sending every item's quads as
/// they are costs about 30 instructions an item, making a damaged one again
/// about 1,500.
const KEPT_SHARE: f32 = 0.1;
/// Frames in a row a kept window would have patched and drawn from (leaves
/// alone repainting, over `KEPT_SHARE` of it) after which a window that
/// lost its kept frame keeps one again (`render_once`): one alone may be a
/// change's last, and keeping costs a whole frame.
const KEEP_AGAIN: u8 = 2;
/// Kept frames lost in a row before they patched once, after which a frame
/// drawn whole keeps none (`GpuRenderer::unpatched_keeps`).
const UNPATCHED_KEEPS: u8 = 2;
/// The share of a kept window's items its spliced subtrees may hold
/// (`render_kept`): past it, walking them again costs more than drawing
/// the frame as usual.
const SPLICE_SHARE: f32 = 0.25;
/// The share of a window a frame's damage must cover for its repainted
/// subtrees to be spliced (`render_kept`): below it, drawing the frame as
/// usual walks less.
const SPLICE_DAMAGE: f32 = 0.5;
/// A plain box (`plain_box`) as `patch_box` makes it: what it paints, its
/// box, its clip and how it composites.
#[derive(Clone, Copy)]
struct PlainBox<'a> {
    kind: &'a NodeKind,
    bounds: Rect,
    clip: Option<Rect>,
    opacity: f32,
    edge_fade: f32,
}
impl<'a> PlainBox<'a> {
    fn of(item: &zgx::scene::PaintItem<'a>) -> Self {
        PlainBox {
            kind: item.kind,
            bounds: item.bounds,
            clip: item.clip,
            opacity: item.effects.opacity,
            edge_fade: item.effects.edge_fade,
        }
    }
}
/// What a kept box (a plain `Quad` or `Panel`) keeps to be patched when
/// layout alone moved or resized it: its clip (its parent's) and how it
/// composites, which only a repaint of more than its box changes.
#[derive(Clone, Copy)]
struct KeptBox {
    clip: Option<Rect>,
    opacity: f32,
    edge_fade: f32,
}
impl KeptBox {
    /// `item`'s, if it is one; `plain`: whether it is a plain box.
    fn of(item: &zgx::scene::PaintItem<'_>, plain: bool) -> Option<Self> {
        let boxed = matches!(item.kind, NodeKind::Quad(_) | NodeKind::Panel { .. });
        (boxed && plain).then_some(KeptBox {
            clip: item.clip,
            opacity: item.effects.opacity,
            edge_fade: item.effects.edge_fade,
        })
    }
}
/// A moved box's clip, and whether it is the whole viewport. The render
/// viewport starts at zero and has finite, positive extents: intersected
/// with itself it keeps its bits, and a box anywhere in it needs no scissor.
#[inline(always)]
fn moved_box_clip(clip: Option<Rect>, viewport: Rect, bounds: Rect) -> Option<(Rect, bool)> {
    // (Bits apart in any coordinate: one test of all four.)
    let apart = |a: Rect, b: Rect| {
        (a.x.to_bits() ^ b.x.to_bits())
            | (a.y.to_bits() ^ b.y.to_bits())
            | (a.width.to_bits() ^ b.width.to_bits())
            | (a.height.to_bits() ^ b.height.to_bits())
    };
    if clip.is_none_or(|clip| apart(clip, viewport) == 0) {
        return viewport.intersects(bounds).then_some((viewport, true));
    }
    // (A clip that meets the viewport lies in it: the viewport starts at
    // zero, so their overlap's start is in it and its width positive.)
    clip.unwrap()
        .intersection(viewport)
        .filter(|_| viewport.intersects(bounds))
        .map(|clip| (clip, false))
}
/// Whether `kind` paints as a plain box would (`plain_box`): a box without
/// decoration, shadows or a pass over its content.
#[inline(always)]
fn plain_quad(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. }
        if style.decoration.is_none() && style.shadows().is_empty() && !style.paints_over())
}
/// Whether `item` is a box `render_kept` makes alone (`patch_box`): a
/// flat colour, or a box without decoration, shadows or a pass over its
/// content, drawn flat (no layer, filter, clip region or fade mask).
#[inline(always)]
fn plain_box(item: &zgx::scene::PaintItem<'_>) -> bool {
    let plain = match item.kind {
        NodeKind::Rect(_) => true,
        NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
            style.decoration.is_none() && style.shadows().is_empty() && !style.paints_over()
        }
        _ => false,
    };
    plain
        && !item.isolated
        && !item.border_pass
        && item.effects.blur_radius <= 0.
        && item.matrix().is_none()
        && item.mask().is_none()
        && item.rounded().iter().all(Option::is_none)
        && item.color().is_none()
}
/// A flat colour's quad over `fade` (a box's place), as it shows.
#[inline(always)]
fn rect_quad(color: Color, fade: [f32; 4], effects: zgx::scene::Effects) -> Quad {
    Quad {
        rect: fade,
        uv: [0.; 4],
        color: rgba(color, effects.opacity),
        fade,
        options: [effects.edge_fade, 0., 0., 0.],
        shape: [0.; 4],
        border: [0.; 4],
        mask: NO_MASK,
    }
}
/// A box's quad with no decoration: its fill, corner radius and border
/// (none when drawn by its border pass, `later`), before `push_panel`.
#[inline(always)]
fn plain_panel_quad(
    style: &zgx::scene::QuadStyle,
    fade: [f32; 4],
    opacity: f32,
    edge_fade: f32,
    later: bool,
) -> Quad {
    let border = match later {
        true => 0.,
        false => style.border_width.max(0.),
    };
    Quad {
        rect: fade,
        uv: [0.; 4],
        color: rgba(style.fill, opacity),
        fade,
        options: [edge_fade, 0., 0., 0.],
        shape: [style.radius.max(0.), border, 0., 1.],
        border: rgba(style.border_color, opacity),
        mask: NO_MASK,
    }
}
/// When `clip` only culls whole quads of those from `start`, apply it on
/// the CPU and draw under the viewport, so a change of clip never splits a
/// batch: neighbouring text, icons and rows become one draw. Quads it
/// would cut and transformed quads keep the scissor. The draw's clip, and
/// the kept quads' bounds when culled here.
#[inline(always)]
fn cull_on_cpu(
    quads: &mut Vec<Quad>,
    start: usize,
    clip: Rect,
    viewport: Rect,
    scale: f32,
) -> (Rect, Option<Rect>) {
    let limit = snap_out(clip, scale);
    // One pass: each quad inside the clip, outside it, or cut.
    let (mut cut, mut outside, mut kept) = (false, false, None::<Rect>);
    for quad in &quads[start..] {
        if quad.options[2] >= 0.5 {
            cut = true;
            break;
        }
        let rect = quad_rect(quad);
        if rect_inside(rect, limit) {
            kept = Some(kept.map_or(rect, |bounds| bounds.union(rect)));
        } else if rect_outside(rect, limit) {
            outside = true;
        } else {
            cut = true;
            break;
        }
    }
    if cut {
        return (clip, None);
    }
    if outside {
        let mut kept = start;
        for index in start..quads.len() {
            if inside(&quads[index], limit) {
                quads[kept] = quads[index];
                kept += 1;
            }
        }
        quads.truncate(kept);
    }
    (viewport, Some(kept.unwrap_or_default()))
}
/// `cull_on_cpu` of a text's glyphs alone (`glyphs_alone`): untransformed
/// quads of sizes never negative, each its own rectangle (`quad_rect`),
/// tested and joined as such in one pass.
#[inline(always)]
fn cull_glyphs(
    quads: &mut Vec<Quad>,
    start: usize,
    clip: Rect,
    viewport: Rect,
    scale: f32,
) -> (Rect, Option<Rect>) {
    let limit = snap_out(clip, scale);
    let (mut outside, mut kept) = (false, None::<Rect>);
    for quad in &quads[start..] {
        let [x, y, width, height] = quad.rect;
        if quad.options[2] >= 0.5 || width < 0. || height < 0. {
            return cull_on_cpu(quads, start, clip, viewport, scale);
        }
        // (As `quad_rect` has it: a zero size's sign dropped.)
        let rect = Rect::new(x, y, width.abs(), height.abs());
        if rect_inside(rect, limit) {
            kept = Some(kept.map_or(rect, |bounds| bounds.union(rect)));
        } else if rect_outside(rect, limit) {
            outside = true;
        } else {
            return (clip, None);
        }
    }
    if outside {
        let mut kept = start;
        for index in start..quads.len() {
            if inside(&quads[index], limit) {
                quads[kept] = quads[index];
                kept += 1;
            }
        }
        quads.truncate(kept);
    }
    (viewport, Some(kept.unwrap_or_default()))
}
/// Close the open item range (`render_flat`'s `item_ranges`): its quads'
/// count, its room (`pad`: a text's spare quads, added empty) and where
/// its draws end.
fn close_range(
    quads: &mut Vec<Quad>,
    draws: &[Draw],
    ranges: &mut [(u32, u32, u32, u32, u32)],
    pad: bool,
) {
    let Some(open) = ranges.last_mut().filter(|range| range.4 == u32::MAX) else {
        return;
    };
    let made = quads.len() as u32 - open.0;
    let room = made + if pad { text_room(made) } else { 0 };
    quads.resize((open.0 + room) as usize, bytemuck::Zeroable::zeroed());
    *open = (open.0, made, room, open.3, draws.len() as u32);
}
/// Frames in a row a kept window would have patched (`would_patch`), read
/// up to `flushes` and content `revision`; after a second in a row it
/// wouldn't, the scene's repaints go unread for `skip` frames (one, then
/// twice as many each time, up to 32), so that flushes stop keeping them
/// where nothing patches.
#[derive(Default)]
struct Patchable {
    run: u8,
    flushes: u64,
    revision: u64,
    skip: u8,
    backoff: u8,
    /// A kept frame was given up since one last drew from it, its splices
    /// walking too much of it (`SPLICE_SHARE`): frames repainting more than
    /// leaves don't count as frames that would patch until one does.
    refused: bool,
}
/// The items a frame loop leaves (`render_flat`, which may return early),
/// dropped out of line: their drop inlined at each of its exits grows the
/// loop, which then costs every frame more instructions (measured: 0.5%).
struct LeftItems<'a, T>(&'a mut std::vec::IntoIter<T>);
impl<T> Drop for LeftItems<'_, T> {
    #[inline(never)]
    fn drop(&mut self) {
        self.0.by_ref().for_each(drop);
    }
}
/// Spare quads a text that changes (one a kept frame patched) keeps for
/// its value to grow into, past its own: few, as a batch drawing the kept
/// frame spans them (empty, they still cost the GPU), and a text
/// outgrowing them moves to the kept frame's end (`render_kept`).
fn text_room(glyphs: u32) -> u32 {
    (glyphs / 4).max(2)
}
/// `map.get(key)`, or in a patch (`PATCH`) the same through `entry`: the
/// frame loop stays the one caller of each map's `get` (and of its search,
/// which `get_mut` shares), inlined there.
#[inline(always)]
fn get_in<'a, const MODE: u8, K: Eq + std::hash::Hash + Clone, V>(
    map: &'a mut FxHashMap<K, V>,
    key: &K,
) -> Option<&'a V> {
    match MODE {
        PATCH => match map.entry(key.clone()) {
            std::collections::hash_map::Entry::Occupied(entry) => Some(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(_) => None,
        },
        _ => map.get(key),
    }
}
/// `map.insert(key, value)`, or in a patch (`PATCH`) the same through
/// `entry`: the frame loop stays the one caller of each map's `insert`.
#[inline(always)]
fn insert_in<const MODE: u8, K: Eq + std::hash::Hash, V>(
    map: &mut FxHashMap<K, V>,
    key: K,
    value: V,
) {
    match MODE {
        PATCH => {
            map.entry(key).insert_entry(value);
        }
        _ => {
            map.insert(key, value);
        }
    }
}
impl GpuStats {
    /// Add what `made` made (shaped, uploaded, rasterized) in a pass that
    /// drew nothing.
    fn add_made(&mut self, made: &GpuStats) {
        self.shaped_nodes += made.shaped_nodes;
        self.glyph_uploads += made.glyph_uploads;
        self.image_uploads += made.image_uploads;
        self.canvas_rasterizations += made.canvas_rasterizations;
        self.canvas_quads += made.canvas_quads;
        self.svg_rasterizations += made.svg_rasterizations;
        self.geometry_rebuilds += made.geometry_rebuilds;
    }
}
/// A backdrop filter's bounds, effects and corner radii (`Draw::blur`).
type Backdrop = (
    Rect,
    zgx::scene::Effects,
    [f32; 4],
    [Option<zgx::scene::RoundRect>; 2],
);
/// What a draw's quads sample: the glyph atlas, an image (or the effect
/// atlas) by id, or a cached layer. One of them, never two.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextureSource {
    Atlas,
    Image(u64),
    /// An image sampled pixelated (`Scene::image_rendering`): its nearest
    /// texel, through a sampler of its own (`NearestBinds`; Metal's and
    /// Direct3D 11's frames bind theirs).
    Pixelated(u64),
    Layer(NodeId),
}
#[derive(Clone)]
struct Draw {
    /// How its quads are drawn; set by `push_draw`.
    shading: Shading,
    /// A backdrop filter: its bounds, effects and corner radii. Boxed: few
    /// draws have one, and a frame copies every draw a few times.
    blur: Option<Box<Backdrop>>,
    start: u32,
    end: u32,
    clip: Rect,
    /// Union of this draw's quads: nothing outside it is ever touched.
    bounds: Rect,
    source: TextureSource,
}
/// A frame's damage, encoded at presentation so it can be drawn into the
/// next retained target and the drawable in one pass (see `present_single_pass`).
struct Deferred {
    vertices: wgpu::Buffer,
    draws: Vec<Draw>,
    regions: Vec<(Rect, (u32, u32, u32, u32))>,
    viewport: Option<wgpu::BindGroup>,
    /// The last filter's vertical half, drawn before `draws`.
    blur: Option<VerticalBlur>,
    /// Clear each region first. False for the tail after a backdrop filter:
    /// the passes before it already cleared and drew the regions.
    clear: bool,
    /// No two regions share a pixel (`merge_damage`), so every clear can
    /// precede every draw without redrawing an overlap twice.
    disjoint: bool,
    /// Per region, the first quad to draw (see `occluded_from`).
    first: Vec<u32>,
    /// Draw into the drawable alone, leaving the retained target as it was
    /// (the next frame then repaints everything): the window changed whole
    /// this frame and the last, so the target is likely to be repainted whole
    /// again before any of it is reused. Saves writing every pixel twice.
    direct: bool,
    /// The polyline strokes' points (group 2), if it draws any.
    points: Option<wgpu::BindGroup>,
    /// Drawn from the kept frame, its instances in paint order up to this
    /// (`kept_ordered`): batched as the kept frame's are when encoded, after
    /// the kept frame has taken its upload back.
    kept: Option<u32>,
}
/// A batch of backdrop filters' last step: each blur drawn back over the
/// original pixels, one draw per filter and damaged output (see
/// `GpuRenderer::blur`).
struct VerticalBlur {
    bind: wgpu::BindGroup,
    /// The filter (its index in the batch) and output of each draw.
    draws: Vec<(u32, Scissor)>,
    /// Drawn with `BlurPipelines::mix` (else over a copy of each area).
    mixed: bool,
}
/// The blur drawn back by dual-source blending: the blur scaled by how much
/// of it shows, and that amount for the target's share.
pub(crate) const BLUR_MIX: &str = "struct Mixed { @location(0) @blend_src(0) color:vec4<f32>, @location(0) @blend_src(1) amount:vec4<f32> }
@fragment fn fs_mix(v:Vertex)->Mixed{let b=blurs[v.index];let blend=blurred(b,v.p.xy);let a=amount(b,v.p.xy)*blend.weight;return Mixed(blend.color*a,vec4(a));}";
/// A filter's header in `blur.wgsl` (`Blur`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurRecord {
    bounds: [f32; 4],
    effect: [f32; 4],
    area: [f32; 4],
    shape: [f32; 4],
    slots: [u32; 4],
    round0: [f32; 4],
    radii0: [f32; 4],
    round1: [f32; 4],
    radii1: [f32; 4],
}
/// A slot of the blur atlas (`Slot` in `blur.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SlotRecord {
    atlas: [f32; 4],
    region: [f32; 4],
    owner: [u32; 4],
}
/// A batch of backdrop filters planned for the atlas (`GpuRenderer::plan_blur`).
struct BlurPlan {
    blurs: Vec<BlurRecord>,
    /// Each slot with its size in texels and whether it is blurred (else a
    /// copy of an area).
    slots: Vec<(SlotRecord, (u32, u32), bool)>,
    /// The filter and output of each draw back.
    draws: Vec<(u32, Scissor)>,
    mixed: bool,
}
/// A backdrop filter a damaged region reaches: its bounds and clip, the
/// outputs it draws, its effects and corner radii.
struct Filter {
    bounds: Rect,
    clip: Rect,
    outputs: Vec<Rect>,
    effects: zgx::scene::Effects,
    corners: [f32; 4],
    /// The rounded clips around it.
    rounded: [Option<zgx::scene::RoundRect>; 2],
}
/// Backdrop blur pipelines (blur.wgsl): average each area down into its
/// atlas slots, blur them across and down at that size, then draw them back
/// over the target, mixed by dual-source blending (`mix`) where the device
/// has it, else over a copy of the area's pixels, into the target or, at
/// presentation, the target and a drawable of `both`'s format.
struct BlurPipelines {
    shader: wgpu::ShaderModule,
    /// Shared by every pipeline so one bind group layout serves each.
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    reduce: wgpu::RenderPipeline,
    across: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    vertical: wgpu::RenderPipeline,
    mix: Option<wgpu::RenderPipeline>,
    both: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
}
impl BlurPipelines {
    fn new(device: &wgpu::Device) -> Self {
        // Dual-source blending needs the WGSL extension enabled for the whole
        // module, which only devices with the feature accept.
        let dual = device
            .features()
            .contains(wgpu::Features::DUAL_SOURCE_BLENDING);
        let source = if dual {
            format!(
                "enable dual_source_blending;\n{}\n{BLUR_MIX}",
                include_str!("blur.wgsl")
            )
        } else {
            include_str!("blur.wgsl").to_owned()
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("backdrop blur"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let records = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop blur"),
            entries: &[
                texture(0),
                texture(1),
                records(2),
                records(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline = |entry, format| {
            let target = Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            });
            Self::pipeline(device, &shader, Some(&layout), entry, &[target])
        };
        let (reduce, across, down, vertical) = (
            pipeline("fs_reduce", BLUR_FORMAT),
            pipeline("fs_across", BLUR_FORMAT),
            pipeline("fs_down", BLUR_FORMAT),
            pipeline("fs", FORMAT),
        );
        // Bilinear: each sample averages up to four texels (see blur.wgsl).
        let linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("backdrop blur"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // out = blur * amount + target * (1 - amount), every channel.
        let mix = dual.then(|| {
            let component = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                operation: wgpu::BlendOperation::Add,
            };
            let target = Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: Some(wgpu::BlendState {
                    color: component,
                    alpha: component,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            });
            Self::pipeline(device, &shader, Some(&layout), "fs_mix", &[target])
        });
        Self {
            shader,
            layout,
            linear,
            reduce,
            across,
            down,
            vertical,
            mix,
            both: None,
        }
    }
    fn pipeline(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        layout: Option<&wgpu::BindGroupLayout>,
        entry: &str,
        targets: &[Option<wgpu::ColorTargetState>],
    ) -> wgpu::RenderPipeline {
        let layout = layout.map(|layout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("vertical blur"),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            })
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("backdrop blur"),
            layout: layout.as_ref(),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets,
            }),
            multiview_mask: None,
            cache: None,
        })
    }
    /// The pipeline drawing `blur` back over the target.
    fn drawing(&self, blur: &VerticalBlur) -> &wgpu::RenderPipeline {
        match &self.mix {
            Some(mix) if blur.mixed => mix,
            _ => &self.vertical,
        }
    }
    /// The vertical pipeline writing the target and a `format` drawable.
    fn both(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        if self.both.as_ref().is_none_or(|(f, _)| *f != format) {
            let target = |format| {
                Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })
            };
            let pipeline = Self::pipeline(
                device,
                &self.shader,
                Some(&self.layout),
                "fs_both",
                &[target(FORMAT), target(format)],
            );
            self.both = Some((format, pipeline));
        }
        &self.both.as_ref().expect("created above").1
    }
}
/// Backdrop filters' scratch: two atlases at least the size a batch's slots
/// need, the blur passes rendering from one into the other.
struct BlurScratch {
    atlas: [wgpu::Texture; 2],
}
/// The spare retained target with the views and bind groups it needs.
struct Retained {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    copy_bind: Option<wgpu::BindGroup>,
    blit_bind: Option<wgpu::BindGroup>,
}
/// Pipelines writing the retained target and a drawable of `format` at once.
struct SinglePass {
    format: wgpu::TextureFormat,
    copy: wgpu::RenderPipeline,
    /// Into the next retained target and the drawable.
    both: PickedPipelines,
    /// Into the drawable alone (`Deferred::direct`).
    direct: PickedPipelines,
}
/// A surface of the renderer's own a window presents through.
#[cfg(target_os = "macos")]
type OwnSurface = layer_present::Buffer;
#[cfg(windows)]
type OwnSurface = dxgi_present::Buffer;
/// Presentation through surfaces of the renderer's own, which keep what they
/// held: macOS's (`layer_present`) and Windows' swap chain
/// (`dxgi_present`).
#[cfg(own_surfaces)]
#[derive(Default)]
struct LayerPresent {
    #[cfg(target_os = "macos")]
    layer: Option<layer_present::Layer>,
    /// The caret the host draws over the frames (`GpuRenderer::set_caret`),
    /// for the layer made after it is set.
    #[cfg(target_os = "macos")]
    caret: Option<layer_present::Caret>,
    /// Windows: the swap chain whose buffers are the surfaces.
    #[cfg(windows)]
    chain: Option<dxgi_present::Chain>,
    /// Surfaces, each with the frame it holds (None: none yet).
    buffers: Vec<(OwnSurface, Option<u64>)>,
    /// The surface the layer shows: the last presented frame.
    shown: Option<usize>,
    /// The surface the next frame draws into (the retained target).
    current: Option<usize>,
    /// Frames presented so far.
    frame: u64,
    /// Pixels each recent frame changed, oldest first.
    history: std::collections::VecDeque<(u64, Vec<Scissor>)>,
    /// Pixels the frame being drawn changes so far.
    changed: Vec<Scissor>,
    /// Regions the current surface takes from the shown one before the
    /// frame draws (`begin_layer_frame`).
    catch_up: Vec<Scissor>,
    /// Frames come at the display's full rate (`GpuRenderer::set_full_rate`):
    /// a third surface is kept for them, and the layer queues them.
    #[cfg(target_os = "macos")]
    full_rate: bool,
    /// Since full-rate frames stopped, until when the video layer stays
    /// (`VIDEO_LINGER`); `None` while they come, or once it went.
    #[cfg(target_os = "macos")]
    video_until: Option<std::time::Instant>,
    /// When the window last changed size. Frames show on the plain layer
    /// until `RESIZE_SETTLE` after it (`Layer::show_plain`).
    #[cfg(target_os = "macos")]
    resized_at: Option<std::time::Instant>,
    /// The layer to present into instead of the window's (an overlay plane).
    #[cfg(target_os = "macos")]
    host: Option<objc2::rc::Retained<objc2_quartz_core::CALayer>>,
    /// Frames show on the plain layer until then (`present_plain_for`), or
    /// always (`plain_only`): they change in the caller's transaction.
    #[cfg(target_os = "macos")]
    plain_until: Option<std::time::Instant>,
    #[cfg(target_os = "macos")]
    plain_only: bool,
    /// Native views share the window (`present_with_native_views`): frames
    /// show on the plain layer, in the transaction that moves them.
    #[cfg(target_os = "macos")]
    with_natives: bool,
    /// Full-rate frames draw into drawables (`PRESENT_DRAWABLE`, or a test's
    /// `debug_present_drawable`).
    #[cfg(target_os = "macos")]
    drawables: Option<bool>,
    /// A scroll is live in the window (`GpuRenderer::set_scrolling`): with
    /// `SCROLL_DRAWABLES`, its full-rate frames draw into drawables.
    #[cfg(target_os = "macos")]
    scrolling: bool,
    /// Where this window is in the full-rate paths' trial (`PRESENT_AUTO`).
    #[cfg(target_os = "macos")]
    trial: Trial,
    /// When the trial started, for its cost (`Trial`).
    #[cfg(target_os = "macos")]
    trial_started: Option<std::time::Instant>,
    /// The energy terms after each half's warm-up and at its end
    /// (`trial_energy`), each read off the main thread: a scan of the I/O
    /// Registry takes ~28 ms (M5 Max), which there made a frame late.
    #[cfg(target_os = "macos")]
    trial_samples: Vec<std::thread::JoinHandle<trial_energy::Sample>>,
    /// The machine the verdict is for (GPU, display and refresh, OS build),
    /// and its verdict (`verdicts`): 0 untried, 1 tiles, 2 drawables.
    #[cfg(target_os = "macos")]
    verdict_key: String,
    #[cfg(target_os = "macos")]
    verdict: u8,
}
#[cfg(own_surfaces)]
impl LayerPresent {
    /// What surface `index` lacks of the latest frame before a frame draws
    /// into it: the changes of every frame since the one it holds (all of it
    /// once those are no longer kept); none when the frame repaints
    /// everything (`whole`) or it holds the latest.
    fn catch_up_for(&self, index: usize, whole: bool, width: u32, height: u32) -> Vec<Scissor> {
        let held = self.buffers[index].1;
        let latest = self.shown.and_then(|shown| self.buffers[shown].1);
        catch_up(&self.history, held, latest, whole, (width, height))
    }
    /// Surface `index` was shown: it holds the newest frame, whose changes
    /// join the history.
    fn presented(&mut self, index: usize) {
        self.frame += 1;
        let frame = self.frame;
        self.buffers[index].1 = Some(frame);
        self.shown = Some(index);
        let changed = std::mem::take(&mut self.changed);
        self.history.push_back((frame, changed));
        if self.history.len() > LAYER_HISTORY {
            self.history.pop_front();
        }
    }
}
/// What a surface holding frame `held` lacks of frame `latest` (the one
/// shown), from `history`, the changes of recent frames: see
/// `LayerPresent::catch_up_for`. All of a `size` surface once the frames
/// after `held` are no longer kept.
#[cfg(own_surfaces)]
fn catch_up(
    history: &std::collections::VecDeque<(u64, Vec<Scissor>)>,
    held: Option<u64>,
    latest: Option<u64>,
    whole: bool,
    (width, height): (u32, u32),
) -> Vec<Scissor> {
    match (whole, latest) {
        (true, _) | (false, None) => Vec::new(),
        (false, Some(latest)) if held == Some(latest) => Vec::new(),
        (false, Some(_)) => match held {
            // The changes of every frame since the one it holds.
            Some(held) if history.front().is_some_and(|(f, _)| *f <= held + 1) => history
                .iter()
                .filter(|(frame, _)| *frame > held)
                .flat_map(|(_, changed)| changed.iter().copied())
                .collect(),
            _ => vec![(0, 0, width, height)],
        },
    }
}
#[cfg(all(test, own_surfaces))]
mod catch_up_tests {
    use super::catch_up;
    #[test]
    fn a_surface_takes_the_changes_since_the_frame_it_holds() {
        let history: std::collections::VecDeque<_> = [
            (3, vec![(0, 0, 4, 4)]),
            (4, vec![(10, 0, 4, 4)]),
            (5, vec![(20, 0, 4, 4), (30, 0, 2, 2)]),
        ]
        .into();
        let size = (100, 50);
        // Two frames behind: the changes of 4 and 5.
        assert_eq!(
            catch_up(&history, Some(3), Some(5), false, size),
            [(10, 0, 4, 4), (20, 0, 4, 4), (30, 0, 2, 2)]
        );
        // Holding the shown frame, or drawn whole: nothing to take.
        assert!(catch_up(&history, Some(5), Some(5), false, size).is_empty());
        assert!(catch_up(&history, Some(3), Some(5), true, size).is_empty());
        // None shown yet: the frame draws whole.
        assert!(catch_up(&history, None, None, false, size).is_empty());
        // Never drawn, or behind what the history keeps: all of it.
        assert_eq!(
            catch_up(&history, None, Some(5), false, size),
            [(0, 0, 100, 50)]
        );
        assert_eq!(
            catch_up(&history, Some(1), Some(5), false, size),
            [(0, 0, 100, 50)]
        );
        // Just inside it: the frame after the one held is the oldest kept.
        assert_eq!(
            catch_up(&history, Some(2), Some(5), false, size).len(),
            4,
            "frames 3, 4 and 5"
        );
    }
}
/// Windows: the swap chain's buffers go before the chain they are of
/// (`dxgi_present::Chain::release`).
#[cfg(windows)]
impl Drop for LayerPresent {
    fn drop(&mut self) {
        if let Some(chain) = &self.chain {
            chain.release(self.buffers.drain(..).map(|(buffer, _)| buffer));
        }
    }
}
#[cfg(target_os = "macos")]
impl LayerPresent {
    /// Whether the next frame draws into a drawable: full rate, and nothing
    /// asking for the plain layer (as `present_layer` decides it).
    fn drawable_frame(&self) -> bool {
        let auto = self.drawables.is_none()
            && *PRESENT_AUTO
            && self.full_rate
            && match self.verdict {
                2 => true,
                1 => false,
                _ => matches!(self.trial, Trial::Drawables { .. }),
            };
        // Opted in, a live scroll's frames present themselves: Metal shows
        // each once its GPU work is done, where a tile is shown only after
        // the main thread waited for that work (`finish_direct`), the next
        // frame's input unread meanwhile. Not during the trial, which
        // measures the paths as they are.
        let scroll = self.scrolling
            && *SCROLL_DRAWABLES
            && !matches!(self.trial, Trial::Tiles { .. } | Trial::Drawables { .. });
        (auto || self.drawables.unwrap_or(*PRESENT_DRAWABLE || scroll) && self.full_rate)
            && !self.plain_only
            && !self.with_natives
            && self
                .resized_at
                .is_none_or(|at| at.elapsed() >= RESIZE_SETTLE)
            && self
                .plain_until
                .is_none_or(|until| std::time::Instant::now() >= until)
    }
    /// Keep the buffers `keep` says, the shown one's index following.
    fn retain_buffers(&mut self, keep: impl Fn(usize, &layer_present::Buffer) -> bool) {
        let mut index = 0;
        let mut shown = self.shown;
        let mut kept = 0;
        self.buffers.retain(|(buffer, _)| {
            let here = index;
            index += 1;
            let keep = Some(here) == self.shown || keep(here, buffer);
            if keep {
                if Some(here) == self.shown {
                    shown = Some(kept);
                }
                kept += 1;
            }
            keep
        });
        self.shown = shown;
    }
    /// Attach the layer frames show through, if not yet: `host`'s, or the
    /// window surface's.
    fn attach(&mut self, surface: Option<&wgpu::Surface<'static>>) -> Result<(), GpuError> {
        if self.layer.is_none()
            && let Some(host) = &self.host
        {
            self.layer = Some(layer_present::Layer::attach(host));
        }
        if self.layer.is_none() {
            let surface = surface.ok_or_else(|| GpuError("a window surface".into()))?;
            // SAFETY: Only the surface's layer is used; nothing destroys it.
            let hal = unsafe { surface.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or_else(|| GpuError("not a Metal surface".into()))?;
            self.layer = Some(layer_present::Layer::attach(&hal.render_layer().lock()));
        }
        Ok(())
    }
    /// For `ZGX_PRESENT_LOG`: which plane this presents.
    fn plane(&self) -> &'static str {
        if self.plain_only { "overlay" } else { "base" }
    }
    /// Per surface, whether the layer may still show it: the one on screen
    /// and any queued after it. Without word from the layer, the two latest.
    fn busy(&self) -> Vec<bool> {
        let displayed = self.layer.as_ref().and_then(|layer| layer.displayed());
        let on_screen = displayed.and_then(|id| {
            self.buffers
                .iter()
                .find(|(buffer, _)| buffer.surface.id() == id)
                .and_then(|(_, frame)| *frame)
        });
        let latest = self.frame;
        self.buffers
            .iter()
            .map(|(_, held)| match (held, on_screen) {
                (None, _) => false,
                (Some(held), Some(on_screen)) => *held >= on_screen,
                (Some(held), None) => held + 2 > latest,
            })
            .collect()
    }
}
/// Surfaces a window presents through at most: one on screen, one queued,
/// and one drawn.
#[cfg(target_os = "macos")]
const LAYER_BUFFERS: usize = 3;
/// Frames of changes kept to bring a surface up to date (older ones copy all).
#[cfg(own_surfaces)]
const LAYER_HISTORY: usize = 8;
/// How long after a size change frames keep to the plain layer, so a live
/// resize's frames land with its window sizes before queueing resumes.
#[cfg(target_os = "macos")]
const RESIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);
/// `ZGX_PRESENT_VIDEO`: frames at the display's full rate are queued on a
/// video layer (`AVSampleBufferDisplayLayer`) instead of shown as the plain
/// layer's tiles, as they were until tiles were measured better (2026-10-25,
/// animate at 120 Hz: CPU 4.84% → 4.03%, memory 253 → 242 MB, the 99th
/// percentile frame interval 20.6 → 17.7 ms, and 13.6 ms with the host
/// aligning frames to refreshes; GPU power and the window server's GPU the
/// same). Kept for a release, to compare with.
#[cfg(target_os = "macos")]
static VIDEO_AT_FULL_RATE: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| zgx::env::var_os("ZGX_PRESENT_VIDEO").is_some());
/// `ZGX_GPU_WARM=0`: the first input after a window idled (`trim`) no longer
/// wakes the GPU ahead of its frame (`GpuRenderer::warm_up`). Measured
/// (2026-10-26, the mail example after idle, 8 gestures a cell): the first
/// frame's present 6.7 → 1.7 ms, the second frame's time and the CPU and
/// instructions per frame shown, at rest and animating, unchanged.
#[cfg(target_os = "macos")]
static WARM_AFTER_IDLE: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| zgx::env::var_os("ZGX_GPU_WARM").is_none_or(|v| v != "0"));
/// `ZGX_DXGI_PRESENT=0`: a Windows window presents through wgpu's swap
/// chain, copying the retained frame into it each frame, as it did until
/// its own was measured better (2026-10-26, RTX 4090, against wgpu's:
/// app GPU 33 to 45% less in animate, vlist, lab, 12% in heavy; the
/// compositor's GPU 16% less in animate and its CPU 7 to 25% less; cycles
/// a frame 2 to 5% fewer; startup to the first text 346 → 280 ms). Kept
/// for a release, to compare with.
#[cfg(windows)]
static DXGI_PRESENT: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| zgx::env::var_os("ZGX_DXGI_PRESENT").is_none_or(|v| v != "0"));
/// `ZGX_PRESENT_DRAWABLE`: full-rate frames draw into `CAMetalLayer`
/// drawables, which Metal presents (`layer_present::Layer::next_drawable`),
/// instead of surfaces of the renderer's own shown as tiles.
#[cfg(target_os = "macos")]
static PRESENT_DRAWABLE: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
    zgx::env::var_os("ZGX_PRESENT_DRAWABLE").is_some_and(|v| v != "0" && v != "auto")
});
/// `ZGX_SCROLL_DRAWABLES=1`: a live scroll's full-rate frames draw into
/// drawables, which Metal presents once their GPU work is done, so the main
/// thread doesn't wait for that work (`LayerPresent::scrolling`). Off, they
/// show as tiles, as at rest. Measured (M1 Max at 120 Hz, a heavy chat):
/// drawables showed 84 of 86 refreshes, but 32 ms from a frame's start to
/// its showing at the median (p99 45), about twice a tile's: opt in, until
/// that is understood.
#[cfg(target_os = "macos")]
static SCROLL_DRAWABLES: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| zgx::env::var_os("ZGX_SCROLL_DRAWABLES").is_some_and(|v| v != "0"));
/// `ZGX_PRESENT_DRAWABLE=auto`: full-rate frames take whichever path costs
/// less on this machine (`TrialRule`), tried once per machine (`verdicts`);
/// slower frames stay tiles.
#[cfg(target_os = "macos")]
static PRESENT_AUTO: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
    zgx::env::var_os("ZGX_PRESENT_DRAWABLE").is_some_and(|v| v == "auto")
});
/// The trial's verdicts (1 tiles, 2 drawables), by machine key (`verdict_key`):
/// kept in the app's defaults, so a machine is tried once, not every launch.
/// `ZGX_PRESENT_RETRIAL` tries again (and stores the new verdict).
#[cfg(target_os = "macos")]
mod verdicts {
    use objc2_foundation::{NSString, NSUserDefaults};
    static CACHE: std::sync::Mutex<Vec<(String, u8)>> = std::sync::Mutex::new(Vec::new());
    fn name(key: &str) -> objc2::rc::Retained<NSString> {
        NSString::from_str(&format!("zgx.presentPath.{key}"))
    }
    /// The verdict for `key`: 0 not tried here yet.
    pub(crate) fn get(key: &str) -> u8 {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, verdict)) = cache.iter().find(|(k, _)| k == key) {
            return *verdict;
        }
        let verdict = if zgx::env::var_os("ZGX_PRESENT_RETRIAL").is_some() {
            0
        } else {
            match NSUserDefaults::standardUserDefaults()
                .stringForKey(&name(key))
                .map(|v| v.to_string())
                .as_deref()
            {
                Some("tiles") => 1,
                Some("drawables") => 2,
                _ => 0,
            }
        };
        cache.push((key.to_owned(), verdict));
        verdict
    }
    /// Keep `verdict` for `key`, here and in the defaults.
    pub(crate) fn set(key: &str, verdict: u8) {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|(k, _)| k != key);
        cache.push((key.to_owned(), verdict));
        let value = NSString::from_str(if verdict == 2 { "drawables" } else { "tiles" });
        // SAFETY: An NSString value under an NSString key, as defaults store.
        unsafe {
            NSUserDefaults::standardUserDefaults().setObject_forKey(Some(&value), &name(key))
        };
    }
    /// The OS's version and build ("Version 26.0.1 (Build 25A362)").
    pub(crate) fn os() -> String {
        objc2_foundation::NSProcessInfo::processInfo()
            .operatingSystemVersionString()
            .to_string()
    }
}
/// Full-rate frames each half of the trial lasts (about two seconds at 120 Hz).
#[cfg(target_os = "macos")]
const TRIAL_FRAMES: u32 = 240;
/// How much smaller a share of frames drawables must skip than tiles to win
/// (`TrialRule::Frames`).
#[cfg(target_os = "macos")]
const TRIAL_MARGIN: f64 = 0.05;
/// Frames each half of the trial runs before it counts (a quarter second at
/// 120 Hz): drawables' layer and pool are made as their half starts.
#[cfg(target_os = "macos")]
const TRIAL_WARMUP: u32 = 30;
/// What a second of GPU work weighs against the CPU's joules
/// (`TrialRule::Energy`, uncalibrated: see `TrialRule`).
#[cfg(target_os = "macos")]
const TRIAL_GPU_WATTS: f64 = 5.;
/// How much less energy drawables must cost than tiles to win
/// (`TrialRule::Energy`): tiles stay unless drawables clearly save. The
/// window server's CPU is no term (another user's process; it reads only
/// for a setuid tool), and on an M5 Max it is what favours tiles.
#[cfg(target_os = "macos")]
const TRIAL_ENERGY_MARGIN: f64 = 0.1;
/// The most frames drawables may skip or show late after warm-up to win by
/// energy. When they pace well they lose none of 210 (M5 Max), or one; when
/// they present as soon as their work is done on an M1 Max at 120 Hz, about
/// 1% come a refresh late (2026-10-27).
#[cfg(target_os = "macos")]
const TRIAL_LOSS_BOUND: f64 = 0.005;
/// How the full-rate trial judges (`ZGX_TRIAL_RULE`): by the share of
/// frames each path skips (the default), or `energy`, which is not
/// calibrated. Energy needs the GPU's power, and a process can read only its
/// busy time: what a busy second costs depends on the GPU's clock, 0.78 W on
/// an M5 Max against 4.44 on an M1 Max (2026-10-28, fitted from system GPU
/// power and busy time, tiles against drawables), so no one weight holds.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq)]
enum TrialRule {
    Energy,
    Frames,
}
/// The trial's energy terms now, read off this thread, if it judges by
/// energy (`trial_energy`; a scan of the I/O Registry takes ~28 ms).
#[cfg(target_os = "macos")]
fn energy_sample() -> Option<std::thread::JoinHandle<trial_energy::Sample>> {
    (*TRIAL_RULE == TrialRule::Energy).then(|| std::thread::spawn(trial_energy::Sample::now))
}
#[cfg(target_os = "macos")]
static TRIAL_RULE: std::sync::LazyLock<TrialRule> = std::sync::LazyLock::new(|| {
    match zgx::env::var_os("ZGX_TRIAL_RULE").is_some_and(|v| v == "energy") {
        true => TrialRule::Energy,
        false => TrialRule::Frames,
    }
});
/// What the trial measured of each path: shares of frames skipped (tiles'
/// only with the probe, `TrialRule::Frames`), and watts (`trial_energy`;
/// none if a sample is missing).
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, Default)]
struct TrialResult {
    tiles_lost: f64,
    drawables_lost: f64,
    /// Drawables' share skipped after their warm-up, and shown late.
    drawables_lost_settled: f64,
    drawables_late_settled: f64,
    tiles_watts: Option<f64>,
    drawables_watts: Option<f64>,
}
/// The trial's verdict: 2 drawables, 1 tiles.
#[cfg(target_os = "macos")]
fn trial_verdict(rule: TrialRule, result: TrialResult) -> u8 {
    let drawables = match rule {
        TrialRule::Frames => result.tiles_lost - result.drawables_lost > TRIAL_MARGIN,
        TrialRule::Energy => match (result.tiles_watts, result.drawables_watts) {
            (Some(tiles), Some(drawables)) => {
                drawables < tiles * (1. - TRIAL_ENERGY_MARGIN)
                    && result.drawables_lost_settled + result.drawables_late_settled
                        <= TRIAL_LOSS_BOUND
            }
            _ => false,
        },
    };
    if drawables { 2 } else { 1 }
}
#[cfg(all(test, target_os = "macos"))]
mod trial_verdict_tests {
    use super::*;
    fn energy(tiles: f64, drawables: f64, lost: f64) -> u8 {
        trial_verdict(
            TrialRule::Energy,
            TrialResult {
                drawables_lost_settled: lost,
                tiles_watts: Some(tiles),
                drawables_watts: Some(drawables),
                ..Default::default()
            },
        )
    }
    #[test]
    fn drawables_win_by_energy_only_beyond_the_margin() {
        // An M1 Max (2026-10-27): tiles 0.71 W, drawables 0.55.
        assert_eq!(energy(0.710, 0.550, 0.001), 2);
        // An M5 Max: tiles 0.892 W, drawables 0.830, under 10% less.
        assert_eq!(energy(0.892, 0.830, 0.001), 1);
        assert_eq!(energy(0.5, 0.5, 0.), 1);
    }
    #[test]
    fn drawables_that_lose_frames_never_win_by_energy() {
        assert_eq!(energy(0.710, 0.550, TRIAL_LOSS_BOUND), 2);
        assert_eq!(energy(0.710, 0.550, 0.02), 1);
    }
    #[test]
    fn drawables_that_pace_late_never_win_by_energy() {
        // An M1 Max presenting drawables as soon as their work is done: about
        // one frame in a hundred a refresh late.
        let late = |share| {
            trial_verdict(
                TrialRule::Energy,
                TrialResult {
                    drawables_late_settled: share,
                    tiles_watts: Some(0.710),
                    drawables_watts: Some(0.550),
                    ..Default::default()
                },
            )
        };
        assert_eq!(late(0.009), 1);
        assert_eq!(late(0.0), 2);
    }
    #[test]
    fn an_energy_trial_missing_a_sample_keeps_tiles() {
        let missing = TrialResult {
            tiles_watts: Some(0.7),
            ..Default::default()
        };
        assert_eq!(trial_verdict(TrialRule::Energy, missing), 1);
    }
    #[test]
    fn by_frames_drawables_win_where_tiles_skip_clearly_more() {
        let frames = |tiles_lost, drawables_lost| {
            trial_verdict(
                TrialRule::Frames,
                TrialResult {
                    tiles_lost,
                    drawables_lost,
                    ..Default::default()
                },
            )
        };
        assert_eq!(frames(0.17, 0.003), 2);
        assert_eq!(frames(0.06, 0.04), 1);
    }
}
/// The once-per-process comparison of the full-rate paths (`PRESENT_AUTO`):
/// tiles, then drawables with their presented feedback, each for
/// `TRIAL_FRAMES`; judged by `TrialRule`.
#[cfg(target_os = "macos")]
#[derive(Default, Clone, Copy)]
enum Trial {
    #[default]
    Untried,
    Tiles {
        frames: u32,
        from: layer_present::Mark,
    },
    Drawables {
        frames: u32,
        from: layer_present::Mark,
        /// Where drawables' feedback stood once `TRIAL_WARMUP` of them had
        /// shown: by presented frames, which come a few frames after those
        /// handed over (the switch to drawables' clock lands in the first).
        settled: Option<layer_present::Mark>,
        tiles: f64,
        /// At `TRIAL_FRAMES`: the shares drawables lost, lost after warm-up
        /// and showed late, while the last sample is read.
        ended: Option<(f64, f64, f64)>,
    },
    Done,
}
/// How long the video layer outlasts full-rate frames (`LayerPresent::video_until`).
#[cfg(target_os = "macos")]
const VIDEO_LINGER: std::time::Duration = std::time::Duration::from_secs(1);
/// A pass's pipelines for each shading, and the one clearing a region.
struct PickedPipelines {
    full: wgpu::RenderPipeline,
    clear: wgpu::RenderPipeline,
    basic: wgpu::RenderPipeline,
    opaque: wgpu::RenderPipeline,
    /// Made when a frame first draws polylines (`picks`).
    polyline: std::cell::OnceCell<wgpu::RenderPipeline>,
}
impl PickedPipelines {
    /// Its pipelines, the polyline one made first if `make` and `polylines`.
    fn picks(&self, polylines: bool, make: impl FnOnce() -> wgpu::RenderPipeline) -> Picks<'_> {
        Picks {
            full: &self.full,
            clear: &self.clear,
            basic: &self.basic,
            opaque: &self.opaque,
            polyline: polylines.then(|| self.polyline.get_or_init(make)),
            quads: None,
        }
    }
}
/// Pipelines for each shading, and the one clearing a region (see
/// `GpuRenderer::encode_damage`).
#[derive(Clone, Copy)]
struct Picks<'p> {
    full: &'p wgpu::RenderPipeline,
    clear: &'p wgpu::RenderPipeline,
    basic: &'p wgpu::RenderPipeline,
    opaque: &'p wgpu::RenderPipeline,
    /// For frames drawing polylines.
    polyline: Option<&'p wgpu::RenderPipeline>,
    /// The rest, made as they're first drawn (a layer's frames).
    quads: Option<&'p QuadPipelines>,
}
impl<'p> Picks<'p> {
    fn get(&self, shading: Shading) -> &'p wgpu::RenderPipeline {
        match shading {
            Shading::Full
            | Shading::Fill(_)
            | Shading::Filter(_)
            | Shading::Blend
            | Shading::FilterBlend(_)
            | Shading::ImageEffect => self.full,
            Shading::Basic => self.basic,
            Shading::Opaque => self.opaque,
            Shading::Polyline => self.polyline.expect("made for a frame with polylines"),
            // Drawn in a gradient text's layer alone (`gradient_text_quads`).
            Shading::SourceIn | Shading::Behind => {
                self.quads.expect("drawn into layers").get(shading)
            }
        }
    }
}
/// A run of draws encoded as one instanced call.
struct Batch {
    rect: Rect,
    source: TextureSource,
    shading: Shading,
    start: u32,
    end: u32,
    /// Device pixels its draws cover, and of them those drawn with a
    /// costlier shader than their own (see `merged_shading`).
    pixels: f32,
    promoted: f32,
}
/// Whether `draw`, scissored to `rect` (the region within its clip), can
/// join a batch of its shading scissored to `batch`, under the batch's: a
/// draw touches no pixel past its bounds (its quads' rasterized
/// rectangles), so lying uncut within both it draws the same pixels under
/// either. In a scroll view the rows wholly inside it (scissored to the
/// window) then join the rows its edge cuts (to the view). Of one shading
/// only: promoted into another's across clips (`merged_shading`), its
/// pixels took the full shader where they had their own, and the GPU's
/// cycles rose 3%. Not a layer's composite (in perspective its quad
/// reaches past `quad_rect`) nor a backdrop filter's draw. (Within in
/// logical units is within once `scissor` rounds both out to device
/// pixels.)
#[inline(never)]
fn scissors_alike(batch: Rect, rect: Rect, draw: &Draw) -> bool {
    rect_inside(draw.bounds, rect)
        && rect_inside(draw.bounds, batch)
        && draw.blur.is_none()
        && !matches!(draw.source, TextureSource::Layer(_))
}
/// Device pixels a batch may draw with a costlier shader than they need, to
/// join neighbours of another shading into one draw call: a draw costs the
/// driver thousands of instructions, shading a small area costs little.
/// Blending opaque quads costs the GPU little more.
const PROMOTED_PIXELS: f32 = 16_384.;
/// As `PROMOTED_PIXELS`, into the full shader, whose cost per pixel is a
/// multiple of the basic one's: at 16K a batch, text promoted beside its
/// rounded panels raised the heavy dashboard's GPU cycles 5 to 8%. Icons,
/// dots and chips still join. (An early return in `shade` for plain quads,
/// computing `basic`'s colour, does not make promotion cheap: the cost is
/// the full shader's registers and flat varyings, which a branch keeps,
/// and the test slowed the quads that need it. Measured: headless GPU time
/// per frame, heavy stream +3.5% with promotion, heavy both's own +1.3%.)
const PROMOTED_FULL_PIXELS: f32 = 1024.;
/// The shading drawing both `a` and `b`, if one can: the full shader draws
/// every basic quad, and blending every opaque one, to the same bytes (see
/// `shading`). Shader fills and image effects keep their own pipelines.
fn merged_shading(a: Shading, b: Shading) -> Option<Shading> {
    let cost = |shading| match shading {
        Shading::Opaque => Some(0),
        Shading::Basic => Some(1),
        Shading::Full => Some(2),
        Shading::Fill(_)
        | Shading::Filter(_)
        | Shading::Blend
        | Shading::FilterBlend(_)
        | Shading::ImageEffect
        | Shading::Polyline
        | Shading::SourceIn
        | Shading::Behind => None,
    };
    Some(if cost(a)? >= cost(b)? { a } else { b })
}
/// Whether `quad`, drawn without its region (`mask.w`), would change
/// pixels at `rect` without it. (Out of line: `Region::covers` stays inlined
/// where items are made.)
#[inline(never)]
fn leaves_region(regions: &[Region], quad: &Quad, rect: Rect, scale: f32) -> bool {
    !regions[quad.mask[3] as usize - 1].covers(rect, scale)
}
fn quad_bounds(quads: &[Quad]) -> Rect {
    quads
        .iter()
        .map(quad_rect)
        .reduce(|a, b| a.union(b))
        .unwrap_or_default()
}
/// A quad's rectangle. Bands of a split panel measure back from its far
/// edge (`push_panel`): a negative width or height.
fn quad_rect(quad: &Quad) -> Rect {
    let [x, y, w, h] = quad.rect;
    let rect = Rect::new(x.min(x + w), y.min(y + h), w.abs(), h.abs());
    if quad.options[2] < 0.5 {
        return rect;
    }
    // Transformed (an affine image or layer): where its corners land.
    let [a, c, b, d] = quad.shape;
    let [tx, ty, ..] = quad.border;
    zgx::affine::Affine { a, b, c, d, tx, ty }.bounds(rect)
}
/// The GPU an interface draws on: the integrated one of a machine with two,
/// which keeps a laptop's discrete GPU asleep.
const POWER: wgpu::PowerPreference = wgpu::PowerPreference::LowPower;
/// `WGPU_FORCE_FALLBACK_ADAPTER=1` (wgpu's own name): the software adapter
/// even where there is a GPU (WARP on Windows, for both Direct3D 12 and
/// 11), as a machine without one draws: to run CI's software lines, or
/// bless a software family of goldens, on a machine with a GPU. Linux takes
/// a software Vulkan driver; macOS has none for Metal, so there the renderer
/// fails to start. Read when a device is made, never per frame.
pub(crate) fn fallback_adapter_forced() -> bool {
    std::env::var_os("WGPU_FORCE_FALLBACK_ADAPTER").is_some_and(|value| value == "1")
}
/// The adapter DXGI ranks first for `POWER`, a GPU before software, with a
/// device opened on it alone (none when Direct3D 12 isn't the instance's
/// backend). wgpu's
/// `request_adapter` opens a Direct3D 12 device on every adapter to rank them,
/// and each driver keeps its memory after the device is released: 20 MiB for
/// a second adapter (an indirect display's) on an NVIDIA machine.
/// With crates.io's wgpu-hal, which lacks it (`zgx_wgpu_hal_patched`, see
/// build.rs), none: `request_adapter` ranks the adapters.
#[cfg(all(windows, not(zgx_wgpu_hal_patched)))]
fn preferred_dx12_adapter(_: &wgpu::Instance) -> Option<wgpu::Adapter> {
    None
}
#[cfg(all(windows, zgx_wgpu_hal_patched))]
fn preferred_dx12_adapter(instance: &wgpu::Instance) -> Option<wgpu::Adapter> {
    // SAFETY: the hal instance is only read (DXGI's adapters listed, one
    // exposed), never destroyed or changed.
    let hal = unsafe { instance.as_hal::<wgpu::hal::api::Dx12>() }?;
    let exposed = hal.expose_preferred_adapter(POWER)?;
    // SAFETY: the adapter was exposed from this instance's own hal instance.
    Some(unsafe { instance.create_adapter_from_hal(exposed) })
}
/// Keep the shaders FXC compiles in the user's local app data between
/// launches (`vendor/wgpu-hal/ZGX_PATCH.md`): a window's first frame
/// compiled its pipelines' shaders every launch, 45 ms of a 280 ms start.
#[cfg(all(windows, zgx_wgpu_hal_patched))]
fn keep_compiled_shaders() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dir = std::env::var_os("LOCALAPPDATA")
            .map(|local| std::path::PathBuf::from(local).join("zgx\\shaders"));
        wgpu::hal::dx12::set_shader_cache_dir(dir);
    });
}
#[cfg(not(all(windows, zgx_wgpu_hal_patched)))]
fn keep_compiled_shaders() {}
/// The compilers Direct3D 12 tries, in order. DXC where the app ships
/// Microsoft's dxcompiler.dll beside its executable (no dxil.dll needed
/// since DXC 1.8): FXC is
/// deprecated, slower to start a window cold, and its optimizer has
/// overflowed its stack on shaders DXC compiles. Otherwise FXC alone, as
/// Direct3D 11's ahead-of-time DXBC is compiled: wgpu would take DXC from a
/// developer shell's PATH, whose code rounds antialiased edges a level
/// apart. `WGPU_DX12_COMPILER` chooses one alone.
fn dx12_compilers() -> Vec<wgpu::Dx12Compiler> {
    if let Some(chosen) = wgpu::Dx12Compiler::from_env() {
        return vec![chosen];
    }
    let shipped = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("dxcompiler.dll")))
        .filter(|dll| cfg!(windows) && dll.is_file());
    match shipped {
        Some(dll) => vec![
            wgpu::Dx12Compiler::DynamicDxc {
                dxc_path: dll.to_string_lossy().into_owned(),
            },
            wgpu::Dx12Compiler::Fxc,
        ],
        None => vec![wgpu::Dx12Compiler::Fxc],
    }
}
/// What the shaders kept between launches (Windows) did in this process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderCacheCounts {
    /// Shaders read back from the cache.
    pub read: u32,
    /// Shaders compiled (none kept for them yet).
    pub compiled: u32,
    /// Shaders compiled that could not be kept: they compile again next
    /// launch.
    pub unwritten: u32,
}
/// What the shaders kept between launches did so far in this process, so a
/// start's marks say whether it compiled them: `None` where none are kept
/// (other platforms, or crates.io's wgpu-hal).
#[cfg(all(windows, zgx_wgpu_hal_patched))]
pub fn shader_cache_counts() -> Option<ShaderCacheCounts> {
    let counts = wgpu::hal::dx12::shader_cache_counts();
    Some(ShaderCacheCounts {
        read: counts.read,
        compiled: counts.compiled,
        unwritten: counts.unwritten,
    })
}
/// What the shaders kept between launches did so far in this process, so a
/// start's marks say whether it compiled them: `None` where none are kept
/// (other platforms, or crates.io's wgpu-hal).
#[cfg(not(all(windows, zgx_wgpu_hal_patched)))]
pub fn shader_cache_counts() -> Option<ShaderCacheCounts> {
    None
}
/// A device on `adapter` and its queue, with zgx's handlers of a lost
/// device and of errors (`GpuContext::prepare`).
fn request_device(
    adapter: &wgpu::Adapter,
    info: &wgpu::AdapterInfo,
    vertex_ring: bool,
    contexts: &LostCells,
) -> Result<(wgpu::Device, wgpu::Queue, LostCell), GpuError> {
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("zgx shared device"),
        required_features: if vertex_ring {
            wgpu::Features::MAPPABLE_PRIMARY_BUFFERS
        } else {
            wgpu::Features::empty()
        } | (adapter.features() & wgpu::Features::DUAL_SOURCE_BLENDING)
            // An app's shaders' bytecode, compiled by FXC elsewhere
            // (`app_fragment`).
            | if info.backend == wgpu::Backend::Dx12 {
                adapter.features() & wgpu::Features::PASSTHROUGH_SHADERS
            } else {
                wgpu::Features::empty()
            },
        required_limits: wgpu::Limits {
            // Direct3D 12 makes its shader-visible view heap this large, in
            // write-combined memory: 30 MiB at the default million, 2 MiB
            // here. An image holds one view and a window holds at most
            // `MAX_IMAGES`: windows share the device, and four at their
            // budget still fit beside their other bind groups.
            max_non_sampler_bindings: if cfg!(windows) {
                1 << 16
            } else {
                wgpu::Limits::default().max_non_sampler_bindings
            },
            // GL (Linux's fallback) asks no more than older GPUs give,
            // with the textures as large as this one makes.
            ..if info.backend == wgpu::Backend::Gl {
                wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits())
            } else {
                Default::default()
            }
        },
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    }))
    .map_err(|e| GpuError(e.to_string()))?;
    // wgpu clears a buffer of its own while creating the device and holds
    // that command buffer until the first submission. Finish it now, so
    // it is never alive beside a frame's: Metal keeps memory for as many
    // command buffers as were ever alive at once, for good.
    queue.submit([]);
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    // A lost device (a driver reset, a GPU removed) fails what follows
    // instead of panicking, as wgpu's default handler does: renders
    // then return `GpuError::is_device_lost`, and the host makes a new
    // renderer. Other errors are logged; a validation error is a bug,
    // which still panics in debug builds.
    let lost: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    {
        let lost = lost.clone();
        let contexts = contexts.clone();
        device.set_device_lost_callback(move |reason, message| {
            let mut slot = lost.lock().unwrap_or_else(|p| p.into_inner());
            if slot.is_none() {
                if message.is_empty() {
                    eprintln!("zgx: the GPU device was lost ({reason:?})");
                } else {
                    eprintln!("zgx: the GPU device was lost ({reason:?}): {message}");
                }
                *slot = Some(if message.is_empty() {
                    format!("{reason:?}")
                } else {
                    message
                });
            }
            // Every context on the device (WARP's, each with a cell of its own).
            let contexts = contexts.lock().unwrap_or_else(|p| p.into_inner());
            for context in contexts.iter().filter_map(std::sync::Weak::upgrade) {
                let mut lost = context.lock().unwrap_or_else(|p| p.into_inner());
                lost.get_or_insert_with(|| slot.clone().unwrap_or_default());
            }
        });
    }
    {
        let lost = lost.clone();
        device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            if lost.lock().map_or(true, |lost| lost.is_some()) {
                return;
            }
            if cfg!(debug_assertions) && matches!(error, wgpu::Error::Validation { .. }) {
                panic!("wgpu: {error}");
            }
            eprintln!("zgx: GPU error: {error}");
        }));
    }
    Ok((device, queue, lost))
}
/// Why a device was lost, once it is (set from any thread).
type LostCell = Arc<std::sync::Mutex<Option<String>>>;
/// The cells of the contexts sharing WARP's device, for its loss to reach.
type LostCells = Arc<std::sync::Mutex<Vec<std::sync::Weak<std::sync::Mutex<Option<String>>>>>>;
/// What contexts on one device share: on WARP, every context in the process
/// (`WARP_DEVICE`), and elsewhere the one context made with it.
struct SharedDevice {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The device's loss.
    lost: LostCell,
    /// On WARP, each context's own cell, so a test can lose one context
    /// alone (`GpuContext::debug_lose_device`); elsewhere the context reads
    /// `lost`.
    contexts: LostCells,
}
/// WARP's device while a context holds it (`GpuContext::prepare`): held
/// weakly, so it goes with the last context on it.
static WARP_DEVICE: std::sync::Mutex<std::sync::Weak<SharedDevice>> =
    std::sync::Mutex::new(std::sync::Weak::new());
/// What a `GpuContext` holds that another thread can make (`GpuContext::prepare`).
struct Prepared {
    /// Keeps WARP's process-wide device for `WARP_DEVICE` to find.
    shared: Arc<SharedDevice>,
    lost: Arc<std::sync::Mutex<Option<String>>>,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    fonts: FontSystem,
    texture_layout: wgpu::BindGroupLayout,
    viewport_layout: wgpu::BindGroupLayout,
    quad_layout: wgpu::PipelineLayout,
    mapped_uploads: bool,
    vertex_ring: bool,
    areas: bool,
    #[cfg(d3d11)]
    d3d11: Option<d3d11_frame::Made>,
}
/// A `GpuContext` being made on another thread (`GpuContext::prepare_in_background`).
pub struct PendingGpuContext(Option<std::thread::JoinHandle<Result<Prepared, GpuError>>>);
impl PendingGpuContext {
    /// The context, once made: waits for it if it is not done yet. Errors
    /// as `GpuContext::new` does (no thread to make it on: a `GpuError` too).
    pub fn join(self) -> Result<GpuContext, GpuError> {
        let handle = self
            .0
            .ok_or_else(|| GpuError("no thread to make the GPU context on".into()))?;
        let prepared = handle
            .join()
            .map_err(|_| GpuError("making the GPU context panicked".into()))??;
        Ok(GpuContext::finish(prepared))
    }
}
/// Shared native GPU device, pipelines, and font database. Clone this cheaply for
/// additional windows; render targets/atlases remain owned by each renderer.
#[derive(Clone)]
pub struct GpuContext {
    inner: Rc<GpuContextInner>,
}
struct GpuContextInner {
    /// Keeps WARP's process-wide device for `WARP_DEVICE` to find.
    shared: Arc<SharedDevice>,
    /// Why the device was lost (driver reset, removal, `debug_lose_device`):
    /// set from wgpu's callback, from any thread.
    lost: Arc<std::sync::Mutex<Option<String>>>,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    fonts: Rc<RefCell<FontSystem>>,
    /// The fonts given it (`add_font`, `replace_font`), for canvases that
    /// draw their text into their raster on the CPU (`Canvas::raster_text`).
    canvas_fonts: canvas::CanvasFonts,
    /// Shaped text shared by layout measurement and every renderer.
    text_cache: Rc<RefCell<prepared::TextCache>>,
    /// `fonts` and `text_cache` as text services a scene with none takes
    /// (`default_text_services`): one for every renderer on this context.
    text_default: Rc<zgx::scene::DefaultTextServices>,
    quads: QuadPipelines,
    /// Bind group layouts of the quad pipelines: texture and sampler, viewport.
    texture_layout: wgpu::BindGroupLayout,
    viewport_layout: wgpu::BindGroupLayout,
    /// Vertex and small uniform uploads are CPU-mapped instead of copied
    /// (memory the CPU and GPU share).
    mapped_uploads: bool,
    /// Per-frame vertices are CPU-written in a ring of buffers the GPU reads
    /// (`upload::MappedRing`): with unified memory, and on Direct3D 12.
    vertex_ring: bool,
    /// For dual-output (single-pass presentation) pipelines.
    quad_layout: wgpu::PipelineLayout,
    /// With `ZGX_D3D11`, the Direct3D 11 device frames are drawn with; wgpu
    /// then runs on its no-op backend (d3d11_frame.rs).
    #[cfg(d3d11)]
    d3d11: Option<Rc<d3d11_frame::Device>>,
}
impl GpuContext {
    /// A context on the system's GPU, found without a window.
    pub fn new() -> Result<Self, GpuError> {
        Self::create(None)
    }
    fn create(window: Option<Arc<winit::window::Window>>) -> Result<Self, GpuError> {
        Ok(Self::finish(Self::prepare(window)?))
    }
    /// Start making a context on another thread (the device, its pipelines
    /// and the system fonts take a few milliseconds), to take with
    /// `PendingGpuContext::join` once a window needs it. Found without a
    /// window: a host that knows any adapter can draw to its windows (macOS)
    /// starts this while its windows are still being made.
    pub fn prepare_in_background() -> PendingGpuContext {
        PendingGpuContext(
            std::thread::Builder::new()
                .name("zgx GPU setup".into())
                .spawn(|| Self::prepare(None))
                .ok(),
        )
    }
    /// What a context is made of that can be made on any thread.
    fn prepare(window: Option<Arc<winit::window::Window>>) -> Result<Prepared, GpuError> {
        // Avoid initializing secondary graphics stacks merely to enumerate
        // adapters. On Linux this can create an unused EGL driver and worker
        // pool even when Vulkan is selected; macOS uses Metal. Windows uses
        // Direct3D 12 alone: it presents through DXGI's flip model, which the
        // compositor schedules, and leaves no Vulkan driver loaded beside it
        // (on NVIDIA, three quarters of the idle CPU and 18 MiB of working
        // set). Vulkan, with the `vulkan` feature, stands in only where
        // Direct3D 12 has no adapter. On Linux, GL stands in where no Vulkan
        // driver is (VMs and older GPUs without one): made only then, so a
        // Vulkan system loads no EGL.
        // `ZGX_D3D11`: frames are drawn with Direct3D 11, and wgpu runs on
        // its no-op backend, opening no driver (d3d11_frame.rs).
        #[cfg(d3d11)]
        let d3d11 = d3d11_frame::ENABLED
            .then(d3d11_frame::Device::new)
            .transpose()?;
        #[cfg(d3d11)]
        let window = window.filter(|_| d3d11.is_none());
        #[cfg(d3d11)]
        let noop = d3d11.is_some();
        #[cfg(not(d3d11))]
        let noop = false;
        // `ZGX_GPU_NOOP`: the renderer's CPU work with no driver or JIT, for
        // the instruction gate (fx_bench).
        let noop = noop || std::env::var_os("ZGX_GPU_NOOP").is_some();
        let attempts: &[wgpu::Backends] = if noop {
            &[wgpu::Backends::NOOP]
        } else if cfg!(all(windows, feature = "vulkan")) {
            &[wgpu::Backends::DX12, wgpu::Backends::VULKAN]
        } else if cfg!(windows) {
            &[wgpu::Backends::DX12]
        } else if cfg!(target_os = "linux") {
            &[wgpu::Backends::PRIMARY, wgpu::Backends::GL]
        } else {
            &[wgpu::Backends::PRIMARY]
        };
        keep_compiled_shaders();
        // Direct3D 12's compiler: DXC where the app ships Microsoft's
        // dxcompiler.dll beside its executable, and FXC if that fails, as
        // where it doesn't (`dx12_compilers`).
        let attempts: Vec<(wgpu::Backends, wgpu::Dx12Compiler)> = attempts
            .iter()
            .flat_map(|&backends| {
                let compilers = match backends {
                    wgpu::Backends::DX12 => dx12_compilers(),
                    _ => vec![wgpu::Dx12Compiler::Fxc],
                };
                compilers
                    .into_iter()
                    .map(move |compiler| (backends, compiler))
            })
            .collect();
        let fallback = fallback_adapter_forced();
        let mut found = Err(GpuError("no graphics backend".into()));
        for (backends, compiler) in attempts {
            let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
            descriptor.backends = backends;
            // Windows: swap chains present through a DirectComposition
            // visual, whose alpha the compositor blends (transparent and
            // glass windows), where a window's own swap chain is opaque.
            descriptor.backend_options.dx12.presentation_system =
                wgpu::Dx12SwapchainKind::DxgiFromVisual;
            descriptor.backend_options.dx12.shader_compiler = compiler.clone();
            descriptor.backend_options.noop.enable = noop;
            descriptor.flags = instance_flags(descriptor.flags, !cfg!(debug_assertions));
            // Which compiler an app's shaders meet on Direct3D 12, and its
            // flags (`app_fragment`).
            #[cfg(windows)]
            APP_SHADER_FXC.get_or_init(|| {
                (
                    matches!(compiler, wgpu::Dx12Compiler::Fxc),
                    descriptor.flags.contains(wgpu::InstanceFlags::DEBUG),
                )
            });
            // EGL draws to the window's display, which it must be given.
            if backends == wgpu::Backends::GL
                && let Some(window) = &window
            {
                descriptor = descriptor.with_display_handle(Box::new(window.clone()));
            }
            let instance = wgpu::Instance::new(descriptor.with_env());
            // No driver for these backends: the next.
            let surface = match window
                .clone()
                .map(|window| instance.create_surface(window))
                .transpose()
            {
                Ok(surface) => surface,
                Err(error) => {
                    found = Err(GpuError(error.to_string()));
                    continue;
                }
            };
            // The software adapter asked for: `request_adapter` alone, keeping
            // only software adapters (Direct3D 11's device takes WARP itself,
            // and the no-op backend has none).
            #[cfg(windows)]
            let preferred = (!fallback)
                .then(|| preferred_dx12_adapter(&instance))
                .flatten()
                .filter(|adapter| {
                    surface
                        .as_ref()
                        .is_none_or(|surface| adapter.is_surface_supported(surface))
                });
            #[cfg(not(windows))]
            let preferred = None;
            let adapter = match preferred {
                Some(adapter) => Ok(adapter),
                None => {
                    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: POWER,
                        compatible_surface: surface.as_ref(),
                        force_fallback_adapter: fallback && !noop,
                    }))
                }
            };
            // An adapter that presents to the window in no format draws
            // nothing there (Mesa's Vulkan, to a window of an X visual of 16
            // or 30 bits, which zgx's own windows no longer take): the next
            // backend, as where there is no driver, rather than a panic.
            let adapter = adapter
                .map_err(|e| GpuError(e.to_string()))
                .and_then(|adapter| match &surface {
                    Some(surface) if surface.get_capabilities(&adapter).formats.is_empty() => Err(
                        GpuError(format!("{backends:?} presents to the window in no format")),
                    ),
                    _ => Ok(adapter),
                });
            let areas = areas_on_gpu(backends, &compiler);
            found = adapter.map(|adapter| (instance, adapter, areas));
            if found.is_ok() {
                break;
            }
        }
        let (instance, adapter, areas) = found?;
        let info = adapter.get_info();
        // CPU-written vertex buffers avoid a GPU copy per frame; only worth it
        // where CPU and GPU share memory.
        let mappable = adapter
            .features()
            .contains(wgpu::Features::MAPPABLE_PRIMARY_BUFFERS);
        // (On the no-op backend, Direct3D 11 takes vertices and texels:
        // neither is written through wgpu.)
        let mapped_uploads =
            !noop && info.device_type == wgpu::DeviceType::IntegratedGpu && mappable;
        // Direct3D 12 also takes vertices written in place, in upload-heap
        // buffers the GPU reads directly (as Direct3D 11's dynamic buffers
        // are): a frame then needs no staging copy, and no command list for
        // it, each of which costs NVIDIA's driver some 30 K cycles. Measured
        // on an RTX 4090: 0.7 to 3.3% fewer cycles a frame, the GPU's time
        // the same. Textures and uniforms keep their copies.
        let vertex_ring =
            !noop && mappable && (mapped_uploads || info.backend == wgpu::Backend::Dx12);
        zgx::profile::mark("GPU adapter");
        // WARP (Direct3D 12 on the CPU) takes one device for the process:
        // renderers on threads, each with a device of its own, made WARP
        // 10.0.26100 fault in `ExecuteCommandLists`, or draw nothing, when
        // they drew at once; one device shared by all of them never did.
        // Direct3D 12 already hands each the same device for the adapter,
        // and it is each wgpu device's own queue that breaks it.
        let warp = !noop
            && info.backend == wgpu::Backend::Dx12
            && info.device_type == wgpu::DeviceType::Cpu;
        let mut warp_slot = warp.then(|| WARP_DEVICE.lock().unwrap_or_else(|p| p.into_inner()));
        // (Not a lost one: a context made after a loss takes a new device.)
        let reusable = warp_slot
            .as_ref()
            .and_then(|slot| slot.upgrade())
            .filter(|shared| shared.lost.lock().is_ok_and(|lost| lost.is_none()));
        let shared = match reusable {
            Some(shared) => shared,
            None => {
                let contexts = LostCells::default();
                let (device, queue, lost) =
                    request_device(&adapter, &info, vertex_ring, &contexts)?;
                let shared = Arc::new(SharedDevice {
                    instance,
                    adapter,
                    device,
                    queue,
                    lost,
                    contexts,
                });
                if let Some(slot) = &mut warp_slot {
                    **slot = Arc::downgrade(&shared);
                }
                shared
            }
        };
        drop(warp_slot);
        let SharedDevice {
            instance,
            adapter,
            device,
            queue,
            lost,
            contexts,
        } = &*shared;
        let lost = match warp {
            false => lost.clone(),
            true => {
                let own = LostCell::default();
                let mut contexts = contexts.lock().unwrap_or_else(|p| p.into_inner());
                contexts.retain(|context| context.strong_count() > 0);
                contexts.push(Arc::downgrade(&own));
                own
            }
        };
        let (instance, adapter, device, queue, lost) = (
            instance.clone(),
            adapter.clone(),
            device.clone(),
            queue.clone(),
            lost.clone(),
        );
        let (texture_layout, viewport_layout, quad_layout) = create_quad_pipelines(&device);
        zgx::profile::mark("GPU device and pipelines");
        let fonts = font_cache::system_fonts();
        zgx::profile::mark("system fonts");
        Ok(Prepared {
            shared,
            lost,
            instance,
            adapter,
            device,
            queue,
            info,
            fonts,
            texture_layout,
            viewport_layout,
            quad_layout,
            mapped_uploads,
            vertex_ring,
            areas,
            #[cfg(d3d11)]
            d3d11: d3d11.map(d3d11_frame::Made),
        })
    }
    /// The context of what `prepare` made, on the thread that uses it.
    fn finish(prepared: Prepared) -> Self {
        let Prepared {
            shared,
            lost,
            instance,
            adapter,
            device,
            queue,
            info,
            fonts,
            texture_layout,
            viewport_layout,
            quad_layout,
            mapped_uploads,
            vertex_ring,
            areas,
            #[cfg(d3d11)]
            d3d11,
        } = prepared;
        let quads = QuadPipelines {
            device: device.clone(),
            shader: Default::default(),
            layout: quad_layout.clone(),
            full: Default::default(),
            full_clear: Default::default(),
            basic: Default::default(),
            basic_clear: Default::default(),
            source_in: Default::default(),
            behind: Default::default(),
            texture_layout: texture_layout.clone(),
            viewport_layout: viewport_layout.clone(),
            polylines: Default::default(),
            areas,
        };
        Self {
            inner: Rc::new({
                let fonts = Rc::new(RefCell::new(fonts));
                let text_cache = Rc::default();
                GpuContextInner {
                    text_default: default_text_services(&fonts, &text_cache),
                    shared,
                    lost,
                    instance,
                    adapter,
                    device,
                    queue,
                    info,
                    fonts,
                    canvas_fonts: Default::default(),
                    text_cache,
                    quads,
                    texture_layout,
                    viewport_layout,
                    quad_layout,
                    mapped_uploads,
                    vertex_ring,
                    #[cfg(d3d11)]
                    d3d11: d3d11.map(|made| Rc::new(made.0)),
                }
            }),
        }
    }
    /// Why its device was lost, once it is: every renderer on it then
    /// fails with `GpuError::is_device_lost`, and a new context is needed.
    pub fn lost(&self) -> Option<String> {
        self.inner
            .lost
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    /// Lose the device, as a driver reset would: for tests of recovery. On
    /// WARP's process-wide device (`WARP_DEVICE`), while other contexts draw
    /// on it, only this context fails from then on.
    pub fn debug_lose_device(&self) {
        if Arc::strong_count(&self.inner.shared) == 1 {
            self.inner.device.destroy();
            let _ = self.inner.device.poll(wgpu::PollType::Poll);
        }
        let mut lost = self.inner.lost.lock().unwrap_or_else(|p| p.into_inner());
        lost.get_or_insert_with(|| "destroyed for a test".into());
    }
    /// Whether `other` is this context (a clone of it), on the same device.
    pub fn shares_device(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
    /// The font system its renderers shape and draw text with.
    pub fn text_system(&self) -> Rc<RefCell<FontSystem>> {
        self.inner.fonts.clone()
    }
    /// Prepared (shaped once) rich text, shared with the renderers.
    pub fn text_cache(&self) -> Rc<RefCell<prepared::TextCache>> {
        self.inner.text_cache.clone()
    }
    /// Shape the families `font` holds with it from now on, in place of
    /// their faces (`FontData::replace_in`), and forget the text prepared
    /// with them. Each renderer on this context then calls
    /// [`GpuRenderer::fonts_replaced`], and each scene
    /// `Scene::fonts_replaced`.
    pub fn replace_font(&self, font: &crate::text::FontData) {
        font.replace_in(&mut self.inner.fonts.borrow_mut());
        self.inner.canvas_fonts.add(font, true);
        self.inner.text_cache.borrow_mut().clear();
    }
    /// Shape text with `font` too, beside the faces its families have
    /// (`FontData::install`), and forget the text shaped before: a family
    /// that fell back shapes in it now. Then as after `replace_font`.
    pub fn add_font(&self, font: &crate::text::FontData) {
        let mut fonts = self.inner.fonts.borrow_mut();
        font.install(&mut fonts);
        fonts.shape_run_cache.clear();
        self.inner.canvas_fonts.add(font, false);
        self.inner.text_cache.borrow_mut().clear();
    }
    /// Shape text as in `language` from now on (`None`: the system's), its
    /// Han characters in that language's fonts
    /// (`FontSystem::set_language`), and forget the text prepared before.
    /// Each renderer then calls [`GpuRenderer::fonts_replaced`], and each
    /// scene `Scene::fonts_replaced`, as after `replace_font`.
    pub fn set_text_language(&self, language: Option<&str>) {
        self.inner.fonts.borrow_mut().set_language(language);
        self.inner.text_cache.borrow_mut().clear();
    }
}
impl GpuError {
    /// The device was lost (`GpuContext::lost`): `reason` says why.
    pub fn device_lost(reason: &str) -> Self {
        Self(format!("GPU device lost: {reason}"))
    }
    /// Whether the device was lost: drop the renderer and make another on
    /// a new context.
    pub fn is_device_lost(&self) -> bool {
        self.0.starts_with("GPU device lost")
    }
    /// Whether a window's surface can't be drawn with the context's adapter
    /// (make a context for that window instead).
    pub fn is_surface_incompatible(&self) -> bool {
        self.0 == "surface is incompatible with the shared GPU adapter"
    }
}
/// Frames a renderer draws before an app that never goes idle checks its
/// font cache (`font_cache::check_pending`): launch's busy frames, ten
/// seconds at 60 Hz.
const LAUNCH_FRAMES: u64 = 600;
/// `font_cache::check_pending`, kept out of the frame it runs in.
#[cold]
#[inline(never)]
fn font_cache_check() {
    font_cache::check_pending();
}
/// Wakes a host's event loop from another thread (`GpuRenderer::set_plane_waker`).
#[cfg(target_os = "macos")]
#[doc(hidden)]
pub type PlaneWaker = std::sync::Arc<dyn Fn() + Send + Sync>;
/// Draws a [`zgx::scene::Scene`] with wgpu (Metal, Direct3D 12 or Vulkan),
/// repainting only its damage, to an offscreen target or a window.
pub struct GpuRenderer {
    context: GpuContext,
    scene_identity: Option<u64>,
    /// Scene content revision the per-node caches were last pruned at.
    pruned_revision: Option<u64>,
    /// The last render's still-open encoder. Present records its blit into it,
    /// so a frame is one command buffer and one submission. Flushed before
    /// anything that rewrites buffers or textures the commands read.
    pending: RefCell<Option<wgpu::CommandEncoder>>,
    /// Viewport uniforms by logical size (f32 bits). Each is written once at
    /// creation; layers and the window switch between them per pass.
    viewports: FxHashMap<(u32, u32, u32), (wgpu::Buffer, wgpu::BindGroup)>,
    /// Bound as the clip regions of frames without any (`Region`).
    no_regions: wgpu::Buffer,
    /// The raster each canvas, decoration or SVG node currently draws. A
    /// resized or restyled node gets a new raster; the old texture goes at
    /// once instead of waiting for a structural prune.
    node_images: FxHashMap<NodeId, u64>,
    /// The images the scene's nodes show, counted (`prune_images`).
    /// Boxed: the renderer's own fields stay where they were.
    scene_images: Option<Box<SceneImages>>,
    /// Gradient ramps (see `axis_gradient`) by their stops, and the ramp
    /// each node draws, so a ramp lives while a node shows it.
    ramps: FxHashMap<RampKey, Arc<zgx::image::ImageData>>,
    node_ramps: FxHashMap<NodeId, RampKey>,
    /// SVG sources that failed to draw, reported once each.
    failed_svgs: rustc_hash::FxHashSet<u64>,
    /// The viewport of the target currently being rendered.
    viewport_bind: Option<wgpu::BindGroup>,
    /// Reused upload memory for per-frame vertices; `write_buffer` would
    /// allocate and free a staging buffer every frame.
    belt: RefCell<wgpu::util::StagingBelt>,
    /// Replaces the belt for vertices on unified-memory adapters and on
    /// Direct3D 12.
    mapped: Option<RefCell<upload::MappedRing>>,
    /// The CPU and GPU share memory: textures and small uniforms are written
    /// in place too (`texture_uploads`), not copied.
    unified: bool,
    /// Texture writes awaiting the next render's upload passes (mapped adapters).
    texture_uploads: upload::TextureUploads,
    instance: wgpu::Instance,
    window: Option<Arc<winit::window::Window>>,
    /// A surface of the caller's, not a winit window's (`for_raw_surface`).
    raw: Option<(wgpu::rwh::RawDisplayHandle, wgpu::rwh::RawWindowHandle)>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    surface: Option<wgpu::Surface<'static>>,
    /// DXGI's account of shown presents (`ZGX_PRESENT_LOG`).
    #[cfg(windows)]
    dxgi_stats: dxgi::PresentStats,
    /// Windows: the alpha mode of a swap chain that lets what is behind
    /// show, for a window made opaque whose background at the first frame
    /// does (`match_background`): DXGI keeps a swap chain's alpha mode for
    /// its life.
    #[cfg(windows)]
    translucent_alpha: wgpu::CompositeAlphaMode,
    /// Whether the first frame's background was checked against the swap
    /// chain's alpha.
    #[cfg(windows)]
    background_matched: bool,
    /// The caller's visual the surface presents into (composition.rs).
    #[cfg(all(windows, feature = "composition"))]
    composition: Option<composition::Target>,
    config: Option<wgpu::SurfaceConfiguration>,
    width: u32,
    height: u32,
    scale: f32,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    /// Single-pass presentation alternates two retained targets: each frame
    /// copies `target` and draws its damage into `spare` and the drawable in
    /// one render pass, then swaps. Two passes (damage, then a present blit)
    /// cost a second Metal command buffer per frame. Dropped by `trim`.
    spare: Option<Retained>,
    /// Bind group sampling `target` for the single-pass copy; swapped with
    /// the spare's so neither is rebuilt per frame.
    target_copy_bind: Option<wgpu::BindGroup>,
    deferred: RefCell<Option<Deferred>>,
    single_pass: Option<SinglePass>,
    /// Non-sRGB view format of the drawable. Through it, drawing directly
    /// stores the same bytes the present blit would for opaque windows (whose
    /// pixels all have alpha 1, so post-multiplied presentation is a no-op).
    single_pass_format: Option<wgpu::TextureFormat>,
    /// Rendering an isolated layer's texture, never presented directly.
    in_layer: bool,
    /// The node whose layer this is, while rendering one: a gradient
    /// text fills its glyphs in its own (`gradient_text_quads`).
    layer_root: Option<NodeId>,
    /// The present blit converts to straight alpha (see `unpremultiplies`).
    /// Single-pass presentation then only matches for opaque windows.
    present_unpremultiplies: bool,
    /// Present by copying the retained target into the drawable: same
    /// format, no alpha conversion. A copy is a memcpy for a software
    /// rasterizer, where a full-window shader pass costs as much as drawing.
    copy_present: bool,
    /// The frame each swap chain image holds, and what recent frames
    /// changed (Linux, Vulkan): a present copies what its image lacks.
    #[cfg(target_os = "linux")]
    swap: SwapHistory,
    /// Frames whose damage was drawn by `present_single_pass`.
    single_pass_frames: u64,
    /// Test hook: an offscreen renderer copies scroll moves' pixels where the
    /// platform would repaint them (`debug_allow_scroll_copies`).
    debug_scroll_copy: bool,
    /// Scroll moves applied as a copy of their pixels, ever.
    scroll_copied: std::cell::Cell<u64>,
    /// A scroll this frame could have copied, had the pixels not been new.
    copy_next: std::cell::Cell<bool>,
    /// Frames presented to the drawable alone (`Deferred::direct`).
    direct_frames: u64,
    /// Frames encoded with Metal directly (`render_direct`).
    metal_frames: u64,
    /// Passes waking the GPU for input after idling (`warm_up`).
    warms: u64,
    /// Work encoded at presentation (single-pass frames), reported with the
    /// next `render`'s statistics.
    present_stats: GpuStats,
    /// Quads use the shader their features need (see `push_draw`).
    split_shading: bool,
    /// A text partly in its clip makes quads for its visible lines alone
    /// (`debug_cull_glyphs`).
    cull_glyphs: bool,
    bind: wgpu::BindGroup,
    blit: Option<wgpu::RenderPipeline>,
    blit_bind: Option<wgpu::BindGroup>,
    atlas: wgpu::Texture,
    atlas_size: u32,
    /// The size `trim_atlas` took it back from.
    atlas_grew_to: u32,
    /// What a frame went on without, each said once (`warn`), and those the
    /// host hasn't taken yet (`take_issues`).
    warned: Vec<String>,
    issues: Vec<String>,
    entries: FxHashMap<AtlasKey, Option<AtlasEntry>>,
    cursor: (u32, u32, u32),
    fonts: Rc<RefCell<FontSystem>>,
    text_cache: Rc<RefCell<prepared::TextCache>>,
    swash: SwashCache,
    layers: FxHashMap<NodeId, LayerCache>,
    /// The items `render_flat` draws next came from a walk that passed a
    /// fade (`Scene::take_painted_fades`): their fade bands draw as one
    /// (bands.rs).
    banding: bool,
    /// Fades drawn item by item (`set_fade_bands`).
    bands_off: bool,
    /// Fade bands were drawn as one since a frame last found none to draw:
    /// no frame is kept to patch, which would fade them item by item
    /// (bands.rs).
    bands_seen: bool,
    /// The ids keying the last frame's band textures in `layers`, and
    /// band textures to draw again (bands.rs).
    band_keys: Vec<(NodeId, wgpu::TextureView)>,
    band_pool: Vec<(wgpu::Texture, wgpu::BindGroup, wgpu::TextureView)>,
    /// Counts rendered frames, to age parked layers (`LAYER_PARK_FRAMES`).
    layer_frame: u64,
    /// A layer prepared last (`prepare_layer`), or one nested in it, drew
    /// its texture from another corner than it had: all of it may differ
    /// where it shows, not only where the scene's damage says.
    layer_shifted: bool,
    layer_origin: (f32, f32),
    shapes: FxHashMap<NodeId, Shaped>,
    /// For text the prepared cache doesn't lay out, a node shaped from each
    /// of its inputs (hashed: see `render_flat`), to copy rather than shape.
    shaped_like: FxHashMap<u64, NodeId>,
    shaped_bytes: usize,
    fresh: bool,
    /// Consecutive presented frames whose damage covered the window.
    whole_frames: u32,
    /// The scene flush the retained target was last rendered from.
    rendered_serial: Option<u64>,
    /// Scroll copies for the next root frame: destination pixel rectangle
    /// and the source offset (see `scroll_copies`).
    copies: Vec<ScrollCopy>,
    atlas_epoch: u64,
    frame: u64,
    vertices: wgpu::Buffer,
    vertex_capacity: usize,
    frame_quads: Vec<Quad>,
    /// The points of the frame's polyline strokes (polyline.wgsl), kept for
    /// their allocation.
    frame_points: Vec<[f32; 2]>,
    /// The window's last whole paint, while only leaves repaint
    /// (`render_kept`).
    kept: Option<Box<Kept>>,
    /// What keeping the window up to date did in a frame drawn as usual.
    patch_stats: GpuStats,
    /// Whether the last frame patched the window as kept (`render_kept`):
    /// one that can't is drawn whole, to keep patching after it.
    patched: bool,
    /// While no frame is kept, how many in a row a kept one would have
    /// patched (`would_patch`).
    patchable: Patchable,
    /// Kept frames in a row lost before they patched once (`render_once`):
    /// past `UNPATCHED_KEEPS`, a frame drawn whole keeps none.
    unpatched_keeps: u8,
    /// Where each item's quads and draws lay in a frame made to keep or
    /// patch (`render_flat`): first quad, quads, room, first draw, draws' end.
    item_ranges: Vec<(u32, u32, u32, u32, u32)>,
    /// A kept frame's clip regions, while it is patched (`PATCH`).
    patch_clips: Vec<Region>,
    /// Each clip region's `Interior`, by index, as `render_flat` first
    /// draws in it.
    region_interiors: Vec<Option<Interior>>,
    /// A plain box's quads and draws as `patch_box` makes them, kept for
    /// their room.
    box_quads: Vec<Quad>,
    box_draws: Vec<Draw>,
    /// While a frame draws from the kept frame: what its quads' patches
    /// wrote, and their bytes, leading the frame's (`render_kept`).
    kept_upload: Option<(upload::KeptQuads, usize)>,
    /// And how many of its quads lie in paint order (`Kept::ordered`).
    kept_ordered: u32,
    /// Keep no frame to patch (`debug_never_keep`): a reference drawing
    /// every frame afresh.
    never_keep: bool,
    /// Texts a kept frame patched (at most `SHAPED_LIKE`): the ones a kept
    /// frame gives spare room to grow into (`text_room`).
    changed_texts: rustc_hash::FxHashSet<NodeId>,
    /// Whether the frame's quads, draws, polyline points and clips
    /// (`patch_clips`) are made already, for the next frame to draw as
    /// they are.
    draw_made: bool,
    /// The frame's draws, kept for their allocation.
    frame_draws: Vec<Draw>,
    /// How many quads and draws frames have used lately (`keep_room`).
    frame_peaks: (usize, usize),
    /// The quads of the lines of a text partly in its clip, made each frame
    /// (see `render_flat`), kept for their allocation.
    visible_glyphs: Vec<TextQuad>,
    /// Whether `visible_glyphs` hold colour glyphs (`COLOR_GLYPH`).
    visible_colored: bool,
    /// Blurred text shadows (`place_blurred_shadows`): none until a text has
    /// one.
    blurred: Option<Box<BlurredShadows>>,
    background: Color,
    /// Image textures: each bound for the quads, its size in bytes, and,
    /// where the CPU wrote its texels, the texture (see `render_direct`).
    images: FxHashMap<u64, ImageTexture>,
    /// A frame went without a kept file's pixels, decoding, or a shader
    /// fill's pipeline, compiling (`take_images_awaited`).
    images_awaited: bool,
    /// The layer `paint_layer` just drew went without something
    /// (`LayerCache::incomplete`).
    layer_incomplete: bool,
    /// The largest glyph, in pixels, the layer being drawn left out as too
    /// large for the atlas (0: none), and whether that layer draws again
    /// at a scale its glyphs fit instead (`prepare_layer`): then it is no
    /// issue.
    layer_glyph_too_large: (f32, bool),
    /// A frame drawn again for layers drawn from another corner
    /// (`render_once`): it is not drawn a third time.
    redrawing_shifted: bool,
    /// Magnified layers being drawn again no finer than their glyphs fit
    /// (`prepare_layer`): each once.
    redrawing_glyphs: Vec<NodeId>,
    /// A kept file's pixels were uploaded (`take_images_uploaded`).
    images_uploaded: bool,
    /// Images' own textures (`upload_image`), by the frame each was last
    /// drawn in (`image_frame`): past the textures' budget, the least
    /// recently drawn go, not drawn this frame, and come back when drawn.
    image_drawn: FxHashMap<u64, u64>,
    /// Textures kept of images no node shows (`bound_images_gone`).
    images_gone: rustc_hash::FxHashSet<u64>,
    /// Whether anything still holds each image uploaded: one nothing holds
    /// can't be shown again, so its texture isn't kept once no node shows it.
    image_holders: FxHashMap<u64, zgx::image::Holders>,
    /// Frames drawn, for `image_drawn`.
    image_frame: u64,
    /// The scene the frame under way draws, for what an image giving way must
    /// spare (`images_showing`). Its marks in `image_drawn` can't say: a frame
    /// paints what it kept without asking for its images.
    frame_scene: Option<std::ptr::NonNull<Scene>>,
    /// The bytes of textures this frame asked for, their pixels decoding:
    /// each decodes to its share of the budget, not all of it.
    images_decoding: usize,
    /// What each image last frame's pixels were decoding for would take whole:
    /// not drawn, but sharing the budget with those drawn (`share_images`).
    images_awaiting: FxHashMap<u64, usize>,
    /// Images and layers that went softer for their budgets: no issue, the
    /// budget doing its job, but `zgx inspect memory` shows them. Written only
    /// as that happens, and read only as asked (`debug_cache_stats`).
    images_softer: FxHashMap<u64, usize>,
    /// Whether any went softer or is waiting: what a frame reads to know if the
    /// budget has any sharing to do, kept as those two change (`images_share`).
    images_to_share: bool,
    /// Textures past the share as it last moved, each with its size: drawn
    /// until what replaces it is ready, so a picture on screen whose pixels
    /// must decode again isn't left blank. Each has `IMAGE_OUTGOING` for its size
    /// in `images`, so a frame's check for what fits sends it the long way.
    images_outgoing: FxHashMap<u64, (u32, u32)>,
    /// What one image's texture may take while images drawn together don't
    /// all fit: the large ones share what the small ones leave, evenly
    /// (`share_images`).
    image_share: usize,
    layers_coarser: FxHashSet<NodeId>,
    /// GPU views' textures, by view (`viewport`): none until one shows.
    views: Option<Box<viewport::Views>>,
    /// Wakes the host when a GPU view's plane has a drawable for a frame it
    /// owes (`set_plane_waker`), from the plane's drawables thread.
    #[cfg(target_os = "macos")]
    plane_wake: Option<PlaneWaker>,
    /// The window's zoom (`set_zoom`): `scale` holds it, the host layer's
    /// points don't.
    #[cfg(target_os = "macos")]
    zoom: f64,
    /// What a popup's backdrop filters blur in place of its own target, the
    /// size of it (`set_backdrop`).
    backdrop: Option<(wgpu::Texture, wgpu::TextureView)>,
    /// A popup's panel whose shadows go undrawn (`set_unshadowed`).
    unshadowed: Option<NodeId>,
    #[cfg(target_os = "macos")]
    native_surfaces: Option<native_surface::SurfaceCache>,
    /// The next present joins the Core Animation transaction that resized the layer.
    #[cfg(target_os = "macos")]
    transaction_present: bool,
    /// Until when the layer keeps a third drawable after a resize (see
    /// `widen_drawables`).
    #[cfg(target_os = "macos")]
    wide_until: Option<std::time::Instant>,
    /// Presentation through surfaces of the renderer's own (layer_present.rs,
    /// dxgi_present.rs).
    #[cfg(own_surfaces)]
    layer_present: Option<LayerPresent>,
    /// Looping animations' sheets (loops.rs), and the shader time of the
    /// frame being drawn into one.
    #[cfg(target_os = "macos")]
    loop_sheets: Vec<loops::LoopSheet>,
    #[cfg(target_os = "macos")]
    loops_said: Vec<loops::LoopSaid>,
    #[cfg(target_os = "macos")]
    loop_watch: loops::LoopWatch,
    #[cfg(target_os = "macos")]
    loop_time: Option<f32>,
    /// Pipelines for frames encoded with Metal directly (`render_direct`);
    /// `None` inside if they could not be built.
    #[cfg(target_os = "macos")]
    metal_frame: std::cell::OnceCell<Option<metal_frame::MetalFrame>>,
    /// A directly encoded frame, committed, until it is known complete.
    #[cfg(target_os = "macos")]
    metal_inflight: RefCell<Option<metal_frame::CommandBuffer>>,
    /// The display the window shows on and its refresh rate, for the
    /// full-rate verdict's key (`set_display_key`).
    #[cfg(target_os = "macos")]
    display_key: String,
    /// When the host began the frame being drawn (`set_frame_start`).
    #[cfg(target_os = "macos")]
    frame_started: Option<std::time::Instant>,
    /// Trimmed, and nothing drawn since: the next input wakes the GPU
    /// (`warm_up`).
    #[cfg(target_os = "macos")]
    idle: bool,
    /// How long the GPU took to run the last direct frame (`take_gpu_time`).
    #[cfg(target_os = "macos")]
    gpu_time: std::cell::Cell<Option<std::time::Duration>>,
    /// The viewport uniform `select_viewport` chose last, for direct frames.
    #[cfg(any(target_os = "macos", d3d11))]
    viewport_values: [f32; 4],
    /// Frames drawn with Direct3D 11 (`ZGX_D3D11`, d3d11_frame.rs): this
    /// renderer's buffers, atlas and offscreen target there.
    #[cfg(d3d11)]
    d3d11: Option<d3d11_frame::State>,
    /// Under Direct3D 11, the Direct3D 12 device GPU views draw with, made
    /// the first time one shows (`None` inside: it couldn't be).
    #[cfg(d3d11)]
    view_host: std::cell::OnceCell<Option<view_host::ViewHost>>,
    /// Shifted texel copies drawn in a render pass (copy_offset.wgsl).
    #[cfg(own_surfaces)]
    copy_offset: Option<wgpu::RenderPipeline>,
    canvases: canvas::CanvasCache,
    /// Pixelated images' nearest samplers (`image_source`): none until one
    /// draws.
    nearest: Option<Box<NearestBinds>>,
    /// Dashed and dotted outlines (`Scene::outline_style`), rasterized: none
    /// until one draws.
    outlines: Option<Box<OutlineRasters>>,
    svgs: svg::SvgCache,
    /// Small untransformed SVG rasters packed into the glyph atlas by image
    /// id, so icons batch with text instead of binding a texture each: each
    /// cell's origin and size (a shrunk atlas packs them again).
    image_cells: FxHashMap<u64, (u32, u32, u32, u32)>,
    /// Frame of the last atlas recycle, to detect thrashing.
    atlas_recycled_at: u64,
    /// Compiled on the first backdrop filter.
    blur_pipelines: Option<BlurPipelines>,
    /// Backdrop blur scratch, shared by the window and every isolated layer.
    blur_scratch: Option<BlurScratch>,
    /// Blurred and masked layers' slots (effects.rs), made with the first.
    effects: Option<effects::EffectAtlas>,
    effect_pipelines: Option<effects::EffectPipelines>,
    /// Slots the next `render_flat` draws before compositing them.
    effect_jobs: Vec<effects::Job>,
    /// Shader fills' pipelines by shader id, compiling or compiled (`None`:
    /// it failed to), and the shaders, for frames drawn with Metal directly.
    fill_pipelines: FxHashMap<u64, compile::Compiled<wgpu::RenderPipeline>>,
    /// How many fill shaders scripts had declared when this renderer last
    /// started compiling them (`compile::declared_since`).
    fills_declared: u64,
    /// Whether, offscreen, a frame waits for a shader still compiling
    /// (`prepare_fill`); a window's never does. (Tests of a window's frames
    /// turn it off.)
    offscreen_waits: bool,
    debug_waits_for_shaders: bool,
    fill_shaders_declared: u64,
    layers_declared: u64,
    /// The image effects' pipeline (image_effects.wgsl), made when an image
    /// first draws with one.
    image_effect_pipeline: std::cell::OnceCell<wgpu::RenderPipeline>,
    fill_shaders: FxHashMap<u64, Arc<zgx::decoration::FillShader>>,
    /// Layer shaders' pipelines by shader id (`None`: it failed to compile).
    filter_pipelines: FxHashMap<u64, compile::Compiled<wgpu::RenderPipeline>>,
    /// Layer shaders' textures, passes and bindings, for those with inputs
    /// (`FillShader::with_inputs`), by shader id.
    chains: FxHashMap<u64, passes::Chain>,
    /// Pipelines of layer shaders whose passes couldn't run this frame, put
    /// aside so the layer composites as painted (`prepare_inputs`).
    parked_inputs: Vec<(bool, u64, compile::Compiled<wgpu::RenderPipeline>)>,
    /// Their shaders, to tell when no scene holds one any more (`trim`).
    filter_shaders: FxHashMap<u64, Arc<zgx::decoration::FillShader>>,
    /// Blended layers' pipelines (blend.wgsl): `BLEND_PLAIN`'s, and each
    /// layer shader's by its id (`None`: it failed to compile).
    blend_pipelines: FxHashMap<u64, compile::Compiled<wgpu::RenderPipeline>>,
    /// Their group 2 (the backdrop) and layout, made with the first.
    blend_layouts: std::cell::OnceCell<(wgpu::BindGroupLayout, wgpu::PipelineLayout)>,
    /// Copies of the target under blended layers, one per target size
    /// (the window's, each layer's), with their bind groups.
    backdrops: FxHashMap<(u32, u32), (wgpu::Texture, wgpu::BindGroup)>,
    /// The presentation plane drawn around native views (planes.rs).
    plane: Option<planes::PlaneState>,
    /// A window with backdrop readers: its walk's damage grown over what
    /// they can read (`render_once`). Boxed: most renderers have none.
    backdrop_walk: Option<Box<BackdropWalk>>,
    /// The last window frame's damage regions, kept only once a test asks
    /// (`debug_keep_regions`).
    debug_regions: Option<Vec<Rect>>,
}
/// What a window frame's walk grows its damage through (kept for the next),
/// and that damage grown, for its `render_flat` (readers alone, no blur), with
/// the frame it is for (`GpuRenderer::frame`): taken in that frame alone.
#[derive(Default)]
struct BackdropWalk {
    filters: Vec<(Rect, f32)>,
    grown: Option<Vec<Rect>>,
    frame: u64,
}
impl GpuRenderer {
    /// A window frame's items with backdrop readers in it, out of
    /// `render_once`'s line: the damage grown first through all each reader
    /// could read (`backdrop_read`, a pixel past it) as well as the blurs,
    /// and with readers alone (no blur) that reach kept for `render_flat`,
    /// which draws it rather than work it out again.
    #[cold]
    #[inline(never)]
    fn walk_over_readers<'s>(
        &mut self,
        scene: &'s Scene,
        damage: &[Rect],
        viewport: Rect,
    ) -> Vec<zgx::scene::PaintItem<'s>> {
        let walk = self.backdrop_walk.get_or_insert_default();
        let filters = &mut walk.filters;
        filters.clear();
        filters.extend(scene.backdrop_filters());
        let blurs = filters.len();
        scene.backdrop_reads(filters);
        let margin = 1. / self.scale;
        for (read, _) in &mut filters[blurs..] {
            *read = read.expand(margin);
        }
        let reach = blur_damage(damage, filters, viewport, self.scale);
        let items = scene.layer_items_within(None, Some(&reach));
        // Damage left, `render_flat` takes it this frame (with none, it
        // returns before it would).
        walk.grown = (blurs == 0 && !damage.is_empty()).then_some(reach);
        walk.frame = self.frame;
        items
    }
}
impl GpuRenderer {
    /// An offscreen renderer `width` by `height` physical pixels, on a context
    /// of its own.
    ///
    /// Its context's text services become its thread's default: a scene on
    /// this thread laid out with none installed (`install_text` not called)
    /// takes them at its next layout, so its text is measured as it is
    /// drawn from its first frame, not by zgx's estimate. Renderers sharing
    /// a context share its text services. Made after another renderer on a
    /// context of its own, its default replaces that one's for scenes laid
    /// out from then; a scene keeps the services it took, whichever renderer
    /// draws it (the first other one to says so, once: `take_issues`). A host
    /// that calls `install_text` keeps what it installed, and chooses.
    /// Every renderer constructor does the same.
    pub fn new(width: u32, height: u32) -> Result<Self, GpuError> {
        let context = GpuContext::new()?;
        Self::create(None, width, height, &context)
    }
    /// A renderer for `window`, on a context made for it, at its size and
    /// scale.
    pub fn for_window(window: Arc<winit::window::Window>) -> Result<Self, GpuError> {
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let context = GpuContext::create(Some(window.clone()))?;
        let mut renderer = Self::create(Some(window), size.width, size.height, &context)?;
        renderer.set_scale_factor(scale);
        Ok(renderer)
    }
    /// Its context, to share with other renderers.
    pub fn context(&self) -> GpuContext {
        self.context.clone()
    }
    /// An offscreen renderer `width` by `height` physical pixels on `context`.
    pub fn new_with_context(
        width: u32,
        height: u32,
        context: &GpuContext,
    ) -> Result<Self, GpuError> {
        Self::create(None, width, height, context)
    }
    /// A renderer for `window` on `context`, at its size and scale.
    pub fn for_window_with_context(
        window: Arc<winit::window::Window>,
        context: &GpuContext,
    ) -> Result<Self, GpuError> {
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let mut renderer = Self::create(Some(window), size.width, size.height, context)?;
        renderer.set_scale_factor(scale);
        Ok(renderer)
    }
    /// A renderer for `window` (on `context`, else a context made for it), at
    /// its size and scale, told whether the window is `transparent` (shows
    /// what is behind it where nothing is drawn). On Windows an opaque
    /// window's swap chain is opaque, so the compositor copies its frames
    /// rather than blending each over what lies behind (7 to 14% of its GPU
    /// time a frame); `for_window` and `for_window_with_context` make swap
    /// chains with alpha.
    pub fn for_window_with_transparency(
        window: Arc<winit::window::Window>,
        context: Option<&GpuContext>,
        transparent: bool,
    ) -> Result<Self, GpuError> {
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let owned;
        let context = match context {
            Some(context) => context,
            None => {
                owned = GpuContext::create(Some(window.clone()))?;
                &owned
            }
        };
        let surface = context
            .inner
            .instance
            .create_surface(window.clone())
            .map_err(|e| GpuError(e.to_string()))?;
        let mut renderer = Self::create_on(
            Some(window),
            Some(surface),
            size.width,
            size.height,
            context,
            transparent,
            true,
        )?;
        renderer.set_scale_factor(scale);
        Ok(renderer)
    }
    /// A renderer presenting to a surface of the caller's, not a winit
    /// window's (a Wayland popup's `wl_surface`), `width` by `height`
    /// physical pixels at `scale`, on `context`.
    ///
    /// # Safety
    ///
    /// `display` and `window` are live objects a surface can be made on (a
    /// `wl_display` and a `wl_surface` of it, say), and stay so until the
    /// renderer is dropped: it makes its surface from them again whenever
    /// the surface is lost (`wgpu::SurfaceTargetUnsafe::RawHandle`'s
    /// requirements, for each surface).
    pub unsafe fn for_raw_surface(
        display: wgpu::rwh::RawDisplayHandle,
        window: wgpu::rwh::RawWindowHandle,
        width: u32,
        height: u32,
        scale: f32,
        context: &GpuContext,
    ) -> Result<Self, GpuError> {
        // SAFETY: the caller keeps the handles valid while the renderer,
        // and so the surface, lives. The instance was made without a display
        // handle, so `display` need not match one.
        let surface = unsafe {
            context
                .inner
                .instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display),
                    raw_window_handle: window,
                })
        }
        .map_err(|e| GpuError(e.to_string()))?;
        let mut renderer =
            Self::create_on(None, Some(surface), width, height, context, true, false)?;
        renderer.raw = Some((display, window));
        renderer.set_scale_factor(scale);
        Ok(renderer)
    }
    fn create(
        window: Option<Arc<winit::window::Window>>,
        width: u32,
        height: u32,
        context: &GpuContext,
    ) -> Result<Self, GpuError> {
        let surface = window
            .clone()
            .map(|window| context.inner.instance.create_surface(window))
            .transpose()
            .map_err(|e| GpuError(e.to_string()))?;
        Self::create_on(window, surface, width, height, context, true, true)
    }
    /// `create` presenting to `surface` (none: offscreen), with alpha where
    /// the window may be `transparent` (else opaque, on Windows). `own`:
    /// `surface` is `window`'s alone, so on Windows it may present through a
    /// swap chain of its own instead (not into a caller's visual).
    fn create_on(
        window: Option<Arc<winit::window::Window>>,
        surface: Option<wgpu::Surface<'static>>,
        width: u32,
        height: u32,
        context: &GpuContext,
        transparent: bool,
        #[cfg_attr(not(windows), allow(unused))] own: bool,
    ) -> Result<Self, GpuError> {
        let instance = context.inner.instance.clone();
        let device = context.inner.device.clone();
        let queue = context.inner.queue.clone();
        let adapter = &context.inner.adapter;
        let info = context.inner.info.clone();
        // Direct3D 11 presents through a swap chain of its own alone (made
        // below): the no-op backend's surface is let go.
        #[cfg(d3d11)]
        let surface = surface.filter(|_| context.inner.d3d11.is_none());
        if surface
            .as_ref()
            .is_some_and(|surface| !adapter.is_surface_supported(surface))
        {
            return Err(GpuError(
                "surface is incompatible with the shared GPU adapter".into(),
            ));
        }
        let width = width.max(1);
        let height = height.max(1);
        // Windows: a swap chain of the renderer's own, whose buffers are the
        // retained frame (dxgi_present.rs); wgpu's surface then stays
        // unconfigured. Without one, wgpu's.
        #[cfg(windows)]
        let hwnd = |window: &Arc<winit::window::Window>| {
            use wgpu::rwh::HasWindowHandle;
            let wgpu::rwh::RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw()
            else {
                return None;
            };
            Some(windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _))
        };
        #[cfg(windows)]
        let own_chain = window
            .as_ref()
            .filter(|_| own && *DXGI_PRESENT && info.backend == wgpu::Backend::Dx12)
            .and_then(|window| {
                let hwnd = hwnd(window)?;
                dxgi_present::Chain::new(&device, hwnd, width, height, transparent)
                    .inspect_err(|e| {
                        eprintln!("zgx: no swap chain of its own ({e}): presenting through wgpu's")
                    })
                    .ok()
            });
        // Direct3D 11 has no other way to present: a window without one fails.
        #[cfg(d3d11)]
        let own_chain = match (&context.inner.d3d11, &window) {
            (Some(d3d11), Some(window)) => {
                let hwnd = hwnd(window).filter(|_| own).ok_or_else(|| {
                    GpuError("Direct3D 11 presents to its own windows alone".into())
                })?;
                Some(dxgi_present::Chain::new_d3d11(
                    &device,
                    d3d11,
                    hwnd,
                    width,
                    height,
                    transparent,
                )?)
            }
            _ => own_chain,
        };
        #[cfg(d3d11)]
        let d3d11 = context
            .inner
            .d3d11
            .clone()
            .map(d3d11_frame::State::new)
            .transpose()?;
        #[cfg(windows)]
        let own_surfaces = own_chain.is_some();
        #[cfg(not(windows))]
        let own_surfaces = false;
        let mut translucent_alpha = wgpu::CompositeAlphaMode::Auto;
        let config = surface.as_ref().map(|s| {
            let mut c = s
                .get_default_config(adapter, width, height)
                .expect("supported surface");
            let caps = s.get_capabilities(adapter);
            translucent_alpha = [
                wgpu::CompositeAlphaMode::PreMultiplied,
                wgpu::CompositeAlphaMode::PostMultiplied,
                wgpu::CompositeAlphaMode::Inherit,
                wgpu::CompositeAlphaMode::Opaque,
            ]
            .into_iter()
            .find(|mode| caps.alpha_modes.contains(mode))
            .unwrap_or(c.alpha_mode);
            // Windows: the compositor blends a swap chain with alpha over
            // what lies behind every frame, opaque or not.
            c.alpha_mode = if cfg!(windows)
                && !transparent
                && caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque)
            {
                wgpu::CompositeAlphaMode::Opaque
            } else {
                translucent_alpha
            };
            c.present_mode = wgpu::PresentMode::AutoVsync;
            // Windows: a present replaces one the compositor has not taken
            // yet (DXGI's flip model at sync interval 0 composes the newest,
            // without tearing), so input drawn between refreshes shows on
            // the next; frames are paced by the compositor's clock instead.
            if cfg!(windows) && caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
                c.present_mode = wgpu::PresentMode::Mailbox;
            }
            // Two drawables: measured no frame-rate or CPU cost against three
            // on Apple silicon, and one fewer window-sized buffer (~13 MB at
            // 5K) while animating. Resizing keeps a third (`widen_drawables`).
            // Windows keeps the default two: its presents replace one another
            // (above), and a second within a refresh must not wait for the
            // compositor to take the first.
            if cfg!(target_os = "macos") {
                c.desired_maximum_frame_latency = 1;
            }
            // The retained target already holds sRGB-encoded bytes, which the
            // compositor reads the same from either format. A non-sRGB drawable
            // takes them as they are, where an sRGB one needs a per-frame
            // aliasing view and cannot be framebuffer-only.
            let plain = c.format.remove_srgb_suffix();
            if caps.formats.contains(&plain) {
                c.format = plain;
            } else if c.format.is_srgb() {
                c.view_formats = vec![plain];
            }
            // Off macOS, where drawables must stay framebuffer-only to be
            // cheap, present by copy when the surface allows it.
            if !cfg!(target_os = "macos")
                && c.format == FORMAT
                && !unpremultiplies(&c)
                && caps.usages.contains(wgpu::TextureUsages::COPY_DST)
            {
                c.usage |= wgpu::TextureUsages::COPY_DST;
                // `ZGX_SWAP_CHECK` reads each image back (`check_swap`).
                #[cfg(target_os = "linux")]
                if *SWAP_CHECK && caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
                    c.usage |= wgpu::TextureUsages::COPY_SRC;
                }
            }
            if !own_surfaces {
                s.configure(&device, &c);
            }
            c
        });
        // Direct3D 11's swap chain is configured as wgpu's would be on
        // Direct3D 12 (what is read of it: its alpha, `match_background`).
        #[cfg(d3d11)]
        let config = match (&d3d11, own_surfaces) {
            (Some(_), true) => {
                translucent_alpha = wgpu::CompositeAlphaMode::PreMultiplied;
                Some(wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: FORMAT,
                    width,
                    height,
                    present_mode: wgpu::PresentMode::Mailbox,
                    desired_maximum_frame_latency: 2,
                    alpha_mode: match transparent {
                        true => translucent_alpha,
                        false => wgpu::CompositeAlphaMode::Opaque,
                    },
                    view_formats: Vec::new(),
                })
            }
            _ => config,
        };
        #[cfg(not(windows))]
        let _ = translucent_alpha;
        let copy_present = !own_surfaces
            && config
                .as_ref()
                .is_some_and(|c| c.usage.contains(wgpu::TextureUsages::COPY_DST));
        // A window on macOS presents surfaces of its own (see `LayerPresent`),
        // and on Windows with its own swap chain.
        let layer_presenting = own_surfaces
            || (cfg!(target_os = "macos")
                && config.is_some()
                && info.backend == wgpu::Backend::Metal);
        // Single-pass presentation writes the drawable in a non-sRGB format,
        // storing the retained target's bytes unchanged, exactly what the
        // present blit produces for opaque pixels.
        let single_pass_format = config
            .as_ref()
            .filter(|_| !layer_presenting)
            .map(|c| c.format.remove_srgb_suffix());
        // Presenting surfaces of its own, each frame draws into one of them
        // (`begin_layer_frame`): a window-sized target would only be dropped,
        // and the driver keeps the memory it held.
        let target = if layer_presenting {
            texture(&device, 1, 1, "retained output placeholder")
        } else {
            texture(&device, width, height, "retained output")
        };
        let view = target.create_view(&Default::default());
        #[cfg(target_os = "macos")]
        if !layer_presenting {
            written(&device, &queue, &view);
        }
        let atlas = sampled_texture(
            &device,
            context.inner.mapped_uploads,
            1,
            1,
            "lazy glyph atlas placeholder",
        );
        let split_shading = true;
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &context.inner.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &atlas.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(
                        &device.create_sampler(&wgpu::SamplerDescriptor::default()),
                    ),
                },
            ],
        });
        let present_unpremultiplies = config.as_ref().is_some_and(unpremultiplies);
        // Presenting surfaces of its own, the target is never blitted (and a
        // blit bind group would keep the first one alive).
        let blit = config
            .as_ref()
            .filter(|_| !layer_presenting)
            .map(|c| blit_pipeline(&device, c.format, unpremultiplies(c)));
        let blit_bind = blit.as_ref().map(|p| blit_group(&device, p, &view));
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("retained frame vertices"),
            size: VERTICES_AT_REST as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        // A scene laid out without `install_text` is measured by its text.
        Scene::set_default_text_services(Some(context.inner.text_default.clone()));
        Ok(Self {
            context: context.clone(),
            #[cfg(windows)]
            dxgi_stats: Default::default(),
            #[cfg(windows)]
            translucent_alpha,
            #[cfg(windows)]
            background_matched: false,
            #[cfg(all(windows, feature = "composition"))]
            composition: None,
            scene_identity: None,
            pruned_revision: None,
            pending: RefCell::new(None),
            viewports: FxHashMap::default(),
            no_regions: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("no clip regions"),
                size: std::mem::size_of::<Region>() as u64,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            }),
            node_images: FxHashMap::default(),
            scene_images: None,
            ramps: FxHashMap::default(),
            node_ramps: FxHashMap::default(),
            failed_svgs: Default::default(),
            viewport_bind: None,
            belt: RefCell::new(wgpu::util::StagingBelt::new(device.clone(), 256 * 1024)),
            mapped: context
                .inner
                .vertex_ring
                .then(|| RefCell::new(upload::MappedRing::new(&device, &queue))),
            unified: context.inner.mapped_uploads,
            texture_uploads: Default::default(),
            instance,
            window,
            raw: None,
            device,
            queue,
            info,
            surface,
            config,
            width,
            height,
            scale: 1.,
            target,
            view,
            split_shading,
            cull_glyphs: true,
            bind,
            blit,
            blit_bind,
            spare: None,
            target_copy_bind: None,
            deferred: RefCell::new(None),
            single_pass: None,
            single_pass_format,
            #[cfg(target_os = "macos")]
            layer_present: layer_presenting.then(LayerPresent::default),
            #[cfg(target_os = "macos")]
            loop_sheets: Vec::new(),
            #[cfg(target_os = "macos")]
            loops_said: Vec::new(),
            #[cfg(target_os = "macos")]
            loop_watch: Default::default(),
            #[cfg(target_os = "macos")]
            loop_time: None,
            #[cfg(windows)]
            layer_present: own_chain.map(|chain| {
                let mut present = LayerPresent::default();
                present.chain = Some(chain);
                present
            }),
            #[cfg(target_os = "macos")]
            metal_frame: Default::default(),
            #[cfg(target_os = "macos")]
            metal_inflight: RefCell::new(None),
            #[cfg(target_os = "macos")]
            idle: false,
            #[cfg(target_os = "macos")]
            #[cfg(target_os = "macos")]
            display_key: String::new(),
            #[cfg(target_os = "macos")]
            frame_started: None,
            #[cfg(target_os = "macos")]
            gpu_time: std::cell::Cell::new(None),
            #[cfg(any(target_os = "macos", d3d11))]
            viewport_values: [0.; 4],
            #[cfg(d3d11)]
            d3d11,
            #[cfg(d3d11)]
            view_host: Default::default(),
            #[cfg(own_surfaces)]
            copy_offset: None,
            in_layer: false,
            layer_root: None,
            present_unpremultiplies,
            copy_present,
            #[cfg(target_os = "linux")]
            swap: SwapHistory::default(),
            single_pass_frames: 0,
            debug_scroll_copy: false,
            scroll_copied: std::cell::Cell::new(0),
            copy_next: std::cell::Cell::new(false),
            direct_frames: 0,
            metal_frames: 0,
            warms: 0,
            present_stats: GpuStats::default(),
            atlas,
            atlas_size: 1,
            atlas_grew_to: 0,
            warned: Vec::new(),
            issues: Vec::new(),
            entries: FxHashMap::default(),
            cursor: (0, 0, 0),
            fonts: context.text_system(),
            text_cache: context.text_cache(),
            swash: SwashCache::new(),
            layers: FxHashMap::default(),
            banding: false,
            bands_off: false,
            bands_seen: false,
            band_keys: Vec::new(),
            band_pool: Vec::new(),
            layer_frame: 0,
            layer_shifted: false,
            layer_origin: (0., 0.),
            shapes: FxHashMap::default(),
            shaped_like: FxHashMap::default(),
            shaped_bytes: 0,
            fresh: true,
            whole_frames: 0,
            rendered_serial: None,
            copies: Vec::new(),
            atlas_epoch: 1,
            frame: 0,
            vertices,
            vertex_capacity: VERTICES_AT_REST,
            frame_quads: Vec::new(),
            frame_points: Vec::new(),
            kept: None,
            patch_stats: GpuStats::default(),
            patched: false,
            patchable: Patchable::default(),
            unpatched_keeps: 0,
            item_ranges: Vec::new(),
            patch_clips: Vec::new(),
            region_interiors: Vec::new(),
            box_quads: Vec::new(),
            kept_upload: None,
            kept_ordered: 0,
            never_keep: false,
            changed_texts: Default::default(),
            box_draws: Vec::new(),
            draw_made: false,
            frame_draws: Vec::new(),
            frame_peaks: (0, 0),
            visible_glyphs: Vec::new(),
            visible_colored: false,
            blurred: None,
            background: Color(0, 0, 0, 0),
            images: FxHashMap::default(),
            images_awaited: false,
            layer_incomplete: false,
            layer_glyph_too_large: (0., false),
            redrawing_shifted: false,
            redrawing_glyphs: Vec::new(),
            images_uploaded: false,
            image_drawn: FxHashMap::default(),
            images_gone: Default::default(),
            image_holders: Default::default(),
            image_frame: 0,
            frame_scene: None,
            images_decoding: 0,
            images_awaiting: FxHashMap::default(),
            images_softer: FxHashMap::default(),
            images_to_share: false,
            images_outgoing: FxHashMap::default(),
            image_share: usize::MAX,
            layers_coarser: FxHashSet::default(),
            views: None,
            #[cfg(target_os = "macos")]
            plane_wake: None,
            backdrop: None,
            unshadowed: None,
            #[cfg(target_os = "macos")]
            zoom: 1.,
            #[cfg(target_os = "macos")]
            native_surfaces: None,
            #[cfg(target_os = "macos")]
            transaction_present: false,
            #[cfg(target_os = "macos")]
            wide_until: None,
            canvases: Default::default(),
            nearest: None,
            outlines: None,
            svgs: Default::default(),
            blur_pipelines: None,
            image_cells: FxHashMap::default(),
            // As if last recycled long ago: the first fill recycles.
            atlas_recycled_at: 0_u64.wrapping_sub(ATLAS_THRASH_FRAMES),
            blur_scratch: None,
            effects: None,
            effect_pipelines: None,
            effect_jobs: Vec::new(),
            fill_pipelines: FxHashMap::default(),
            fills_declared: 0,
            offscreen_waits: true,
            debug_waits_for_shaders: false,
            fill_shaders_declared: 0,
            layers_declared: 0,
            filter_pipelines: FxHashMap::default(),
            chains: FxHashMap::default(),
            parked_inputs: Vec::new(),
            blend_pipelines: FxHashMap::default(),
            blend_layouts: std::cell::OnceCell::new(),
            backdrops: FxHashMap::default(),
            image_effect_pipeline: Default::default(),
            fill_shaders: FxHashMap::default(),
            filter_shaders: FxHashMap::default(),
            plane: None,
            backdrop_walk: None,
            debug_regions: None,
        })
    }
    /// Bytes the native device reports as allocated (Metal only), including
    /// driver-owned resources that the cache counters cannot see.
    pub fn debug_device_allocated_bytes(&self) -> Option<u64> {
        #[cfg(target_os = "macos")]
        {
            use objc2_metal::MTLDevice;
            // SAFETY: The raw device is only queried; nothing destroys it.
            let metal = unsafe { self.device.as_hal::<wgpu::hal::api::Metal>() }?;
            Some(metal.raw_device().currentAllocatedSize() as u64)
        }
        #[cfg(not(target_os = "macos"))]
        None
    }
    /// The renderer's lines of a memory report: text it has prepared and
    /// rasterized, and what it keeps on the GPU.
    pub fn memory(&self, report: &mut zgx::memory::Report) {
        let stats = self.debug_cache_stats();
        let (lines, measures) = self.text_cache.borrow().bytes();
        {
            // Fonts given as bytes are on the heap; the system's are mapped
            // from their files, kept by the system and clean.
            let fonts = self.fonts.borrow();
            let (mut faces, mut seen) = (0, rustc_hash::FxHashSet::default());
            let (mut held, mut mapped, mut listed) = (0, 0, 0);
            for face in fonts.db().faces() {
                faces += 1;
                // The database's entry: its names, and its file's path.
                listed += size_of::<cosmic_text::fontdb::FaceInfo>()
                    + face.families.capacity()
                        * size_of::<(String, cosmic_text::fontdb::Language)>()
                    + face
                        .families
                        .iter()
                        .map(|(name, _)| name.capacity())
                        .sum::<usize>()
                    + face.post_script_name.capacity()
                    + match &face.source {
                        cosmic_text::fontdb::Source::File(path)
                        | cosmic_text::fontdb::Source::SharedFile(path, _) => {
                            path.as_os_str().len()
                        }
                        cosmic_text::fontdb::Source::Binary(_) => 0,
                    };
                match &face.source {
                    cosmic_text::fontdb::Source::Binary(data) => {
                        if seen.insert(std::sync::Arc::as_ptr(data) as *const () as usize) {
                            held += (**data).as_ref().len();
                        }
                    }
                    cosmic_text::fontdb::Source::SharedFile(_, data) => {
                        if seen.insert(std::sync::Arc::as_ptr(data) as *const () as usize) {
                            mapped += (**data).as_ref().len();
                        }
                    }
                    cosmic_text::fontdb::Source::File(_) => {}
                }
            }
            report.add("text", "font database", Some(faces), Some(listed));
            report.add(
                "text",
                "fonts loaded (parsed, uncounted)",
                Some(fonts.loaded_fonts()),
                None,
            );
            report.add("text", "fonts given as bytes", Some(faces), Some(held));
            report.add(
                "text",
                "fonts mapped from files (their size)",
                None,
                Some(mapped),
            );
        }
        let runs = &self.fonts.borrow().shape_run_cache;
        report.add(
            "text",
            "shaped nodes",
            Some(stats.shaped_nodes),
            Some(stats.shaped_bytes),
        );
        report.add("text", "prepared lines", None, Some(lines));
        report.add("text", "measures", None, Some(measures));
        report.add("text", "shaped runs", Some(runs.len()), Some(runs.bytes()));
        report.add(
            "text",
            "shaping buffers and caches",
            None,
            Some(self.fonts.borrow().cache_bytes()),
        );
        let images: usize = (self.swash.image_cache.values().flatten())
            .map(|image| image.data.capacity())
            .sum();
        report.add(
            "text",
            "rasterized glyphs",
            Some(stats.swash_images),
            Some(images),
        );
        report.add("text", "glyph outlines", Some(stats.swash_outlines), None);
        report.add(
            "gpu",
            "glyph atlas",
            Some(stats.atlas_entries),
            Some(stats.atlas_bytes),
        );
        report.add(
            "gpu",
            "image textures",
            Some(stats.image_textures),
            Some(stats.image_bytes),
        );
        report.add(
            "gpu",
            "images softer for the budget",
            Some(stats.images_softer),
            None,
        );
        report.add(
            "gpu",
            "layers",
            Some(stats.layer_textures),
            Some(stats.layer_bytes),
        );
        // Each layer: its node, its content's pixels and scale, and whether
        // it is still one (`zgx inspect memory` names the node).
        let live: usize = self.layers.values().map(LayerCache::live_bytes).sum();
        report.add("gpu", "layers live", Some(self.layers.len()), Some(live));
        let mut layers: Vec<_> = self.layers.iter().collect();
        layers.sort_by_key(|(_, layer)| std::cmp::Reverse(layer.bytes));
        for (id, layer) in layers {
            let (w, h) = (layer.texture.width() as f32, layer.texture.height() as f32);
            let state = match (layer.parked, layer.transformed) {
                (true, _) => ", parked",
                (false, true) => ", transformed",
                (false, false) => "",
            };
            let coarser = match self.layers_coarser.contains(id) {
                true => ", coarser for its budget",
                false => "",
            };
            report.add(
                "gpu layer",
                format!(
                    "@{} {}x{} px at {:.2}x, {:.1} MiB live{state}{coarser}",
                    id.number(),
                    (w * layer.uv[0]).round(),
                    (h * layer.uv[1]).round(),
                    self.scale * f32::from_bits(layer.factor),
                    layer.live_bytes() as f32 / MB,
                ),
                None,
                Some(layer.bytes),
            );
        }
        report.add("gpu", "canvases", None, Some(stats.canvas_raster_bytes));
        report.add("gpu", "svgs", None, Some(stats.svg_raster_bytes));
        report.add(
            "gpu",
            "vertex buffers",
            Some(stats.vertex_buffers),
            Some(stats.vertex_buffer_bytes),
        );
        if let Some(bytes) = self.debug_device_allocated_bytes() {
            report.add("gpu", "device allocated", None, Some(bytes as usize));
        }
    }
    /// Whether the glyph atlas has grown past its first size
    /// (`trim_atlas` gives that back).
    pub fn atlas_grown(&self) -> bool {
        self.atlas_size > INITIAL_ATLAS
    }
    /// The glyph atlas down to the smallest that holds what the window's
    /// text and icons draw now, for a window idle a long while: up to 16 MB
    /// of the GPU's memory, grown for a page's peak. The glyphs its text
    /// holds (`shapes`) and its icons' cells are copied across, not
    /// rasterized again, so the next frame draws from them as before; what
    /// went with pages since gone is dropped. An atlas its screen fills is
    /// kept. (Rasterizing a dense page again took 9 to 18 ms on an M5 for a
    /// screen of CJK text, a hitch on the frame after the user came back.)
    pub fn trim_atlas(&mut self) {
        if !self.atlas_grown() {
            return;
        }
        self.flush_pending();
        if !self.shrink_atlas() {
            // No copy here (Direct3D 11, uploads still queued): started over.
            self.atlas_grew_to = self.atlas_size;
            self.kept = None;
            self.reset_atlas(INITIAL_ATLAS);
            self.atlas_recycled_at = self.frame;
        }
        // No frame comes to let the old one go: the device, polled, does.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
    /// `trim_atlas`'s copy into a smaller atlas: false where it can't copy,
    /// true when done or when no smaller atlas holds what is live.
    fn shrink_atlas(&mut self) -> bool {
        if !self.texture_uploads.is_empty() {
            return false;
        }
        // Live: the glyphs of the text the renderer holds, and every icon
        // and wave tile (few, and not told apart by who shows them).
        let mut live: FxHashMap<AtlasKey, Option<AtlasEntry>> = FxHashMap::default();
        for shaped in self.shapes.values() {
            for glyph in &shaped.glyphs {
                let key = AtlasKey::from(glyph.key);
                if let Some(entry) = self.entries.get(&key) {
                    live.insert(key, *entry);
                }
            }
        }
        let glyphs: Vec<(AtlasKey, AtlasEntry)> = live
            .iter()
            .filter_map(|(key, entry)| entry.map(|entry| (*key, entry)))
            .collect();
        let icons: Vec<(u64, (u32, u32, u32, u32))> = self
            .image_cells
            .iter()
            .map(|(id, cell)| (*id, *cell))
            .collect();
        let sizes: Vec<(u32, u32)> = glyphs
            .iter()
            .map(|(_, e)| (e.width, e.height))
            .chain(icons.iter().map(|(_, (_, _, w, h))| (*w, *h)))
            .collect();
        let Some(Packed {
            size,
            origins,
            cursor,
        }) = pack_cells(&sizes, self.atlas_size)
        else {
            return true;
        };
        let atlas = sampled_texture(&self.device, self.unified, size, size, "glyph atlas");
        // Each cell with its one-texel gutter, as `atlas_cell` wrote it.
        let cells = glyphs
            .iter()
            .map(|(_, e)| (e.x, e.y))
            .chain(icons.iter().map(|(_, (x, y, _, _))| (*x, *y)))
            .zip(&sizes)
            .zip(&origins)
            .map(|(((x, y), (w, h)), (to_x, to_y))| {
                (x - 1, y - 1, w + 2, h + 2, to_x - 1, to_y - 1)
            });
        // Direct3D 11: its own atlas's cells copied into one the new size.
        #[cfg(d3d11)]
        if let Some(state) = &mut self.d3d11 {
            let Some(old) = state.atlas.take() else {
                return false;
            };
            let new = match state.device.surface(size, size, false) {
                Ok(new) => new,
                Err(_) => {
                    state.atlas = Some(old);
                    return false;
                }
            };
            state.device.copy_cells(&old, &new, cells);
            state.atlas = Some(new);
            return self.atlas_shrunk(atlas, size, cursor, live, glyphs, icons, origins);
        }
        #[cfg(target_os = "macos")]
        let copied_on_cpu = self.unified;
        #[cfg(not(target_os = "macos"))]
        let copied_on_cpu = false;
        if copied_on_cpu {
            // Shared textures the CPU writes: read and written in place.
            #[cfg(target_os = "macos")]
            for (x, y, w, h, to_x, to_y) in cells {
                upload::copy_texels(&self.atlas, &atlas, (x, y, w, h), (to_x, to_y));
            }
        } else {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("glyph atlas shrunk"),
                });
            for (x, y, w, h, to_x, to_y) in cells {
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.atlas,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &atlas,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: to_x,
                            y: to_y,
                            z: 0,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
            }
            self.queue.submit([encoder.finish()]);
        }
        self.atlas_shrunk(atlas, size, cursor, live, glyphs, icons, origins)
    }
    /// The atlas `shrink_atlas` copied into, `size` square, taken: the live
    /// glyphs and icons at their `origins` there.
    #[allow(clippy::too_many_arguments)]
    fn atlas_shrunk(
        &mut self,
        atlas: wgpu::Texture,
        size: u32,
        cursor: (u32, u32, u32),
        live: FxHashMap<AtlasKey, Option<AtlasEntry>>,
        glyphs: Vec<(AtlasKey, AtlasEntry)>,
        icons: Vec<(u64, (u32, u32, u32, u32))>,
        origins: Vec<(u32, u32)>,
    ) -> bool {
        let mut entries: FxHashMap<AtlasKey, Option<AtlasEntry>> =
            live.into_iter().filter(|(_, e)| e.is_none()).collect();
        let mut origins = origins.into_iter();
        for ((key, entry), (x, y)) in glyphs.into_iter().zip(origins.by_ref()) {
            entries.insert(key, Some(AtlasEntry { x, y, ..entry }));
        }
        self.image_cells = icons
            .into_iter()
            .zip(origins)
            .map(|((id, (_, _, w, h)), (x, y))| (id, (x, y, w, h)))
            .collect();
        self.entries = entries;
        if let Some(blurred) = &mut self.blurred {
            blurred.glyphs.clear();
        }
        self.atlas_grew_to = self.atlas_size;
        self.atlas = atlas;
        self.atlas_size = size;
        self.bind = self.texture_bind(&self.atlas.create_view(&Default::default()), "glyph atlas");
        self.cursor = cursor;
        // Every placed quad's cell moved: placed again from `entries`.
        self.atlas_epoch = self.atlas_epoch.wrapping_add(1);
        self.kept = None;
        self.atlas_recycled_at = self.frame;
        true
    }
    /// What its caches hold now.
    pub fn debug_cache_stats(&self) -> DebugCacheStats {
        let vertex_buffers = self.mapped.as_ref().map_or(1, |ring| ring.borrow().len());
        // Direct3D 11's ring holds them there (d3d11_frame.rs).
        #[cfg(d3d11)]
        let vertex_buffers = self
            .d3d11
            .as_ref()
            .map_or(vertex_buffers, |state| state.ring.len());
        DebugCacheStats {
            shaped_nodes: self.shapes.len(),
            shaped_bytes: self.shapes.values().map(Shaped::bytes).sum(),
            cached_quads: self.shapes.values().map(|s| s.quads.len()).sum(),
            atlas_entries: self.entries.len(),
            atlas_bytes: (self.atlas_size * self.atlas_size * 4) as usize,
            swash_images: self.swash.image_cache.len(),
            swash_outlines: self.swash.outline_command_cache.len(),
            image_textures: self.images.len(),
            image_bytes: self.images.values().map(|i| i.1).sum(),
            images_softer: (self.images_softer.keys())
                .filter(|id| self.images.contains_key(id))
                .count(),
            canvas_raster_bytes: self.canvases.bytes(),
            svg_raster_bytes: self.svgs.bytes(),
            vertex_buffer_bytes: self.vertex_capacity,
            vertex_buffers,
            layer_textures: self.layers.len(),
            layers_coarser: (self.layers_coarser.iter())
                .filter(|id| self.layers.contains_key(id))
                .count(),
            layer_bytes: self.layers.values().map(|l| l.bytes).sum(),
            // (Not zgx's own plain blend's, kept as any built-in pipeline is.)
            shader_pipelines: self.fill_pipelines.len()
                + self.filter_pipelines.len()
                + self
                    .blend_pipelines
                    .keys()
                    .filter(|id| **id != BLEND_PLAIN)
                    .count(),
            #[cfg(target_os = "macos")]
            surface_collections: self
                .native_surfaces
                .as_ref()
                .map_or(0, |cache| cache.collections()),
            #[cfg(not(target_os = "macos"))]
            surface_collections: 0,
        }
    }
    /// At the first frame, a swap chain made opaque under a background that
    /// lets what is behind show is made again with alpha. (One with alpha
    /// under an opaque background is right as it is, if dearer to compose.)
    #[cfg(windows)]
    fn match_background(&mut self) -> Result<(), GpuError> {
        if std::mem::replace(&mut self.background_matched, true) {
            return Ok(());
        }
        let opaque = self
            .config
            .as_ref()
            .is_some_and(|c| c.alpha_mode == wgpu::CompositeAlphaMode::Opaque);
        if !opaque || self.background.3 == 255 {
            return Ok(());
        }
        if let Some(config) = &mut self.config {
            config.alpha_mode = self.translucent_alpha;
        }
        // Its own swap chain is made again with alpha, before a buffer of it
        // becomes the target (`begin_layer_frame`).
        if let Some(present) = &mut self.layer_present
            && let Some(chain) = present.chain.take()
        {
            chain.release(present.buffers.drain(..).map(|(buffer, _)| buffer));
            present.shown = None;
            present.history.clear();
            // Only before the first frame, which draws it whole.
            present.chain = Some(chain.remade(true)?);
            return Ok(());
        }
        // The old swap chain, and the window's composition target with it,
        // go before the new one is made.
        self.surface = None;
        surface::SurfaceSource::recreate(self)?;
        surface::SurfaceSource::configure(self);
        Ok(())
    }
    /// Whether its window's swap chain presents opaque (the compositor
    /// ignores its alpha). None offscreen.
    pub fn presents_opaque(&self) -> Option<bool> {
        (self.config.as_ref()).map(|c| c.alpha_mode == wgpu::CompositeAlphaMode::Opaque)
    }
    /// Show `caret` over the window's frames, drawn and blinked by Core
    /// Animation (`zgx::caret_blink::NativeCaret`), or none. Call inside the
    /// transaction that presents the frame which stops (or starts) drawing
    /// the caret itself, so the two never show together; alone, it makes
    /// its own. Nothing where frames show through a layer of the
    /// renderer's own.
    #[cfg(target_os = "macos")]
    pub fn set_caret(&mut self, caret: Option<Caret>) {
        if let Some(present) = self.layer_present.as_mut() {
            present.caret = caret;
            if let Some(layer) = present.layer.as_mut() {
                layer.set_caret(caret);
            }
        }
    }
    /// The window's zoom (`Ui::zoom`), which `set_scale_factor`'s scale
    /// holds. Layers placed over the frames (a baked loop's, a GPU view's)
    /// go in the host layer's points, `scale / zoom` pixels each, where the
    /// frames' own points are `scale` pixels.
    #[cfg(target_os = "macos")]
    pub fn set_zoom(&mut self, zoom: f64) {
        self.zoom = zoom;
    }
    /// Whether a caret set with `set_caret` can show: frames show through
    /// a layer the renderer presents to.
    #[cfg(target_os = "macos")]
    pub fn shows_carets(&self) -> bool {
        self.layer_present.is_some()
    }
    /// Test hook: keep no frame to patch, drawing each afresh (a reference
    /// for frames patched into a kept one).
    #[doc(hidden)]
    pub fn debug_never_keep(&mut self) {
        self.never_keep = true;
        self.kept = None;
    }
    /// The colour the target clears to where nothing is drawn.
    pub fn set_background(&mut self, color: Color) {
        if self.background != color {
            self.background = color;
            self.fresh = true;
        }
    }
    /// The GPU adapter it renders with.
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.info.clone()
    }
    /// The font system it shapes and draws text with.
    pub fn text_system(&self) -> Rc<RefCell<FontSystem>> {
        self.fonts.clone()
    }
    /// Prepared rich text shared with layout measurement.
    pub fn text_cache(&self) -> Rc<RefCell<prepared::TextCache>> {
        self.text_cache.clone()
    }
    /// Install this renderer's text services on `scene`: native shaping,
    /// and measurement from the prepared text it draws from, so layout and
    /// drawing break lines identically. Call `Ui::refresh_text_geometry`
    /// afterwards for a scene with mounted text. Without it a scene takes a
    /// renderer's text services by itself (`GpuRenderer::new`); installing
    /// says which, and keeps them.
    pub fn install_text(&self, scene: &mut Scene) {
        install_text(scene, &self.text_system(), &self.text_cache());
    }
    /// A font its context shapes with was replaced
    /// (`GpuContext::replace_font`): the glyphs and text shapes it keeps are
    /// given up, and the next frame draws them again.
    pub fn fonts_replaced(&mut self) {
        self.flush_pending();
        self.recycle_atlas();
        self.layers.clear();
        self.shapes.clear();
        self.shaped_bytes = 0;
        self.fresh = true;
    }
    /// Physical pixels per logical point (at least 0.1; 1 if not finite).
    pub fn set_scale_factor(&mut self, scale: f32) {
        let scale = if scale.is_finite() {
            scale.max(0.1)
        } else {
            1.
        };
        if self.scale != scale {
            self.flush_pending();
            self.recycle_atlas();
            self.scale = scale;
            self.layers.clear();
            self.shapes.clear();
            // GPU views draw their next frames at the new scale.
            if let Some(views) = &mut self.views {
                for image in views.forget() {
                    self.images.remove(&image);
                }
            }
            self.fresh = true;
        }
    }
    /// What the window's swap chain images hold is no longer known (an
    /// expose, the window shown again, a change of mode): each present copies
    /// all of the frame until each image has taken one. Nothing to forget
    /// where presents copy all of it anyway (all but Linux's Vulkan).
    pub fn forget_presented(&mut self) {
        #[cfg(target_os = "linux")]
        self.swap.forget();
    }
    /// Whether the size changed.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return false;
        }
        #[cfg(own_surfaces)]
        if let Some(present) = &mut self.layer_present {
            present.current = None;
            present.changed.clear();
            present.catch_up.clear();
            #[cfg(target_os = "macos")]
            {
                present.resized_at = Some(std::time::Instant::now());
            }
        }
        self.flush_pending();
        // Layers stay: what they hold changes with layout (their revision),
        // not with the window's size. (Clearing them made a live resize
        // allocate and paint every layer, blurs and all, at every step.)
        // Scratch grows on demand; release it when the window shrinks a lot.
        if self.blur_scratch.as_ref().is_some_and(|scratch| {
            scratch
                .atlas
                .iter()
                .any(|texture| texture.width() > width * 2 || texture.height() > height * 2)
        }) {
            self.blur_scratch = None;
        }
        self.width = width;
        self.height = height;
        self.spare = None;
        self.target_copy_bind = None;
        // A surface of the renderer's own becomes the target
        // (`begin_layer_frame`), resized there on Windows.
        #[cfg(own_surfaces)]
        let layered = self.layer_present.is_some();
        #[cfg(not(own_surfaces))]
        let layered = false;
        let (target_width, target_height) = if layered { (1, 1) } else { (width, height) };
        self.target = texture(&self.device, target_width, target_height, "retained output");
        self.view = self.target.create_view(&Default::default());
        #[cfg(target_os = "macos")]
        if self.layer_present.is_none() {
            written(&self.device, &self.queue, &self.view);
        }
        // Windows presenting through its own swap chain resizes it now, as
        // the window changes size (a swap chain left at the old size shows
        // black beyond it until the next frame), and leaves wgpu's surface
        // unconfigured. The target no longer holds an old buffer (above).
        #[cfg(windows)]
        if let Some(present) = &mut self.layer_present
            && let Some(chain) = &mut present.chain
        {
            chain.release(present.buffers.drain(..).map(|(buffer, _)| buffer));
            present.shown = None;
            present.history.clear();
            let resized = chain
                .resize(width, height)
                .and_then(|()| chain.buffers(&self.device, width, height));
            match resized {
                Ok(buffers) => present.buffers = buffers.into_iter().map(|b| (b, None)).collect(),
                // The next frame tries again (`begin_layer_frame`).
                Err(error) => eprintln!("zgx: {error}"),
            }
        }
        let own_chain = cfg!(windows) && layered;
        if !own_chain && let (Some(s), Some(c)) = (&self.surface, &mut self.config) {
            c.width = width;
            c.height = height;
            s.configure(&self.device, c);
        }
        // A new swap chain: what its images hold is unknown.
        #[cfg(target_os = "linux")]
        self.swap.forget();
        // AppKit shows the resized layer when its transaction commits. A
        // drawable presented outside it lands a frame late, so the window
        // briefly shows stretched or empty content.
        #[cfg(target_os = "macos")]
        {
            self.set_transaction_present(true);
            self.widen_drawables();
        }
        self.blit_bind = self
            .blit
            .as_ref()
            .map(|p| blit_group(&self.device, p, &self.view));
        self.fresh = true;
        true
    }
    /// Render as `render`, without counting `damage` among what the frame
    /// changed: a repaint for comparison (`ZGX_DAMAGE_CHECK`) must leave
    /// later frames to bring their surfaces up to date as they otherwise would.
    pub fn render_uncounted(
        &mut self,
        scene: &Scene,
        damage: &[Rect],
    ) -> Result<GpuStats, GpuError> {
        #[cfg(own_surfaces)]
        let changed = self
            .layer_present
            .as_ref()
            .map(|present| present.changed.clone());
        #[cfg(target_os = "linux")]
        let swap_changed = self.swap.changed.clone();
        let stats = self.render(scene, damage);
        #[cfg(own_surfaces)]
        if let (Some(present), Some(changed)) = (self.layer_present.as_mut(), changed) {
            present.changed = changed;
        }
        #[cfg(target_os = "linux")]
        {
            self.swap.changed = swap_changed;
        }
        stats
    }
    /// Draw the new frames of GPU views shown on planes of their own
    /// (`zgx::gpu_view::set_planed`), for a frame with nothing else to draw:
    /// a host calls it when `zgx::gpu_view::plane_frames` moved and the
    /// scene has no damage. Whether any was drawn (each presents itself).
    pub fn render_planes(&mut self, scene: &Scene) -> Result<bool, GpuError> {
        if self.views.is_none() {
            return Ok(false);
        }
        self.render_planes_now(scene)
    }
    /// Whether a GPU view's plane still owes a frame (macOS): its last
    /// drawable was not yet shown, so its new frame waits for a tick rather
    /// than the main thread for a drawable. A host calls `render_planes`
    /// again soon, though nothing else asks for a frame.
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn planes_owed(&self) -> bool {
        self.views.as_ref().is_some_and(|views| views.owed)
    }
    /// Wake the host (macOS), from a plane's drawables thread, when a GPU
    /// view's plane that owed a frame has a drawable for it (`planes_owed`):
    /// it then calls `render_planes` at once (`planes_room`).
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn set_plane_waker(&mut self, wake: PlaneWaker) {
        self.plane_wake = Some(wake);
    }
    /// Whether a shown GPU view's plane has a drawable for its owed frame.
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn planes_room(&self) -> bool {
        self.layer_present
            .as_ref()
            .and_then(|present| present.layer.as_ref())
            .is_some_and(|layer| layer.planes_room())
    }
    /// Draw `scene`'s `damage` (logical rectangles) to the target, and report
    /// what it did.
    pub fn render(&mut self, scene: &Scene, damage: &[Rect]) -> Result<GpuStats, GpuError> {
        // Drawn first by this renderer, a scene that took another context's
        // text services (the thread's last made) keeps them: said, once
        // (`take_issues`).
        if scene.drawn_with_other_text_services(&self.context.inner.text_default) {
            self.warn(
                "a scene measured with another renderer's text services is drawn by this one: \
                 install_text chooses",
            );
        }
        #[cfg(target_os = "macos")]
        {
            self.loop_watch.set_damage(damage.len());
            self.idle = false;
            if let Some(views) = &mut self.views {
                views.owed = false;
            }
        }
        let presented = std::mem::take(&mut self.present_stats);
        if compile::declared() + zgx::decoration::layer_shaders_declared() != self.fills_declared {
            self.start_declared_fills();
        }
        self.image_frame += 1;
        self.images_decoding = 0;
        if self.images_to_share {
            self.share_images();
            self.images_awaiting.clear();
            self.images_share();
        }
        if self.image_frame == LAUNCH_FRAMES {
            // An app that never goes idle checks its font cache here, once.
            font_cache_check();
        }
        let mut stats = {
            let _span = zgx::profile::span(zgx::profile::Phase::Paint);
            self.render_inner(scene, damage)?
        };
        if self.views.is_some() {
            self.settle_planes(scene);
        }
        if stats.shaped_nodes > 0 {
            // Words shaped by measurement or this frame are reused by the next
            // shaping frames (streaming text repeats nearly all of them, and a
            // changing value most of its words); ones unused a few shaping
            // frames go, and the table shrinks after a burst, so the cache
            // stays the size of visible text (see `ShapeRunCache::trim`).
            self.fonts.borrow_mut().shape_run_cache.trim(SHAPE_RUN_AGES);
            // Prepared text likewise: what is drawn, and a little of the rest.
            let mut cache = self.text_cache.borrow_mut();
            if cache.needs_trim() {
                cache.trim(self.shapes.values().filter_map(|shaped| shaped.prepared));
            }
        }
        stats.draw_calls += presented.draw_calls;
        stats.instances += presented.instances;
        // Offscreen, drawing is the whole frame; a window's ends once it
        // shows (`present_with_notify`), its wait for the GPU and for its
        // point in a refresh counted in it, not in the next.
        if !self.presents() {
            zgx::profile::frame();
        }
        Ok(stats)
    }
    /// Whether frames are shown (a window's surface, or surfaces of its
    /// own), so `present_with_notify` ends them.
    fn presents(&self) -> bool {
        #[cfg(own_surfaces)]
        if self.layer_present.is_some() {
            return true;
        }
        self.surface.is_some()
    }
    /// Whether a frame since the last call went without an image's pixels,
    /// still decoding: the host draws again when they are
    /// (`zgx::image::set_decoded_wake`).
    pub fn take_images_awaited(&mut self) -> bool {
        std::mem::take(&mut self.images_awaited)
    }
    /// Pixels a frame went without are decoded: whether it did, and if so,
    /// the next frame draws all of the window again (a kept frame would
    /// patch only what the scene changed, and the scene didn't).
    pub fn images_decoded(&mut self) -> bool {
        let awaited = self.take_images_awaited();
        if awaited {
            self.repaint_all();
        }
        awaited
    }
    /// Whether each backdrop held (`backdrop`) may be compressed by the GPU.
    /// For tests.
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn debug_backdrops_compress(&self) -> Vec<bool> {
        let backdrops = self.backdrops.values();
        backdrops
            .map(|(texture, _)| upload::may_compress(texture))
            .collect()
    }
    /// The size of `image`'s texture, if it has one. For tests and devtools.
    #[doc(hidden)]
    pub fn debug_image_size(&self, image: &zgx::image::ImageData) -> Option<(u32, u32)> {
        let held = |id: &u64, size| self.images_outgoing.get(id).copied().unwrap_or(size);
        self.images
            .get(&image.id())
            .map(|entry| match held(&image.id(), entry.3) {
                (0, 0) => (image.width(), image.height()),
                held => held,
            })
    }
    /// Whether a kept file's pixels were uploaded since the last call: their
    /// decoded copy may go (`zgx::image::release_decoded`).
    pub fn take_images_uploaded(&mut self) -> bool {
        std::mem::take(&mut self.images_uploaded)
    }
    /// What frames went on without since the last call (a layer past its
    /// budget, an image with no texture), each said once: a host hands
    /// these on (`zgx::issues::report`). Frames don't fail over them.
    pub fn take_issues(&mut self) -> Vec<String> {
        std::mem::take(&mut self.issues)
    }
    /// A layer went coarser for its budget: `zgx inspect memory` shows it.
    #[cold]
    fn note_layer_coarser(&mut self, id: NodeId) {
        let layers = &self.layers;
        self.layers_coarser
            .retain(|other| *other == id || layers.contains_key(other));
        self.layers_coarser.insert(id);
    }
    /// The bytes isolated layers' textures may take together: four of the
    /// window's frames, and at least `LAYER_BUDGET`. A layer as large as the
    /// window is a frame, so a window of them (an intro's outgoing and
    /// incoming pages, each its own layer) fits.
    fn layer_budget(&self) -> usize {
        (self.width as usize * self.height as usize * 4 * 4).max(LAYER_BUDGET)
    }
    #[cold]
    fn warn(&mut self, issue: &str) {
        if !self.warned.iter().any(|said| said == issue) {
            self.warned.push(issue.to_string());
            self.issues.push(issue.to_string());
        }
    }
    fn render_inner(&mut self, scene: &Scene, damage: &[Rect]) -> Result<GpuStats, GpuError> {
        // For as long as this frame runs, and no longer (`images_showing`).
        self.frame_scene = Some(std::ptr::NonNull::from(scene));
        let result = self.render_frame(scene, damage);
        self.frame_scene = None;
        #[cfg(target_os = "macos")]
        if let Some(cache) = &mut self.native_surfaces {
            cache.collect();
        }
        result
    }
    fn render_frame(&mut self, scene: &Scene, damage: &[Rect]) -> Result<GpuStats, GpuError> {
        if let Some(reason) = self.context.lost() {
            return Err(GpuError::device_lost(&reason));
        }
        loop {
            match self.render_once(scene, damage) {
                Err(error)
                    if error.0 == "glyph atlas capacity exceeded in one frame"
                        && self.atlas_size < MAX_ATLAS =>
                {
                    // Back to the size it had reached, not a doubling at a
                    // time, each of which rasterizes the frame's glyphs.
                    let size = (self.atlas_size * 2).max(self.atlas_grew_to);
                    self.reset_atlas(size.min(MAX_ATLAS))
                }
                result => {
                    #[cfg(target_os = "macos")]
                    if result.is_ok() {
                        self.sync_loops(scene);
                    }
                    return result;
                }
            }
        }
    }
    fn render_once(&mut self, scene: &Scene, damage: &[Rect]) -> Result<GpuStats, GpuError> {
        // Layer shaders with inputs: their pipelines and this frame's passes,
        // before anything draws them.
        if scene.has_layer_inputs() {
            self.prepare_inputs(scene);
        }
        // A frame deferred but never presented draws into the retained target
        // before this one prunes images or layers it may reference.
        self.flush_pending();
        if !self.band_keys.is_empty() {
            self.reclaim_bands();
        }
        if self.scene_identity != Some(scene.identity()) {
            self.scene_identity = Some(scene.identity());
            self.layers.clear();
            self.shapes.clear();
            self.node_images.clear();
            self.outlines = None;
            self.shaped_bytes = 0;
            self.fresh = true;
            self.pruned_revision = None;
            self.scene_images = None;
        }
        // Caches only go stale when nodes are added, removed or change kind.
        if self.pruned_revision != Some(scene.content_revision()) {
            self.pruned_revision = Some(scene.content_revision());
            self.prune_moved(scene);
        }
        let frame = self.layer_frame;
        self.layers.retain(|id, cache| {
            cache.parked = scene.contains(*id) && !scene.is_layer(*id);
            scene.contains(*id)
                && (!cache.parked || cache.transformed && frame - cache.used < LAYER_PARK_FRAMES)
        });
        if let Some(atlas) = &mut self.effects {
            let layers = &self.layers;
            atlas
                .slots
                .retain(|(id, _), _| layers.get(id).is_some_and(|cache| !cache.parked));
            // Unwanted a while, the atlas goes; the next effect makes another.
            if atlas.slots.is_empty() && frame - atlas.used >= LAYER_PARK_FRAMES {
                self.effects = None;
                self.images.remove(&effects::EFFECT_ATLAS);
            }
        }
        // Scroll moves this frame can copy; the rest repaint their clip.
        let (copies, repaint) = match self.plane_offset() {
            // An overlay plane's surface repaints what scrolls.
            Some(_) => (Vec::new(), self.plane_scrolled(scene)),
            None => self.scroll_copies(scene, damage),
        };
        self.copies = copies;
        let extended;
        let damage = if repaint.is_empty() {
            damage
        } else {
            extended = [damage, &repaint].concat();
            &extended[..]
        };
        // An overlay plane draws in its own box's space.
        let local;
        let damage = match self.plane_damage(damage) {
            Some(within) => {
                local = within;
                &local[..]
            }
            None => damage,
        };
        if damage.is_empty() && !self.fresh && self.copies.is_empty() {
            return Ok(GpuStats::default());
        }
        // Visit only subtrees that paint inside the damage the frame will
        // actually repaint. Backdrop blur samples beyond its own subtree, so
        // scenes using it keep the full walk.
        let viewport = Rect::new(
            0.,
            0.,
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        );
        let merged = (!self.fresh && !scene.has_backdrop_blur())
            .then(|| merge_damage(damage, viewport, self.scale));
        // A popup window's renderer walks its root's subtree alone.
        let start = self.plane_popup();
        if start.is_some_and(|root| !scene.contains(root)) {
            return Ok(GpuStats::default());
        }
        // Only leaves repainted since the window was drawn whole: their
        // instances patched in place, the rest drawn as they were.
        let unpatched = self.kept.as_ref().is_some_and(|kept| !kept.patched);
        if let Some(regions) = &merged
            && let Some(stats) = self.render_kept(scene, damage, regions, viewport)?
        {
            self.unpatched_keeps = 0;
            self.patchable.refused = false;
            if let Some(kept) = &mut self.kept {
                kept.patched = true;
            }
            self.patched = true;
            self.rendered_serial = Some(scene.flush_serial());
            return Ok(stats);
        }
        // A kept frame lost before it patched once (one kept each frame a
        // fling repaints whole, lost to the next): after a few, frames drawn
        // whole keep none until frames in a row would patch.
        // (Still kept: patched, drawn as usual for its little damage.)
        match &mut self.kept {
            None if unpatched => {
                self.unpatched_keeps = self.unpatched_keeps.saturating_add(1);
            }
            Some(kept) if merged.is_some() => {
                kept.patched = true;
                self.unpatched_keeps = 0;
            }
            _ => {}
        }
        // Kept up to date, drawn as usual this frame (little damage).
        // (A kept frame's items would hold what a layer now plays.)
        #[cfg(target_os = "macos")]
        let loops_baked = !scene.baked_loops().is_empty();
        #[cfg(not(target_os = "macos"))]
        let loops_baked = false;
        if loops_baked {
            self.kept = None;
        }
        let maintained = self.kept.is_some();
        let was_patched = std::mem::replace(&mut self.patched, maintained);
        // Lost (a change it couldn't patch, before it had patched): kept
        // again once a few frames in a row show it would patch.
        let patchable = match (&merged, maintained) {
            (Some(regions), false) => self.would_patch(scene, regions, viewport),
            _ => 0,
        };
        // A frame that would patch: the next drawn whole keeps again.
        if patchable > 0 {
            self.unpatched_keeps = 0;
        }
        // Drawn whole and kept to patch from: a frame whole anyway (the
        // first, a full repaint), one after a patched frame, so that
        // patching goes on after a change it can't patch, or one after
        // frames it would have patched.
        let keep = !maintained
            && !self.never_keep
            && !loops_baked
            && self.plane.is_none()
            && !self.in_layer
            && !self.atlas_filling()
            && !scene.has_backdrop_blur()
            && !scene.has_overflow_fades()
            && !self.bands_seen
            && (self.fresh
                || was_patched
                || patchable >= KEEP_AGAIN
                || self.unpatched_keeps < UNPATCHED_KEEPS
                    && damage.iter().any(|d| {
                        d.x <= 0.
                            && d.y <= 0.
                            && d.x + d.width >= viewport.width
                            && d.y + d.height >= viewport.height
                    }));
        if !maintained {
            self.kept = None;
        }
        let mut inherited = Vec::new();
        scene.take_painted_fades();
        let (items, merged) = match &merged {
            _ if keep => (
                scene.layer_items_recorded(&mut inherited),
                merged.map(|_| vec![viewport]),
            ),
            Some(regions) => (
                scene.layer_items_within(start, Some(&self.plane_world(regions))),
                merged,
            ),
            // Backdrop blur: the frame draws where its damage reaches
            // through every filter it touches (`render_flat`'s
            // `blur_damage`), from those the walk finds. Grown first through
            // every filter there could be, each as far as it could reach
            // (`Scene::backdrop_filters`), the damage covers all of that, so
            // a walk within it finds the same items there, in the same order,
            // and the same pixels are drawn: what lies outside it draws
            // nothing this frame. (The window's own walk alone: a plane's,
            // a popup's and a layer's place their items otherwise.)
            // Layer shaders reading the backdrop the same: the frame grows its
            // damage over all each reads that it touches (`backdrop_read`, a
            // pixel past it), so the walk grows first through all each could.
            // With readers alone (no blur), what the frame draws is that
            // reach: `render_flat` takes it rather than work it out again.
            None if !self.fresh && start.is_none() && self.plane.is_none() && !self.in_layer => {
                match scene.has_backdrop_readers() {
                    false => {
                        let reach =
                            blur_damage(damage, &scene.backdrop_filters(), viewport, self.scale);
                        (scene.layer_items_within(None, Some(&reach)), merged)
                    }
                    true => (self.walk_over_readers(scene, damage, viewport), merged),
                }
            }
            None => (scene.layer_items(start), merged),
        };
        // Fades its walk passed draw their bands as one; a frame kept to
        // patch would draw them item by item.
        // A frame kept to patch fades item by item: kept only where that is
        // exact (no band of a fade it walked holds items that stack).
        let painted = scene.take_painted_fades();
        let banding = painted && self.band_capable();
        if !painted && merged.is_none() {
            self.bands_seen = false;
        }
        let stacked = banding && keep && self.needs_bands(scene, &items, &[viewport], viewport);
        if stacked {
            self.bands_seen = true;
        }
        // Around native views, only this plane's paint (planes.rs).
        #[cfg(target_os = "macos")]
        let items = self.place_loops(scene, items);
        let items = self.plane_items(items);
        let mut layer_stats = GpuStats::default();
        self.layer_frame += 1;
        // Where layers drawn from another corner show: damaged whole.
        let mut shifted = Vec::new();
        for item in &items {
            if item.isolated && item.effects.opacity > 0. {
                let shown = self.shown(item, damage, viewport);
                let placing = Placing {
                    item,
                    origin: self.layer_origin,
                    scale: self.scale,
                    shows: Shows::Known(viewport),
                };
                self.layer_shifted = false;
                self.prepare_layer(
                    scene,
                    item.id,
                    1.,
                    true,
                    shown,
                    &placing,
                    &mut layer_stats,
                    LAYER_ROOM,
                )?;
                if std::mem::take(&mut self.layer_shifted)
                    && let Some(cache) = self.layers.get(&item.id)
                {
                    let content = cache.content.expand(1.);
                    let window = item.projective().map_or(content, |map| map.bounds(content));
                    if let Some(window) = window.intersection(item.clip.unwrap_or(viewport)) {
                        shifted.push(window);
                    }
                }
            }
        }
        // The frame again with those damaged too: its layers are current
        // now, drawn from the same corners, so it goes this way once (a
        // layer drawn again there, waiting on a picture, draws from the
        // same corner; were it not to, this frame goes on rather than
        // around again).
        if !shifted.is_empty() && self.plane.is_none() && !self.redrawing_shifted {
            let damage: Vec<Rect> = damage.iter().copied().chain(shifted).collect();
            self.redrawing_shifted = true;
            let again = self.render_once(scene, &damage);
            self.redrawing_shifted = false;
            let mut stats = again?;
            add_layer_stats(&mut stats, &layer_stats);
            stats.layer_repaints += layer_stats.layer_repaints - 1;
            stats.layer_partial_repaints += layer_stats.layer_partial_repaints;
            stats.layer_texture_allocations += layer_stats.layer_texture_allocations;
            return Ok(stats);
        }
        // Damaged where it shows but not drawn (hidden, culled): what changed
        // in it then is known nowhere, so it repaints whole next.
        let frame = self.layer_frame;
        for cache in self.layers.values_mut() {
            if cache.used != frame
                && cache
                    .watched
                    .is_some_and(|watched| damage.iter().any(|d| d.intersects(watched)))
            {
                cache.watched = None;
            }
        }
        self.prepare_effects(scene, &items)?;
        // Kept only if every item draws flat (no layer, no effect) from
        // what the scene marks when it changes.
        let keep = keep
            && !stacked
            && self.effect_jobs.is_empty()
            // A box on a plane draws nothing a patch could follow.
            && !self.views.as_ref().is_some_and(|views| views.has_planes())
            && items
                .iter()
                .all(|item| !item.isolated && !changes_unmarked(item.kind));
        self.banding = banding;
        let mut stats = match keep {
            true => self.render_keeping(scene, items, inherited, damage, merged, viewport)?,
            false => self.render_flat::<FRAME>(scene, items, damage, merged, false)?,
        };
        stats.add_made(&std::mem::take(&mut self.patch_stats));
        self.rendered_serial = Some(scene.flush_serial());
        stats.layer_repaints = layer_stats.layer_repaints;
        stats.layer_partial_repaints = layer_stats.layer_partial_repaints;
        stats.layer_cache_hits = layer_stats.layer_cache_hits;
        stats.layer_texture_allocations = layer_stats.layer_texture_allocations;
        stats.damaged_pixels += layer_stats.damaged_pixels;
        stats.draw_calls += layer_stats.draw_calls;
        stats.render_passes += layer_stats.render_passes;
        stats.backdrop_copies += layer_stats.backdrop_copies;
        stats.blur_passes += layer_stats.blur_passes;
        stats.effect_passes += layer_stats.effect_passes;
        stats.instances += layer_stats.instances;
        stats.shaped_nodes += layer_stats.shaped_nodes;
        stats.glyph_uploads += layer_stats.glyph_uploads;
        stats.image_uploads += layer_stats.image_uploads;
        stats.native_surface_imports += layer_stats.native_surface_imports;
        stats.native_surface_conversions += layer_stats.native_surface_conversions;
        stats.canvas_rasterizations += layer_stats.canvas_rasterizations;
        stats.svg_rasterizations += layer_stats.svg_rasterizations;
        stats.geometry_rebuilds += layer_stats.geometry_rebuilds;
        stats.vertex_buffer_allocations += layer_stats.vertex_buffer_allocations;
        Ok(stats)
    }
    /// Drop cached shapes, canvases, SVGs and textures of nodes that are gone,
    /// looking over all it keeps.
    fn prune(&mut self, scene: &Scene) {
        self.node_images.retain(|id, _| scene.contains(*id));
        self.node_ramps.retain(|id, _| scene.contains(*id));
        self.shapes.retain(|id, _| {
            scene.contains(*id)
                && matches!(
                    scene.kind(*id),
                    NodeKind::Text { .. } | NodeKind::RichText { .. }
                )
        });
        self.shaped_bytes = self.shapes.values().map(Shaped::bytes).sum();
        // Blurred shadows go with their texts' shapes; with the last, all.
        if let Some(blurred) = &mut self.blurred {
            blurred.texts.retain(|id, _| self.shapes.contains_key(id));
            if blurred.texts.is_empty() {
                self.blurred = None;
            }
        }
        self.canvases.retain(scene);
        self.svgs.retain(scene);
        // The images nodes show, counted from here on as they change; with
        // none uploaded, not looked for (`SceneImages::counted`).
        let images = self.scene_images.get_or_insert_default();
        images.counted = false;
        if !self.images.is_empty() {
            images.walk(scene);
        }
        self.prune_rest(scene);
    }
    /// What `prune` does when the content revision moved: for the nodes the
    /// scene names as changed since the last (`Scene::resource_changes`),
    /// or with images no longer shown (`SceneImages::dropped`); past a gap,
    /// for all. With neither (a canvas drawn again, a node appended), none.
    /// Out of line: the frame runs as before.
    #[inline(never)]
    fn prune_moved(&mut self, scene: &Scene) {
        let (covered, changed) = scene.resource_changes();
        match &mut self.scene_images {
            Some(images) if *covered.start() <= images.through + 1 => {
                // Counted from the first move with textures to keep; after
                // that, the few counted are looked at again (a node given
                // another kind isn't named: an image node made a box lets
                // go of its image, as it did when all nodes were walked for).
                let uncounted = !images.counted && !self.images.is_empty();
                if images.counted {
                    images.recheck(scene);
                }
                let dropped = images.dropped || uncounted;
                // Rasters of nodes no longer drawing one (a box's detailed
                // decoration taken off) go as they did: they are few, and
                // the scene doesn't name a node given another kind.
                if !self.canvases.is_empty() {
                    self.canvases.retain(scene);
                }
                if !changed.is_empty() || dropped {
                    self.prune_changed(scene, changed);
                } else {
                    #[cfg(target_os = "macos")]
                    self.retain_lone_surfaces(scene);
                }
            }
            _ => self.prune(scene),
        }
        if let Some(images) = &mut self.scene_images {
            images.through = *covered.end();
        }
    }
    /// `prune` for the nodes `changed` alone (`Scene::resource_changes`):
    /// those gone, or showing something else.
    fn prune_changed(&mut self, scene: &Scene, changed: &[NodeId]) {
        let images = self.scene_images.get_or_insert_default();
        if images.counted {
            for &id in changed {
                let kind = scene.contains(id).then(|| scene.kind(id));
                images.show(id, kind.and_then(shown_image));
            }
        } else if !self.images.is_empty() {
            images.walk(scene);
        }
        for &id in changed {
            let kind = scene.contains(id).then(|| scene.kind(id));
            if kind.is_none() {
                self.node_images.remove(&id);
                self.node_ramps.remove(&id);
            }
            if !matches!(
                kind,
                Some(NodeKind::Text { .. } | NodeKind::RichText { .. })
            ) && let Some(shaped) = self.shapes.remove(&id)
            {
                self.shaped_bytes -= shaped.bytes();
                if let Some(blurred) = &mut self.blurred {
                    blurred.texts.remove(&id);
                }
            }
        }
        if self
            .blurred
            .as_ref()
            .is_some_and(|blurred| blurred.texts.is_empty())
        {
            self.blurred = None;
        }
        self.canvases.retain_changed(scene, changed);
        self.svgs.retain_changed(scene, changed);
        self.prune_rest(scene);
    }
    /// What `prune` and `prune_changed` share: ramps, outlines and textures
    /// none shows.
    fn prune_rest(&mut self, scene: &Scene) {
        let shown: std::collections::HashSet<&RampKey> = self.node_ramps.values().collect();
        // A ramp no box shows stays while its texture does (kept with the
        // images no node shows, until the window goes idle): the same
        // gradient shown again, as a page pushed again shows its edge, takes
        // that texture rather than a new one each time.
        let images = &self.images;
        self.ramps
            .retain(|key, ramp| shown.contains(key) || images.contains_key(&ramp.id()));
        // Pixelated images' bind groups go with their textures; with the last, all.
        if let Some(nearest) = &mut self.nearest {
            nearest.binds.retain(|id, _| self.images.contains_key(id));
            if nearest.binds.is_empty() {
                self.nearest = None;
            }
        }
        // Outlines still dashed or dotted keep their rasters; with the last, all.
        if let Some(outlines) = &mut self.outlines {
            let styled = |id| {
                scene.contains(id) && scene.outline_style(id) != zgx::decoration::BorderStyle::Solid
            };
            outlines.cache.retain_nodes(styled);
            outlines.images.retain(|id, _| styled(*id));
            if outlines.images.is_empty() {
                self.outlines = None;
            }
        }
        #[cfg(target_os = "macos")]
        self.retain_lone_surfaces(scene);
        if !self.images.is_empty() {
            self.prune_images(scene);
        }
        self.retain_views();
    }
    /// `prune` for image textures: those no node, canvas, ramp, picture or
    /// layer shows go, within the bound kept for pictures that may come back.
    /// Those nodes show are counted (`SceneImages`), not walked for. Out of
    /// line: `prune` stays as it was.
    #[inline(never)]
    fn prune_images(&mut self, scene: &Scene) {
        let images = self.scene_images.get_or_insert_default();
        if !images.counted {
            images.walk(scene);
        }
        images.dropped = false;
        #[cfg(debug_assertions)]
        {
            let shown = scene_image_ids(scene);
            assert!(
                images.users.keys().all(|id| shown.contains(id)),
                "an image counted that no node shows"
            );
        }
        // Those nodes show, and the rest: canvases', boxes' ramps (the rest
        // go as other unshown images do), pictures', effect layers' atlas and
        // masks.
        let mut others = FxHashSet::default();
        others.extend(self.canvases.image_ids());
        others.extend(
            (self.node_ramps.values())
                .filter_map(|key| self.ramps.get(key))
                .map(|ramp| ramp.id()),
        );
        others.extend(self.svgs.image_ids());
        others.insert(effects::EFFECT_ATLAS);
        others.extend(
            self.layers
                .keys()
                .filter(|id| scene.contains(**id))
                .filter_map(|id| scene.mask(*id).map(|mask| mask.image.id())),
        );
        let walked = &self.scene_images.as_deref().expect("counted above").users;
        let live = |id: &u64| walked.contains_key(id) || others.contains(id);
        #[cfg(target_os = "macos")]
        if let Some(cache) = &mut self.native_surfaces {
            let mut live_images: FxHashSet<u64> = walked.keys().copied().collect();
            live_images.extend(others.iter().copied());
            cache.retain(&live_images);
        }
        // A picture whose row went (a list scrolled past) may come back
        // with its pixels long given up, to decode blank again: its
        // texture stays, within a bound, until the window goes idle.
        // Others' are gone with their nodes; they have no `image_drawn`.
        // Nor one nothing holds any more (a video's frame, replaced by
        // the next): no node can show it again.
        let (drawn, holders) = (&self.image_drawn, &self.image_holders);
        self.images.retain(|id, _| {
            live(id) || drawn.contains_key(id) && holders.get(id).is_none_or(|h| h.any())
        });
        let images = &self.images;
        self.image_holders.retain(|id, _| images.contains_key(id));
        self.images_gone.clear();
        self.images_gone
            .extend(self.images.keys().filter(|id| !live(id)));
        self.bound_images_gone();
        let images = &self.images;
        self.image_drawn.retain(|id, _| images.contains_key(id));
    }
    /// Textures of `images_gone` past their bound, least recently drawn first.
    fn bound_images_gone(&mut self) {
        let mut bytes: usize = (self.images_gone.iter())
            .filter_map(|id| self.images.get(id))
            .map(|entry| entry.1)
            .sum();
        let mut count = self.images_gone.len();
        if bytes <= IMAGES_GONE_BYTES && count <= IMAGES_GONE_COUNT {
            return;
        }
        let mut gone: Vec<(u64, u64)> = (self.images_gone.iter())
            .map(|id| (self.image_drawn.get(id).copied().unwrap_or(0), *id))
            .collect();
        gone.sort_unstable();
        for (_, id) in gone {
            if bytes <= IMAGES_GONE_BYTES && count <= IMAGES_GONE_COUNT {
                break;
            }
            if let Some(entry) = self.images.remove(&id) {
                bytes -= entry.1;
            }
            self.images_gone.remove(&id);
            count -= 1;
        }
    }
    /// Surfaces with no picture beside them (a video player alone): let go
    /// of those no longer shown as frames are drawn, as `prune_images` does
    /// with pictures. Else a playing window kept each frame's until it
    /// rested.
    #[cfg(target_os = "macos")]
    fn retain_lone_surfaces(&mut self, scene: &Scene) {
        if self.images.is_empty()
            && let Some(cache) = &mut self.native_surfaces
        {
            let live = (scene.paint_items())
                .filter_map(|item| match item.kind {
                    NodeKind::NativeSurface(frame) => Some(frame.id()),
                    _ => None,
                })
                .collect();
            cache.retain(&live);
        }
    }
    /// Forget GPU views whose frames' textures are gone.
    fn retain_views(&mut self) {
        if let Some(views) = &mut self.views {
            let images = &self.images;
            views.retain_cached(|image| images.contains_key(&image));
        }
    }
    /// Bring `id`'s layer texture up to date. `magnification`: how much its
    /// parent's content is magnified on screen (by transformed ancestors);
    /// with its own scale, it picks the raster scale (`raster_step`).
    /// `exact`: that magnification is zooms' alone (`Scene::set_zoom`), and
    /// the layer rasterizes at it exactly, showing texel for pixel.
    /// `shown`: how a layer of the window's own shows there, when all of it
    /// does: changed, it repaints only where the window's damage reaches it.
    /// `room`: the bytes its texture may take (`LAYER_ROOM`, or what the
    /// budget has left: it then rasterizes coarser, never fails).
    /// `placing`: what says where its texels land in the target it
    /// composites into, and what of it can show there.
    #[allow(clippy::too_many_arguments)]
    fn prepare_layer(
        &mut self,
        scene: &Scene,
        id: NodeId,
        magnified: f32,
        exact_around: bool,
        shown: Option<Shown<'_>>,
        placing: &Placing<'_>,
        stats: &mut GpuStats,
        room: usize,
    ) -> Result<(), GpuError> {
        // Most layers, most frames: not magnified nor placed, and kept as
        // they were. Planned and checked as layers always were, apart from
        // all a magnified one needs (`prepare_layer_planned`).
        debug_assert!(
            self.effect_jobs.is_empty(),
            "effect jobs left for another draw"
        );
        let local = scene.local_transform(id);
        let zoom = scene.zoom(id);
        let magnification = magnified * local.magnification() * zoom;
        let magnifying = magnification > 1. && magnification.is_finite();
        if !magnifying
            && let Some(cache) = self.layers.get_mut(&id)
            && !cache.placement().cropped
            && cache.placement().seen.grids.is_none()
            && cache.revision == scene.layer_revision(id)
            && !cache.incomplete
        {
            let exact = exact_around && local.is_identity();
            let step = raster_step(magnification, Some(cache.step));
            let factor = match exact && magnification != 1. {
                true => magnification,
                false => step_scale(step),
            };
            if cache.step == step && cache.factor == factor.to_bits() {
                let root = scene.bounds(id);
                let serial = scene.flush_serial();
                let seen = &cache.placement().seen;
                let before = match seen.frame == serial {
                    true => seen.before,
                    false => seen.magnification,
                };
                cache.bounds.x += root.x - cache.root_origin.0;
                cache.bounds.y += root.y - cache.root_origin.1;
                cache.content.x += root.x - cache.root_origin.0;
                cache.content.y += root.y - cache.root_origin.1;
                cache.root_origin = (root.x, root.y);
                cache.transformed = !local.is_identity() || zoom != 1. || scene.blurs_or_masks(id);
                cache.placement_mut().seen = Seen {
                    frame: serial,
                    magnification,
                    before,
                    grids: None,
                };
                cache.used = self.layer_frame;
                cache.parked = false;
                // A match, not a closure: kept in line.
                cache.watched = match shown {
                    Some(shown) => shown.window(cache.content),
                    None => None,
                };
                stats.layer_cache_hits += 1;
                return Ok(());
            }
        }
        self.prepare_layer_planned(
            scene,
            id,
            magnified,
            exact_around,
            shown,
            placing,
            stats,
            room,
        )
    }

    /// `prepare_layer`, planned in full: magnified, placed, or drawn.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn prepare_layer_planned(
        &mut self,
        scene: &Scene,
        id: NodeId,
        magnified: f32,
        exact_around: bool,
        shown: Option<Shown<'_>>,
        placing: &Placing<'_>,
        stats: &mut GpuStats,
        room: usize,
    ) -> Result<(), GpuError> {
        // Effect slots are drawn by the draw that follows their planning
        // (`prepare_effects`): a layer's own draw would take any planned
        // before it, for layers it may hold out of the cache meanwhile.
        debug_assert!(
            self.effect_jobs.is_empty(),
            "effect jobs left for another draw"
        );
        let revision = scene.layer_revision(id);
        let root = scene.bounds(id);
        let local = scene.local_transform(id);
        let zoom = scene.zoom(id);
        // Parked a while once it has none (`LAYER_PARK_FRAMES`).
        let transformed = !local.is_identity()
            || zoom != 1.
            || scene.filter_blur(id) > 0.
            || scene.mask(id).is_some();
        let magnification = magnified * local.magnification() * zoom;
        // Zoomed (and not otherwise scaled): at its scale exactly, crisp.
        let exact = exact_around && local.is_identity();
        // Shown larger than laid out (scaled, or inside something scaled):
        // at the scale it shows at exactly too, as its content would draw
        // there, rather than a smaller raster stretched soft. Smaller, it
        // keeps to steps: sampled down, a near scale shows as sharp.
        let magnifying = magnification > 1. && magnification.is_finite();
        // Moving (scaled, moved or turned since the frame before, or what
        // holds it was), it keeps the raster it has while that is fine
        // enough, and its texels where they were: its content drawn again
        // lands on them alike, so it moves as one picture rather than its
        // glyphs and lines drawn afresh, on other pixels, each frame.
        // Still, it is drawn at its scale exactly (the first frame it
        // shows included).
        // One look at what it holds: every layer comes here every frame.
        let cached = self.layers.get(&id);
        let (cropped, current, glyphs) = cached.map_or((false, None, 0.), |cache| {
            let placement = cache.placement();
            (placement.cropped, Some(cache.step), placement.glyphs)
        });
        let serial = scene.flush_serial();
        // Where it lands, for one that reads it: magnified, drawn in part
        // when it was, or placed the frame before. Any other is planned as
        // a layer always was, at no cost of this.
        let placing_read = magnifying
            || cropped
            || cached.is_some_and(|cache| cache.placement().seen.grids.is_some());
        let (placed, moving, kept, seen) = match placing_read {
            true => plan_placed(
                placing,
                cached,
                magnification,
                magnifying || cropped,
                serial,
            ),
            false => {
                let before = cached.map_or(magnification, |cache| {
                    let seen = &cache.placement().seen;
                    match seen.frame == serial {
                        true => seen.before,
                        false => seen.magnification,
                    }
                });
                let seen = Seen {
                    frame: serial,
                    magnification,
                    before,
                    grids: None,
                };
                (
                    None,
                    cached.is_some() && before != magnification,
                    None,
                    seen,
                )
            }
        };
        let mut step = raster_step(magnification, current);
        let mut factor = match (exact || magnifying) && magnification != 1. {
            true if exact || !moving => magnification,
            true => match kept {
                // Fine enough (stretched by a hundredth at most) and not
                // past √2 finer than it shows.
                Some(kept)
                    if kept * 1.01 >= magnification
                        && kept <= magnification * std::f32::consts::SQRT_2 =>
                {
                    kept
                }
                // Else the next √2 step at or above it: room to grow into.
                _ => step_scale((2. * magnification.log2() - 1e-3).ceil() as i32),
            },
            false => step_scale(step),
        };
        if magnifying && !exact {
            // Its step follows its raster, which holds while it moves.
            step = ((2. * factor.log2()).round() as i32)
                .clamp(*RASTER_STEPS.start(), *RASTER_STEPS.end());
        }
        // No finer than its largest glyph fits the atlas, once a raster
        // found it too large (`glyph_too_large`): finer, it would not draw.
        let capped = magnifying && glyphs > 0. && factor > GLYPH_ROOM / glyphs;
        if capped {
            factor = (GLYPH_ROOM / glyphs).max(1.);
        }
        let exact = exact && !capped;
        // What of it can show: all of it may not fit the texture limits or
        // the budget at that scale, while the part in view does. Not where
        // something reaches past its pixels (a blur, a mask, a shader, a
        // drop shadow).
        let visible = placed
            .and_then(|placed| placed.visible)
            .filter(|_| !reaches_past(scene, id));
        let frame = self.layer_frame;
        // Still and at its scale, its texels on its target's pixels: drawn
        // again if they moved off them (it moved, and stopped).
        let aligned_scale = (magnifying && !exact && !moving).then_some(self.scale * factor);
        let off_pixels = |cache: &LayerCache| {
            let Some(grid) =
                aligned_scale.and_then(|scale| placed.and_then(|placed| placed.aligned(scale)))
            else {
                return false;
            };
            let corner = (
                cache.bounds.x + root.x - cache.root_origin.0,
                cache.bounds.y + root.y - cache.root_origin.1,
            );
            let (px, py) = grid.point(corner.0, corner.1);
            (px - px.round()).abs() > 1e-3 || (py - py.round()).abs() > 1e-3
        };
        if let Some(cache) = self.layers.get_mut(&id)
            && cache.revision == revision
            && !cache.incomplete
            && cache.step == step
            && cache.factor == factor.to_bits()
            && !off_pixels(cache)
            // Drawn in part: that part still holds all that shows.
            && (!cache.placement().cropped
                || visible.is_some_and(|visible| {
                    let moved = |r: Rect| {
                        Rect::new(
                            r.x + root.x - cache.root_origin.0,
                            r.y + root.y - cache.root_origin.1,
                            r.width,
                            r.height,
                        )
                    };
                    visible
                        .intersection(moved(cache.content))
                        .is_none_or(|visible| within(moved(cache.bounds), visible))
                }))
        {
            cache.bounds.x += root.x - cache.root_origin.0;
            cache.bounds.y += root.y - cache.root_origin.1;
            cache.content.x += root.x - cache.root_origin.0;
            cache.content.y += root.y - cache.root_origin.1;
            cache.root_origin = (root.x, root.y);
            cache.transformed = transformed;
            cache.placement_mut().seen = seen;
            cache.used = frame;
            cache.parked = false;
            cache.watched = shown.and_then(|shown| shown.window(cache.content));
            stats.layer_cache_hits += 1;
            return Ok(());
        }
        let content = scene.layer_bounds(id);
        let limit = self.device.limits().max_texture_dimension_2d;
        let budget = self.layer_budget();
        // No room of its own: one layer as large as the window takes what it
        // needs, as long as the budget holds it.
        let room = room.min(budget);
        // Magnified past what the texture limits or the budget allow, a
        // layer renders at the finest scale that fits.
        let pixels = |factor: f32, area: Rect| {
            let scale = self.scale * factor;
            (area.width + 2. / scale) * scale * (area.height + 2. / scale) * scale
        };
        let fits = |factor: f32, area: Rect| {
            (area.width.max(area.height) + 2.) * self.scale * factor <= limit as f32
                && pixels(factor, area) * 4. <= room as f32
        };
        // What it draws: all of it, or magnified past what fits, what of it
        // shows, with a quarter of that around it to pan into before it is
        // drawn again; or just what shows.
        // Moving with room to grow into, but no room for it: at the scale
        // it shows at, as still.
        if magnifying && moving && !fits(factor, content) && fits(magnification, content) {
            factor = magnification;
            step = ((2. * factor.log2()).round() as i32)
                .clamp(*RASTER_STEPS.start(), *RASTER_STEPS.end());
        }
        let mut area = content;
        let mut cropped = false;
        if magnifying
            && !fits(factor, content)
            && let Some(visible) = visible
            && let Some(shows) = visible.intersection(content)
        {
            let around = (visible.expand(0.25 * shows.width.max(shows.height)))
                .intersection(content)
                .unwrap_or(shows);
            area = match fits(factor, around) {
                true => around,
                false => shows,
            };
            cropped = true;
        }
        let fits = |factor: f32| fits(factor, area);
        // Too large at the scale it shows at: the finest step that fits.
        let at_scale = (exact || magnifying) && fits(factor);
        let exact = exact && at_scale;
        let mut coarser = false;
        if !at_scale {
            while step > 0 && !fits(step_scale(step)) {
                step -= 1;
            }
            factor = step_scale(step);
            // Too large at any step: as coarse as it takes.
            coarser = !fits(factor);
            if coarser {
                let (width, height) = (area.width * self.scale, area.height * self.scale);
                let side = (limit as f32 - 2.) / width.max(height).max(1.);
                let room = (room as f32 / 4. / (width * height).max(1.)).sqrt() * 0.9;
                factor = factor.min(side).min(room).max(1. / 4096.);
            }
        }
        if coarser {
            self.note_layer_coarser(id);
        }
        let device_scale = self.scale;
        let scale = device_scale * factor;
        // A transparent texel border lets linear sampling preserve fractional
        // translation coverage (and smooth a rotated layer's edges) instead
        // of clamping opaque edge pixels.
        let bounds = area.expand(1. / scale);
        // At a zoom's scale, its texels on the device's pixels: counted from
        // its box's corner, which layout puts on a pixel. Magnified, at the
        // scale it shows at and not turned: on its target's pixels too,
        // from where they land there, so it composites texel for pixel
        // rather than resampled half a pixel off.
        let aligned = placed
            .filter(|_| at_scale && !exact && !moving && factor != 1.)
            .and_then(|placed| placed.aligned(scale));
        // Moving, its texels where they were at this scale: its grid's
        // phase from its box's corner kept as it was (worked out again from
        // where they landed, it drifts a little each frame, and now and then
        // by a texel); else counted from its box's corner.
        let kept_phase = self
            .layers
            .get(&id)
            .filter(|cache| moving && cache.factor == factor.to_bits())
            .map(|cache| cache.placement().phase);
        let phase = kept_phase.unwrap_or((0., 0.));
        // The grid's line at or before `edge`, from `from` (the box's
        // corner): a whole texel past it or short of it when `edge` lies a
        // hair either side of a line, as the border texel does.
        let on_grid = |edge: f32, from: f32, phase: f32| {
            from + phase + ((edge - from - phase) * scale + 1e-3).floor() / scale
        };
        let (x, y) = match (exact && factor != 1., aligned) {
            (true, _) => (
                root.x + ((bounds.x - root.x) * scale).floor() / scale,
                root.y + ((bounds.y - root.y) * scale).floor() / scale,
            ),
            (false, None) if magnifying => (
                on_grid(bounds.x, root.x, phase.0),
                on_grid(bounds.y, root.y, phase.1),
            ),
            (false, Some(grid)) => {
                // Back from the texel corner to the pixel corner before it.
                let back = |at: f32, by: f32| {
                    let fraction = match by > 0. {
                        true => at - at.floor(),
                        false => at.ceil() - at,
                    };
                    fraction / by.abs()
                };
                let (px, py) = grid.point(bounds.x, bounds.y);
                (bounds.x - back(px, grid.a), bounds.y - back(py, grid.d))
            }
            (false, None) => (bounds.x, bounds.y),
        };
        let (width, height) = match (x, y) == (bounds.x, bounds.y) {
            true => (bounds.width, bounds.height),
            false => (bounds.x + bounds.width - x, bounds.y + bounds.height - y),
        };
        let width = (width * scale).ceil().max(1.) as u32;
        let height = (height * scale).ceil().max(1.) as u32;
        let bounds = Rect::new(x, y, width as f32 / scale, height as f32 / scale);
        let phase = kept_phase.unwrap_or_else(|| {
            let within = |at: f32, from: f32| {
                let off = at - from;
                off - (off * scale).floor() / scale
            };
            (within(x, root.x), within(y, root.y))
        });
        // Where all of it shows in the window, if it does.
        let window = shown.and_then(|shown| shown.window(content));
        // Its window's damage, back in its own space: where its pixels can
        // have changed. All of it shows, so a change anywhere in it is
        // damage there (the window repaints from it nowhere else). A
        // blurred, masked or shaded layer's texels reach past where they
        // are: it repaints whole.
        let plain = || {
            scene.filter_blur(id) <= 0.
                && scene.mask(id).is_none()
                && scene.layer_shader(id).is_none()
        };
        let partial = shown
            .filter(|_| window.is_some() && plain())
            .and_then(|shown| {
                let cache = self.layers.get(&id)?;
                let kept = !cache.parked
                    && cache.watched.is_some()
                    && cache.step == step
                    && cache.factor == factor.to_bits()
                    && cache.bounds == bounds
                    && cache.root_origin == (root.x, root.y);
                let inverse = shown.map.inverse()?;
                kept.then(|| {
                    shown
                        .damage
                        .iter()
                        .filter_map(|damage| {
                            inverse
                                .bounds(*damage)
                                .expand(1. / scale)
                                .intersection(bounds)
                        })
                        .map(|rect| {
                            Rect::new(
                                rect.x - bounds.x,
                                rect.y - bounds.y,
                                rect.width,
                                rect.height,
                            )
                        })
                        .collect::<Vec<Rect>>()
                })
            });
        let mut items = scene.layer_items(Some(id));
        let banding = self.band_capable() && scene.take_painted_fades();
        self.plane_filter(&mut items);
        // A layer nested in it drawn from another corner differs anywhere
        // in it, and so does it.
        let mut shifted = false;
        // What of it shows, which is all its layers can show in: worked out
        // here when it was placed, else by a layer in it that reads it (one
        // further in may be magnified when it isn't).
        let shows = match placed {
            Some(_) => Shows::Known(
                (visible.and_then(|visible| visible.intersection(bounds))).unwrap_or(bounds),
            ),
            None => Shows::Inside {
                parent: placing,
                scene,
                id,
                bounds,
            },
        };
        for item in &items {
            if item.isolated && item.effects.opacity > 0. {
                self.layer_shifted = false;
                let placing = Placing {
                    item,
                    origin: (bounds.x, bounds.y),
                    scale,
                    shows,
                };
                self.prepare_layer(
                    scene,
                    item.id,
                    magnification,
                    exact,
                    None,
                    &placing,
                    stats,
                    LAYER_ROOM,
                )?;
                shifted |= self.layer_shifted;
            }
        }
        // Drawn from another corner (its content grew left or up): its
        // texels round otherwise, and what didn't change shows a level off
        // where the window keeps its last frame (`render_once`). Not at a
        // new scale as it moves: the transform that moved it damaged all it
        // covers, and every texel differs anyway.
        shifted |= self.layers.get(&id).is_some_and(|cache| {
            (
                cache.bounds.x - cache.root_origin.0,
                cache.bounds.y - cache.root_origin.1,
            ) != (bounds.x - root.x, bounds.y - root.y)
                && !(moving && cache.factor != factor.to_bits())
        });
        self.layer_shifted = shifted;
        self.prepare_effects(scene, &items)?;
        if let Some(damage) = partial {
            let mut cache = self.layers.remove(&id).expect("kept above");
            cache.placement_mut().seen = seen;
            self.banding = banding;
            let (texture, result) =
                self.paint_layer(scene, items, bounds, cache.texture, scale, &damage, false);
            self.layers.insert(
                id,
                LayerCache {
                    texture,
                    revision,
                    transformed,
                    used: frame,
                    watched: window,
                    incomplete: std::mem::take(&mut self.layer_incomplete),
                    ..cache
                },
            );
            let result = result?;
            add_layer_stats(stats, &result);
            stats.layer_partial_repaints += 1;
            return Ok(());
        }
        // What its texture takes: rounded up as allocations are, not just
        // its pixels (the budget counts what is allocated).
        let bytes = width.next_multiple_of(64).min(limit) as usize
            * height.next_multiple_of(64).min(limit) as usize
            * 4;
        let mut existing = self.layers.get(&id).map_or(0, |l| l.bytes);
        let mut used: usize = self.layers.values().map(|l| l.bytes).sum();
        let over = |count: usize, used: usize, existing: usize, known: bool| {
            (!known && count >= 1024) || used - existing + bytes > 64 * 1024 * 1024
        };
        if over(
            self.layers.len(),
            used,
            existing,
            self.layers.contains_key(&id),
        ) {
            // Parked layers give way first.
            self.layers
                .retain(|other, _| *other == id || scene.is_layer(*other));
            existing = self.layers.get(&id).map_or(0, |l| l.bytes);
            used = self.layers.values().map(|l| l.bytes).sum();
        }
        // Then layers this frame has not drawn (hidden, or not shown since),
        // least lately drawn first: each draws again when it next shows.
        // Kept while the budget lasts, they would leave the layers drawn now
        // as coarse as a texel, their content gone.
        if used - existing + bytes > budget {
            let frame = self.layer_frame;
            let mut idle: Vec<(u64, NodeId, usize)> = (self.layers.iter())
                .filter(|(other, cache)| **other != id && cache.used != frame)
                .map(|(other, cache)| (cache.used, *other, cache.bytes))
                .collect();
            idle.sort_unstable_by_key(|(drawn, ..)| *drawn);
            for (_, other, held) in idle {
                if used - existing + bytes <= budget {
                    break;
                }
                self.layers.remove(&other);
                used -= held;
            }
        }
        // Past the budget or the texture limits: again, in the room left,
        // coarser. Once: a texture of one texel still takes its 64 by 64.
        if (used - existing + bytes > budget || width > limit || height > limit) && room == budget {
            self.note_layer_coarser(id);
            let room = budget.saturating_sub(used - existing).min(budget - 1);
            // Its layers' effect slots, planned above, are planned again
            // there. Left queued, the first of its layers drawn again (one
            // waiting on a picture) would take them, its own slot's among
            // them, while its texture is out of the cache.
            let planned = std::mem::take(&mut self.effect_jobs);
            self.forget_effects(&planned);
            return self.prepare_layer(
                scene,
                id,
                magnified,
                exact_around,
                shown,
                placing,
                stats,
                room,
            );
        }
        // Animated layers change size every frame: round allocations up and
        // reuse any texture that fits without wasting more than half of it.
        let (alloc_w, alloc_h) = (
            width.next_multiple_of(64).min(limit),
            height.next_multiple_of(64).min(limit),
        );
        let mut existing = self.layers.remove(&id);
        // Its placement's box, kept: drawn again, it allocates nothing more.
        let placement = existing.as_mut().and_then(|cache| cache.placement.take());
        let (texture, bind) = if let Some(cache) = existing.filter(|cache| {
            let (w, h) = (cache.texture.width(), cache.texture.height());
            w >= width
                && h >= height
                && u64::from(w) * u64::from(h) <= 2 * u64::from(alloc_w) * u64::from(alloc_h)
        }) {
            (cache.texture, cache.bind)
        } else {
            let texture = texture(&self.device, alloc_w, alloc_h, "isolated subtree");
            let bind = self.texture_bind(
                &texture.create_view(&Default::default()),
                "isolated subtree",
            );
            stats.layer_texture_allocations += 1;
            (texture, bind)
        };
        let bytes = texture.width() as usize * texture.height() as usize * 4;
        // The content occupies the top-left corner; the quad samples just that.
        let uv = [
            width as f32 / texture.width() as f32,
            height as f32 / texture.height() as f32,
        ];
        let whole = [Rect::new(0., 0., bounds.width, bounds.height)];
        self.banding = banding;
        // Magnified, a glyph too large for the atlas is left out: it draws
        // again no finer than that glyph fits.
        let coarser_glyphs = magnifying && factor > 1.;
        let outer = std::mem::replace(&mut self.layer_glyph_too_large, (0., coarser_glyphs));
        let (texture, result) =
            self.paint_layer(scene, items, bounds, texture, scale, &whole, true);
        let (too_large, _) = std::mem::replace(&mut self.layer_glyph_too_large, outer);
        let result = result?;
        add_layer_stats(stats, &result);
        let found = match coarser_glyphs && too_large > 0. {
            true => glyphs.max(too_large / factor),
            false => glyphs,
        };
        self.layers.insert(
            id,
            LayerCache {
                texture,
                bind,
                bounds,
                revision,
                root_origin: (root.x, root.y),
                bytes,
                uv,
                step,
                factor: factor.to_bits(),
                exact: exact && factor != 1.,
                placement: Some({
                    let mut placement = placement.unwrap_or_else(|| Box::new(PLACEMENT));
                    *placement = Placement {
                        cropped,
                        glyphs: found,
                        seen,
                        phase,
                    };
                    placement
                }),
                transformed,
                used: frame,
                parked: false,
                watched: window,
                content,
                incomplete: std::mem::take(&mut self.layer_incomplete),
            },
        );
        if found > glyphs && !self.redrawing_glyphs.contains(&id) {
            self.redrawing_glyphs.push(id);
            let again = self.prepare_layer(
                scene,
                id,
                magnified,
                exact_around,
                shown,
                placing,
                stats,
                room,
            );
            self.redrawing_glyphs.pop();
            return again;
        }
        Ok(())
    }
    /// Draw `items` into a layer's `texture` at `scale`, its content's
    /// corner at `bounds`' (nested layers find theirs from it): where
    /// `damage` says (in its own points from that corner), all of it
    /// when `fresh` (its slack too, so an earlier larger frame never bleeds
    /// into sampling at the content's edge).
    #[allow(clippy::too_many_arguments)]
    fn paint_layer(
        &mut self,
        scene: &Scene,
        mut items: Vec<zgx::scene::PaintItem<'_>>,
        bounds: Rect,
        texture: wgpu::Texture,
        scale: f32,
        damage: &[Rect],
        fresh: bool,
    ) -> (wgpu::Texture, Result<GpuStats, GpuError>) {
        let view = texture.create_view(&Default::default());
        // Whether this layer goes without something, apart from what the
        // frame went without before it (`layer_incomplete`).
        let awaited = std::mem::take(&mut self.images_awaited);
        let target = std::mem::replace(&mut self.target, texture);
        let old_view = std::mem::replace(&mut self.view, view);
        let old_size = (self.width, self.height);
        self.width = self.target.width();
        self.height = self.target.height();
        let background = self.background;
        self.background = Color(0, 0, 0, 0);
        let was_fresh = std::mem::replace(&mut self.fresh, fresh);
        // Everything inside draws at the layer's raster scale: glyphs,
        // canvases and nested layers rasterize as finely as it shows.
        let device_scale = std::mem::replace(&mut self.scale, scale);
        #[cfg(target_os = "macos")]
        self.refuse_layered_loops(scene, &items);
        zgx::scene::translate_items(&mut items, -bounds.x, -bounds.y);
        // Nested layer images remain world-positioned in their cache; pass the offset
        // separately so the same texture can be composed into any ancestor layer.
        let old_origin = std::mem::replace(&mut self.layer_origin, (bounds.x, bounds.y));
        let in_layer = std::mem::replace(&mut self.in_layer, true);
        let root = std::mem::replace(&mut self.layer_root, items.first().map(|item| item.id));
        let result = self.render_flat::<FRAME>(scene, items, damage, None, false);
        self.layer_root = root;
        self.in_layer = in_layer;
        self.layer_origin = old_origin;
        let texture = std::mem::replace(&mut self.target, target);
        self.view = old_view;
        self.width = old_size.0;
        self.height = old_size.1;
        self.scale = device_scale;
        self.background = background;
        self.fresh = was_fresh;
        self.layer_incomplete = self.images_awaited;
        self.images_awaited |= awaited;
        (texture, result)
    }
    /// How `item`, a layer in the window's own pass, shows there: its map
    /// (none: as laid out), what of the window it can show in, and the
    /// window's damage this frame. `None` in perspective.
    fn shown<'a>(
        &self,
        item: &zgx::scene::PaintItem<'_>,
        damage: &'a [Rect],
        viewport: Rect,
    ) -> Option<Shown<'a>> {
        let map = match item.projective() {
            Some(map) => map.affine()?,
            None => zgx::affine::Affine::IDENTITY,
        };
        let clip = item.clip.unwrap_or(viewport).intersection(viewport)?;
        Some(Shown { damage, map, clip })
    }
    fn select_viewport(&mut self, width: f32, height: f32) {
        let key = self.viewport_entry(width, height);
        self.viewport_bind = self.viewports.get(&key).map(|(_, bind)| bind.clone());
        #[cfg(any(target_os = "macos", d3d11))]
        {
            self.viewport_values = [width, height, self.scale, 0.];
        }
    }
    /// The `viewports` key of a target `width` × `height` points, its
    /// uniform made if new.
    fn viewport_entry(&mut self, width: f32, height: f32) -> (u32, u32, u32) {
        let key = (width.to_bits(), height.to_bits(), self.scale.to_bits());
        if !self.viewports.contains_key(&key) {
            // Layer sizes come and go; keep the map small.
            if self.viewports.len() >= 32 {
                self.viewports.clear();
            }
            let buffer = upload::init_buffer(
                &self.device,
                self.unified,
                "viewport",
                // Logical size, and device pixels per logical pixel.
                bytemuck::cast_slice(&[width, height, self.scale, 0.]),
                wgpu::BufferUsages::UNIFORM,
            );
            let bind = self.viewport_group(&buffer, self.no_regions.as_entire_binding());
            self.viewports.insert(key, (buffer, bind));
        }
        key
    }
    /// The pipeline drawing `shading` if a shader fill's or the image
    /// effects', once compiled.
    fn fill_pipeline(&self, shading: Shading) -> Option<&wgpu::RenderPipeline> {
        match shading {
            Shading::Fill(id) => self.fill_pipelines.get(&id)?.get(),
            Shading::Filter(id) => self.filter_pipelines.get(&id)?.get(),
            Shading::Blend => self.blend_pipelines.get(&BLEND_PLAIN)?.get(),
            Shading::FilterBlend(id) => self.blend_pipelines.get(&id)?.get(),
            Shading::ImageEffect => self.image_effect_pipeline.get(),
            _ => None,
        }
    }
    /// Compile the image effects' pipeline if not yet.
    fn prepare_image_effects(&self) {
        self.image_effect_pipeline.get_or_init(|| {
            let module = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("image effects"),
                    source: wgpu::ShaderSource::Wgsl(image_effect_source().into()),
                });
            fill_pipeline(
                &self.device,
                &module,
                &module,
                &self.context.inner.quad_layout,
                "zgx_image_effect",
            )
        });
    }
    /// A shader's pipeline from WGSL `source`, built by `build(vertex,
    /// fragment)`, or none if it fails. `apps`: an app's own shader, which
    /// no gate has compiled: on Direct3D 12 with FXC its fragment stage
    /// compiles in a process of its own (`app_fragment`), so one FXC can't
    /// survive fails alone, drawing nothing.
    fn app_built(
        device: &wgpu::Device,
        backend: wgpu::Backend,
        apps: bool,
        source: &str,
        entry: &str,
        layout: AppLayout,
        build: impl FnOnce(&wgpu::ShaderModule, &wgpu::ShaderModule) -> wgpu::RenderPipeline,
    ) -> Option<wgpu::RenderPipeline> {
        #[cfg(windows)]
        let fragment =
            match apps.then(|| Self::app_fragment(device, backend, source, entry, layout)) {
                Some(Err(error)) => {
                    eprintln!("zgx: {error}");
                    return None;
                }
                Some(Ok(fragment)) => fragment,
                None => None,
            };
        #[cfg(not(windows))]
        let fragment: Option<wgpu::ShaderModule> = {
            let _ = (backend, apps, entry, layout);
            None
        };
        shader_pipeline(device, || {
            let module = single_use_module(device, "shader", source);
            build(&module, fragment.as_ref().unwrap_or(&module))
        })
    }
    /// An app's fragment stage compiled by FXC in a process of its own and
    /// handed to wgpu as bytecode (`dx12_fill`, `fxc_child`), where wgpu's
    /// Direct3D 12 would compile it with FXC here; `None` builds it as wgpu
    /// does (another backend or compiler, no passthrough, no child).
    #[cfg(windows)]
    fn app_fragment(
        device: &wgpu::Device,
        backend: wgpu::Backend,
        source: &str,
        entry: &str,
        layout: AppLayout,
    ) -> Result<Option<wgpu::ShaderModule>, String> {
        let Some(&(true, debug)) = APP_SHADER_FXC.get() else {
            return Ok(None);
        };
        if backend != wgpu::Backend::Dx12
            || !device
                .features()
                .contains(wgpu::Features::PASSTHROUGH_SHADERS)
            || APP_SHADERS_IN_PROCESS.load(std::sync::atomic::Ordering::Relaxed)
            || !fxc_child::available()
        {
            return Ok(None);
        }
        std::hint::black_box(fxc_child::keep());
        let layout = match layout {
            AppLayout::Quads => dx12_fill::Layout::Quads,
            AppLayout::Blend => dx12_fill::Layout::Blend,
            AppLayout::Inputs { blend, textures } => dx12_fill::Layout::Inputs { blend, textures },
        };
        // A shader naga can't write as HLSL is wgpu's to report.
        let Ok((hlsl, name)) = dx12_fill::fragment_hlsl(source, entry, layout) else {
            return Ok(None);
        };
        // As wgpu-hal compiles it: strict, and unoptimized with debug info
        // under the instance's DEBUG.
        const STRICTNESS: u32 = 1 << 11;
        const DEBUG_SKIP_OPTIMIZATION: u32 = (1 << 0) | (1 << 2);
        let flags = STRICTNESS | if debug { DEBUG_SKIP_OPTIMIZATION } else { 0 };
        let job = fxc_child::Job {
            source: &hlsl,
            entry_point: &name,
            target: "ps_5_1",
            flags,
        };
        let code = match fxc_child::compile(&job) {
            None => return Ok(None),
            Some(Err(error)) => return Err(error),
            Some(Ok(code)) => code,
        };
        // SAFETY: DXBC FXC compiled from the HLSL `dx12_fill` writes for
        // this layout's root signature (checked by `app_shaders_d3d12`).
        Ok(Some(unsafe {
            device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some("app shader"),
                dxil: Some(code.into()),
                ..Default::default()
            })
        }))
    }
    /// Whether frames wait for a shader still compiling: an offscreen
    /// renderer's have no deadline and nothing to wake.
    fn waits_for_shaders(&self) -> bool {
        self.debug_waits_for_shaders || self.offscreen_waits && !self.presents()
    }
    /// What Direct3D 11 frames have left out so far, each kind once
    /// (`ZGX_D3D11`, the `d3d11` feature). For tests.
    #[cfg(d3d11)]
    #[doc(hidden)]
    pub fn debug_d3d11_left_out() -> Vec<&'static str> {
        d3d11_frame::left_out_so_far()
    }
    /// Keep each window frame's damage regions from now on, for
    /// `debug_regions`: a test placing what a frame reads across them.
    #[doc(hidden)]
    pub fn debug_keep_regions(&mut self) {
        self.debug_regions.get_or_insert_default();
    }
    /// The last window frame's damage regions, once `debug_keep_regions`.
    #[doc(hidden)]
    pub fn debug_regions(&self) -> &[Rect] {
        self.debug_regions.as_deref().unwrap_or_default()
    }
    /// Have this renderer's frames wait for shaders compiling, though it
    /// presents: a test comparing its pixels with an offscreen one's.
    #[doc(hidden)]
    pub fn debug_wait_for_shaders(&mut self) {
        self.debug_waits_for_shaders = true;
    }
    /// Have this offscreen renderer's frames go on without what is made off
    /// them (a shader compiling, a new zoom's canvas raster), as a window's
    /// do: a test of what they show meanwhile.
    #[doc(hidden)]
    pub fn debug_go_without(&mut self) {
        self.offscreen_waits = false;
    }
    /// Whether `shader`'s fill can draw: its pipeline compiled, else started
    /// compiling if new (`start_fill`). A window's frame goes on without
    /// one still compiling, drawing the box's colour, and draws again once
    /// it is (`images_awaited`); an offscreen renderer waits for it.
    fn prepare_fill(&mut self, shader: &Arc<zgx::decoration::FillShader>) -> bool {
        let wait = self.waits_for_shaders();
        if let Some(compiled) = self.fill_pipelines.get_mut(&shader.id()) {
            return match compiled.ready(wait) {
                Some(ready) => ready,
                None => {
                    self.images_awaited = true;
                    false
                }
            };
        }
        self.start_fill(shader);
        self.prepare_fill(shader)
    }
    /// Start compiling `shader`'s fill off the frame, if not yet.
    #[cold]
    fn start_fill(&mut self, shader: &Arc<zgx::decoration::FillShader>) {
        if self.fill_pipelines.contains_key(&shader.id()) {
            return;
        }
        let (device, layout) = (self.device.clone(), self.context.inner.quad_layout.clone());
        let source = fill_source(shader.source());
        let backend = self.info.backend;
        // An app's own shader (not the built-in radial one): on Direct3D 12
        // with FXC its fragment stage compiles in a process of its own.
        let apps = shader.source() != zgx::decoration::radial_shader().source();
        let slot = compile::start(self.waits_for_shaders(), move || {
            fill_validates(&source)
                .then(|| {
                    Self::app_built(
                        &device,
                        backend,
                        apps,
                        &source,
                        "zgx_fill",
                        AppLayout::Quads,
                        |vertex, fragment| {
                            fill_pipeline(&device, vertex, fragment, &layout, "zgx_fill")
                        },
                    )
                })
                .flatten()
        });
        self.fill_shaders.insert(shader.id(), shader.clone());
        (self.fill_pipelines).insert(shader.id(), compile::Compiled::Pending(slot));
        #[cfg(target_os = "macos")]
        if let Some(Some(frame)) = self.metal_frame.get() {
            frame.start_fill(shader, self.waits_for_shaders());
        }
        #[cfg(d3d11)]
        if let Some(state) = &self.d3d11 {
            state
                .device
                .start_app_shader(shader, d3d11_frame::AppShader::Fill);
        }
    }
    /// Start compiling the fills scripts declared since the last frame
    /// (`warm_fill_shader`), before anything draws them.
    #[cold]
    fn start_declared_fills(&mut self) {
        if let Some(declared) = compile::declared_since(&mut self.fill_shaders_declared) {
            for shader in &declared {
                self.start_fill(shader);
            }
        }
        // Layer shaders that will first draw in a hurry: a one-shot's. One
        // with inputs compiles with their bindings, as the frame's pre-pass
        // would start it (`prepare_inputs`).
        for shader in zgx::decoration::declared_layer_shaders(&mut self.layers_declared) {
            match shader.inputs() {
                Some(inputs) => self.start_inputs(&shader, inputs),
                None => {
                    self.prepare_filter(&shader);
                }
            }
        }
        self.fills_declared = compile::declared() + zgx::decoration::layer_shaders_declared();
    }
    /// Whether layer shader `shader` can draw: its pipeline compiled, else
    /// started compiling if new, off the frame as a fill's (`prepare_fill`):
    /// until then its layer composites as painted. zgx's own colour filter
    /// waits, as it's a matter of what the view looks like, not an effect
    /// joining it.
    fn prepare_filter(&mut self, shader: &Arc<zgx::decoration::FillShader>) -> bool {
        if shader.reads_backdrop() {
            // Only ever drawn reading the backdrop's group.
            self.filter_shaders.insert(shader.id(), shader.clone());
            return self.prepare_blend(Some(shader));
        }
        let apps = shader.source() != zgx::decoration::color_filter_shader().source();
        if !self.filter_pipelines.contains_key(&shader.id()) {
            let (device, layout) = (self.device.clone(), self.context.inner.quad_layout.clone());
            let source = filter_source(shader.source());
            let backend = self.info.backend;
            let slot = compile::start(self.waits_for_shaders(), move || {
                fill_validates(&source)
                    .then(|| {
                        Self::app_built(
                            &device,
                            backend,
                            apps,
                            &source,
                            "zgx_filter",
                            AppLayout::Quads,
                            |vertex, fragment| {
                                fill_pipeline(&device, vertex, fragment, &layout, "zgx_filter")
                            },
                        )
                    })
                    .flatten()
            });
            self.filter_shaders.insert(shader.id(), shader.clone());
            (self.filter_pipelines).insert(shader.id(), compile::Compiled::Pending(slot));
        }
        let wait = !apps || self.waits_for_shaders();
        let ready = self
            .filter_pipelines
            .get_mut(&shader.id())
            .and_then(|compiled| compiled.ready(wait));
        match ready {
            Some(ready) => ready,
            None => {
                self.images_awaited = true;
                false
            }
        }
    }
    /// The blended layers' group 2 (the backdrop) and pipeline layout.
    fn blend_layouts(&self) -> &(wgpu::BindGroupLayout, wgpu::PipelineLayout) {
        self.blend_layouts.get_or_init(|| {
            let backdrop = self
                .device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("blend backdrop"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    }],
                });
            let layout = self
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("blended layers"),
                    bind_group_layouts: &[
                        Some(&self.context.inner.texture_layout),
                        Some(&self.context.inner.viewport_layout),
                        Some(&backdrop),
                    ],
                    immediate_size: 0,
                });
            (backdrop, layout)
        })
    }
    /// Whether blended layers can draw, plainly or through layer shader
    /// `shader`: compiled, else started compiling if new. Blending plainly or
    /// through zgx's colour filter waits for it, as a frame always has; an
    /// app's shader goes off the frame (`prepare_filter`).
    fn prepare_blend(&mut self, shader: Option<&zgx::decoration::FillShader>) -> bool {
        let key = shader.map_or(BLEND_PLAIN, |shader| shader.id());
        // An app's layer shader, blended: not zgx's own blend or colour filter.
        let apps = shader.is_some_and(|shader| {
            shader.source() != zgx::decoration::color_filter_shader().source()
        });
        if !self.blend_pipelines.contains_key(&key) {
            let (source, entry) = match shader {
                Some(shader) => (filter_source(shader.source()), "zgx_filter_blended"),
                None => (blend_source(), "zgx_blended"),
            };
            let layout = self.blend_layouts().1.clone();
            let device = self.device.clone();
            let backend = self.info.backend;
            let slot = compile::start(self.waits_for_shaders(), move || {
                fill_validates(&source)
                    .then(|| {
                        Self::app_built(
                            &device,
                            backend,
                            apps,
                            &source,
                            entry,
                            AppLayout::Blend,
                            |vertex, fragment| {
                                blend_pipeline(&device, vertex, fragment, &layout, entry)
                            },
                        )
                    })
                    .flatten()
            });
            self.blend_pipelines
                .insert(key, compile::Compiled::Pending(slot));
        }
        let wait = !apps || self.waits_for_shaders();
        let ready = self
            .blend_pipelines
            .get_mut(&key)
            .and_then(|compiled| compiled.ready(wait));
        match ready {
            Some(ready) => ready,
            None => {
                self.images_awaited = true;
                false
            }
        }
    }
    /// The backdrop for the current target (`target`'s size): made once
    /// per size, its bind group with it. Making one for a new size drops
    /// those no target has any more (the window's former sizes, a layer's),
    /// so a window resized again and again keeps none of its old ones.
    fn backdrop(&mut self) -> &(wgpu::Texture, wgpu::BindGroup) {
        let size = (self.target.width(), self.target.height());
        if !self.backdrops.contains_key(&size) {
            let window = (self.width, self.height);
            let layers = &self.layers;
            self.backdrops.retain(|&held, _| {
                held == window
                    || layers
                        .values()
                        .any(|layer| (layer.texture.width(), layer.texture.height()) == held)
            });
            // Copied into in parts and read straight after: uncompressed on
            // Apple GPUs, where a read could miss part of the copy.
            #[cfg(target_os = "macos")]
            let texture = {
                let texture =
                    upload::uncompressed_texture(&self.device, size.0, size.1, "blend backdrop");
                written(
                    &self.device,
                    &self.queue,
                    &texture.create_view(&Default::default()),
                );
                texture
            };
            #[cfg(not(target_os = "macos"))]
            let texture = texture(&self.device, size.0, size.1, "blend backdrop");
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blend backdrop"),
                layout: &self.blend_layouts().0,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                }],
            });
            self.backdrops.insert(size, (texture, bind));
        }
        &self.backdrops[&size]
    }
    /// Group 1 of the quad pipelines: the viewport `uniform` and `regions`.
    fn viewport_group(
        &self,
        uniform: &wgpu::Buffer,
        regions: wgpu::BindingResource<'_>,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport"),
            layout: &self.context.inner.viewport_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: regions,
                },
            ],
        })
    }
    fn texture_bind(&self, view: &wgpu::TextureView, label: &str) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.context.inner.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.device.create_sampler(
                        &wgpu::SamplerDescriptor {
                            mag_filter: wgpu::FilterMode::Linear,
                            min_filter: wgpu::FilterMode::Linear,
                            ..Default::default()
                        },
                    )),
                },
            ],
        })
    }
    /// Frames in a row, this one included, whose changes a kept window
    /// would have patched and drawn from: leaves alone repainting (no
    /// composite change, nothing mounted or removed), over `KEPT_SHARE` of
    /// the window, with no flush unread between them.
    fn would_patch(&mut self, scene: &Scene, regions: &[Rect], viewport: Rect) -> u8 {
        let last = &mut self.patchable;
        if last.skip > 0 {
            last.skip -= 1;
            last.run = 0;
            return 0;
        }
        let (covered, repaints) = scene.repaints();
        let area = |r: &Rect| r.width * r.height;
        // (Flushes unread since, while skipping: read again from the next.)
        let read = *covered.start() == last.flushes + 1;
        // (Repainted subtrees and nodes mounted or removed are spliced;
        // since a splice walked too much, only leaves alone, patched in
        // place.)
        let patches = read
            && !repaints.is_empty()
            && regions.iter().map(area).sum::<f32>() >= KEPT_SHARE * area(&viewport)
            && (!last.refused
                || last.revision == scene.content_revision()
                    && repaints.iter().all(|&(id, flags)| {
                        flags & zgx::scene::repaint::COMPOSITE == 0
                            && scene.contains(id)
                            && scene.children(id).is_empty()
                    }));
        (last.run, last.skip, last.backoff) = match (read, patches) {
            (_, true) => (last.run.saturating_add(1), 0, 0),
            (false, false) => (0, 0, last.backoff),
            (true, false) => (0, last.backoff, (last.backoff * 2).clamp(1, 32)),
        };
        (last.flushes, last.revision) = (*covered.end(), scene.content_revision());
        last.run
    }
    /// Whether the glyph atlas is filling up: the next frame drawn as usual
    /// recycles it.
    #[inline(always)]
    fn atlas_filling(&self) -> bool {
        self.cursor.1 + self.cursor.2 > self.atlas_size * 3 / 4 || self.entries.len() >= 8192
    }
    /// Draw `items`, the window's whole paint as the walk found them with
    /// what they `inherited`, and keep it to patch: their quads and draws
    /// made with where each item's lie (`render_flat::<PATCH>`), then drawn
    /// as made.
    fn render_keeping(
        &mut self,
        scene: &Scene,
        items: Vec<zgx::scene::PaintItem<'_>>,
        inherited: Vec<zgx::scene::Inherited>,
        damage: &[Rect],
        merged: Option<Vec<Rect>>,
        viewport: Rect,
    ) -> Result<GpuStats, GpuError> {
        let ids: Vec<NodeId> = items.iter().map(|item| item.id).collect();
        let boxed: Vec<Option<KeptBox>> = items
            .iter()
            .map(|item| match item.kind {
                // (Only boxes keep one: no other item is weighed.)
                NodeKind::Quad(_) | NodeKind::Panel { .. } => KeptBox::of(item, plain_box(item)),
                _ => None,
            })
            .collect();
        self.patch_clips.clear();
        let made =
            self.render_flat::<PATCH>(scene, items, &[viewport], Some(vec![viewport]), true)?;
        let ranges = std::mem::take(&mut self.item_ranges);
        // Not shader fills or polylines, whose uniforms and points change
        // without the scene marking their nodes.
        self.draw_made = true;
        if ranges.len() != ids.len()
            || inherited.len() != ids.len()
            || !self.frame_points.is_empty()
            || self.frame_draws.iter().any(|draw| {
                matches!(
                    draw.shading,
                    Shading::Fill(_)
                        | Shading::Filter(_)
                        | Shading::Blend
                        | Shading::FilterBlend(_)
                        | Shading::Polyline
                )
            })
        {
            self.item_ranges = ranges;
            let mut drawn = self.render_flat::<FRAME>(scene, Vec::new(), damage, merged, false)?;
            drawn.add_made(&made);
            return Ok(drawn);
        }
        let len = self.frame_quads.len();
        let draws = self.frame_draws.clone();
        let clips = self.patch_clips.clone();
        let mut drawn = self.render_flat::<FRAME>(scene, Vec::new(), damage, merged, false)?;
        drawn.add_made(&made);
        let mut quads = std::mem::take(&mut self.frame_quads);
        // (Any copies a layer frame catches up with come after.)
        quads.truncate(len);
        let items: Vec<KeptItem> = ids
            .into_iter()
            .zip(inherited)
            .zip(&ranges)
            .zip(boxed)
            .map(
                |(((id, inherited), &(start, len, room, first, end)), boxed)| KeptItem {
                    id,
                    inherited,
                    start,
                    len,
                    room,
                    draws: first..end,
                    boxed,
                    end: 0,
                },
            )
            .collect();
        let mut items = items;
        subtree_ends(scene, &mut items, 0);
        let by_node = KeptIndex::new(&items);
        self.item_ranges = ranges;
        self.kept = Some(Box::new(Kept {
            quads,
            draws,
            clips,
            items,
            by_node,
            flushes: *scene.repaints().0.end(),
            revision: scene.content_revision(),
            identity: scene.identity(),
            atlas_epoch: self.atlas_epoch,
            viewport,
            scale: self.scale,
            background: self.background,
            patched: false,
            upload: upload::KeptQuads::new(),
            ordered: len as u32,
        }));
        Ok(drawn)
    }
    /// Draw the window as it was kept, where the scene repainted only
    /// leaves since: each one's instances made again (`render_flat::<PATCH>`,
    /// as a frame makes them) into its place, every other item's as they
    /// were. Only the repainted items are visited (the scene names them),
    /// not the window's. `None` when it can't: then the frame is drawn as
    /// usual.
    // Out of line: once a frame, and its loops over the repainted items then
    // compile alone, whatever `render_once` around it holds.
    #[inline(never)]
    fn render_kept(
        &mut self,
        scene: &Scene,
        damage: &[Rect],
        regions: &[Rect],
        viewport: Rect,
    ) -> Result<Option<GpuStats>, GpuError> {
        let Some(mut kept) = self.kept.take() else {
            return Ok(None);
        };
        let (covered, repaints) = scene.repaints();
        // Kept while the scene and target are those it was drawn for, and
        // the atlas keeps its glyphs where they were.
        // (Nodes mounted or removed lie in subtrees spliced below.)
        if *covered.start() != kept.flushes + 1
            || kept.identity != scene.identity()
            || kept.atlas_epoch != self.atlas_epoch
            || kept.viewport != viewport
            || kept.scale != self.scale
            || kept.background != self.background
            || self.fresh
            || self.plane.is_some()
            || self.in_layer
            || !self.effect_jobs.is_empty()
            || scene.has_backdrop_blur()
            || scene.has_overflow_fades()
            || self.atlas_filling()
        {
            return Ok(self.recycle_kept(*kept));
        }
        // (What this frame's patches write counts as one version of it.)
        kept.upload.patch();
        // The repainted items, worked out alone from what they inherited,
        // with their places: only leaves, painting as many items as before
        // (a box painting over what it holds has a border pass after it).
        let mut items = Vec::new();
        let mut places = Vec::new();
        // Plain boxes, made into their places as they are found
        // (`patch_box`), as `render_flat` makes them: whether all fit.
        let (mut boxes, mut boxes_fit) = (0, true);
        let (mut box_quads, mut box_draws) = (
            std::mem::take(&mut self.box_quads),
            std::mem::take(&mut self.box_draws),
        );
        // Subtrees repainted (a container, a composite change, a changed
        // list of children, a node not worked out alone): each walked again
        // whole from what its root inherited and spliced over the items it
        // had (`splice`). Kept items of removed nodes and mounted nodes not
        // kept must lie in one.
        let mut splices: Vec<(usize, usize, NodeId)> = Vec::new();
        let (mut leaving, mut entering) = (Vec::new(), Vec::new());
        for &(id, flags) in repaints {
            let first = kept.by_node.get(id, &kept.items);
            if !scene.contains(id) {
                leaving.extend(first);
                continue;
            }
            let Some(first) = first else {
                entering.push(id);
                continue;
            };
            if flags & (zgx::scene::repaint::COMPOSITE | zgx::scene::repaint::CHILDREN) != 0
                || !scene.children(id).is_empty()
            {
                splices.push((first, kept.items[first].end as usize, id));
                continue;
            }
            // A kept box layout alone moved or resized (`BOX_ONLY`): the same
            // paint and compositing at its new box, without working its item
            // out.
            if flags & zgx::scene::repaint::BOX_ONLY != 0
                && let Some(boxed) = kept.items[first].boxed
                && kept.items.get(first + 1).is_none_or(|next| next.id != id)
            {
                let (bounds, kind) = scene.box_in(id, &kept.items[first].inherited);
                if plain_quad(kind) {
                    boxes += 1;
                    if self.patch_box_moved(&mut kept, first, bounds, boxed.clip, viewport) {
                        continue;
                    }
                    let parts = PlainBox {
                        kind,
                        bounds,
                        clip: boxed.clip,
                        opacity: boxed.opacity,
                        edge_fade: boxed.edge_fade,
                    };
                    boxes_fit = boxes_fit
                        && self.patch_box(
                            &mut kept,
                            first,
                            parts,
                            viewport,
                            &mut box_quads,
                            &mut box_draws,
                        );
                    continue;
                }
            }
            let over = matches!(scene.kind(id),
                NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } if quad.paints_over());
            let count = 1 + usize::from(over);
            if kept.items[first..]
                .iter()
                .take_while(|item| item.id == id)
                .count()
                != count
            {
                splices.push((first, kept.items[first].end as usize, id));
                continue;
            }
            for place in first..first + count {
                let Some(item) = scene.item_in(id, &kept.items[place].inherited) else {
                    splices.push((first, kept.items[first].end as usize, id));
                    break;
                };
                let plain = plain_box(&item);
                kept.items[place].boxed = KeptBox::of(&item, plain);
                if plain {
                    boxes += 1;
                    boxes_fit = boxes_fit
                        && self.patch_box(
                            &mut kept,
                            place,
                            PlainBox::of(&item),
                            viewport,
                            &mut box_quads,
                            &mut box_draws,
                        );
                } else {
                    items.push(item);
                    places.push(place);
                }
            }
        }
        // Outermost first, nested ones dropped.
        splices.sort_unstable_by_key(|&(first, _, _)| first);
        splices.dedup_by(|inner, outer| inner.0 < outer.1);
        let spliced = |place: usize| {
            let at = splices.partition_point(|&(first, _, _)| first <= place);
            at > 0 && place < splices[at - 1].1
        };
        // Drawn from the kept frame only with damage over `KEPT_SHARE` of
        // the window: kept up to date otherwise only where that is cheap
        // (leaves alone).
        let area = |r: &Rect| r.width * r.height;
        let from_kept = regions.iter().map(area).sum::<f32>() >= KEPT_SHARE * area(&viewport);
        // Walked again, a subtree costs about what drawing it as usual does,
        // and drawing from the kept frame about what drawing half the window
        // does: spliced only while the subtrees are a small part of the
        // window and the damage most of it.
        let walked: usize = splices.iter().map(|&(first, end, _)| end - first).sum();
        let splices_pay = walked as f32 <= SPLICE_SHARE * kept.items.len() as f32
            && regions.iter().map(area).sum::<f32>() >= SPLICE_DAMAGE * area(&viewport);
        // (Such frames count as frames that wouldn't patch: keeping again
        // backs off, `would_patch`.)
        if !splices.is_empty() && !splices_pay {
            self.patchable.refused = true;
        }
        if (!splices.is_empty() && (!from_kept || !splices_pay))
            || !leaving.iter().all(|&place| spliced(place))
            || !entering.iter().all(|&id| {
                splices
                    .iter()
                    .any(|&(_, _, root)| scene.is_within(id, root))
            })
        {
            (self.box_quads, self.box_draws) = (box_quads, box_draws);
            return Ok(self.recycle_kept(*kept));
        }
        if !splices.is_empty() {
            let mut keeps = places.iter().map(|&place| !spliced(place));
            items.retain(|_| keeps.next().unwrap());
            places.retain(|&place| !spliced(place));
        }
        // (Plain boxes alone, as a bound colour's: nothing for it to make.)
        let mut stats = GpuStats::default();
        if !items.is_empty() {
            self.patch_clips = std::mem::take(&mut kept.clips);
            stats =
                self.render_flat::<PATCH>(scene, items, &[viewport], Some(vec![viewport]), false)?;
            kept.clips = std::mem::take(&mut self.patch_clips);
        } else {
            // (As a patch of no items leaves them.)
            self.item_ranges.clear();
            self.frame_points.clear();
        }
        let ranges = std::mem::take(&mut self.item_ranges);
        // Into their places, unless one grew past its room, drew more draws
        // or polylines (whose points a kept frame doesn't hold).
        // A text outgrowing its room moves to the kept frame's end: its
        // draws keep their places among the others, so it paints where it
        // did; its quads alone lie elsewhere. (A text's quads, textured,
        // never hide a region whole: `occluded_from` skips none for it.)
        let moves = |item: &KeptItem, len: u32| {
            len > item.room && matches!(scene.kind(item.id), NodeKind::Text { .. })
        };
        let fits = self.frame_points.is_empty()
            && ranges.len() == places.len()
            && ranges
                .iter()
                .zip(&places)
                .all(|(&(_, len, _, first, end), &place)| {
                    let item = &kept.items[place];
                    (len <= item.room || moves(item, len))
                        && end - first == item.draws.end - item.draws.start
                });
        if fits {
            for (&(start, len, _, first, end), &place) in ranges.iter().zip(&places) {
                let item = &mut kept.items[place];
                if matches!(scene.kind(item.id), NodeKind::Text { .. }) {
                    if self.changed_texts.len() >= SHAPED_LIKE {
                        self.changed_texts.clear();
                    }
                    self.changed_texts.insert(item.id);
                }
                if len > item.room {
                    let room = len + text_room(len);
                    // At the physical tail its room grows where it is;
                    // otherwise emptied where it was, at the end again.
                    if item.start + item.room != kept.quads.len() as u32 {
                        let old = item.start as usize..(item.start + item.len) as usize;
                        kept.upload.wrote(item.start, item.start + item.len);
                        kept.quads[old].fill(bytemuck::Zeroable::zeroed());
                        item.start = kept.quads.len() as u32;
                    }
                    (item.len, item.room) = (0, room);
                    kept.quads.resize(
                        item.start as usize + room as usize,
                        bytemuck::Zeroable::zeroed(),
                    );
                }
                kept.upload
                    .wrote(item.start, item.start + len.max(item.len));
                let (at, made) = (item.start as usize, len as usize);
                kept.quads[at..at + made]
                    .copy_from_slice(&self.frame_quads[start as usize..(start + len) as usize]);
                // What a longer value left past this one's, emptied.
                if made < item.len as usize {
                    kept.quads[at + made..at + item.len as usize]
                        .fill(bytemuck::Zeroable::zeroed());
                }
                item.len = len;
                for (to, draw) in
                    (item.draws.start..).zip(&self.frame_draws[first as usize..end as usize])
                {
                    kept.draws[to as usize] = Draw {
                        start: draw.start - start + item.start,
                        end: draw.end - start + item.start,
                        ..draw.clone()
                    };
                }
            }
            stats.patched_items = places.len();
        }
        self.item_ranges = ranges;
        (self.box_quads, self.box_draws) = (box_quads, box_draws);
        let fits = fits && boxes_fit;
        if fits {
            stats.patched_items += boxes;
        }
        if !fits {
            // Drawn as usual, reporting what was made.
            stats.patched_items = 0;
            self.patch_stats = stats;
            return Ok(self.recycle_kept(*kept));
        }
        // (A subtree's quads lie together only until a text moves: kept
        // again, laid out in order.)
        if !splices.is_empty() && kept.quads.len() as u32 != kept.ordered {
            stats.patched_items = 0;
            self.patch_stats = stats;
            return Ok(self.recycle_kept(*kept));
        }
        if !splices.is_empty() {
            match self.splice(scene, &mut kept, &splices, viewport)? {
                Some(made) => {
                    kept.ordered = kept.quads.len() as u32;
                    stats.add_made(&made);
                    stats.patched_items += made.patched_items;
                }
                None => {
                    stats.patched_items = 0;
                    self.patch_stats = stats;
                    return Ok(self.recycle_kept(*kept));
                }
            }
        }
        kept.revision = scene.content_revision();
        kept.flushes = *covered.end();
        // Drawn from it when the damage covers enough of the window that
        // making each damaged item again costs more than sending all of
        // them as they are; otherwise drawn as usual, and kept up to date.
        let area = |r: &Rect| r.width * r.height;
        if regions.iter().map(area).sum::<f32>() < KEPT_SHARE * area(&viewport) {
            self.kept = Some(kept);
            stats.patched_items = 0;
            self.patch_stats = stats;
            return Ok(None);
        }
        let len = kept.quads.len();
        std::mem::swap(&mut self.frame_quads, &mut kept.quads);
        // Its draws lent too, unless a frame may be deferred to its present
        // (`Deferred`), which takes them.
        let (lent, draws) = (self.single_pass_format.is_none(), kept.draws.len());
        match lent {
            true => std::mem::swap(&mut self.frame_draws, &mut kept.draws),
            false => self.frame_draws.clone_from(&kept.draws),
        }
        self.patch_clips.clone_from(&kept.clips);
        self.frame_points.clear();
        self.draw_made = true;
        // Its quads lead the frame's: a vertex buffer still holding them
        // takes only what was written since.
        kept.upload.drawn(len * size_of::<Quad>());
        self.kept_ordered = kept.ordered;
        self.kept_upload = Some((std::mem::take(&mut kept.upload), len * size_of::<Quad>()));
        let drawn =
            self.render_flat::<FRAME>(scene, Vec::new(), damage, Some(regions.to_vec()), false);
        kept.upload = self
            .kept_upload
            .take()
            .map(|(upload, _)| upload)
            .unwrap_or_default();
        std::mem::swap(&mut self.frame_quads, &mut kept.quads);
        kept.quads.truncate(len);
        if lent {
            std::mem::swap(&mut self.frame_draws, &mut kept.draws);
            kept.draws.truncate(draws);
        }
        let mut drawn = drawn?;
        drawn.patched_items = stats.patched_items;
        drawn.add_made(&stats);
        self.kept = Some(kept);
        Ok(Some(drawn))
    }
    /// A kept frame given up: its quads' room goes back to the frame's,
    /// which keeping had taken (the next frame would grow it again from
    /// nothing). `None`, for `render_kept` to return.
    /// Whether `text`'s node is one a kept frame patched (`changed_texts`).
    fn changes(&self, text: Option<NodeId>) -> bool {
        text.is_some_and(|id| self.changed_texts.contains(&id))
    }
    fn recycle_kept(&mut self, kept: Kept) -> Option<GpuStats> {
        let mut quads = kept.quads;
        if quads.capacity() > self.frame_quads.capacity() {
            quads.clear();
            self.frame_quads = quads;
        }
        None
    }
    /// Move kept plain box `place`, drawn as one quad, to `bounds` (layout
    /// alone moved or resized it, `BOX_ONLY`): its quad's colours, corners
    /// and border are what the same style and compositing made, so only
    /// its box and what follows from it are made again (its shading, its
    /// cull against `clip`, its draw), as `patch_box` makes them. False,
    /// with nothing changed, unless it is drawn as one quad before and
    /// after: then `patch_box` makes it.
    fn patch_box_moved(
        &self,
        kept: &mut Kept,
        place: usize,
        bounds: Rect,
        clip: Option<Rect>,
        viewport: Rect,
    ) -> bool {
        let kept_item = &kept.items[place];
        if kept_item.len != 1 || kept_item.draws.end - kept_item.draws.start != 1 {
            return false;
        }
        let at = kept_item.start as usize;
        let mut quad = kept.quads[at];
        // (Made whole and untransformed: its box is its rectangle and fade.)
        if quad.rect != quad.fade || quad.options[2] >= 0.5 {
            return false;
        }
        let fade = [bounds.x, bounds.y, bounds.width, bounds.height];
        (quad.rect, quad.fade) = (fade, fade);
        // (A flat colour's quad isn't split; a box's is, past a size.)
        if quad.shape[3] > 0.5 && !panel_whole(&quad, self.split_shading) {
            return false;
        }
        let Some((clip, whole_clip)) = moved_box_clip(clip, viewport, bounds) else {
            return false;
        };
        let rect = quad_rect(&quad);
        let limit = snap_out(clip, self.scale);
        if rect_outside(rect, limit) {
            return false;
        }
        // Only its geometry moved: the rest of the quad stays in place,
        // unless it was drawn without its region and moved where the region
        // changes pixels (`Region::covers`): then drawn in it again.
        let left = quad.mask[3] > 0. && leaves_region(&kept.clips, &quad, rect, self.scale);
        let stored = &mut kept.quads[at];
        stored.rect = quad.rect;
        stored.fade = quad.fade;
        if left {
            (stored.mask[0], stored.mask[3]) = (stored.mask[3], 0.);
        }
        kept.upload.wrote(at as u32, at as u32 + 1);
        let draw = &mut kept.draws[kept_item.draws.start as usize];
        draw.shading = match self.split_shading {
            true => shading(stored, self.scale),
            false => Shading::Full,
        };
        draw.clip = match whole_clip || rect_inside(rect, limit) {
            true => viewport,
            false => clip,
        };
        draw.bounds = rect;
        true
    }
    /// Make plain box `item`'s quads and draws (`plain_box`), as
    /// `render_flat::<PATCH>` makes them, into its place in `kept`: false
    /// if they don't fit there (more quads than its room, other draws).
    fn patch_box(
        &self,
        kept: &mut Kept,
        place: usize,
        boxed: PlainBox<'_>,
        viewport: Rect,
        quads: &mut Vec<Quad>,
        draws: &mut Vec<Draw>,
    ) -> bool {
        quads.clear();
        draws.clear();
        let PlainBox {
            kind,
            bounds,
            clip,
            opacity,
            edge_fade,
        } = boxed;
        let clip = clip.unwrap_or(viewport).intersection(viewport);
        if let Some(clip) = clip.filter(|_| opacity > 0.) {
            let painted = match kind {
                NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
                    style.paint_bounds(bounds)
                }
                _ => bounds,
            };
            if viewport.intersects(clip) && viewport.intersects(painted) {
                let fade = [bounds.x, bounds.y, bounds.width, bounds.height];
                match kind {
                    NodeKind::Rect(color) => quads.push(rect_quad(
                        *color,
                        fade,
                        zgx::scene::Effects {
                            opacity,
                            edge_fade,
                            ..Default::default()
                        },
                    )),
                    NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => push_panel(
                        quads,
                        plain_panel_quad(style, fade, opacity, edge_fade, false),
                        self.scale,
                        self.split_shading,
                    ),
                    _ => unreachable!("a plain box"),
                }
                // One quad, not culled whole (the usual box): its draw as
                // `cull_on_cpu` and `push_draw` make it, written into place.
                if let [quad] = quads[..]
                    && quad.options[2] < 0.5
                    && let rect = quad_rect(&quad)
                    && let limit = snap_out(clip, self.scale)
                    && !rect_outside(rect, limit)
                {
                    let kept_item = &mut kept.items[place];
                    if kept_item.room == 0 || kept_item.draws.end - kept_item.draws.start != 1 {
                        return false;
                    }
                    kept.upload
                        .wrote(kept_item.start, kept_item.start + kept_item.len.max(1));
                    let at = kept_item.start as usize;
                    kept.quads[at] = quad;
                    // What a longer value left past this one's, emptied.
                    if kept_item.len > 1 {
                        kept.quads[at + 1..at + kept_item.len as usize]
                            .fill(bytemuck::Zeroable::zeroed());
                    }
                    kept_item.len = 1;
                    kept.draws[kept_item.draws.start as usize] = Draw {
                        shading: match self.split_shading {
                            true => shading(&quad, self.scale),
                            false => Shading::Full,
                        },
                        source: TextureSource::Atlas,
                        blur: None,
                        start: at as u32,
                        end: at as u32 + 1,
                        clip: match rect_inside(rect, limit) {
                            true => viewport,
                            false => clip,
                        },
                        bounds: rect,
                    };
                    return true;
                }
                let (draw_clip, culled) = cull_on_cpu(quads, 0, clip, viewport, self.scale);
                if !quads.is_empty() {
                    let draw = Draw {
                        shading: Shading::Full,
                        source: TextureSource::Atlas,
                        blur: None,
                        start: 0,
                        end: quads.len() as u32,
                        clip: draw_clip,
                        bounds: culled.unwrap_or_else(|| quad_bounds(quads)),
                    };
                    push_draw(
                        draws,
                        quads,
                        draw,
                        self.split_shading,
                        false,
                        self.scale,
                        None,
                    );
                }
            }
        }
        let kept_item = &mut kept.items[place];
        let len = quads.len() as u32;
        if len > kept_item.room || draws.len() as u32 != kept_item.draws.end - kept_item.draws.start
        {
            return false;
        }
        kept.upload
            .wrote(kept_item.start, kept_item.start + len.max(kept_item.len));
        let at = kept_item.start as usize;
        kept.quads[at..at + quads.len()].copy_from_slice(quads);
        // What a longer value left past this one's, emptied.
        if len < kept_item.len {
            kept.quads[at + quads.len()..at + kept_item.len as usize]
                .fill(bytemuck::Zeroable::zeroed());
        }
        kept_item.len = len;
        for (to, draw) in (kept_item.draws.start..).zip(draws.drain(..)) {
            kept.draws[to as usize] = Draw {
                start: draw.start + kept_item.start,
                end: draw.end + kept_item.start,
                ..draw
            };
        }
        true
    }
    /// Walk each of `splices`' subtrees again (`first` and `end` of its
    /// items in `kept`, and its root) from what its root inherited, make
    /// their items (`render_flat::<PATCH>`, recording as a kept frame does)
    /// and put them over the old ones, moving what follows. `None` when
    /// they can't be kept (a layer, an effect, a shader fill or polyline):
    /// then the frame is drawn as usual.
    fn splice(
        &mut self,
        scene: &Scene,
        kept: &mut Kept,
        splices: &[(usize, usize, NodeId)],
        viewport: Rect,
    ) -> Result<Option<GpuStats>, GpuError> {
        let mut walked = Vec::new();
        let mut inherited = Vec::new();
        let mut counts = Vec::with_capacity(splices.len());
        let mut out = Vec::new();
        for &(first, _, root) in splices {
            let items = scene.layer_items_from(root, &kept.items[first].inherited, &mut out);
            if out.len() != items.len()
                || items
                    .iter()
                    .any(|item| item.isolated || changes_unmarked(item.kind))
            {
                return Ok(None);
            }
            counts.push(items.len());
            walked.extend(items);
            inherited.append(&mut out);
        }
        let ids: Vec<NodeId> = walked.iter().map(|item| item.id).collect();
        self.patch_clips = std::mem::take(&mut kept.clips);
        let made =
            self.render_flat::<PATCH>(scene, walked, &[viewport], Some(vec![viewport]), true)?;
        kept.clips = std::mem::take(&mut self.patch_clips);
        let ranges = std::mem::take(&mut self.item_ranges);
        let keeps = ranges.len() == ids.len()
            && self.frame_points.is_empty()
            && !self.frame_draws.iter().any(|draw| {
                matches!(
                    draw.shading,
                    Shading::Fill(_)
                        | Shading::Filter(_)
                        | Shading::Blend
                        | Shading::FilterBlend(_)
                        | Shading::Polyline
                )
            });
        if !keeps {
            self.item_ranges = ranges;
            return Ok(None);
        }
        // Each splice's items, numbered where they were made.
        let mut ids = ids.into_iter();
        let mut inherited = inherited.into_iter();
        let mut made_items: Vec<Vec<KeptItem>> = Vec::with_capacity(splices.len());
        let mut at = 0;
        for &count in &counts {
            let new: Vec<KeptItem> = ranges[at..at + count]
                .iter()
                .map(|&(start, len, room, first, end)| KeptItem {
                    id: ids.next().expect("an item"),
                    inherited: inherited.next().expect("an item"),
                    start,
                    len,
                    room,
                    draws: first..end,
                    boxed: None,
                    end: 0,
                })
                .collect();
            made_items.push(new);
            at += count;
        }
        // The nodes whose items were spliced over, and whether any splice
        // changed how many items there are: if none did, only the spliced
        // items are found again by node (`by_node`).
        let mut replaced = Vec::new();
        let mut renumbered = false;
        let mut shifted_from = u32::MAX;
        // Last first: what a splice moves lies after it, spliced already.
        for (&(first, end, _), mut new) in splices.iter().zip(made_items).rev() {
            let old = &kept.items[first..end];
            if matches!(kept.by_node, KeptIndex::Sparse(_)) {
                replaced.extend(old.iter().map(|item| item.id));
            }
            let quads_from = old[0].start;
            let quads_to = old.last().map_or(quads_from, |item| item.start + item.room);
            let draws_from = old[0].draws.start;
            let draws_to = old.last().map_or(draws_from, |item| item.draws.end);
            // The new items' quads and draws, where they were made.
            let (made_from, made_to) = match (new.first(), new.last()) {
                (Some(a), Some(b)) => (a.start, b.start + b.room),
                _ => (0, 0),
            };
            let (made_draws_from, made_draws_to) = match (new.first(), new.last()) {
                (Some(a), Some(b)) => (a.draws.start, b.draws.end),
                _ => (0, 0),
            };
            kept.quads.splice(
                quads_from as usize..quads_to as usize,
                self.frame_quads[made_from as usize..made_to as usize]
                    .iter()
                    .copied(),
            );
            let moved = |at: u32| at - made_from + quads_from;
            kept.draws.splice(
                draws_from as usize..draws_to as usize,
                self.frame_draws[made_draws_from as usize..made_draws_to as usize]
                    .iter()
                    .map(|draw| Draw {
                        start: moved(draw.start),
                        end: moved(draw.end),
                        ..draw.clone()
                    }),
            );
            let quad_shift = (made_to - made_from) as i64 - (quads_to - quads_from) as i64;
            // Written over: its own quads, or from them on when what
            // follows moves.
            match quad_shift {
                0 => kept.upload.wrote(quads_from, quads_to),
                _ => shifted_from = shifted_from.min(quads_from),
            }
            let draw_shift =
                (made_draws_to - made_draws_from) as i64 - (draws_to - draws_from) as i64;
            let item_shift = new.len() as i64 - (end - first) as i64;
            let shift = |value: &mut u32, by: i64| *value = (*value as i64 + by) as u32;
            for item in &mut new {
                item.start = moved(item.start);
                item.draws = item.draws.start - made_draws_from + draws_from
                    ..item.draws.end - made_draws_from + draws_from;
            }
            subtree_ends(scene, &mut new, first as u32);
            renumbered |= item_shift != 0;
            // What follows moves; the subtrees holding this one end later.
            // (Nothing, when it is as long as what it replaces.)
            if (quad_shift, draw_shift, item_shift) != (0, 0, 0) {
                for item in &mut kept.items[end..] {
                    shift(&mut item.start, quad_shift);
                    shift(&mut item.draws.start, draw_shift);
                    shift(&mut item.draws.end, draw_shift);
                    shift(&mut item.end, item_shift);
                }
                for item in &mut kept.items[..first] {
                    if item.end as usize >= end {
                        shift(&mut item.end, item_shift);
                    }
                }
                let moved_from = draws_from as usize + (made_draws_to - made_draws_from) as usize;
                for draw in &mut kept.draws[moved_from..] {
                    shift(&mut draw.start, quad_shift);
                    shift(&mut draw.end, quad_shift);
                }
            }
            kept.items.splice(first..end, new);
        }
        if shifted_from != u32::MAX {
            kept.upload.wrote(shifted_from, kept.quads.len() as u32);
        }
        // (Each node's first item: a box's border pass comes after it.)
        kept.by_node
            .patch(&kept.items, renumbered, replaced, splices);
        self.item_ranges = ranges;
        let mut made = made;
        made.patched_items = counts.iter().sum();
        Ok(Some(made))
    }
    /// `merged`: `damage` merged into regions already (`merge_damage`).
    fn render_flat<const MODE: u8>(
        &mut self,
        scene: &Scene,
        items: Vec<zgx::scene::PaintItem<'_>>,
        damage: &[Rect],
        merged: Option<Vec<Rect>>,
        record: bool,
    ) -> Result<GpuStats, GpuError> {
        // The effect slots this frame takes to draw: a frame that fails
        // (a layer's glyphs past the atlas) draws none of them, and they
        // draw again with the next, or they'd composite what their slots
        // held before, for good.
        let planned: Vec<(NodeId, effects::Purpose)> = match MODE {
            PATCH => Vec::new(),
            _ => (self.effect_jobs.iter())
                .map(|job| (job.node, job.purpose))
                .collect(),
        };
        let result = self.render_flat_taking::<MODE>(scene, items, damage, merged, record);
        if result.is_err() {
            self.forget_slots(&planned);
        }
        result
    }
    fn render_flat_taking<const MODE: u8>(
        &mut self,
        scene: &Scene,
        items: Vec<zgx::scene::PaintItem<'_>>,
        damage: &[Rect],
        merged: Option<Vec<Rect>>,
        record: bool,
    ) -> Result<GpuStats, GpuError> {
        let mut stats = GpuStats::default();
        // Made already (a kept frame, patched, or one made to keep): drawn
        // as it is (it holds its background).
        let made = MODE != PATCH && std::mem::take(&mut self.draw_made);
        let copies = if self.in_layer || self.fresh || MODE == PATCH {
            Vec::new()
        } else {
            std::mem::take(&mut self.copies)
        };
        // Effect slots `prepare_effects` planned for these items: drawn
        // before anything composites them.
        let effect_jobs = match MODE {
            PATCH => Vec::new(),
            _ => std::mem::take(&mut self.effect_jobs),
        };
        if damage.is_empty() && !self.fresh && copies.is_empty() {
            self.forget_effects(&effect_jobs);
            return Ok(stats);
        }
        // Earlier commands read the vertex and uniform buffers rewritten below.
        if MODE != PATCH {
            self.flush_pending();
        }
        let viewport = Rect::new(
            0.,
            0.,
            self.width as f32 / self.scale,
            self.height as f32 / self.scale,
        );
        if MODE != PATCH {
            self.select_viewport(viewport.width, viewport.height);
        }
        // Whether the scene's own damage covers the window (not a repaint
        // forced by a stale target): see `Deferred::direct`.
        let whole = damage.iter().any(|d| {
            d.x <= 0.
                && d.y <= 0.
                && d.x + d.width >= viewport.width
                && d.y + d.height >= viewport.height
        });
        let has_blur = items
            .iter()
            .any(|item| item.effects.opacity > 0. && item.effects.blur_radius > 0.);
        // Layer shaders reading what's behind them depend on it as a filter does.
        let readers = scene.has_backdrop_readers();
        // Reconstruct every sampled backdrop pixel before replaying filters. A
        // dependency can reach another filter, so expand to a fixed point.
        let damage = if self.fresh {
            vec![viewport]
        } else if MODE != PATCH && (has_blur || readers) {
            // (Patching draws nothing: its damage is the window's.) Readers
            // alone: grown by the walk already, over all they can read.
            if readers
                && !self.in_layer
                && let Some(grown) = self
                    .backdrop_walk
                    .as_mut()
                    .filter(|walk| walk.frame == self.frame)
                    .and_then(|walk| walk.grown.take())
            {
                grown
            } else {
                let dependencies: Vec<_> = items
                    .iter()
                    .filter_map(|item| {
                        if readers
                            && item.isolated
                            && item.effects.opacity > 0.
                            && let Some((area, _)) = self.backdrop_read(scene, item, viewport)
                        {
                            return Some((area, 0.));
                        }
                        if item.effects.opacity <= 0. || item.effects.blur_radius <= 0. {
                            return None;
                        }
                        let output = item
                            .bounds
                            .intersection(item.clip.unwrap_or(viewport))?
                            .intersection(viewport)?;
                        Some((output, item.effects.blur_radius))
                    })
                    .collect();
                blur_damage(damage, &dependencies, viewport, self.scale)
            }
        } else {
            merged.unwrap_or_else(|| merge_damage(damage, viewport, self.scale))
        };
        if damage.is_empty() && copies.is_empty() {
            self.forget_effects(&effect_jobs);
            return Ok(stats);
        }
        if MODE != PATCH {
            self.frame = self.frame.wrapping_add(1);
        }
        // Fade bands drawn as one (bands.rs): their items drawn first, in
        // passes of this frame's own.
        let banding = std::mem::take(&mut self.banding) && MODE == FRAME && !record && !made;
        let (items, mut bands) = match banding {
            true => self.band(scene, items, &damage, viewport),
            false => (items, Vec::new()),
        };
        // Kept frames wait while bands are drawn (`bands_seen`).
        if banding && !self.in_layer {
            self.bands_seen = !bands.is_empty();
            if self.bands_seen {
                self.kept = None;
            }
        }
        // (A kept frame's glyphs stay where they are: patching it gave way
        // before the atlas filled, `render_kept`.)
        if MODE != PATCH && !made && self.atlas_filling() {
            // Filling again soon after the last recycle means the visible
            // working set does not fit: grow rather than re-rasterize every
            // glyph and icon each time it fills.
            let thrashing = self.frame.wrapping_sub(self.atlas_recycled_at) < ATLAS_THRASH_FRAMES;
            if thrashing && self.atlas_size < MAX_ATLAS && self.entries.len() < 8192 {
                self.reset_atlas((self.atlas_size * 2).min(MAX_ATLAS));
            } else {
                self.recycle_atlas();
            }
            self.atlas_recycled_at = self.frame;
        }
        let mut quads = std::mem::take(&mut self.frame_quads);
        if !made {
            quads.clear();
        }
        if !made && (MODE != PATCH || record) {
            quads.push(Quad {
                rect: [0., 0., viewport.width, viewport.height],
                uv: [0.; 4],
                color: rgba(self.background, 1.),
                fade: [0.; 4],
                options: [0.; 4],
                shape: [0.; 4],
                border: [0.; 4],
                mask: NO_MASK,
            });
        }
        let mut points = std::mem::take(&mut self.frame_points);
        if !made {
            points.clear();
        }
        let mut draws = std::mem::take(&mut self.frame_draws);
        // What backdrop-reading layers read, by draw (`backdrop_read`).
        let mut reads: Vec<(usize, Rect)> = Vec::new();
        // The clip regions quads lie in (`Quad::mask`): a kept frame's, to
        // patch it, or those of a frame made already.
        self.region_interiors.clear();
        let mut clips: Vec<Region> = match MODE {
            PATCH => std::mem::take(&mut self.patch_clips),
            _ if made => std::mem::take(&mut self.patch_clips),
            _ => Vec::new(),
        };
        if !made {
            draws.clear();
        }
        // To keep or patch: where each item's quads and draws lie, noted as
        // the next begins (an item may end anywhere in the loop's body).
        let mut ranges = match MODE {
            PATCH => std::mem::take(&mut self.item_ranges),
            _ => Vec::new(),
        };
        ranges.clear();
        // The open range's text, if one: its spare room, if it changes.
        let mut text = None;
        // Each band phase's items (bands.rs) in its texture's points, then
        // the frame's: one loop, the draws told apart where each phase ends.
        let (frame_items, frame_viewport, origin) = (items, viewport, self.layer_origin);
        let mut frame_items = Some(frame_items);
        let mut phase = 0;
        let mut ends = Vec::new();
        loop {
            let (items, damage, viewport) = match bands.get_mut(phase) {
                Some(band) => {
                    self.layer_origin = band.origin;
                    (
                        std::mem::take(&mut band.items),
                        std::borrow::Cow::Owned(band.damage.clone()),
                        band.viewport,
                    )
                }
                None => {
                    self.layer_origin = origin;
                    let items = frame_items.take().expect("the frame's items");
                    (
                        items,
                        std::borrow::Cow::Borrowed(&damage[..]),
                        frame_viewport,
                    )
                }
            };
            let damage = &damage[..];
            let mut items = items.into_iter();
            let items = LeftItems(&mut items);
            for item in items.0.by_ref() {
                if MODE == PATCH {
                    close_range(
                        &mut quads,
                        &draws,
                        &mut ranges,
                        record && self.changes(text),
                    );
                    ranges.push((quads.len() as u32, 0, 0, draws.len() as u32, u32::MAX));
                    text = matches!(item.kind, NodeKind::Text { .. }).then_some(item.id);
                }
                // Retained hidden subtrees must not allocate textures, shape text,
                // or trigger filters until their effective opacity is visible.
                if item.effects.opacity <= 0. {
                    continue;
                }
                // A text's own clip, its box, apart where its shadows reach
                // past it: then `node_clip` takes them in, for them alone
                // (`draw_text_shadows`).
                let mut own_clip = None;
                let node_clip = if !item.isolated
                    && matches!(item.kind, NodeKind::Text { .. } | NodeKind::RichText { .. })
                {
                    let clip = item.clip.unwrap_or(viewport);
                    let own = clip.intersection(item.bounds).unwrap_or_default();
                    match scene.has_text_shadows() {
                        true => shadowed(scene, item.id, item.bounds, clip, own, &mut own_clip),
                        false => own,
                    }
                } else {
                    item.clip.unwrap_or(viewport)
                };
                // Where its shadows' quads lie among its own
                // (`draw_text_shadows`).
                let mut shadow_quads = (0u32, 0u32);
                let Some(clip) = node_clip.intersection(viewport) else {
                    continue;
                };

                // A transformed layer's matrix, from this target's origin.
                let matrix = item.projective().map(|matrix| {
                    use zgx::affine::{Affine, Projective};
                    let (x, y) = self.layer_origin;
                    Projective::from(Affine::translation(x, y))
                        .then(matrix)
                        .then(Projective::from(Affine::translation(-x, -y)))
                });
                // A layer composited through its layer shader, once that compiles.
                let filter = item
                    .isolated
                    .then(|| scene.layer_shader(item.id))
                    .flatten()
                    .and_then(|layer| Some((layer.shader.clone()?, layer)))
                    .filter(|(shader, _)| self.prepare_filter(shader));
                // And blended with its backdrop, once that compiles (or
                // reading it, plainly over it).
                let blend = item
                    .isolated
                    .then(|| scene.blend(item.id))
                    .filter(|mode| {
                        *mode != zgx::scene::BlendMode::Normal
                            || filter
                                .as_ref()
                                .is_some_and(|(shader, _)| shader.reads_backdrop())
                    })
                    .filter(|_| self.prepare_blend(filter.as_ref().map(|(shader, _)| &**shader)));
                // How far past its box a layer shader paints.
                let bleed = filter.as_ref().map_or(0., |(_, layer)| layer.bleed);
                // Blurred or masked: composited from its effect slot.
                let effected = item.isolated
                    && (item.effects.filter_blur > 0. || scene.mask(item.id).is_some());
                let effect = effected
                    .then(|| self.effect_slot(item.id, effects::Purpose::Content))
                    .flatten();
                // Its mask decoding: nothing yet, rather than all of it.
                if effected && effect.is_none() && scene.mask(item.id).is_some() {
                    continue;
                }
                // And its drop shadow, from its own slot.
                let shadow = match item.isolated && scene.has_drop_shadows() {
                    true => scene
                        .drop_shadow(item.id)
                        .copied()
                        .zip(self.effect_slot(item.id, effects::Purpose::Shadow).copied()),
                    false => None,
                };
                if !damage.iter().any(|d| {
                    d.intersects(clip)
                        && d.intersects(if item.isolated {
                            let b = self.layers[&item.id].bounds;
                            let b = Rect::new(
                                b.x - self.layer_origin.0,
                                b.y - self.layer_origin.1,
                                b.width,
                                b.height,
                            );
                            let b = match bleed > 0. {
                                true => b.union(item.bounds.expand(bleed)),
                                false => b,
                            };
                            let b = effect.map_or(b, |slot| b.expand(slot.spread));
                            let b = shadow.map_or(b, |(shadow, slot)| {
                                let o = shadow.offset;
                                let moved = Rect::new(b.x + o.x, b.y + o.y, b.width, b.height);
                                b.union(moved.expand(slot.spread))
                            });
                            matrix.map_or(b, |matrix| matrix.bounds(b))
                        } else if let NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } =
                            item.kind
                        {
                            style.paint_bounds(item.bounds)
                        } else if let NodeKind::Svg(svg) = item.kind {
                            svg.paint_bounds(item.bounds)
                        } else if let NodeKind::Image(image) = item.kind {
                            image.paint_bounds(item.bounds)
                        } else {
                            item.bounds
                        })
                }) {
                    continue;
                }
                let start = quads.len() as u32;
                // A shader fill's draw uses its pipeline.
                let mut fill_shading = None;
                if item.isolated {
                    let layer = self.layers.get(&item.id).expect("prepared layer");
                    let bounds = Rect::new(
                        layer.bounds.x - self.layer_origin.0,
                        layer.bounds.y - self.layer_origin.1,
                        layer.bounds.width,
                        layer.bounds.height,
                    );
                    let (bounds, uv) = match (effect, &self.effects) {
                        (Some(slot), Some(atlas)) => slot.composite(bounds, atlas.size()),
                        _ => (bounds, [0., 0., layer.uv[0], layer.uv[1]]),
                    };
                    // Where the layer's pixels lie from the box's top-left,
                    // for a layer shader; and its quad reaching over its
                    // bleed, its uv going on past them.
                    let painted = [
                        bounds.x - item.bounds.x,
                        bounds.y - item.bounds.y,
                        bounds.x + bounds.width - item.bounds.x,
                        bounds.y + bounds.height - item.bounds.y,
                    ];
                    let (bounds, uv) = match bleed > 0. {
                        true => bled_quad(bounds, uv, item.bounds.expand(bleed)),
                        false => (bounds, uv),
                    };
                    // What it reads of what's behind it, copied for it.
                    let reading = filter
                        .as_ref()
                        .filter(|(shader, _)| shader.reads_backdrop())
                        .and_then(|_| self.backdrop_read(scene, &item, viewport));
                    // Transformed: the quad's corners move in the vertex shader,
                    // as an affine image's; edge fades stay in its own frame. In
                    // perspective (`options.z` 2), divided by `w` there too.
                    let mut m = matrix.unwrap_or_default();
                    let projective = matrix.is_some_and(|m| m.affine().is_none());
                    // Zoomed: its corner on a pixel of this target, so its texels
                    // land on pixels (`prepare_layer`).
                    if layer.exact && !projective {
                        let (x, y) = m.point(item.bounds.x, item.bounds.y);
                        m.tx += (x * self.scale).round() / self.scale - x;
                        m.ty += (y * self.scale).round() / self.scale - y;
                    }
                    quads.push(Quad {
                        rect: [bounds.x, bounds.y, bounds.width, bounds.height],
                        uv,
                        color: [1., 1., 1., item.effects.opacity],
                        fade: [
                            item.bounds.x,
                            item.bounds.y,
                            item.bounds.width,
                            item.bounds.height,
                        ],
                        options: [
                            item.effects.edge_fade,
                            2.,
                            match (matrix.is_some(), projective) {
                                (_, true) => 2.,
                                (true, false) => 1.,
                                (false, false) => 0.,
                            },
                            0.,
                        ],
                        shape: match matrix {
                            Some(_) => [m.a, m.c, m.b, m.d],
                            None => [0.; 4],
                        },
                        border: match (matrix, projective) {
                            (Some(_), true) => [m.tx, m.ty, m.px, m.py],
                            (Some(_), false) => [m.tx, m.ty, 0., 0.],
                            (None, _) => [0.; 4],
                        },
                        mask: [
                            item_region(&mut clips, &item),
                            filter.as_ref().map_or(0., |(_, layer)| {
                                let radii = blur_corners(item.kind, item.bounds);
                                let read = reading.map_or([0.; 4], |(_, from_box)| from_box);
                                let frame = layer_frame((bounds, uv), item.bounds);
                                let region =
                                    Region::layer_uniforms(layer, radii, painted, read, frame);
                                region_index(&mut clips, Some(region))
                            }),
                            blend.map_or(0., |mode| mode as u8 as f32),
                            0.,
                        ],
                    });
                    // Its drop shadow under it: its quad moved, from the
                    // shadow's slot.
                    let start = match (shadow, &self.effects) {
                        (Some((cast, slot)), Some(atlas)) => {
                            let layer_quad = quads.pop().expect("pushed above");
                            let o = cast.offset;
                            let moved = Rect::new(
                                bounds.x + o.x,
                                bounds.y + o.y,
                                bounds.width,
                                bounds.height,
                            );
                            let (rect, uv) = slot.composite(moved, atlas.size());
                            let alpha = f32::from(cast.color.3) / 255.;
                            quads.push(Quad {
                                rect: [rect.x, rect.y, rect.width, rect.height],
                                uv,
                                color: [1., 1., 1., item.effects.opacity * alpha],
                                mask: [layer_quad.mask[0], 0., 0., 0.],
                                ..layer_quad
                            });
                            draws.push(Draw {
                                shading: Shading::Full,
                                start,
                                end: start + 1,
                                clip,
                                bounds: quad_bounds(&quads[start as usize..]),
                                source: TextureSource::Image(effects::EFFECT_ATLAS),
                                blur: None,
                            });
                            quads.push(layer_quad);
                            start + 1
                        }
                        _ => start,
                    };
                    if let Some((read, _)) = reading {
                        reads.push((draws.len(), read));
                    }
                    draws.push(Draw {
                        shading: match (&filter, blend) {
                            (Some((shader, _)), Some(_)) => Shading::FilterBlend(shader.id()),
                            (None, Some(_)) => Shading::Blend,
                            (Some((shader, _)), None) => Shading::Filter(shader.id()),
                            (None, None) => Shading::Full,
                        },
                        start,
                        end: quads.len() as u32,
                        clip,
                        bounds: quad_bounds(&quads[start as usize..]),
                        // Effect layers sample the effect atlas, and batch.
                        source: match effect {
                            Some(_) => TextureSource::Image(effects::EFFECT_ATLAS),
                            None => TextureSource::Layer(item.id),
                        },
                        blur: (item.effects.blur_radius > 0.).then(|| {
                            Box::new((
                                item.bounds,
                                item.effects,
                                blur_corners(item.kind, item.bounds),
                                item.rounded(),
                            ))
                        }),
                    });
                    continue;
                }
                let bounds = item.bounds;
                let fade = [bounds.x, bounds.y, bounds.width, bounds.height];
                let mut canvas_image = None;
                // A canvas drawn as quads clips them to its box, as its raster is.
                let mut canvas_clip = None;
                // And its polyline strokes, if any, drawn apart (`push_draw`).
                let mut polylines = false;
                // And a text whose quads are its glyphs alone (no selection, no
                // decorations): drawn with one shading (`push_draw`).
                let mut glyphs_alone = false;
                // A gradient text in its own layer: its gradient's image, where
                // its glyphs start and its selected glyphs (`gradient_text_quads`).
                let mut gradient_text: Option<(u64, usize, Vec<Quad>)> = None;
                match item.kind {
                    NodeKind::Canvas(canvas) => {
                        if bounds.width <= 0. || bounds.height <= 0. {
                            continue;
                        }
                        // A grid under its drawing (a board's dots), clipped to
                        // its box: marks placed by the grid's view, none drawn.
                        if scene.has_grid_fills()
                            && let Some(grid) = scene.grid_fill(item.id)
                        {
                            grid_quads(grid, bounds, item.effects.opacity, &mut quads);
                            canvas_clip = Some(bounds);
                        }
                        let view = scene.canvas_view(item.id);
                        // Its own opacity and edge fade apply to it as a whole:
                        // drawn in quads, each would take them alone.
                        if item.effects.opacity >= 1.
                            && item.effects.edge_fade <= 0.
                            && let Some(drawn) = canvas_quads(
                                canvas,
                                (bounds.x, bounds.y),
                                view,
                                self.context.inner.quads.areas,
                                &mut points,
                            )
                        {
                            if let Some(old) = self.node_images.remove(&item.id) {
                                self.images.remove(&old);
                            }
                            self.canvases.forget(item.id);
                            for quad in drawn {
                                match quad {
                                    CanvasQuad::Fill(quad) => {
                                        push_panel(&mut quads, quad, self.scale, self.split_shading)
                                    }
                                    CanvasQuad::Ring(quad) => quads.push(quad),
                                    CanvasQuad::Polyline(quad) => {
                                        polylines = true;
                                        quads.push(quad)
                                    }
                                }
                            }
                            canvas_clip = Some(bounds);
                            stats.canvas_quads += quads.len() - start as usize;
                        } else {
                            let before = self.canvases.rasterizations();
                            let (width, height, scale) = (bounds.width, bounds.height, self.scale);
                            // Drawn to be panned: where the view puts it. A
                            // new zoom's raster is made off a window's frame
                            // (the zoom before's scaled meanwhile).
                            let wait = self.waits_for_shaders();
                            let mut placed = None;
                            let fonts = self.context.inner.canvas_fonts.clone();
                            let id = self.canvas_texture(item.id, &mut stats, |cache| {
                                cache.set_fonts(&fonts);
                                let (raster, placement) = cache.get_panned(
                                    item.id, canvas, view, width, height, scale, wait,
                                )?;
                                placed = Some(placement);
                                Ok(raster)
                            })?;
                            stats.canvas_rasterizations +=
                                (self.canvases.rasterizations() - before) as usize;
                            let canvas::Placement { rect, uv, scaled } = placed.expect("placed");
                            // Drawn again once that raster is made.
                            self.images_awaited |= scaled;
                            if rect[2] > 0. && rect[3] > 0. {
                                canvas_image = Some(id);
                                quads.push(Quad {
                                    rect: [
                                        bounds.x + rect[0],
                                        bounds.y + rect[1],
                                        rect[2],
                                        rect[3],
                                    ],
                                    uv,
                                    color: [1., 1., 1., item.effects.opacity],
                                    fade,
                                    options: [item.effects.edge_fade, 2., 0., 0.],
                                    shape: [0.; 4],
                                    border: [0.; 4],
                                    mask: NO_MASK,
                                });
                            }
                        }
                    }
                    NodeKind::Rect(color) => quads.push(rect_quad(*color, fade, item.effects)),
                    NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
                        if style.shadows().len() > 32 {
                            self.warn("a panel has more than 32 shadows: the first 32 draw");
                        }
                        // A solid border (any widths per side, any radii) is a
                        // ring the quad shader draws; dashed ones rasterize with
                        // the fill.
                        let ring = style.has_border()
                            && style.decoration.as_ref().is_none_or(|detail| {
                                detail.border_style == zgx::decoration::BorderStyle::Solid
                            });
                        let (opacity, edge_fade) = (item.effects.opacity, item.effects.edge_fade);
                        if item.border_pass {
                            // Its border alone, over what it holds, and its outline.
                            if ring && style.border_over {
                                quads.push(ring_quad(style, bounds, opacity, edge_fade));
                            }
                            if let Some(outline) = style.outline.filter(|o| o.visible()) {
                                if scene.has_outline_styles()
                                    && scene.outline_style(item.id)
                                        != zgx::decoration::BorderStyle::Solid
                                {
                                    let (id, quad) = self.styled_outline(
                                        (item.id, style, &outline),
                                        scene.outline_style(item.id),
                                        bounds,
                                        (opacity, edge_fade),
                                        &mut stats,
                                    )?;
                                    canvas_image = Some(id);
                                    quads.push(quad);
                                } else {
                                    quads.push(outline_quad(
                                        &outline, style, bounds, opacity, edge_fade,
                                    ));
                                    // And its inner ring, just within it.
                                    if let Some((width, color)) = scene.outline_inner(item.id) {
                                        let inner = zgx::scene::Outline {
                                            width,
                                            color,
                                            offset: outline.offset - width,
                                        };
                                        if inner.visible() {
                                            quads.push(outline_quad(
                                                &inner, style, bounds, opacity, edge_fade,
                                            ));
                                        }
                                    }
                                }
                            }
                        } else {
                            // (None for a popup's panel its system shadows,
                            // `set_unshadowed`.)
                            let shown = if self.unshadowed == Some(item.id) {
                                0
                            } else {
                                32
                            };
                            for shadow in style.shadows().iter().take(shown) {
                                let core = bounds.expand(shadow.spread.max(0.));
                                let core = Rect::new(
                                    core.x + shadow.offset.x,
                                    core.y + shadow.offset.y,
                                    core.width,
                                    core.height,
                                );
                                // Three sigmas out (`blur_radius` is the sigma).
                                let outer = core.expand(shadow.blur_radius.max(0.) * 3.);
                                quads.push(Quad {
                                    rect: [outer.x, outer.y, outer.width, outer.height],
                                    uv: [0.; 4],
                                    color: rgba(shadow.color, item.effects.opacity),
                                    fade: [core.x, core.y, core.width, core.height],
                                    options: [0., 0., 0., 1.],
                                    shape: [
                                        style.radius + shadow.spread.max(0.),
                                        0.,
                                        shadow.blur_radius.max(0.001),
                                        0.,
                                    ],
                                    border: decoration::shadow_corners(
                                        style,
                                        core.width,
                                        core.height,
                                        shadow.spread.max(0.),
                                    ),
                                    mask: NO_MASK,
                                });
                            }
                            // Drawn by its border pass instead (`border_over`).
                            let later = style.border_over && ring;
                            // Per-corner radii over a flat fill need no raster: the
                            // quad shader draws per-corner rounded rects, and their
                            // solid borders as rings. Gradients, patterns and
                            // dashes still rasterize.
                            let flat_fill = style.decoration.as_deref().and_then(|detail| {
                                let fill = match &detail.background {
                                    // An image or shader paints over the fill colour.
                                    None
                                    | Some(
                                        zgx::decoration::Background::Image { .. }
                                        | zgx::decoration::Background::Shader(_),
                                    ) => style.fill,
                                    Some(zgx::decoration::Background::Brush(
                                        zgx::canvas::Brush::Solid(color),
                                    )) => *color,
                                    Some(_) => return None,
                                };
                                (!style.has_border() || ring).then_some(fill)
                            });
                            if let Some(fill) = flat_fill {
                                if let Some(old) = self.node_images.remove(&item.id) {
                                    self.images.remove(&old);
                                }
                                push_panel(
                                    &mut quads,
                                    Quad {
                                        rect: fade,
                                        uv: [0.; 4],
                                        color: rgba(fill, opacity),
                                        fade,
                                        options: [edge_fade, 0., 0., 1.],
                                        shape: [0., 0., 0., 1.],
                                        border: decoration::shadow_corners(
                                            style,
                                            bounds.width,
                                            bounds.height,
                                            0.,
                                        ),
                                        mask: NO_MASK,
                                    },
                                    self.scale,
                                    self.split_shading,
                                );
                                if let Some(zgx::decoration::Background::Image { image, fit }) =
                                    style
                                        .decoration
                                        .as_deref()
                                        .and_then(|d| d.background.as_ref())
                                    && let Some(quad) = background_quad(
                                        image, *fit, style, bounds, opacity, edge_fade,
                                    )
                                    && self.ensure_shown_image(
                                        item.id,
                                        image,
                                        Some((
                                            quad.rect[2] * self.scale,
                                            quad.rect[3] * self.scale,
                                        )),
                                        &mut stats,
                                    )?
                                {
                                    quads.push(quad);
                                    canvas_image = Some(image.id());
                                }
                                if let Some(zgx::decoration::Background::Shader(shader)) = style
                                    .decoration
                                    .as_deref()
                                    .and_then(|d| d.background.as_ref())
                                    && self.prepare_fill(shader)
                                {
                                    #[allow(unused_mut)]
                                    let mut uniforms = scene.shader_uniforms(item.id);
                                    #[cfg(target_os = "macos")]
                                    if let Some(time) = self.loop_time {
                                        uniforms.time = time;
                                    }
                                    let uniforms = Region::uniforms(uniforms);
                                    let record = region_index(&mut clips, Some(uniforms));
                                    quads.push(Quad {
                                        rect: fade,
                                        uv: [0.; 4],
                                        color: [1., 1., 1., opacity],
                                        fade,
                                        options: [edge_fade, 0., 0., 1.],
                                        shape: [0., 0., 0., 1.],
                                        border: decoration::shadow_corners(
                                            style,
                                            bounds.width,
                                            bounds.height,
                                            0.,
                                        ),
                                        mask: [0., record, 0., 0.],
                                    });
                                    fill_shading = Some(Shading::Fill(shader.id()));
                                }
                                // (A shader fill shades this item's quads: no insets over it.)
                                if fill_shading.is_none() {
                                    polylines |= push_insets(
                                        &mut quads,
                                        &mut points,
                                        style,
                                        bounds,
                                        opacity,
                                        edge_fade,
                                    );
                                }
                                if ring && !later {
                                    quads.push(ring_quad(style, bounds, opacity, edge_fade));
                                }
                            } else if let Some((key, uv)) = (!style.has_border() || ring)
                                .then(|| axis_gradient(style, bounds))
                                .flatten()
                            {
                                // A gradient along one axis: a ramp texture
                                // stretched over the box (its corners cut by the
                                // quad shader), not a raster of the whole box.
                                if let Some(old) = self.node_images.remove(&item.id) {
                                    self.images.remove(&old);
                                }
                                let ramp = match get_in::<MODE, _, _>(&mut self.ramps, &key) {
                                    Some(ramp) => ramp.clone(),
                                    None => {
                                        let ramp = Arc::new(ramp_image(&key)?);
                                        self.ramps.insert(key.clone(), ramp.clone());
                                        ramp
                                    }
                                };
                                if !self.images.contains_key(&ramp.id()) {
                                    self.upload_image(&ramp)?;
                                    stats.image_uploads += 1;
                                }
                                insert_in::<MODE, _, _>(&mut self.node_ramps, item.id, key);
                                canvas_image = Some(ramp.id());
                                quads.push(Quad {
                                    rect: fade,
                                    uv,
                                    color: [1., 1., 1., opacity],
                                    fade,
                                    options: [edge_fade, 2., 0., 1.],
                                    shape: [0., 0., 0., 1.],
                                    border: decoration::shadow_corners(
                                        style,
                                        bounds.width,
                                        bounds.height,
                                        0.,
                                    ),
                                    mask: NO_MASK,
                                });
                                polylines |= push_insets(
                                    &mut quads,
                                    &mut points,
                                    style,
                                    bounds,
                                    opacity,
                                    edge_fade,
                                );
                                if ring && !later {
                                    quads.push(ring_quad(style, bounds, opacity, edge_fade));
                                }
                            } else if style.decoration.is_some()
                                && bounds.width > 0.
                                && bounds.height > 0.
                            {
                                // Its border left to the border pass, when it has one.
                                let drawn = match later {
                                    true => std::borrow::Cow::Owned(zgx::scene::QuadStyle {
                                        border_width: 0.,
                                        decoration: style.decoration.as_deref().map(|detail| {
                                            Arc::new(zgx::decoration::Decoration {
                                                border_widths: None,
                                                ..detail.clone()
                                            })
                                        }),
                                        ..style.clone()
                                    }),
                                    false => std::borrow::Cow::Borrowed(style),
                                };
                                let before = self.canvases.rasterizations();
                                let (width, height, scale) =
                                    (bounds.width, bounds.height, self.scale);
                                let id = self.canvas_texture(item.id, &mut stats, |cache| {
                                    cache.get_decoration(item.id, &drawn, width, height, scale)
                                })?;
                                stats.canvas_rasterizations +=
                                    (self.canvases.rasterizations() - before) as usize;
                                canvas_image = Some(id);
                                quads.push(Quad {
                                    rect: fade,
                                    uv: [0., 0., 1., 1.],
                                    color: [1., 1., 1., opacity],
                                    fade,
                                    options: [edge_fade, 2., 0., 0.],
                                    shape: [0.; 4],
                                    border: [0.; 4],
                                    mask: NO_MASK,
                                });
                            } else {
                                push_panel(
                                    &mut quads,
                                    plain_panel_quad(style, fade, opacity, edge_fade, later),
                                    self.scale,
                                    self.split_shading,
                                );
                            }
                        }
                    }
                    NodeKind::Svg(svg) => {
                        if svg.paint_bounds(bounds).width <= 0.
                            || svg.paint_bounds(bounds).height <= 0.
                        {
                            continue;
                        }
                        let before = self.svgs.rasterizations();
                        // An SVG that won't draw (bad markup, too large) shows
                        // nothing; it is the content's fault, not the frame's.
                        let image = match self.svgs.get(
                            item.id,
                            svg,
                            bounds.width,
                            bounds.height,
                            self.scale,
                        ) {
                            Ok(image) => image,
                            Err(error) => {
                                if self.failed_svgs.insert(svg.source_id()) {
                                    eprintln!("zgx: an SVG doesn't draw: {}", error.0);
                                }
                                continue;
                            }
                        };
                        stats.svg_rasterizations += (self.svgs.rasterizations() - before) as usize;
                        // Nodes share SVG rasters: `prune` releases their textures
                        // once no node shows them (a new source prunes).
                        let transform = image.paint_transform(bounds);
                        let transformed = image.transform() != zgx::affine::Affine::IDENTITY;
                        // Icon-sized rasters live in the glyph atlas and batch with
                        // text; larger ones keep a texture of their own.
                        let uv = if image.width() <= ATLAS_IMAGE_MAX
                            && image.height() <= ATLAS_IMAGE_MAX
                        {
                            let (x, y) = match self.image_cells.get(&image.id()) {
                                Some(&(x, y, _, _)) => (x, y),
                                None => {
                                    let (width, height) = (image.width(), image.height());
                                    let cell = self.atlas_cell(width, height, |rgba| {
                                        premultiply_into_cell(rgba, image.pixels(), width);
                                    })?;
                                    self.image_cells
                                        .insert(image.id(), (cell.0, cell.1, width, height));
                                    stats.image_uploads += 1;
                                    cell
                                }
                            };
                            let atlas = self.atlas_size as f32;
                            [
                                x as f32 / atlas,
                                y as f32 / atlas,
                                image.width() as f32 / atlas,
                                image.height() as f32 / atlas,
                            ]
                        } else {
                            if !self.ensure_image(&image, None, &mut stats)? {
                                continue;
                            }
                            canvas_image = Some(image.id());
                            [0., 0., 1., 1.]
                        };
                        quads.push(Quad {
                            rect: fade,
                            uv,
                            color: [1., 1., 1., item.effects.opacity],
                            fade,
                            options: [
                                item.effects.edge_fade,
                                2.,
                                if transformed { 1. } else { 0. },
                                0.,
                            ],
                            shape: if transformed {
                                [transform.a, transform.c, transform.b, transform.d]
                            } else {
                                [0.; 4]
                            },
                            border: if transformed {
                                [transform.tx, transform.ty, 0., 0.]
                            } else {
                                [0.; 4]
                            },
                            mask: NO_MASK,
                        });
                    }
                    #[cfg(target_os = "macos")]
                    NodeKind::NativeSurface(frame) => {
                        // (Never kept, so never patched: `changes_unmarked`.)
                        if MODE != PATCH && !self.images.contains_key(&frame.id()) {
                            let bytes = frame.width() as usize
                                * frame.height() as usize
                                * if frame.format() == zgx::native_surface::SurfaceFormat::Bgra {
                                    4
                                } else {
                                    6
                                };
                            if bytes + self.images.values().map(|e| e.1).sum::<usize>()
                                > 64 * 1024 * 1024
                            {
                                self.warn(
                                    "native surfaces past the 64 MiB budget of image textures \
                                     aren't drawn",
                                );
                                continue;
                            }
                            if self.native_surfaces.is_none() {
                                match native_surface::SurfaceCache::new(&self.device, &self.queue) {
                                    Ok(cache) => self.native_surfaces = Some(cache),
                                    Err(error) => {
                                        self.warn(&error.0);
                                        continue;
                                    }
                                }
                            }
                            let cache = self.native_surfaces.as_mut().unwrap();
                            if let Err(error) = cache.import(frame.clone()) {
                                self.warn(&error.0);
                                continue;
                            }
                            stats.native_surface_imports += 1;
                            stats.native_surface_conversions += usize::from(
                                frame.format() == zgx::native_surface::SurfaceFormat::Nv12FullRange,
                            );
                            let view = cache.texture(frame.id()).create_view(&Default::default());
                            let bytes = cache.bytes(frame.id());
                            let bind = self.texture_bind(&view, "native CoreVideo frame");
                            self.images.insert(frame.id(), (bind, bytes, None, (0, 0)));
                        }
                        self.drew_surface(item.id, frame.id());
                        canvas_image = Some(frame.id());
                        quads.push(Quad {
                            rect: fade,
                            uv: [0., 0., 1., 1.],
                            color: [1., 1., 1., item.effects.opacity],
                            fade,
                            options: [item.effects.edge_fade, 2., 0., 0.],
                            shape: [0.; 4],
                            border: [0.; 4],
                            mask: NO_MASK,
                        });
                    }
                    NodeKind::Image(image) => {
                        if image.paint_bounds(bounds).width <= 0.
                            || image.paint_bounds(bounds).height <= 0.
                        {
                            continue;
                        }
                        // A GPU view's frame on a plane of its own: nothing
                        // to draw here.
                        if image.gpu_frame().is_some() && self.plane_item(scene, &item, image)? {
                            stats.image_uploads += 1;
                            continue;
                        }
                        // (A view taking its layer drew itself already.) At the
                        // size it is drawn: its box, through its own transform,
                        // in this target's pixels (a layer's scale holds any
                        // zoom or transform around it).
                        let stretch = {
                            let m = image.transform();
                            (m.a.hypot(m.b)).max(m.c.hypot(m.d))
                        };
                        let drawn = (
                            bounds.width * self.scale * stretch,
                            bounds.height * self.scale * stretch,
                        );
                        if !self.ensure_shown_image(item.id, image, Some(drawn), &mut stats)? {
                            continue;
                        }
                        let transform = image.paint_transform(bounds);
                        let transformed = image.transform() != zgx::affine::Affine::IDENTITY;
                        // An effect: its own quad, drawn with its own pipeline.
                        if !matches!(image.effect(), zgx::image::ImageEffect::Plain) {
                            let reveal = scene.shader_uniforms(item.id).params;
                            let (opacity, edge) = (item.effects.opacity, item.effects.edge_fade);
                            quads.push(image_effect_quad(
                                image, transform, opacity, edge, fade, reveal,
                            ));
                            self.prepare_image_effects();
                            fill_shading = Some(Shading::ImageEffect);
                        } else if scene.has_image_slices()
                            && !transformed
                            && let Some(slice) = scene.image_slice(item.id)
                        {
                            // In nine pieces (a nine-patch): out of line.
                            let (opacity, edge) = (item.effects.opacity, item.effects.edge_fade);
                            push_image_slices(&mut quads, image, slice, fade, (opacity, edge));
                        } else {
                            // Images have no procedural border/shape. Reuse those
                            // attribute lanes rather than growing every glyph instance.
                            quads.push(Quad {
                                rect: fade,
                                uv: [0., 0., 1., 1.],
                                color: [1., 1., 1., item.effects.opacity],
                                fade,
                                options: [
                                    item.effects.edge_fade,
                                    2.,
                                    if transformed { 1. } else { 0. },
                                    0.,
                                ],
                                shape: if transformed {
                                    [transform.a, transform.c, transform.b, transform.d]
                                } else {
                                    [0.; 4]
                                },
                                border: if transformed {
                                    [transform.tx, transform.ty, 0., 0.]
                                } else {
                                    [0.; 4]
                                },
                                mask: NO_MASK,
                            });
                        }
                    }
                    kind @ (NodeKind::Text { .. } | NodeKind::RichText { .. }) => {
                        let rich = match kind {
                            NodeKind::RichText { text } => Some(text.clone()),
                            _ => None,
                        };
                        let rich_content = rich.as_ref().map(|r| r.text_arc());
                        let (text, color, font_size) = match kind {
                            NodeKind::Text {
                                text,
                                color,
                                font_size,
                            } => (text, *color, *font_size),
                            NodeKind::RichText { .. } => (
                                rich_content.as_ref().unwrap(),
                                zgx::scene::Color(255, 255, 255, 255),
                                16.,
                            ),
                            _ => unreachable!(),
                        };
                        if !font_size.is_finite() || font_size <= 0. {
                            self.warn("text with a font size not finite and positive isn't drawn");
                            continue;
                        }
                        // One lookup for both: a colour-only rich change (fades,
                        // highlights) keeps glyph positions, so the cached shape
                        // is recoloured instead of re-shaped; then whether it is
                        // stale.
                        // Its lines wrap at its box's width, or narrower if
                        // they are balanced.
                        let (wrap, inset) = scene.text_wrap(item.id, bounds.width);
                        let stale = self.shapes.get_mut(&item.id).is_none_or(|shaped| {
                            if let Some(new) = &rich
                                && let Some(old) = &shaped.rich
                                && !Arc::ptr_eq(old, new)
                                && old.same_shape(new)
                                && shaped.width == wrap
                                && shaped.height == bounds.height
                                && shaped.text_options == item.text_options
                                && shaped.font == *item.font
                            {
                                for decoration in &mut shaped.decorations {
                                    decoration.color = decoration.color_in(new);
                                }
                                shaped.rich = Some(new.clone());
                                shaped.atlas_epoch = self.atlas_epoch.wrapping_sub(1);
                            }
                            shaped.rich != rich
                                || shaped.text_options != item.text_options
                                || shaped.text != *text
                                || shaped.font != *item.font
                                || shaped.size != font_size
                                || shaped.width != wrap
                                || shaped.height != bounds.height
                                || shaped.scale != self.scale
                        });
                        // A shape made this frame, held here until placed: into
                        // the map once, not in and out again to place it.
                        let mut fresh = None;
                        if stale {
                            // The old shape's glyphs and quads hold the new one's:
                            // a changed value is as many allocations fewer.
                            let (mut spare_glyphs, mut spare_quads) = (Vec::new(), Vec::new());
                            if let Some(old) = self.shapes.remove(&item.id) {
                                self.shaped_bytes -= old.bytes();
                                (spare_glyphs, spare_quads) = (old.glyphs, old.quads);
                                spare_glyphs.clear();
                                spare_quads.clear();
                            }
                            let mut fonts = self.fonts.borrow_mut();
                            // Default-option rich text: the prepared lines layout
                            // measured with, laid out at this width, no reshaping.
                            let prepared = rich
                                .as_ref()
                                .filter(|rich| prepared::TextCache::handles(rich));
                            let plain = rich.is_none()
                                && item.text_options == Default::default()
                                && prepared::TextCache::handles_plain(text, font_size);
                            // Text the prepared cache doesn't lay out (with clamping
                            // or an ellipsis): another node's shape of the same
                            // inputs, copied rather than shaped again (a list moved
                            // by a row shows in each what the next showed).
                            let content = (!plain && prepared.is_none()).then(|| {
                                use std::hash::{Hash, Hasher};
                                let mut hasher = rustc_hash::FxHasher::default();
                                text.hash(&mut hasher);
                                font_size.to_bits().hash(&mut hasher);
                                wrap.to_bits().hash(&mut hasher);
                                bounds.height.to_bits().hash(&mut hasher);
                                hasher.finish()
                            });
                            // (A twin's quads come with whether they hold colour glyphs.)
                            let mut twin_colored = false;
                            let twin = content
                                .and_then(|content| self.shaped_like.get(&content))
                                .and_then(|id| self.shapes.get(id))
                                .filter(|twin| {
                                    twin.text == *text
                                        && twin.size == font_size
                                        && twin.width == wrap
                                        && twin.height == bounds.height
                                        && twin.scale == self.scale
                                        && twin.text_options == item.text_options
                                        && twin.font == *item.font
                                        && twin.rich == rich
                                })
                                .map(|twin| {
                                    spare_glyphs.extend_from_slice(&twin.glyphs);
                                    let placed = twin.atlas_epoch == self.atlas_epoch;
                                    if placed {
                                        spare_quads.extend_from_slice(&twin.quads);
                                        twin_colored = twin.colored;
                                    }
                                    (twin.decorations.clone(), placed.then_some(twin.atlas_epoch))
                                });
                            let b = if let Some((decorations, epoch)) = twin {
                                Err((std::mem::take(&mut spare_glyphs), decorations, None, epoch))
                            } else if plain {
                                let mut cache = self.text_cache.borrow_mut();
                                let laid = cache.layout_plain(
                                    &mut fonts,
                                    text,
                                    font_size,
                                    item.font,
                                    Some(wrap),
                                );
                                // Plain text has one run: no line starts to find it by.
                                let glyphs = glyphs_of(
                                    std::mem::take(&mut spare_glyphs),
                                    laid.runs(Some(bounds.height)),
                                    &[],
                                    None,
                                    self.scale,
                                );
                                let underlines =
                                    item.font.underline.map_or_else(Vec::new, |line| {
                                        text::plain_underlines(
                                            laid.runs(Some(bounds.height)),
                                            font_size,
                                            line,
                                        )
                                    });
                                Err((glyphs, underlines, Some(laid.key()), None))
                            } else if let Some(rich) = prepared {
                                let mut cache = self.text_cache.borrow_mut();
                                let laid = cache.layout(&mut fonts, rich, Some(wrap));
                                let starts = laid.line_starts();
                                let glyphs = glyphs_of(
                                    std::mem::take(&mut spare_glyphs),
                                    laid.runs(Some(bounds.height)),
                                    &starts,
                                    Some(rich),
                                    self.scale,
                                );
                                let decorations = text::rich_decorations_in(
                                    laid.runs(Some(bounds.height)),
                                    &starts,
                                    rich,
                                );
                                Err((glyphs, decorations, Some(laid.key()), None))
                            } else if let Some(rich) = &rich {
                                Ok(text::rich_buffer(
                                    &mut fonts,
                                    rich,
                                    Some(wrap),
                                    Some(bounds.height),
                                ))
                            } else if item.text_options != Default::default() {
                                Ok(text::display_buffer(
                                    &mut fonts,
                                    text,
                                    item.font,
                                    font_size,
                                    Some(wrap),
                                    Some(bounds.height),
                                    item.text_options,
                                ))
                            } else {
                                let mut b = Buffer::new_empty(Metrics::new(
                                    font_size.max(1.),
                                    item.font.line_height.resolve(font_size),
                                ));
                                b.set_size(text::wrap_width(Some(wrap)), Some(bounds.height));
                                text::set_buffer_text(
                                    &mut fonts,
                                    &mut b,
                                    text,
                                    item.font,
                                    font_size.max(1.),
                                );
                                b.shape_until_scroll(&mut fonts, false);
                                Ok(b)
                            };
                            let (glyphs, decorations, prepared, epoch) = match b {
                                Err(made) => made,
                                Ok(b) => (
                                    glyphs_of(
                                        std::mem::take(&mut spare_glyphs),
                                        b.layout_runs(),
                                        &text::line_starts(&b),
                                        rich.as_deref(),
                                        self.scale,
                                    ),
                                    match (&rich, item.font.underline) {
                                        (Some(rich), _) => text::rich_decorations(&b, rich),
                                        (None, Some(line)) => {
                                            text::plain_underlines(b.layout_runs(), font_size, line)
                                        }
                                        (None, None) => Vec::new(),
                                    },
                                    None,
                                    None,
                                ),
                            };
                            let shaped = Shaped {
                                decorations,
                                rich: rich.clone(),
                                text_options: item.text_options,
                                font: item.font.clone(),
                                text: text.clone(),
                                size: font_size,
                                width: wrap,
                                height: bounds.height,
                                scale: self.scale,
                                glyphs,
                                quads: spare_quads,
                                colored: twin_colored,
                                atlas_epoch: epoch.unwrap_or(0),
                                last_used: self.frame,
                                prepared,
                            };
                            fresh = Some(shaped);
                            stats.shaped_nodes += 1;
                            if let Some(content) = content {
                                if self.shaped_like.len() >= SHAPED_LIKE {
                                    self.shaped_like.clear();
                                }
                                insert_in::<MODE, _, _>(&mut self.shaped_like, content, item.id);
                            }
                        }
                        let snap = |value: f32| (value * self.scale).round() / self.scale;
                        let (x, y) = (snap(bounds.x), snap(bounds.y));
                        // Its glyphs' origin: right of its box's, its lines
                        // balanced and centred or right-aligned.
                        let glyph_x = snap(bounds.x + inset);
                        // Partly outside its clip (a long document scrolled in a
                        // view): its lines that reach the clip, as quads made this
                        // frame (`visible_glyphs`), rather than every glyph's.
                        // (Its box against its clip and the viewport, not their
                        // intersection with it: an edge worked out again can be
                        // a rounding off the box's.)
                        let holds = |outer: Rect| {
                            outer.x <= bounds.x
                                && outer.y <= bounds.y
                                && outer.x + outer.width >= bounds.x + bounds.width
                                && outer.y + outer.height >= bounds.y + bounds.height
                        };
                        // Find's marks, under the selection.
                        let marks = scene.text_marks(item.id);
                        if !marks.is_empty() {
                            push_marks(&mut quads, scene, &item, bounds, (x, y), self.scale);
                        }
                        // A text selection, under the glyphs.
                        let highlight = scene.text_highlight(item.id);
                        if let Some(highlight) = highlight {
                            if highlight.radius > 0. {
                                push_highlight(
                                    &mut quads,
                                    highlight,
                                    (x, y),
                                    self.scale,
                                    item.effects.opacity,
                                    item.effects.edge_fade,
                                );
                            } else {
                                quads.extend(highlight.rects.iter().map(|r| Quad {
                                    rect: [bounds.x + r.x, bounds.y + r.y, r.width, r.height],
                                    uv: [0.; 4],
                                    color: rgba(highlight.color, item.effects.opacity),
                                    fade,
                                    options: [item.effects.edge_fade, 0., 0., 0.],
                                    shape: [0.; 4],
                                    border: [0.; 4],
                                    mask: NO_MASK,
                                }));
                            }
                        }
                        // Its selected glyphs, in the selected-text colour.
                        let selected = highlight.filter(|h| h.text_color.is_some());
                        // On whole device pixels, as a browser draws text: an
                        // atlas bitmap drawn a fraction off is sampled between
                        // its texels, and a hinted one-pixel stroke (a minus
                        // sign) turns into two rows at half strength.
                        // Plain text's one colour, worked out once for its glyphs.
                        let plain_color = rgba(color, item.effects.opacity);
                        // Its quads from the shape's, placed and coloured.
                        // (One extend, written straight into the room it makes: a
                        // push a glyph would check that room each time.)
                        // Its shadows, under it: the last lowest (CSS's list),
                        // each its glyphs again, moved on whole device pixels.
                        let shadows = scene.text_shadows(item.id);
                        let scale = self.scale;
                        // Blurred: all its shadows placed after, under its glyphs
                        // (`place_blurred_shadows`), from where they start.
                        let blurred = shadows.iter().any(|shadow| shadow.blur > 0.);
                        // Where its shadows' quads went, to be drawn apart.
                        let shadows_at = std::cell::Cell::new((usize::MAX, 0));
                        // Rounded span backgrounds (inline code chips): each
                        // piece rounds on its own box (`round_own_boxes`).
                        let rounded_spans = rich
                            .as_ref()
                            .is_some_and(|rich| rich.runs().iter().any(|run| run.radius > 0.));
                        // Holding colour glyphs (`COLOR_GLYPH`): its shadows are
                        // their alphas, as blurred ones are (`place_blurred_shadows`).
                        let colored_at = std::cell::Cell::new(false);
                        // Filled with a gradient, as its layer's root: its glyphs
                        // a mask, its selected ones again above (`gradient_text_quads`).
                        let filled = match scene.has_text_gradients()
                            && self.layer_root == Some(item.id)
                            && item.effects.blur_radius <= 0.
                        {
                            true => self.gradient_text_image(scene, item.id, &mut stats)?,
                            false => None,
                        };
                        let glyphs_from = std::cell::Cell::new(0);
                        let selected_glyphs = std::cell::RefCell::new(Vec::new());
                        let place = |quads: &mut Vec<Quad>, source: &[TextQuad], colored: bool| {
                            let from = quads.len();
                            colored_at.set(colored);
                            if !shadows.is_empty() {
                                shadows_at.set((quads.len(), quads.len()));
                            }
                            for shadow in shadows.iter().rev().filter(|_| !blurred && !colored) {
                                let shadow_color = rgba(shadow.color, item.effects.opacity);
                                place_text_quads(
                                    quads,
                                    source,
                                    x + (shadow.offset.x * scale).round() / scale,
                                    y + (shadow.offset.y * scale).round() / scale,
                                    fade,
                                    item.effects.edge_fade,
                                    |_| shadow_color,
                                );
                            }
                            if !shadows.is_empty() {
                                shadows_at.set((shadows_at.get().0, quads.len()));
                            }
                            glyphs_from.set(quads.len());
                            if filled.is_some() {
                                let mask = [1., 1., 1., item.effects.opacity];
                                let edge_fade = item.effects.edge_fade;
                                place_text_quads(
                                    quads,
                                    source,
                                    glyph_x,
                                    y,
                                    fade,
                                    edge_fade,
                                    |_| mask,
                                );
                                if let Some(highlight) = selected {
                                    let mut chosen = selected_glyphs.borrow_mut();
                                    place_text_quads(
                                        &mut chosen,
                                        source,
                                        glyph_x,
                                        y,
                                        fade,
                                        edge_fade,
                                        |cached| {
                                            (cached.paint[1] > 0.5)
                                                .then(|| {
                                                    highlight.glyph_color(
                                                        inset
                                                            + cached.rect[0]
                                                            + cached.rect[2] * 0.5,
                                                        cached.rect[1] + cached.rect[3] * 0.5,
                                                    )
                                                })
                                                .flatten()
                                                .map_or([0.; 4], |color| {
                                                    rgba(color, item.effects.opacity)
                                                })
                                        },
                                    );
                                    chosen.retain(|quad| quad.color[3] > 0.);
                                }
                            // Plain and unselected: one colour, chosen once.
                            } else if selected.is_none() && rich.is_none() && !colored {
                                place_text_quads(
                                    quads,
                                    source,
                                    glyph_x,
                                    y,
                                    fade,
                                    item.effects.edge_fade,
                                    |_| plain_color,
                                );
                            } else {
                                place_text_quads(
                                    quads,
                                    source,
                                    glyph_x,
                                    y,
                                    fade,
                                    item.effects.edge_fade,
                                    |cached| {
                                        // A colour glyph keeps its colours, selected
                                        // or not, with its text's alpha.
                                        if cached.paint[2] == COLOR_GLYPH {
                                            let alpha = match rich.is_some() {
                                                true => cached.color[3] * item.effects.opacity,
                                                false => plain_color[3],
                                            };
                                            return [1., 1., 1., alpha];
                                        }
                                        let recolor = selected
                                            .filter(|_| cached.paint[1] > 0.5)
                                            .and_then(|h| {
                                                h.glyph_color(
                                                    inset + cached.rect[0] + cached.rect[2] * 0.5,
                                                    cached.rect[1] + cached.rect[3] * 0.5,
                                                )
                                            });
                                        if let Some(color) = recolor {
                                            rgba(color, item.effects.opacity)
                                        } else if rich.is_some() {
                                            [
                                                cached.color[0],
                                                cached.color[1],
                                                cached.color[2],
                                                cached.color[3] * item.effects.opacity,
                                            ]
                                        } else {
                                            plain_color
                                        }
                                    },
                                );
                            }
                            if rounded_spans {
                                round_own_boxes(&mut quads[from..]);
                            }
                        };
                        // Its shape as it was, inside its clip and with the atlas it
                        // was placed in: its quads, borrowed where they are kept.
                        let whole =
                            !self.cull_glyphs || (holds(viewport) && item.clip.is_none_or(holds));
                        if whole
                            && let Some(mut shaped) =
                                fresh.take_if(|shaped| shaped.atlas_epoch == self.atlas_epoch)
                        {
                            // (A twin's, placed in this atlas already.)
                            place(&mut quads, &shaped.quads, shaped.colored);
                            glyphs_alone = marks.is_empty()
                                && highlight.is_none()
                                && shaped.decorations.is_empty();
                            shaped.last_used = self.frame;
                            self.shaped_bytes += shaped.bytes();
                            self.shapes.insert(item.id, shaped);
                        } else if whole
                            && fresh.is_none()
                            && let Some(shaped) = self.shapes.get_mut(&item.id)
                            && shaped.atlas_epoch == self.atlas_epoch
                        {
                            place(&mut quads, &shaped.quads, shaped.colored);
                            glyphs_alone = marks.is_empty()
                                && highlight.is_none()
                                && shaped.decorations.is_empty();
                            shaped.last_used = self.frame;
                        } else {
                            let mut shaped = match fresh {
                                Some(shaped) => shaped,
                                None => {
                                    let shaped = self.shapes.remove(&item.id).unwrap();
                                    self.shaped_bytes -= shaped.bytes();
                                    shaped
                                }
                            };
                            let partial = self.cull_glyphs
                                && !(holds(viewport) && item.clip.is_none_or(holds))
                                && self.visible_lines(
                                    &shaped,
                                    clip.y - y,
                                    clip.height,
                                    font_size,
                                    &mut stats,
                                )?;
                            if !partial && shaped.atlas_epoch != self.atlas_epoch {
                                shaped.quads.clear();
                                shaped.quads.extend(
                                    shaped
                                        .decorations
                                        .iter()
                                        .filter(|d| d.background)
                                        .map(rich_decoration_quad),
                                );
                                shaped.quads.reserve(shaped.glyphs.len());
                                shaped.colored = false;
                                for glyph in &shaped.glyphs {
                                    if let Some(a) = self.glyph::<MODE>(glyph.key, &mut stats)? {
                                        shaped.colored |= a.color;
                                        shaped.quads.push(TextQuad {
                                            rect: [
                                                (glyph.x + a.left as f32) / self.scale,
                                                (glyph.y - a.top as f32) / self.scale,
                                                a.width as f32 / self.scale,
                                                a.height as f32 / self.scale,
                                            ],
                                            uv: [
                                                a.x as f32 / self.atlas_size as f32,
                                                a.y as f32 / self.atlas_size as f32,
                                                a.width as f32 / self.atlas_size as f32,
                                                a.height as f32 / self.atlas_size as f32,
                                            ],
                                            color: shaped.rich.as_ref().map_or([0.; 4], |rich| {
                                                rgba(rich.runs()[glyph.run].color, 1.)
                                            }),
                                            paint: [
                                                0.,
                                                glyph_texture(a.color, self.scale),
                                                if a.color { COLOR_GLYPH } else { 0. },
                                                0.,
                                            ],
                                        });
                                    }
                                }
                                for decoration in
                                    shaped.decorations.iter().filter(|d| !d.background)
                                {
                                    self.line_quads(
                                        decoration,
                                        shaped.rich.as_deref(),
                                        &mut shaped.quads,
                                        &mut stats,
                                    )?;
                                }
                                shaped.atlas_epoch = self.atlas_epoch;
                                stats.geometry_rebuilds += 1;
                            }
                            let visible = std::mem::take(&mut self.visible_glyphs);
                            let colored = match partial {
                                true => self.visible_colored,
                                false => shaped.colored,
                            };
                            place(
                                &mut quads,
                                if partial { &visible } else { &shaped.quads },
                                colored,
                            );
                            glyphs_alone = marks.is_empty()
                                && highlight.is_none()
                                && shaped.decorations.is_empty();
                            self.visible_glyphs = visible;
                            shaped.last_used = self.frame;
                            let bytes = shaped.bytes();
                            if bytes <= SHAPE_BUDGET {
                                let mut used = self.shaped_bytes;
                                while self.shapes.len() >= 1024 || used + bytes > SHAPE_BUDGET {
                                    let Some(id) = self
                                        .shapes
                                        .iter()
                                        .min_by_key(|(_, s)| s.last_used)
                                        .map(|(id, _)| *id)
                                    else {
                                        break;
                                    };
                                    used -= self.shapes.remove(&id).unwrap().bytes();
                                }
                                self.shaped_bytes = used + bytes;
                                self.shapes.insert(item.id, shaped);
                            }
                        }
                        let (at, mut end) = shadows_at.get();
                        if at != usize::MAX {
                            if blurred || colored_at.get() {
                                end = at
                                    + self.place_blurred_shadows(
                                        &mut quads,
                                        at,
                                        (item.id, stale),
                                        shadows,
                                        (x, y, fade),
                                        (item.effects.edge_fade, item.effects.opacity),
                                        &mut stats,
                                    )?;
                            }
                            shadow_quads = (at as u32, end as u32);
                        }
                        if let Some(image) = filled {
                            // (Blurred shadows went in where its glyphs started.)
                            let glyphs = match shadow_quads.1 > shadow_quads.0 {
                                true => shadow_quads.1 as usize,
                                false => glyphs_from.get(),
                            };
                            gradient_text = Some((image, glyphs, selected_glyphs.into_inner()));
                            glyphs_alone = false;
                        }
                    }
                    NodeKind::Container(_) => {}
                }
                // A canvas drawn as quads: clipped to its box.
                let clip = match canvas_clip.map(|canvas: Rect| clip.intersection(canvas)) {
                    None => clip,
                    Some(Some(inside)) => inside,
                    Some(None) => {
                        quads.truncate(start as usize);
                        continue;
                    }
                };
                // Inside rounded clips or a `fade_edges` container: cut and faded
                // in the shader (its quads' lane is set once they're culled).
                let region = item_region(&mut clips, &item);
                let in_region = region > 0.;
                // A shadowed text: its shadows drawn first under the clip that
                // takes them in, then the rest under its own.
                let (start, clip) = match own_clip {
                    Some(own)
                        if shadow_quads.1 > shadow_quads.0
                            && item.effects.blur_radius <= 0.
                            && gradient_text.is_none() =>
                    {
                        let glyphs = match item.effects.edge_fade != 0. || in_region {
                            true => Shading::Full,
                            false => Shading::Basic,
                        };
                        let start = self.draw_text_shadows(
                            (&mut quads, &mut draws),
                            shadow_quads,
                            (start, own, clip, viewport),
                            (&clips, region, glyphs),
                        );
                        (start, own.intersection(viewport).unwrap_or_default())
                    }
                    Some(own) => (start, own.intersection(viewport).unwrap_or_default()),
                    None => (start, clip),
                };
                // When the clip only culls whole quads, apply it on the CPU and
                // draw under the viewport, so a change of clip never splits a
                // batch: neighbouring text, icons and rows become one draw. Quads
                // it would cut, transformed quads and filters keep the scissor.
                // (The kept quads' bounds, when the clip culls on the CPU.)
                // A gradient text's runs, in their order (under the scissor).
                let filled_runs = gradient_text.take().map(|(image, glyphs, selected)| {
                    let gradient = Quad {
                        rect: fade,
                        uv: [0., 0., 1., 1.],
                        color: [1.; 4],
                        fade,
                        options: [item.effects.edge_fade, 2., 0., 0.],
                        shape: [0.; 4],
                        border: [0.; 4],
                        mask: NO_MASK,
                    };
                    let runs = gradient_text_quads(&mut quads, (start, glyphs), gradient, selected);
                    (image, runs)
                });
                let (draw_clip, culled_bounds) = match item.effects.blur_radius <= 0. {
                    true if filled_runs.is_some() => (clip, None),
                    true if glyphs_alone => {
                        cull_glyphs(&mut quads, start as usize, clip, viewport, self.scale)
                    }
                    true => cull_on_cpu(&mut quads, start as usize, clip, viewport, self.scale),
                    false => (clip, None),
                };
                if quads.len() as u32 > start || item.effects.blur_radius > 0. {
                    let drawn =
                        culled_bounds.unwrap_or_else(|| quad_bounds(&quads[start as usize..]));
                    if region > 0. {
                        // Unless the item lies wholly where its region changes
                        // nothing (a row well inside a rounded card): then drawn
                        // without it, as the plain quads it is, its region's
                        // index in `mask.w` for a box moved alone
                        // (`patch_box_moved`). (Not in perspective, which
                        // `quad_rect` bounds only in part, nor blurred.)
                        let blurred = item.effects.blur_radius > 0.;
                        let item_quads = &mut quads[start as usize..];
                        let interiors = &mut self.region_interiors;
                        mark_region(
                            interiors, &clips, region, item_quads, drawn, blurred, self.scale,
                        );
                    }
                    let draw = Draw {
                        shading: fill_shading.unwrap_or(Shading::Full),
                        blur: (item.effects.blur_radius > 0.).then(|| {
                            Box::new((
                                bounds,
                                item.effects,
                                blur_corners(item.kind, item.bounds),
                                item.rounded(),
                            ))
                        }),
                        start,
                        end: quads.len() as u32,
                        clip: draw_clip,
                        bounds: drawn,
                        source: if let Some(image) = canvas_image {
                            self.image_source(scene, item.id, image)
                        } else if let NodeKind::Image(image) = item.kind {
                            self.image_source(scene, item.id, image.id())
                        } else {
                            TextureSource::Atlas
                        },
                    };
                    if let Some((image, runs)) = filled_runs {
                        push_gradient_text(&mut draws, &quads, &draw, runs, image);
                        continue;
                    }
                    let split = self.split_shading;
                    // A glyph's quad (`glyph_quad`) is plain unless faded or in a
                    // clip region, and textured: one shading for all of them.
                    let glyphs =
                        glyphs_alone.then_some(match item.effects.edge_fade != 0. || in_region {
                            true => Shading::Full,
                            false => Shading::Basic,
                        });
                    push_draw(
                        &mut draws, &quads, draw, split, polylines, self.scale, glyphs,
                    );
                }
            }
            if phase == bands.len() {
                break;
            }
            ends.push(draws.len());
            phase += 1;
        }
        // The bands' draws apart from the frame's.
        if let Some(&last) = ends.last() {
            let frame = draws.split_off(last);
            let mut banded = std::mem::replace(&mut draws, frame);
            for index in (0..bands.len()).rev() {
                let start = if index == 0 { 0 } else { ends[index - 1] };
                bands[index].draws = banded.split_off(start);
            }
        }
        if MODE == PATCH {
            close_range(
                &mut quads,
                &draws,
                &mut ranges,
                record && self.changes(text),
            );
            self.item_ranges = ranges;
            self.frame_quads = quads;
            self.frame_draws = draws;
            self.frame_points = points;
            self.patch_clips = clips;
            return Ok(stats);
        }
        // Effect passes' quads ride in the frame's vertex buffer.
        let effect_first = quads.len() as u32;
        quads.extend(Self::effect_quads(&effect_jobs));
        // The surface this frame draws into, when presenting through the
        // layer: shown surfaces stay as they are.
        #[cfg(own_surfaces)]
        let shown = if self.in_layer {
            None
        } else {
            self.begin_layer_frame(self.fresh || whole)?;
            self.layer_present
                .as_ref()
                .and_then(|present| present.shown)
        };
        #[cfg(not(own_surfaces))]
        let shown: Option<usize> = None;
        let regions: Vec<(Rect, (u32, u32, u32, u32))> = damage
            .iter()
            .filter_map(|region| {
                scissor(*region, self.scale, self.width, self.height).map(|s| (*region, s))
            })
            .collect();
        for (_, (_, _, w, h)) in &regions {
            stats.damaged_pixels += u64::from(*w) * u64::from(*h);
        }
        if !self.in_layer {
            stats.regions = regions.len();
            if let Some(kept) = &mut self.debug_regions {
                kept.clear();
                kept.extend(regions.iter().map(|(region, _)| *region));
            }
        }
        // What this frame changes in the retained target, for surfaces that
        // must catch up later (`begin_layer_frame`).
        #[cfg(own_surfaces)]
        if !self.in_layer
            && let Some(present) = &mut self.layer_present
        {
            present.changed.extend(regions.iter().map(|(_, s)| *s));
            present.changed.extend(copies.iter().map(|(s, _)| *s));
        }
        // And for the swap chain's images, which take it at the present
        // (a window's alone: offscreen, nothing presents).
        #[cfg(target_os = "linux")]
        if self.surface.is_some() {
            self.swap_changes(&regions, &copies);
        }
        // What the surface lacks: changes since it was last shown that this
        // frame doesn't repaint, and scrolled content; both drawn from the
        // shown surface at the start of the frame's first pass, as instances
        // after the frame's quads.
        #[cfg(own_surfaces)]
        let mut catch_up = shown.and_then(|shown| {
            let (bind, draws) = self.catch_up_draws(shown, &copies, &regions)?;
            stats.scroll_copies += copies.len();
            let start = quads.len() as u32;
            quads.extend(draws.into_iter().map(|((x, y, w, h), (dx, dy))| Quad {
                rect: [x as f32, y as f32, w as f32, h as f32],
                uv: [dx as f32, dy as f32, 0., 0.],
                ..bytemuck::Zeroable::zeroed()
            }));
            Some((bind, start..quads.len() as u32))
        });
        let needed = quads.len() * std::mem::size_of::<Quad>();
        if needed > VERTEX_BUDGET {
            return Err(GpuError("frame geometry exceeds 32 MiB budget".into()));
        }
        // A clean filter elsewhere in the retained target needs no replay and
        // must not force every unrelated quad into its own render pass.
        let has_damaged_blur = has_blur
            && draws.iter().any(|draw| {
                draw.blur.as_deref().is_some_and(|&(bounds, _, _, _)| {
                    damage.iter().any(|region| {
                        region
                            .intersection(draw.clip)
                            .and_then(|rect| rect.intersection(bounds))
                            .and_then(|output| scissor(output, self.scale, self.width, self.height))
                            .is_some()
                    })
                })
            });
        // The part of a filter's output inside `region`, if it reaches it.
        let filtered = |draw: &Draw, region: &Rect| {
            let &(bounds, _, _, _) = draw.blur.as_deref()?;
            region.intersection(draw.clip)?.intersection(bounds)
        };
        // A filter samples everything painted before it, so the filters a
        // damaged region reaches end passes. A filter joins the batch before
        // it when nothing drawn since that batch's first filter reaches its
        // area (its backdrop is complete there already): the batch blurs
        // together and draws back at its start, in one split. It may also
        // sit on another filter of the batch whose blur alone reaches it, at
        // most half as wide there (a frosted pill on a softly blurred edge):
        // it then blurs the backdrop under that blur, which differs from
        // blurring the blur by under an eighth of its radius. Between
        // splits, every region shares one pass: a scissor per region, draws
        // batched within it.
        let mut batches: Vec<(usize, Vec<usize>)> = Vec::new();
        if has_damaged_blur {
            let margin = 1. / self.scale;
            // What a draw paints: its quads and, for a filter, its blur.
            let reaches = |draw: &Draw, area: Rect| {
                let blur = draw
                    .blur
                    .as_deref()
                    .and_then(|(bounds, _, _, _)| bounds.intersection(draw.clip));
                [draw.bounds.intersection(draw.clip), blur]
                    .into_iter()
                    .flatten()
                    .any(|painted| painted.intersection(area).is_some())
            };
            for i in 0..draws.len() {
                let draw = &draws[i];
                if !regions
                    .iter()
                    .any(|(region, _)| filtered(draw, region).is_some())
                {
                    continue;
                }
                let Some(&(bounds, effects, _, _)) = draw.blur.as_deref() else {
                    continue;
                };
                let area = bounds.intersection(draw.clip);
                let joins = area
                    .zip(batches.last())
                    .is_some_and(|(area, (start, filters))| {
                        let area = area.expand(margin);
                        (*start..i).all(|k| {
                            let before = &draws[k];
                            let quads = before.bounds.intersection(before.clip);
                            !reaches(before, area)
                                || filters.contains(&k)
                                    && quads.is_none_or(|quads| quads.intersection(area).is_none())
                                    && before.blur.as_deref().is_some_and(
                                        |&(under, beneath, _, _)| {
                                            2. * peak_blur(under, &beneath, area)
                                                <= effects.blur_radius
                                        },
                                    )
                        })
                    });
                match batches.last_mut() {
                    Some((_, filters)) if joins => filters.push(i),
                    _ => batches.push((i, vec![i])),
                }
            }
        }
        // Polyline strokes read their points (group 2, or Metal's `POINTS`).
        // Every stroke appends at least two points before making its quad.
        let polylines = !points.is_empty()
            && draws
                .iter()
                .chain(bands.iter().flat_map(|band| &band.draws))
                .any(|draw| draw.shading == Shading::Polyline);
        // `ZGX_D3D11`: every frame is drawn with Direct3D 11 (d3d11_frame.rs),
        // from the same data `render_direct` takes.
        #[cfg(d3d11)]
        if self.d3d11.is_some() {
            let mut draws = draws;
            self.d3d11_app_shaders(&mut draws);
            self.d3d11_leave_out(&mut draws);
            for band in &mut bands {
                self.d3d11_app_shaders(&mut band.draws);
                self.d3d11_leave_out(&mut band.draws);
            }
            let lacks = (bands.iter().map(|band| band.draws.as_slice()))
                .chain(std::iter::once(draws.as_slice()))
                .find_map(|draws| self.d3d11_lacks(draws));
            if let Some(lacks) = lacks {
                let message = format!(
                    "ZGX_D3D11 draws no {lacks} yet, and leaving it out could mislead \
                     (docs/windows.md, Direct3D 11)"
                );
                eprintln!("zgx: {message}");
                if cfg!(debug_assertions) {
                    panic!("{message}");
                }
                return Err(GpuError(message));
            }
            let catch_up = catch_up.map(|(_, instances)| instances);
            let stats = self.render_d3d11(
                quads,
                draws,
                &regions,
                &copies,
                catch_up,
                shown,
                &clips,
                &points,
                (&effect_jobs, effect_first),
                &reads,
                &batches,
                &bands,
                stats,
            )?;
            self.frame_points = points;
            return Ok(stats);
        }
        // Frames of quads (images the CPU wrote, layer composites, nothing
        // else recorded, any filters mixed by dual-source blending) skip
        // wgpu: see `metal_frame`. Layers repainted this frame were submitted
        // to the queue by `flush_pending` above, so they land first: a frame
        // animating only layer transforms is a single direct pass.
        #[cfg(target_os = "macos")]
        if !self.in_layer
            && self.mapped.is_some()
            && self.texture_uploads.is_empty()
            && self.pending.get_mut().is_none()
            && (shown.is_some() || copies.is_empty())
            && (self.layer_present.is_some() || self.surface.is_none())
            && self.single_pass_format.is_none()
            && draws
                .iter()
                .chain(bands.iter().flat_map(|band| &band.draws))
                .all(|draw| match draw.source {
                TextureSource::Atlas => true,
                TextureSource::Layer(id) => self.layers.contains_key(&id),
                TextureSource::Image(id) | TextureSource::Pixelated(id) => {
                    self.images.get(&id).is_some_and(|image| image.2.is_some())
                }
            })
            && (batches.is_empty()
                || self
                    .device
                    .features()
                    .contains(wgpu::Features::DUAL_SOURCE_BLENDING))
            // Shader fills and polylines compiled for Metal too.
            && (!polylines
                || self
                    .metal_frame
                    .get_or_init(|| metal_frame::MetalFrame::new(&self.device, &self.queue).ok())
                    .as_ref()
                    .is_some_and(|frame| frame.polylines()))
            // Special draws are emitted only after preparing their wgpu
            // pipeline. These caches live for the renderer's lifetime, so
            // empty caches prove the shading scan would accept every draw.
            && ((self.fill_pipelines.is_empty()
                && self.filter_pipelines.is_empty()
                && self.blend_pipelines.is_empty()
                && self.image_effect_pipeline.get().is_none())
                || draws
                    .iter()
                    .chain(bands.iter().flat_map(|band| &band.draws))
                    .all(|draw| match draw.shading {
                Shading::Fill(id) => self.fill_shaders.get(&id).is_some_and(|shader| {
                    self.metal_frame
                        .get_or_init(|| metal_frame::MetalFrame::new(&self.device, &self.queue).ok())
                        .as_ref()
                        .is_some_and(|frame| {
                            frame.fills(shader, self.offscreen_waits && !self.presents())
                        })
                }),
                Shading::ImageEffect => self
                    .metal_frame
                    .get_or_init(|| metal_frame::MetalFrame::new(&self.device, &self.queue).ok())
                    .as_ref()
                    .is_some_and(|frame| frame.image_effects()),
                // Layer shaders and blend modes take the wgpu path, as layers'
                // own frames do (gradient text's, `!self.in_layer` above).
                Shading::Filter(_)
                | Shading::Blend
                | Shading::FilterBlend(_)
                | Shading::SourceIn
                | Shading::Behind => false,
                _ => true,
            }))
            && effect_jobs.iter().all(|job| {
                job.mask
                    .is_none_or(|id| self.images.get(&id).is_some_and(|image| image.2.is_some()))
            })
            && self
                .metal_frame
                .get_or_init(|| metal_frame::MetalFrame::new(&self.device, &self.queue).ok())
                .as_ref()
                .is_some_and(|frame| {
                    (batches.is_empty() || frame.blurs())
                        && (effect_jobs.is_empty() || frame.effects())
                })
        {
            let catch_up = catch_up.map(|(_, instances)| instances);
            let effects = (&effect_jobs[..], effect_first);
            let strokes = polylines.then_some(&points[..]);
            let stats = self.render_direct(
                quads, draws, &regions, &batches, catch_up, shown, effects, &clips, strokes,
                &bands, stats,
            );
            self.frame_points = points;
            return Ok(stats);
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        // Glyphs rasterized while building this frame land before any pass
        // samples them. Counted as glyph uploads, not scene render passes.
        self.texture_uploads.encode(&self.device, &mut encoder);
        // Scrolled with surfaces of its own, the copies are drawn below.
        if shown.is_none() && !copies.is_empty() {
            let spare = self.spare_texture();
            for ((x, y, w, h), (sx, sy)) in &copies {
                let extent = wgpu::Extent3d {
                    width: *w,
                    height: *h,
                    depth_or_array_layers: 1,
                };
                let (from_x, from_y) = ((*x as i32 + sx) as u32, (*y as i32 + sy) as u32);
                encoder.copy_texture_to_texture(
                    texel_at(&self.target, from_x, from_y),
                    texel_at(&spare, *x, *y),
                    extent,
                );
                encoder.copy_texture_to_texture(
                    texel_at(&spare, *x, *y),
                    texel_at(&self.target, *x, *y),
                    extent,
                );
                stats.scroll_copies += 1;
            }
        }
        // Every batch of filters planned now: their records ride in the
        // frame's vertex buffer after the quads, and the atlas fits them all.
        let plans: Vec<Option<BlurPlan>> = batches
            .iter()
            .map(|(_, members)| self.plan_blur(&batch_filters(&draws, &regions, members)))
            .collect();
        let bytes: &[u8] = bytemuck::cast_slice(&quads);
        let align = self.device.limits().min_storage_buffer_offset_alignment as usize;
        // Each part at an aligned offset in the buffer.
        let mut records = Vec::new();
        let mut record = |part: &[u8]| {
            let at = (bytes.len() + records.len()).next_multiple_of(align);
            records.resize(at - bytes.len(), 0);
            records.extend_from_slice(part);
            (at as u64, part.len() as u64)
        };
        // The clip regions too, bound with the viewport for this frame.
        let region_range = (!clips.is_empty()).then(|| record(bytemuck::cast_slice(&clips)));
        // The polyline strokes' points too, bound as group 2 where drawn.
        let points_range = polylines.then(|| record(bytemuck::cast_slice(&points)));
        let ranges: Vec<Option<[(u64, u64); 2]>> = plans
            .iter()
            .map(|plan| {
                let plan = plan.as_ref()?;
                let slots: Vec<SlotRecord> = plan.slots.iter().map(|slot| slot.0).collect();
                Some([
                    record(bytemuck::cast_slice(&plan.blurs)),
                    record(bytemuck::cast_slice(&slots)),
                ])
            })
            .collect();
        let needed = bytes.len() + records.len();
        let vertices = if let Some(mapped) = &mut self.mapped {
            let mapped = mapped.get_mut();
            let parts = [bytes, &records];
            let (vertices, allocated) = match &self.kept_upload {
                Some((kept, kept_bytes)) => {
                    mapped.write_kept(&parts, kept, *kept_bytes, size_of::<Quad>())
                }
                None => mapped.write_parts(&parts),
            };
            stats.vertex_buffer_allocations += usize::from(allocated);
            self.vertex_capacity = mapped.bytes();
            vertices
        } else {
            if needed > self.vertex_capacity {
                self.vertex_capacity = needed.next_power_of_two();
                self.vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("retained frame vertices"),
                    size: self.vertex_capacity as u64,
                    usage: wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                });
                stats.vertex_buffer_allocations += 1;
            }
            let vertices = self.vertices.clone();
            let mut view = self.belt.get_mut().write_buffer(
                &mut encoder,
                &vertices,
                0,
                wgpu::BufferSize::new(needed as u64).expect("background quad"),
            );
            view.slice(..bytes.len()).copy_from_slice(bytes);
            view.slice(bytes.len()..).copy_from_slice(&records);
            drop(view);
            self.belt.get_mut().finish();
            vertices
        };
        if let Some((offset, size)) = region_range {
            let uniform = self
                .viewports
                .get(&(
                    viewport.width.to_bits(),
                    viewport.height.to_bits(),
                    self.scale.to_bits(),
                ))
                .map(|(uniform, _)| uniform.clone())
                .expect("selected above");
            let regions = wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &vertices,
                offset,
                size: wgpu::BufferSize::new(size),
            });
            self.viewport_bind = Some(self.viewport_group(&uniform, regions));
        }
        let points_bind = points_range.map(|(offset, size)| {
            let polylines = self.context.inner.quads.polylines();
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("polyline points"),
                layout: &polylines.points_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &vertices,
                        offset,
                        size: wgpu::BufferSize::new(size),
                    }),
                }],
            })
        });
        self.encode_effects(
            &mut encoder,
            &vertices,
            &effect_jobs,
            effect_first,
            &mut stats,
        );
        for band in &bands {
            self.encode_band(
                &mut encoder,
                &vertices,
                region_range,
                points_bind.as_ref(),
                band,
                &mut stats,
            );
        }
        // The pass splits at each batch of backdrop filters (blurred between
        // passes) and at each blended layer a region reaches (the target
        // under it copied between passes, for it to read): `Some(batch)` or
        // `None`, by draw.
        let mut cuts: Vec<(usize, Option<usize>)> = batches
            .iter()
            .enumerate()
            .map(|(batch, (start, _))| (*start, Some(batch)))
            // A blended draw is made only once `prepare_blend` made its
            // pipeline: with none made, no draw splits the pass.
            .chain(
                (0..if self.blend_pipelines.is_empty() {
                    0
                } else {
                    draws.len()
                })
                    .filter_map(|i| {
                        let draw = &draws[i];
                        (matches!(draw.shading, Shading::Blend | Shading::FilterBlend(_))
                            && regions
                                .iter()
                                .any(|(region, _)| region.intersects(draw.bounds)))
                        .then_some((i, None))
                    }),
            )
            .collect();
        // Backdrop-reading layers that needn't see each other share a copy
        // and a pass: those later in the frame with nothing drawn since the
        // first that touches what they read or paint.
        let sharing = share_backdrops(&draws, &reads);
        if !sharing.is_empty() {
            cuts.retain(|(draw, batch)| {
                batch.is_some() || !sharing.iter().any(|(_, shared)| shared.contains(draw))
            });
        }
        // A filter's blur before a blend at the same draw.
        cuts.sort_by_key(|&(draw, batch)| (draw, batch.is_none()));
        let splits: Vec<usize> = cuts.iter().map(|(draw, _)| *draw).collect();
        // The window's own frame can be drawn at presentation in the same pass
        // as the drawable (`present_single_pass`). With damaged filters, only
        // the draws after the last one wait: everything before it must land
        // in the target for the filter to sample.
        // Under the topmost opaque quad covering a whole region, nothing shows:
        // skip the clear and every quad below it. Filters sample what lies
        // beneath them, so frames that replay one draw everything.
        let first = if splits.is_empty() {
            occluded_from(&quads, &draws, &regions, self.scale)
        } else {
            vec![0; regions.len()]
        };
        // Shader fills and image effects have no pipelines writing two
        // targets at once.
        let defer = !self.in_layer
            && self.single_pass_format.is_some()
            && !draws.iter().any(|draw| {
                matches!(
                    draw.shading,
                    Shading::Fill(_)
                        | Shading::Filter(_)
                        | Shading::Blend
                        | Shading::FilterBlend(_)
                        | Shading::ImageEffect
                )
            })
            && (!self.present_unpremultiplies || self.background.3 == 255)
            && !self.copy_present
            && copies.is_empty();
        if !defer && !self.in_layer {
            // Drawn into the target now: no streak of drawable-only frames.
            self.whole_frames = 0;
        }
        let mut from = 0;
        // Each batch of filters is drawn back at the start of the pass after
        // its blur passes; each blended layer draws first in the pass after
        // its backdrop's copy (the draw, and the copy's bind group).
        let mut vertical = None;
        let mut backdrop: Option<(usize, wgpu::BindGroup)> = None;
        for (segment, end) in splits.iter().copied().chain([draws.len()]).enumerate() {
            if defer && end == draws.len() {
                stats.render_passes += 1;
                let blur = vertical.take();
                // (Not when the next frame could copy: a frame left out of the
                // target refuses it, which paints whole and goes direct, and
                // so on while the view moves.)
                let direct = whole
                    && self.whole_frames > 0
                    && segment == 0
                    && blur.is_none()
                    && !self.copy_next.get();
                self.whole_frames = if whole {
                    self.whole_frames.saturating_add(1)
                } else {
                    0
                };
                *self.deferred.get_mut() = Some(Deferred {
                    vertices: vertices.clone(),
                    draws: draws.split_off(from),
                    regions,
                    viewport: self.viewport_bind.clone(),
                    blur,
                    clear: segment == 0,
                    disjoint: !has_blur,
                    first,
                    direct,
                    points: points_bind.clone(),
                    kept: self.kept_now(),
                });
                break;
            }
            // (A blur's second pass goes in a pass of its own before a copy
            // at the same draw: what reads the backdrop reads it blurred.)
            if segment == 0 || end > from || vertical.is_some() {
                stats.render_passes += 1;
                let label = ["damage 0", "damage 1", "damage 2", "damage 3"][segment.min(3)];
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                #[cfg(own_surfaces)]
                if let Some((bind, instances)) = catch_up.take() {
                    pass.set_pipeline(self.copy_offset.as_ref().expect("made with the draws"));
                    pass.set_bind_group(0, &bind, &[]);
                    pass.set_vertex_buffer(0, vertices.slice(..));
                    pass.draw(0..4, instances);
                    stats.draw_calls += 1;
                }
                if let Some(blur) = vertical.take() {
                    let pipeline = self
                        .blur_pipelines
                        .as_ref()
                        .expect("filtered")
                        .drawing(&blur);
                    Self::encode_vertical(&mut pass, pipeline, &blur, &mut stats);
                }
                pass.set_vertex_buffer(0, vertices.slice(..));
                pass.set_bind_group(0, &self.bind, &[]);
                pass.set_bind_group(1, self.viewport_bind.as_ref(), &[]);
                if let Some(points) = &points_bind {
                    pass.set_bind_group(2, points, &[]);
                }
                let quads = &self.context.inner.quads;
                let kept = self.kept_now();
                for (index, (region, (x, y, w, h))) in regions.iter().enumerate() {
                    pass.set_scissor_rect(*x, *y, *w, *h);
                    let first = first[index];
                    if segment == 0 && first == 0 {
                        pass.set_pipeline(quads.clear(self.split_shading));
                        pass.set_bind_group(0, &self.bind, &[]);
                        pass.draw(0..4, 0..1);
                        stats.draw_calls += 1;
                    }
                    let mut rest = &draws[from..end];
                    if let Some((at, bind)) = &backdrop
                        && *at == from
                        && let Some(draw) = rest.first()
                    {
                        self.encode_blended(&mut pass, *region, draw, bind, &mut stats);
                        // Those sharing its copy.
                        for (first, shared) in &sharing {
                            if *first == *at {
                                for other in shared {
                                    let other = &draws[*other];
                                    self.encode_blended(
                                        &mut pass, *region, other, bind, &mut stats,
                                    );
                                }
                            }
                        }
                        if let Some(points) = &points_bind {
                            pass.set_bind_group(2, points, &[]);
                        }
                        rest = &rest[1..];
                    }
                    self.encode_draws(
                        &mut pass,
                        &|shading| {
                            self.fill_pipeline(shading)
                                .unwrap_or_else(|| quads.get(shading))
                        },
                        *region,
                        rest,
                        first,
                        kept,
                        &mut stats,
                    );
                }
            }
            if segment == cuts.len() {
                break;
            }
            match cuts[segment] {
                (_, Some(batch)) => {
                    vertical =
                        plans[batch]
                            .as_ref()
                            .zip(ranges[batch])
                            .and_then(|(plan, ranges)| {
                                self.blur(&mut encoder, plan, &vertices, ranges, &mut stats)
                            });
                }
                (at, None) => {
                    let read = (reads.iter())
                        .filter(|(draw, _)| {
                            *draw == at
                                || sharing
                                    .iter()
                                    .any(|(first, shared)| *first == at && shared.contains(draw))
                        })
                        .map(|(_, read)| *read)
                        .reduce(Rect::union);
                    stats.backdrop_copies += 1;
                    backdrop = Some((
                        at,
                        self.copy_backdrop(&mut encoder, &draws[at], read, &regions),
                    ));
                }
            }
            // The first filtered draw's own quads open the next pass.
            from = end;
        }
        *self.pending.get_mut() = Some(encoder);
        self.fresh = false;
        keep_room(&mut quads, &mut self.frame_peaks.0);
        keep_room(&mut draws, &mut self.frame_peaks.1);
        self.frame_quads = quads;
        self.frame_points = points;
        // (A kept frame's, lent: given back as they are.)
        if self.kept_upload.is_none() {
            draws.clear();
        }
        self.frame_draws = draws;
        Ok(stats)
    }
    /// Record this frame with Metal directly (`metal_frame`): the pass the
    /// wgpu path records for a frame of plain draws, committed at once.
    #[cfg(target_os = "macos")]
    #[allow(clippy::too_many_arguments)]
    fn render_direct(
        &mut self,
        mut quads: Vec<Quad>,
        mut draws: Vec<Draw>,
        regions: &[(Rect, Scissor)],
        batches: &[(usize, Vec<usize>)],
        catch_up: Option<std::ops::Range<u32>>,
        shown: Option<usize>,
        (effect_jobs, effect_first): (&[effects::Job], u32),
        clips: &[Region],
        points: Option<&[[f32; 2]]>,
        bands: &[bands::BandPhase<'_>],
        mut stats: GpuStats,
    ) -> GpuStats {
        let _span = zgx::profile::span(zgx::profile::Phase::Encode);
        // A drawable frame is not waited for as it presents (Metal presents
        // it); its ring buffers are free once it completes, normally long
        // before the next frame starts.
        self.finish_direct();
        let vertices = {
            use objc2::Message;
            let mapped = self
                .mapped
                .as_mut()
                .expect("checked by the caller")
                .get_mut();
            // Polyline strokes' points after the quads, where drawn.
            let parts = [
                bytemuck::cast_slice(&quads),
                points.map_or(&[][..], bytemuck::cast_slice),
            ];
            let (_, allocated) = match &self.kept_upload {
                Some((kept, bytes)) => mapped.write_kept(&parts, kept, *bytes, size_of::<Quad>()),
                None => mapped.write_parts(&parts),
            };
            stats.vertex_buffer_allocations += usize::from(allocated);
            self.vertex_capacity = mapped.bytes();
            mapped.last_metal().retain()
        };
        self.whole_frames = 0;
        // As `render`'s wgpu passes: a split at each batch of filters.
        let first = if batches.is_empty() {
            occluded_from(&quads, &draws, regions, self.scale)
        } else {
            vec![0; regions.len()]
        };
        // Every batch planned first: the atlas then fits them all.
        let plans: Vec<_> = batches
            .iter()
            .map(|(_, members)| {
                let plan = self.plan_blur(&batch_filters(&draws, regions, members))?;
                let slots: Vec<SlotRecord> = plan.slots.iter().map(|slot| slot.0).collect();
                let rects: Vec<Scissor> = plan
                    .slots
                    .iter()
                    .map(|(record, (w, h), _)| {
                        (record.atlas[0] as u32, record.atlas[1] as u32, *w, *h)
                    })
                    .collect();
                Some((plan, slots, rects))
            })
            .collect();
        let committed = {
            let frame = self
                .metal_frame
                .get()
                .and_then(Option::as_ref)
                .expect("checked by the caller");
            // Metal's textures, each HAL guard dropped before the next is
            // taken (wgpu's lock is not reentrant).
            let raw = |texture: &wgpu::Texture| {
                use objc2::Message;
                // SAFETY: The raw texture is retained for a command buffer (which
                // retains it too), never destroyed; the guard is dropped at once.
                unsafe { texture.as_hal::<wgpu::hal::api::Metal>() }
                    .expect("a Metal texture")
                    .raw_handle()
                    .retain()
            };
            // The retained target: through the layer, this frame's surface.
            let (target, atlas) = (raw(&self.target), raw(&self.atlas));
            let blur_atlas = self
                .blur_scratch
                .as_ref()
                .filter(|_| plans.iter().any(Option::is_some))
                .map(|scratch| scratch.atlas.each_ref().map(raw));
            // Effect slots drawn first, each pass of them into its atlas.
            let effect_passes = self.metal_effect_passes(effect_jobs, effect_first, &raw);
            let band_targets: Vec<_> = bands.iter().map(|band| raw(&band.texture)).collect();
            for pass in &effect_passes {
                stats.render_passes += 1;
                stats.effect_passes += 1;
                stats.draw_calls += pass.draws.len();
            }
            let mut pass = frame.begin(
                &target,
                &vertices,
                self.viewport_values,
                bytemuck::cast_slice(clips),
                // After the quads (as written above).
                points.map(|points| (quads.len() * size_of::<Quad>(), size_of_val(points))),
                &effect_passes,
                bands.first().map(|band| {
                    let size = band.viewport;
                    (&*band_targets[0], [size.width, size.height, self.scale, 0.])
                }),
            );
            // A fade's bands first (bands.rs), each into its texture.
            for (index, band) in bands.iter().enumerate() {
                if index > 0 {
                    let size = band.viewport;
                    pass.switch(
                        &band_targets[index],
                        true,
                        [size.width, size.height, self.scale, 0.],
                    );
                }
                pass.texture(&atlas);
                let mut sampled = TextureSource::Atlas;
                let kept = self.kept_now();
                for (region, region_scissor) in &band.regions {
                    pass.scissor(*region_scissor);
                    self.batches(*region, &band.draws, 0, kept, |batch| {
                        let Some(rect) = scissor(batch.rect, self.scale, self.width, self.height)
                        else {
                            return;
                        };
                        if batch.source != sampled {
                            pass.nearest(matches!(batch.source, TextureSource::Pixelated(_)));
                            let texture = match batch.source {
                                TextureSource::Image(image) | TextureSource::Pixelated(image) => {
                                    self.images.get(&image).and_then(|image| image.2.as_ref())
                                }
                                TextureSource::Layer(layer) => {
                                    self.layers.get(&layer).map(|layer| &layer.texture)
                                }
                                TextureSource::Atlas => None,
                            };
                            match texture {
                                Some(texture) => pass.texture(&raw(texture)),
                                None => pass.texture(&atlas),
                            }
                            sampled = batch.source;
                        }
                        pass.scissor(rect);
                        pass.quads(frame, batch.shading, false, batch.start..batch.end);
                        stats.draw_calls += 1;
                        stats.instances += (batch.end - batch.start) as usize;
                    });
                }
                stats.render_passes += 1;
            }
            if !bands.is_empty() {
                pass.switch(&target, false, self.viewport_values);
            }
            let present = self.layer_present.as_ref();
            if let (Some(instances), Some(present), Some(shown)) = (catch_up, present, shown) {
                pass.texture(&present.buffers[shown].0.metal);
                pass.copies(frame, instances);
                stats.draw_calls += 1;
            }
            pass.texture(&atlas);
            let clear = if self.split_shading {
                Shading::Basic
            } else {
                Shading::Full
            };
            // The image or layer the quads sample (neither: the glyph atlas).
            let mut sampled = TextureSource::Atlas;
            let splits = batches.iter().map(|(start, _)| *start);
            let mut from = 0;
            // The batch drawn back at the start of the pass after its blur.
            let mut blurred = None;
            for (segment, end) in splits.chain([draws.len()]).enumerate() {
                if let Some((plan, slots, _)) = blurred.take() {
                    let plan: &BlurPlan = plan;
                    let records = metal_frame::BlurRecords {
                        blurs: bytemuck::cast_slice(&plan.blurs),
                        slots: bytemuck::cast_slice(slots),
                    };
                    let blur = blur_atlas.as_ref().expect("planned");
                    pass.mixes(frame, &blur[0], &records, &plan.draws);
                    stats.draw_calls += plan.draws.len();
                }
                let kept = self.kept_now();
                for (index, (region, region_scissor)) in regions.iter().enumerate() {
                    pass.scissor(*region_scissor);
                    if segment == 0 && first[index] == 0 {
                        pass.quads(frame, clear, true, 0..1);
                        stats.draw_calls += 1;
                    }
                    self.batches(*region, &draws[from..end], first[index], kept, |batch| {
                        let Some(rect) = scissor(batch.rect, self.scale, self.width, self.height)
                        else {
                            return;
                        };
                        if batch.source != sampled {
                            pass.nearest(matches!(batch.source, TextureSource::Pixelated(_)));
                            let texture = match batch.source {
                                TextureSource::Image(image) | TextureSource::Pixelated(image) => {
                                    self.images.get(&image).and_then(|image| image.2.as_ref())
                                }
                                TextureSource::Layer(layer) => {
                                    self.layers.get(&layer).map(|layer| &layer.texture)
                                }
                                TextureSource::Atlas => None,
                            };
                            match texture {
                                Some(texture) => pass.texture(&raw(texture)),
                                None => pass.texture(&atlas),
                            }
                            sampled = batch.source;
                        }
                        pass.scissor(rect);
                        pass.quads(frame, batch.shading, false, batch.start..batch.end);
                        stats.draw_calls += 1;
                        stats.instances += (batch.end - batch.start) as usize;
                    });
                }
                let Some(Some((plan, slots, rects))) = plans.get(segment) else {
                    from = end;
                    continue;
                };
                let records = metal_frame::BlurRecords {
                    blurs: bytemuck::cast_slice(&plan.blurs),
                    slots: bytemuck::cast_slice(slots),
                };
                let blur = blur_atlas.as_ref().expect("planned");
                pass.blur(frame, &target, [&blur[0], &blur[1]], &records, rects);
                pass.resume(&target);
                pass.texture(&atlas);
                sampled = TextureSource::Atlas;
                stats.draw_calls += 3 * rects.iter().filter(|r| r.2 > 0 && r.3 > 0).count();
                stats.render_passes += 4;
                stats.blur_passes += 3;
                blurred = Some((plan, slots, rects));
                from = end;
            }
            let _span = zgx::profile::span(zgx::profile::Phase::Submit);
            // Into a drawable: presented by this command buffer, once done;
            // not when a GPU view's plane moves with this frame, both then
            // presenting in its transaction (`present_layer`).
            let planes_moved = self
                .views
                .as_ref()
                .is_some_and(|views| views.planes_moved());
            let drawable = self
                .layer_present
                .as_mut()
                .filter(|present| {
                    !planes_moved
                        && present
                            .current
                            .is_some_and(|index| present.buffers[index].0.drawable)
                })
                .and_then(|present| present.layer.as_mut())
                .and_then(|layer| layer.take_drawable());
            pass.commit_presenting(
                drawable
                    .as_deref()
                    .map(objc2::runtime::ProtocolObject::<dyn objc2_metal::MTLDrawable>::from_ref),
            )
        };
        *self.metal_inflight.get_mut() = Some(committed);
        self.metal_frames += 1;
        stats.render_passes += 1;
        self.fresh = false;
        keep_room(&mut quads, &mut self.frame_peaks.0);
        keep_room(&mut draws, &mut self.frame_peaks.1);
        self.frame_quads = quads;
        // (A kept frame's, lent: given back as they are.)
        if self.kept_upload.is_none() {
            draws.clear();
        }
        self.frame_draws = draws;
        stats
    }
    /// Shader fills and layer shaders (those not reading the backdrop)
    /// drawn with Direct3D 11's pixel shaders for them
    /// (`d3d11_frame::Device::app_shader`): one still compiling draws as
    /// `Full` meanwhile (a fill its box's colour, a layer as painted) and the
    /// window draws again once it is (`images_awaited`), as wgpu's frames go
    /// without one; one that didn't compile draws so.
    #[cfg(d3d11)]
    fn d3d11_app_shaders(&mut self, draws: &mut [Draw]) {
        use d3d11_frame::AppShader;
        let Some(state) = &self.d3d11 else {
            return;
        };
        let mut awaited = false;
        let wait = self.waits_for_shaders();
        for draw in draws {
            let (id, shaders, kind) = match draw.shading {
                Shading::Fill(id) => (id, &self.fill_shaders, AppShader::Fill),
                Shading::Filter(id) => (id, &self.filter_shaders, AppShader::Filter),
                Shading::FilterBlend(id) => (id, &self.filter_shaders, AppShader::FilterBlend),
                _ => continue,
            };
            let ready = shaders
                .get(&id)
                .map(|shader| state.device.app_shader(shader, kind, wait));
            // Glass not compiled yet isn't drawn, as Direct3D 12's blended
            // draw waits for its pipeline; anything else draws as painted.
            let glass = kind == AppShader::FilterBlend;
            match ready {
                Some(Some(true)) => {}
                Some(None) => {
                    awaited = true;
                    match glass {
                        true => draw.end = draw.start,
                        false => draw.shading = Shading::Full,
                    }
                }
                _ if glass => draw.end = draw.start,
                _ => draw.shading = Shading::Full,
            }
        }
        self.images_awaited |= awaited;
    }
    /// What Direct3D 11 leaves out of a frame rather than fail it, where
    /// what's left can't show a picture a user would wrongly trust; each kind
    /// named once (`d3d11_frame::left_out`):
    /// - images a GPU view draws, where the views' device couldn't be made
    ///   (`view_host`): left empty, visibly missing.
    ///
    /// Content blurs, masks and drop shadows draw (`d3d11_effects`), and
    /// backdrop blurs and layer shaders reading the backdrop (glass, frosted
    /// or not) over a copy of it (`render_d3d11`).
    #[cfg(d3d11)]
    fn d3d11_leave_out(&self, draws: &mut [Draw]) {
        for draw in draws.iter_mut() {
            if let TextureSource::Image(id) | TextureSource::Pixelated(id) = draw.source
                && id != effects::EFFECT_ATLAS
                && draw.end > draw.start
                && self.d3d11_texture(draw.source).is_none()
            {
                d3d11_frame::left_out("images a GPU view draws");
                draw.end = draw.start;
            }
        }
    }
    /// What a frame needs that Direct3D 11 neither draws nor leaves out
    /// (`d3d11_leave_out`), if anything: it draws what the Windows suite's
    /// scenes do, shader fills and layer shaders not reading the backdrop.
    #[cfg(d3d11)]
    fn d3d11_lacks(&self, draws: &[Draw]) -> Option<&'static str> {
        draws
            .iter()
            .filter(|draw| draw.end > draw.start)
            .find_map(|draw| match (draw.source, draw.shading) {
                (TextureSource::Image(effects::EFFECT_ATLAS), _) => None,
                (TextureSource::Image(_) | TextureSource::Pixelated(_), _)
                    if self.d3d11_texture(draw.source).is_none() =>
                {
                    Some("GPU-drawn image")
                }
                (
                    _,
                    Shading::Full
                    | Shading::Basic
                    | Shading::Opaque
                    | Shading::Polyline
                    | Shading::SourceIn
                    | Shading::Behind
                    // Compiled (`d3d11_app_shaders`).
                    | Shading::Fill(_)
                    | Shading::Filter(_),
                ) => None,
                // Compiled (`d3d11_app_shaders`), over a copy of the backdrop
                // (taken after a blur under it has drawn, `render_d3d11`).
                (_, Shading::FilterBlend(_)) => None,
                // Its pixel stage made when first drawn (`d3d11_frame::Frame::quads`).
                (_, Shading::ImageEffect) => None,
                // Over a copy of the target under it (`d3d11_frame::Frame::blended`).
                (_, Shading::Blend) => None,
            })
    }
    /// What a batch of `source` samples with Direct3D 11: the glyph atlas,
    /// a layer's content or an image's texels, if it has them.
    #[cfg(d3d11)]
    fn d3d11_texture(
        &self,
        source: TextureSource,
    ) -> Option<&windows::Win32::Graphics::Direct3D11::ID3D11ShaderResourceView> {
        let state = self.d3d11.as_ref()?;
        let surface = match source {
            TextureSource::Atlas => state.atlas.as_ref(),
            TextureSource::Layer(id) => state.layers.get(&self.layers.get(&id)?.texture),
            TextureSource::Image(id) | TextureSource::Pixelated(id) => {
                state.textures.get(self.images.get(&id)?.2.as_ref()?)
            }
        };
        surface?.srv.as_ref()
    }
    /// One batch of a frame drawn with Direct3D 11 (`render_d3d11`): its
    /// texture bound if another, its scissor, its quads.
    #[cfg(d3d11)]
    fn d3d11_batch(
        &self,
        frame: &mut d3d11_frame::Frame<'_>,
        sampled: &mut TextureSource,
        unpainted: &mut bool,
        stats: &mut GpuStats,
        batch: Batch,
    ) {
        let Some(rect) = scissor(batch.rect, self.scale, self.width, self.height) else {
            return;
        };
        if batch.source != *sampled {
            let Some(view) = self.d3d11_texture(batch.source) else {
                *unpainted = true;
                return;
            };
            frame.texture(view);
            frame.nearest(matches!(batch.source, TextureSource::Pixelated(_)));
            *sampled = batch.source;
        }
        frame.scissor(rect);
        frame.quads(batch.shading, false, batch.start..batch.end);
        stats.draw_calls += 1;
        stats.instances += (batch.end - batch.start) as usize;
    }
    /// Fade bands' passes with Direct3D 11 (`render_d3d11`): each band's
    /// texture (one of Direct3D 11's standing for its wgpu texture, as a
    /// layer's does) cleared, then its draws in its regions at its viewport.
    #[cfg(d3d11)]
    fn d3d11_bands(
        &mut self,
        bands: &[bands::BandPhase<'_>],
        clips: &[Region],
        vertices: &windows::Win32::Graphics::Direct3D11::ID3D11Buffer,
        stats: &mut GpuStats,
    ) -> Result<(), GpuError> {
        let kept = self.kept_now();
        let mut unpainted = false;
        for band in bands {
            let state = self.d3d11.as_mut().expect("checked by the caller");
            let device = state.device.clone();
            let size = (band.texture.width(), band.texture.height());
            if state
                .layers
                .get(&band.texture)
                .is_none_or(|t| (t.width, t.height) != size)
            {
                let surface = device.surface(size.0, size.1, true)?;
                state.layers.insert(band.texture.clone(), surface);
            }
            let viewport = [band.viewport.width, band.viewport.height, self.scale, 0.];
            state.set_frame(viewport, bytemuck::cast_slice(clips))?;
            let state = self.d3d11.as_ref().expect("checked by the caller");
            let target = &state.layers[&band.texture];
            device.clear(target);
            let mut frame = device.begin(state, target, vertices);
            let atlas = state.atlas.as_ref().and_then(|atlas| atlas.srv.as_ref());
            frame.texture(atlas.expect("made by the caller"));
            let mut sampled = TextureSource::Atlas;
            for (region, _) in &band.regions {
                self.batches(*region, &band.draws, 0, kept, |batch| {
                    self.d3d11_batch(&mut frame, &mut sampled, &mut unpainted, stats, batch)
                });
            }
            frame.end();
            stats.render_passes += 1;
        }
        // The frame's viewport again.
        let state = self.d3d11.as_mut().expect("checked by the caller");
        state.set_frame(self.viewport_values, bytemuck::cast_slice(clips))?;
        match unpainted {
            true => Err(GpuError(
                "Direct3D 11 has no texels for a layer or image a fade band draws".into(),
            )),
            false => Ok(()),
        }
    }
    /// Layer effects' passes with Direct3D 11 (`d3d11_frame::Device::effects`):
    /// `jobs`' quads at instance `first` on, as `effect_quads` lays them out,
    /// across into the scratch atlas, then down into the effect atlas, which
    /// composites as an image does. The atlas and its scratch twin are made
    /// at the shell's size, again when it is made anew.
    #[cfg(d3d11)]
    fn d3d11_effects(
        &mut self,
        jobs: &[effects::Job],
        first: u32,
        vertices: &windows::Win32::Graphics::Direct3D11::ID3D11Buffer,
        stats: &mut GpuStats,
    ) -> Result<(), GpuError> {
        let missing = || GpuError("Direct3D 11 has no texels for a layer an effect reads".into());
        let atlas = self.effects.as_ref().expect("planned with its jobs");
        let state = self.d3d11.as_mut().expect("checked by the caller");
        let device = state.device.clone();
        let (width, height) = (atlas.output.width(), atlas.output.height());
        if !state.textures.contains_key(&atlas.output) {
            state
                .textures
                .insert(atlas.output.clone(), device.surface(width, height, true)?);
        }
        if state
            .effects
            .as_ref()
            .is_none_or(|(scratch, ..)| *scratch != atlas.scratch)
        {
            state.effects = Some((
                atlas.scratch.clone(),
                device.surface(width, height, true)?,
                device.surface(1, 1, false)?,
            ));
        }
        let state = self.d3d11.as_ref().expect("checked by the caller");
        let (_, scratch, blank) = state.effects.as_ref().expect("made above");
        let output = &state.textures[&atlas.output];
        let srv = |surface: &'_ d3d11_frame::Surface| surface.srv.clone();
        let blank = srv(blank).expect("sampled");
        let scratch_srv = srv(scratch).expect("sampled");
        let mut views = Vec::new();
        for down in [false, true] {
            for job in jobs.iter().filter(|job| down || job.across.is_some()) {
                let source = match down && job.across.is_some() {
                    true => Some(scratch_srv.clone()),
                    false => state
                        .layers
                        .get(&self.layers.get(&job.node).ok_or_else(missing)?.texture)
                        .and_then(srv),
                };
                // A mask without texels masks with the blank one, as wgpu's.
                let mask = match (down, job.mask) {
                    (true, Some(id)) => self
                        .images
                        .get(&id)
                        .and_then(|entry| entry.2.as_ref())
                        .and_then(|texture| state.textures.get(texture))
                        .and_then(srv),
                    _ => None,
                };
                views.push((
                    down,
                    source.ok_or_else(missing)?,
                    mask.unwrap_or(blank.clone()),
                ));
            }
        }
        let mut passes: Vec<d3d11_frame::EffectPass<'_>> = Vec::new();
        for (index, (down, source, mask)) in views.iter().enumerate() {
            let target = if *down { output } else { scratch };
            if passes.last().is_none_or(|pass| pass.down != *down) {
                passes.push(d3d11_frame::EffectPass {
                    target,
                    down: *down,
                    draws: Vec::new(),
                });
            }
            let pass = passes.last_mut().expect("pushed above");
            pass.draws.push(d3d11_frame::EffectDraw {
                source,
                mask,
                instance: first + index as u32,
            });
        }
        for pass in &passes {
            stats.render_passes += 1;
            stats.effect_passes += 1;
            stats.draw_calls += pass.draws.len();
        }
        device.effects(&passes, vertices);
        Ok(())
    }
    /// Record this frame with Direct3D 11 (`d3d11_frame`): the pass the
    /// wgpu path records for a frame of plain draws, as `render_direct`
    /// records it with Metal.
    #[cfg(d3d11)]
    #[allow(clippy::too_many_arguments)]
    fn render_d3d11(
        &mut self,
        mut quads: Vec<Quad>,
        mut draws: Vec<Draw>,
        regions: &[(Rect, Scissor)],
        copies: &[ScrollCopy],
        catch_up: Option<std::ops::Range<u32>>,
        shown: Option<usize>,
        clips: &[Region],
        points: &[[f32; 2]],
        (effect_jobs, effect_first): (&[effects::Job], u32),
        reads: &[(usize, Rect)],
        batches: &[(usize, Vec<usize>)],
        bands: &[bands::BandPhase<'_>],
        mut stats: GpuStats,
    ) -> Result<GpuStats, GpuError> {
        let _span = zgx::profile::span(zgx::profile::Phase::Encode);
        let (width, height) = (self.width, self.height);
        // Backdrop blurs a batch at a time, planned as Direct3D 12 plans them
        // (`render_flat`): the records and slots, the atlas's size.
        let plans: Vec<Option<BlurPlan>> = batches
            .iter()
            .map(|(_, members)| self.plan_blur(&batch_filters(&draws, regions, members)))
            .collect();
        let atlas_size = self.blur_scratch.as_ref().map(|scratch| {
            let texture = &scratch.atlas[0];
            (texture.width(), texture.height())
        });
        let state = self.d3d11.as_mut().expect("checked by the caller");
        let device = state.device.clone();
        let parts = [bytemuck::cast_slice(&quads)];
        let (vertices, allocated) = match &self.kept_upload {
            Some((kept, bytes)) => {
                state
                    .ring
                    .write_kept(&device, &parts, kept, *bytes, size_of::<Quad>())?
            }
            None => state.ring.write_parts(&device, &parts)?,
        };
        stats.vertex_buffer_allocations += usize::from(allocated);
        self.vertex_capacity = state.ring.bytes();
        state.set_frame(self.viewport_values, bytemuck::cast_slice(clips))?;
        state.set_points(bytemuck::cast_slice(points))?;
        // Effect slots drawn first, each pass of them into its atlas.
        if !effect_jobs.is_empty() {
            self.d3d11_effects(effect_jobs, effect_first, &vertices, &mut stats)?;
        }
        let state = self.d3d11.as_mut().expect("checked by the caller");
        // Offscreen, the retained target is the renderer's own (and its
        // spare, for scroll copies), made at its size.
        let presenting = self.layer_present.is_some();
        // A layer's content (`paint_layer`): drawn into the texture standing
        // for the shell's, made at its size.
        if self.in_layer {
            if state
                .layers
                .get(&self.target)
                .is_none_or(|t| (t.width, t.height) != (width, height))
            {
                let surface = device.surface(width, height, true)?;
                state.layers.insert(self.target.clone(), surface);
            }
        } else if !presenting
            && state
                .target
                .as_ref()
                .is_none_or(|t| (t.width, t.height) != (width, height))
        {
            state.target = Some(device.surface(width, height, true)?);
        }
        if shown.is_none()
            && !copies.is_empty()
            && state
                .spare
                .as_ref()
                .is_none_or(|t| (t.width, t.height) != (width, height))
        {
            state.spare = Some(device.surface(width, height, true)?);
        }
        if state.atlas.is_none() {
            state.atlas = Some(device.surface(1, 1, false)?);
        }
        // Fade bands (bands.rs): what reaches each run's bands drawn unfaded
        // into a texture of its own before the frame composites it through
        // the mask, as `encode_band` draws it.
        if !bands.is_empty() {
            self.d3d11_bands(bands, clips, &vertices, &mut stats)?;
        }
        let state = self.d3d11.as_mut().expect("checked by the caller");
        // Blended layers read a copy of the target under them, of its size.
        let blended: Vec<usize> = draws
            .iter()
            .enumerate()
            .filter(|(_, draw)| {
                matches!(draw.shading, Shading::Blend | Shading::FilterBlend(_))
                    && draw.end > draw.start
            })
            .map(|(index, _)| index)
            .collect();
        if !blended.is_empty()
            && state
                .backdrop
                .as_ref()
                .is_none_or(|b| (b.width, b.height) != (width, height))
        {
            state.backdrop = Some(device.surface(width, height, false)?);
        }
        // The blur's atlas pair at the plans' size; each batch's records.
        if let Some((atlas_width, atlas_height)) =
            atlas_size.filter(|_| plans.iter().any(Option::is_some))
            && state
                .blur_atlas
                .as_ref()
                .is_none_or(|a| (a[0].width, a[0].height) != (atlas_width, atlas_height))
        {
            state.blur_atlas = Some(device.blur_atlas(atlas_width, atlas_height)?);
        }
        let records = plans
            .iter()
            .map(|plan| {
                let Some(plan) = plan else { return Ok(None) };
                let slots: Vec<SlotRecord> = plan.slots.iter().map(|slot| slot.0).collect();
                Ok(Some([
                    device.records(bytemuck::cast_slice(&plan.blurs))?,
                    device.records(bytemuck::cast_slice(&slots))?,
                ]))
            })
            .collect::<Result<Vec<_>, GpuError>>()?;
        self.whole_frames = 0;
        // Filters sample what lies beneath them: a frame blurring draws it all
        // (as `render_flat`'s, whose split passes do).
        let first = match batches.is_empty() {
            true => occluded_from(&quads, &draws, regions, self.scale),
            false => vec![0; regions.len()],
        };
        let state = self.d3d11.as_ref().expect("checked by the caller");
        let views = self
            .layer_present
            .as_ref()
            .filter(|_| !self.in_layer)
            .and_then(|present| present.chain.as_ref())
            .and_then(|chain| chain.d3d11_views());
        // The layer's texture, the swap chain's buffer to draw (buffer 0; 1
        // is the one shown), or the renderer's own.
        let target = match &views {
            _ if self.in_layer => &state.layers[&self.target],
            Some(views) => &views[0],
            None => state.target.as_ref().expect("made above"),
        };
        // Scrolled without a shown surface to draw them from, the copies are
        // made in place, as wgpu's path makes them.
        if shown.is_none() && !copies.is_empty() {
            device.scroll(target, state.spare.as_ref().expect("made above"), copies);
            stats.scroll_copies += copies.len();
        }
        let mut frame = device.begin(state, target, &vertices);
        if let (Some(instances), Some(views)) = (catch_up, &views) {
            frame.texture(views[1].srv.as_ref().expect("sampled"));
            frame.copies(instances);
            stats.draw_calls += 1;
        }
        let atlas_view = state
            .atlas
            .as_ref()
            .and_then(|atlas| atlas.srv.as_ref())
            .expect("made above");
        frame.texture(atlas_view);
        // What batches sample: the atlas, a layer's content or an image.
        let mut sampled = TextureSource::Atlas;
        let mut unpainted = false;
        let clear = if self.split_shading {
            Shading::Basic
        } else {
            Shading::Full
        };
        let kept = self.kept_now();
        // As Direct3D 12 splits its pass (`render_flat`): every region's draws
        // up to each batch of backdrop blurs or blended draw. A batch's blur
        // passes then draw into its atlas, and it draws back over the target
        // (its first draw's own quads draw after). A blended draw: the target
        // under it copied (all it reads, within the regions), then it in every
        // region. A blur comes first where both start at a draw (frosted
        // glass reads what its blur drew). A layer shader reading the backdrop
        // (glass) may read past its own region.
        let mut cuts: Vec<(usize, Option<usize>)> = (batches.iter().enumerate())
            .map(|(batch, (start, _))| (*start, Some(batch)))
            .chain(blended.iter().map(|&at| (at, None)))
            .collect();
        cuts.sort_by_key(|&(at, batch)| (at, batch.is_none()));
        let mut from = 0;
        // Each region is cleared once, in the first segment: a batch of blurs
        // at the first draw (a popup's glass) leaves `from` at 0, and its blur
        // drawn back must not be cleared again.
        let mut cleared = false;
        for &(at, batch) in cuts.iter().chain(std::iter::once(&(draws.len(), None))) {
            for (index, (region, region_scissor)) in regions.iter().enumerate() {
                frame.scissor(*region_scissor);
                if !cleared && first[index] == 0 {
                    frame.quads(clear, true, 0..1);
                    stats.draw_calls += 1;
                }
                self.batches(*region, &draws[from..at], first[index], kept, |batch| {
                    self.d3d11_batch(&mut frame, &mut sampled, &mut unpainted, &mut stats, batch)
                });
            }
            cleared = true;
            if let Some(batch) = batch {
                from = at;
                let (Some(plan), Some(records), Some(atlas)) =
                    (&plans[batch], &records[batch], &state.blur_atlas)
                else {
                    continue;
                };
                let slots: Vec<d3d11_frame::BlurSlot> = (plan.slots.iter())
                    .map(|(record, (w, h), blurred)| d3d11_frame::BlurSlot {
                        texels: (record.atlas[0] as u32, record.atlas[1] as u32, *w, *h),
                        blurred: *blurred,
                    })
                    .collect();
                let copies = plan.slots.iter().any(|slot| !slot.2);
                let records = [&records[0], &records[1]];
                let backdrop = state.popup_backdrop.as_ref();
                let drawn = frame.blur(target, backdrop, atlas, records, &slots, copies)
                    && frame.blur_back(target, &atlas[0], records, &plan.draws, plan.mixed);
                if drawn {
                    stats.blur_passes += 3;
                    stats.render_passes += 3;
                    stats.draw_calls += plan.draws.len()
                        + slots.iter().filter(|slot| slot.blurred).count() * 2
                        + slots.len();
                } else {
                    unpainted = true;
                }
                // The frame's bindings again, and the glyph atlas sampled.
                frame.rebind(state, target, &vertices);
                frame.texture(atlas_view);
                sampled = TextureSource::Atlas;
                continue;
            }
            from = at + 1;
            let Some(draw) = draws.get(at) else {
                continue;
            };
            // What it reads (`backdrop_read`), or what lies under it.
            let under = reads
                .iter()
                .filter(|(reader, _)| *reader == at)
                .map(|(_, read)| *read)
                .reduce(Rect::union)
                .or_else(|| draw.bounds.intersection(draw.clip));
            let copied = regions
                .iter()
                .filter_map(|(region, _)| region.intersection(under?))
                .reduce(Rect::union)
                .and_then(|area| scissor(area, self.scale, self.width, self.height));
            let Some(copied) = copied else {
                continue;
            };
            let backdrop = state.backdrop.as_ref().expect("made above");
            frame.copy_backdrop(target, backdrop, copied);
            stats.backdrop_copies += 1;
            if draw.source != sampled {
                let Some(view) = self.d3d11_texture(draw.source) else {
                    unpainted = true;
                    continue;
                };
                frame.texture(view);
                frame.nearest(matches!(draw.source, TextureSource::Pixelated(_)));
                sampled = draw.source;
            }
            let glass = match draw.shading {
                Shading::FilterBlend(id) => Some(id),
                _ => None,
            };
            for (index, (region, _)) in regions.iter().enumerate() {
                let Some(rect) = region
                    .intersection(draw.clip)
                    .and_then(|rect| rect.intersection(draw.bounds))
                    .and_then(|rect| scissor(rect, self.scale, self.width, self.height))
                    .filter(|_| draw.end > first[index])
                else {
                    continue;
                };
                if frame.blended(glass, backdrop, rect, draw.start..draw.end) {
                    stats.draw_calls += 1;
                    stats.instances += (draw.end - draw.start) as usize;
                } else {
                    unpainted = true;
                }
            }
        }
        {
            let _span = zgx::profile::span(zgx::profile::Phase::Submit);
            frame.end();
        }
        drop(views);
        let state = self.d3d11.as_mut().expect("checked by the caller");
        state.ring.submitted(&device.context);
        // Layers dropped, or made again at another size, leave their
        // textures behind: one stands for each layer kept.
        if !self.in_layer && state.layers.len() > self.layers.len() {
            // A wgpu texture hashes and compares by its id alone.
            #[allow(clippy::mutable_key_type)]
            let kept: rustc_hash::FxHashSet<&wgpu::Texture> =
                self.layers.values().map(|layer| &layer.texture).collect();
            state.layers.retain(|texture, _| kept.contains(texture));
        }
        // So do images given back (`upload_image`).
        if !self.in_layer && state.textures.len() > self.images.len() {
            #[allow(clippy::mutable_key_type)]
            let kept: rustc_hash::FxHashSet<&wgpu::Texture> = self
                .images
                .values()
                .filter_map(|entry| entry.2.as_ref())
                .collect();
            state.textures.retain(|texture, _| kept.contains(texture));
        }
        if unpainted {
            return Err(GpuError(
                "Direct3D 11 has no texels for a layer or image drawn before they were".into(),
            ));
        }
        stats.render_passes += 1;
        self.fresh = false;
        keep_room(&mut quads, &mut self.frame_peaks.0);
        keep_room(&mut draws, &mut self.frame_peaks.1);
        self.frame_quads = quads;
        // (A kept frame's, lent: given back as they are.)
        if self.kept_upload.is_none() {
            draws.clear();
        }
        self.frame_draws = draws;
        Ok(stats)
    }
    /// Wait for a directly encoded frame (`render_direct`) to complete.
    #[cfg(target_os = "macos")]
    fn finish_direct(&self) -> bool {
        use objc2_metal::MTLCommandBuffer;
        let Some(buffer) = self.metal_inflight.borrow_mut().take() else {
            return false;
        };
        buffer.waitUntilCompleted();
        let ran = buffer.GPUEndTime() - buffer.GPUStartTime();
        self.gpu_time
            .set((ran > 0.).then(|| std::time::Duration::from_secs_f64(ran)));
        if let Some(mapped) = &self.mapped {
            mapped.borrow_mut().completed();
        }
        true
    }
    /// The spare retained texture (single-pass presentation's other target),
    /// created at the window's size if needed.
    fn spare_texture(&mut self) -> wgpu::Texture {
        if self
            .spare
            .as_ref()
            .is_none_or(|t| t.texture.width() != self.width || t.texture.height() != self.height)
        {
            let texture = texture(&self.device, self.width, self.height, "retained output");
            let view = texture.create_view(&Default::default());
            self.spare = Some(Retained {
                texture,
                view,
                copy_bind: None,
                blit_bind: None,
            });
        }
        self.spare.as_ref().expect("created above").texture.clone()
    }
    /// Split the last flush's scroll moves into pixel copies this frame can
    /// apply and clips it must repaint instead. A copy is exact only when the
    /// retained target holds the previous flush, both the clip and the shift
    /// land on whole pixels, and within the clip nothing but the content
    /// changes position: whatever lies under it is hidden by one opaque
    /// rectangle, and anything over it is repainted anyway.
    #[inline]
    fn scroll_copies(&self, scene: &Scene, damage: &[Rect]) -> (Vec<ScrollCopy>, Vec<Rect>) {
        // A frame with no scroll moved, nearly every one that isn't scrolling,
        // doesn't reach the work below, which is a call and a big frame.
        let serial = scene.flush_serial();
        self.copy_next.set(false);
        if scene.scroll_moves().iter().all(|m| m.serial != serial) {
            return (Vec::new(), Vec::new());
        }
        self.scroll_copies_of(scene, damage, serial)
    }
    #[inline(never)]
    fn scroll_copies_of(
        &self,
        scene: &Scene,
        damage: &[Rect],
        serial: u64,
    ) -> (Vec<ScrollCopy>, Vec<Rect>) {
        let mut copies = Vec::new();
        let mut repaint = Vec::new();
        let mut carried = Vec::new();
        // On Apple GPUs a process's first texture blit makes the driver hold
        // over 100 MB of graphics memory (see vendor/wgpu-hal/ZGX_PATCH.md):
        // repaint there, unless presenting surfaces of its own, which copy
        // by drawing from the shown surface. That holds the previous flush
        // only once it was presented.
        #[cfg(own_surfaces)]
        let copyable = match self.layer_present.as_ref() {
            Some(present) => present.current.is_none(),
            None => !cfg!(target_os = "macos"),
        };
        #[cfg(not(own_surfaces))]
        let copyable = true;
        let copyable = copyable || self.debug_scroll_copy;
        for scroll in scene.scroll_moves() {
            if scroll.serial != serial {
                continue;
            }
            carried.clear();
            let whole = |value: f32| {
                let scaled = value * self.scale;
                ((scaled - scaled.round()).abs() < 1e-3).then_some(scaled.round() as i32)
            };
            // The copy of `clip`'s pixels the move makes, if whole.
            let pixels_of = |clip: Rect| {
                let clip = clip.intersection(Rect::new(
                    0.,
                    0.,
                    self.width as f32 / self.scale,
                    self.height as f32 / self.scale,
                ))?;
                let (x, y) = (whole(clip.x)?, whole(clip.y)?);
                let (right, bottom) = (whole(clip.x + clip.width)?, whole(clip.y + clip.height)?);
                let (dx, dy) = (whole(scroll.dx)?, whole(scroll.dy)?);
                // Destination: the clip minus the exposed strip; source: the
                // same pixels before the move.
                let (x0, x1) = (x.max(x + dx), right.min(right + dx));
                let (y0, y1) = (y.max(y + dy), bottom.min(bottom + dy));
                (x0 < x1 && y0 < y1 && x0 >= 0 && y0 >= 0).then(|| {
                    (
                        (x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32),
                        (-dx, -dy),
                    )
                })
            };
            let pixels = pixels_of(scroll.clip);
            // (Only single-pass frames go direct.)
            if self.fresh
                && self.single_pass_format.is_some()
                && pixels.is_some()
                && copyable
                && zgx::switches::SCROLL_COPY.on()
                && !self.copy_next.get()
            {
                let mut fade = None;
                let clear =
                    copy_blocker(scene, scroll, damage, self.scale, &mut carried, &mut fade)
                        .is_none()
                        && fade.is_none_or(|mask| {
                            self.fade_copy(scene, scroll, mask, &pixels_of).is_some()
                        })
                        && scene.scroll_corners(scroll.node).is_some();
                carried.clear();
                self.copy_next.set(clear);
            }
            // Why it repaints, for devtools (`zgx inspect frames`).
            let refused: Result<_, fn(&Scene) -> String> = match pixels {
                _ if !zgx::switches::SCROLL_COPY.on() => {
                    Err(|_| "switched off (zgx inspect switch scroll_copy on)".into())
                }
                None => Err(|_| "its view or its move falls between device pixels".into()),
                Some(_) if !copyable => Err(|_| {
                    "the window presents through the system's surface, whose first copy holds \
                     100 MB of graphics memory (macOS)"
                        .into()
                }),
                Some(_) if self.fresh => {
                    Err(|_| "the window's pixels are new (a first frame, a resize)".into())
                }
                Some(_) if self.rendered_serial != Some(serial - 1) => {
                    Err(|_| "the frame before it was not drawn".into())
                }
                Some(copy) => Ok(copy),
            };
            // Under a fade, the interior's pixels move (the mask is 1 there)
            // and the rest of the view repaints: its bands, and what of the
            // interior the copy leaves.
            let refused = refused.map(|copy| {
                let mut fade = None;
                match copy_blocker(scene, scroll, damage, self.scale, &mut carried, &mut fade) {
                    None => match fade {
                        None => (copy, None),
                        Some(mask) => match self.fade_copy(scene, scroll, mask, &pixels_of) {
                            Some((copy, rest)) => {
                                carried.extend(rest);
                                (copy, None)
                            }
                            None => (copy, Some(CopyBlocker::Fade(scroll.node, mask))),
                        },
                    },
                    blocker => (copy, blocker),
                }
            });
            match refused {
                Ok((copy, blocker)) => match blocker {
                    None => match scene.scroll_corners(scroll.node) {
                        // Rounded clips around it that stay where they are:
                        // the copy moves their edges and what lies outside
                        // them, so their corners repaint, where they are and
                        // where the copy took their pixels.
                        Some(corners) => {
                            zgx::profile::note(|| {
                                format!("scroll @{} copied its pixels", scroll.node.number())
                            });
                            for corner in corners {
                                let moved = Rect::new(
                                    corner.x + scroll.dx,
                                    corner.y + scroll.dy,
                                    corner.width,
                                    corner.height,
                                );
                                repaint.extend(
                                    [
                                        corner.intersection(scroll.clip),
                                        moved.intersection(scroll.clip),
                                    ]
                                    .into_iter()
                                    .flatten(),
                                );
                            }
                            repaint.append(&mut carried);
                            copies.push(copy);
                            self.scroll_copied.set(self.scroll_copied.get() + 1);
                        }
                        None => {
                            zgx::profile::note(|| {
                                format!(
                                    "scroll @{} repainted its view: a rounded clip around it \
                                     fades or is inset",
                                    scroll.node.number()
                                )
                            });
                            repaint.push(scroll.clip);
                        }
                    },
                    Some(blocker) => {
                        zgx::profile::note(|| {
                            format!(
                                "scroll @{} repainted its view: {}",
                                scroll.node.number(),
                                blocker.describe(scene)
                            )
                        });
                        repaint.push(scroll.clip);
                    }
                },
                Err(why) => {
                    zgx::profile::note(|| {
                        format!(
                            "scroll @{} repainted its view: {}",
                            scroll.node.number(),
                            why(scene)
                        )
                    });
                    repaint.push(scroll.clip);
                }
            }
        }
        (copies, repaint)
    }
    /// Submit commands recorded by the last render, if any.
    fn flush_pending(&self) {
        #[cfg(target_os = "macos")]
        self.finish_direct();
        let Some(mut pending) = self.pending.borrow_mut().take() else {
            return;
        };
        // Not presented (skipped, hidden or read back): draw it the two-pass
        // way so the retained target is complete.
        if let Some(deferred) = self.deferred.borrow_mut().take() {
            let mut pass = pending.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("damage"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let mut stats = GpuStats::default();
            if let Some(blur) = &deferred.blur {
                let pipeline = self
                    .blur_pipelines
                    .as_ref()
                    .expect("filtered")
                    .drawing(blur);
                Self::encode_vertical(&mut pass, pipeline, blur, &mut stats);
            }
            let quads = &self.context.inner.quads;
            let picks = Picks {
                full: quads.get(Shading::Full),
                clear: quads.clear(false),
                basic: quads.get(Shading::Basic),
                opaque: quads.get(Shading::Opaque),
                polyline: deferred
                    .points
                    .is_some()
                    .then(|| quads.get(Shading::Polyline)),
                quads: Some(quads),
            };
            self.encode_damage(&mut pass, &deferred, picks, &mut stats);
        }
        self.queue.submit([pending.finish()]);
        self.belt.borrow_mut().recall();
        self.submitted();
        if let Some(mapped) = &self.mapped {
            mapped.borrow_mut().submitted();
        }
    }
    fn submitted(&self) {
        #[cfg(target_os = "macos")]
        if let Some(cache) = &self.native_surfaces {
            cache.submitted();
        }
    }
    /// The polyline pipeline for single-pass presentation to `format`:
    /// into the retained target too, or the drawable `alone`.
    fn polyline_drawing(&self, format: wgpu::TextureFormat, alone: bool) -> wgpu::RenderPipeline {
        let polylines = self.context.inner.quads.polylines();
        polyline_pipeline(
            &self.device,
            &polylines.module,
            &polylines.layout,
            Some(format),
            alone,
        )
    }
    /// Clear and redraw every damaged region of a deferred frame.
    fn encode_damage(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        frame: &Deferred,
        picks: Picks<'_>,
        stats: &mut GpuStats,
    ) {
        pass.set_vertex_buffer(0, frame.vertices.slice(..));
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_bind_group(1, frame.viewport.as_ref(), &[]);
        if let Some(points) = &frame.points {
            pass.set_bind_group(2, points, &[]);
        }
        let (clear, quads) = (picks.clear, picks.full);
        let pick = |shading| picks.get(shading);
        let first = |index: usize| frame.first.get(index).copied().unwrap_or(0);
        if !frame.clear {
            pass.set_pipeline(quads);
            for (index, (region, (x, y, w, h))) in frame.regions.iter().enumerate() {
                pass.set_scissor_rect(*x, *y, *w, *h);
                self.encode_draws(
                    pass,
                    &pick,
                    *region,
                    &frame.draws,
                    first(index),
                    frame.kept,
                    stats,
                );
            }
            return;
        }
        if frame.disjoint {
            // One pipeline switch per phase instead of two per region; many
            // small animations each damage a region of their own.
            pass.set_pipeline(clear);
            for (index, (_, (x, y, w, h))) in frame.regions.iter().enumerate() {
                if first(index) == 0 {
                    pass.set_scissor_rect(*x, *y, *w, *h);
                    pass.draw(0..4, 0..1);
                    stats.draw_calls += 1;
                }
            }
            pass.set_pipeline(quads);
            for (index, (region, _)) in frame.regions.iter().enumerate() {
                self.encode_draws(
                    pass,
                    &pick,
                    *region,
                    &frame.draws,
                    first(index),
                    frame.kept,
                    stats,
                );
            }
            return;
        }
        for (index, (region, (x, y, w, h))) in frame.regions.iter().enumerate() {
            pass.set_scissor_rect(*x, *y, *w, *h);
            if first(index) == 0 {
                pass.set_pipeline(clear);
                pass.set_bind_group(0, &self.bind, &[]);
                pass.draw(0..4, 0..1);
                stats.draw_calls += 1;
            }
            pass.set_pipeline(quads);
            self.encode_draws(
                pass,
                &pick,
                *region,
                &frame.draws,
                first(index),
                frame.kept,
                stats,
            );
        }
    }
    /// One render pass per frame: copy the retained image into the spare
    /// target and `output`, then redraw the damage into both, and swap. Every
    /// pixel matches the two-pass route because `output`'s format needs no
    /// conversion (see `single_pass_format`).
    fn present_single_pass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Deferred,
        output: &wgpu::TextureView,
        format: wgpu::TextureFormat,
    ) {
        if self.single_pass.as_ref().is_none_or(|p| p.format != format) {
            let inner = &self.context.inner;
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("single-pass copy"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("copy_both.wgsl").into()),
                });
            let copy = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("single-pass copy"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(FORMAT.into()), Some(format.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
            self.target_copy_bind = None;
            if let Some(spare) = &mut self.spare {
                spare.copy_bind = None;
            }
            let both = |blend: bool, basic| {
                quad_pipeline(
                    &self.device,
                    inner.quads.shader(),
                    &inner.quad_layout,
                    blend.then_some(OVER),
                    Some(format),
                    basic,
                )
            };
            let direct = |blend, basic| {
                drawable_pipeline(
                    &self.device,
                    inner.quads.shader(),
                    &inner.quad_layout,
                    blend,
                    format,
                    basic,
                )
            };
            self.single_pass = Some(SinglePass {
                format,
                copy,
                both: PickedPipelines {
                    full: both(true, false),
                    clear: both(false, false),
                    basic: both(true, true),
                    opaque: both(false, true),
                    polyline: Default::default(),
                },
                direct: PickedPipelines {
                    full: direct(true, false),
                    clear: direct(false, false),
                    basic: direct(true, true),
                    opaque: direct(false, true),
                    polyline: Default::default(),
                },
            });
        }
        if !frame.regions.is_empty() {
            self.single_pass_frames += 1;
        }
        let polylines = frame.points.is_some();
        let clear = wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        };
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: clear,
            })
        };
        // A frame that clears and redraws the whole target keeps none of the
        // last one: nothing is copied, so it is drawn into the target itself
        // (or the drawable alone, `direct`) and needs no spare.
        let redrawn = frame.clear
            && frame.regions.iter().any(|(_, (x, y, w, h))| {
                (*x, *y) == (0, 0) && *w >= self.width && *h >= self.height
            });
        if frame.direct || redrawn {
            let pipelines = self.single_pass.as_ref().expect("created above");
            let mut stats = GpuStats::default();
            {
                let both = [attachment(&self.view), attachment(output)];
                let alone = [attachment(output)];
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("frame"),
                    color_attachments: if frame.direct { &alone } else { &both },
                    ..Default::default()
                });
                let picks = if frame.direct {
                    pipelines
                        .direct
                        .picks(polylines, || self.polyline_drawing(format, true))
                } else {
                    pipelines
                        .both
                        .picks(polylines, || self.polyline_drawing(format, false))
                };
                self.encode_damage(&mut pass, frame, picks, &mut stats);
            }
            self.present_stats.draw_calls += stats.draw_calls;
            self.present_stats.instances += stats.instances;
            if frame.direct {
                // The target still holds an earlier frame: the next render
                // repaints everything.
                self.fresh = true;
                self.direct_frames += 1;
            }
            return;
        }
        if self
            .spare
            .as_ref()
            .is_none_or(|t| t.texture.width() != self.width || t.texture.height() != self.height)
        {
            let texture = texture(&self.device, self.width, self.height, "retained output");
            let view = texture.create_view(&Default::default());
            self.spare = Some(Retained {
                texture,
                view,
                copy_bind: None,
                blit_bind: None,
            });
        }
        if frame.blur.is_some() {
            let device = &self.device;
            let blur = self.blur_pipelines.as_mut().expect("filtered");
            blur.both(device, format);
        }
        let pipelines = self.single_pass.as_ref().expect("created above");
        let previous = self
            .target_copy_bind
            .get_or_insert_with(|| blit_group(&self.device, &pipelines.copy, &self.view))
            .clone();
        let spare = self.spare.take().expect("created above");
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[attachment(&spare.view), attachment(output)],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.copy);
            pass.set_bind_group(0, &previous, &[]);
            pass.draw(0..3, 0..1);
            let mut stats = GpuStats::default();
            if let Some(blur) = &frame.blur {
                let both = self.blur_pipelines.as_ref().and_then(|b| b.both.as_ref());
                let (_, pipeline) = both.expect("created above");
                Self::encode_vertical(&mut pass, pipeline, blur, &mut stats);
            }
            let picks = pipelines
                .both
                .picks(polylines, || self.polyline_drawing(format, false));
            self.encode_damage(&mut pass, frame, picks, &mut stats);
            self.present_stats.draw_calls += stats.draw_calls + 1;
            self.present_stats.instances += stats.instances;
        }
        // The spare now holds this frame; the old target becomes the spare,
        // keeping its view and bind groups for when it is drawn into again.
        let Retained {
            texture,
            view,
            copy_bind,
            blit_bind,
        } = spare;
        let old = Retained {
            texture: std::mem::replace(&mut self.target, texture),
            view: std::mem::replace(&mut self.view, view),
            copy_bind: std::mem::replace(&mut self.target_copy_bind, copy_bind),
            blit_bind: std::mem::replace(&mut self.blit_bind, blit_bind),
        };
        self.spare = Some(old);
        if self.blit_bind.is_none() {
            self.blit_bind = self
                .blit
                .as_ref()
                .map(|p| blit_group(&self.device, p, &self.view));
        }
    }
    /// Encode `draws` clipped to `region`. Consecutive draws sharing a
    /// texture and clip become one instanced call; their quads are contiguous,
    /// so paint order holds. A draw entirely outside the region may be folded
    /// into a batch: the scissor discards its quads.
    /// Everything `item`'s layer shader can read of what's behind it, in this
    /// target's space, if it reads it and shows: all of this frame's repaint
    /// if any of it does (`blur_damage`), so no pixel it reads holds its own
    /// last frame. And the same from the box's top-left, as it asks for it.
    fn backdrop_read(
        &self,
        scene: &Scene,
        item: &zgx::scene::PaintItem<'_>,
        viewport: Rect,
    ) -> Option<(Rect, [f32; 4])> {
        let layer = scene.layer_shader(item.id)?;
        if !layer.reads_backdrop() {
            return None;
        }
        let output = item.bounds.expand(layer.bleed);
        let clip = item.clip.unwrap_or(viewport);
        // A read at the edge of what it can read filters with the pixel past
        // it: that's copied too.
        let margin = 1. / self.scale;
        let from_box = |read: Rect| {
            [
                read.x - item.bounds.x,
                read.y - item.bounds.y,
                read.x + read.width - item.bounds.x,
                read.y + read.height - item.bounds.y,
            ]
        };
        // A box shadow grows the layer its composite replaces past what the
        // shader reads: what's under all of it is copied.
        let shadowed = matches!(
            item.kind,
            NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } if !style.shadows().is_empty()
        );
        let Some(matrix) = item.projective() else {
            // What's clipped away is never asked for, however large the box.
            let read = output
                .intersection(clip)?
                .expand(layer.reach)
                .intersection(viewport)?;
            let copied = match shadowed {
                true => self.under_shadowed_layer(item, read, None, viewport),
                false => read,
            };
            return Some((copied.expand(margin), from_box(read)));
        };
        use zgx::affine::{Affine, Projective};
        let (x, y) = self.layer_origin;
        let matrix = Projective::from(Affine::translation(x, y))
            .then(matrix)
            .then(Projective::from(Affine::translation(-x, -y)));
        matrix.bounds(output).intersection(clip)?;
        let whole = output.expand(layer.reach);
        let read = matrix.bounds(whole).intersection(viewport)?;
        let copied = match shadowed {
            true => self.under_shadowed_layer(item, read, Some(&matrix), viewport),
            false => read,
        };
        Some((copied.expand(margin), from_box(whole)))
    }
    /// `read` grown to all a shadowed backdrop reader's layer covers (in this
    /// target's space, through `matrix` if it's transformed), within
    /// `viewport`: its composite replaces every pixel of it.
    #[cold]
    #[inline(never)]
    fn under_shadowed_layer(
        &self,
        item: &zgx::scene::PaintItem<'_>,
        read: Rect,
        matrix: Option<&zgx::affine::Projective>,
        viewport: Rect,
    ) -> Rect {
        let Some(layer) = self.layers.get(&item.id) else {
            return read;
        };
        let bounds = Rect::new(
            layer.bounds.x - self.layer_origin.0,
            layer.bounds.y - self.layer_origin.1,
            layer.bounds.width,
            layer.bounds.height,
        );
        let bounds = matrix.map_or(bounds, |matrix| matrix.bounds(bounds));
        bounds
            .intersection(viewport)
            .map_or(read, |bounds| read.union(bounds))
    }
    /// Copy what the target holds under blended `draw`, within `regions`,
    /// into the backdrop for its size (and reading the backdrop, `read`
    /// around it); the backdrop's bind group.
    fn copy_backdrop(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        draw: &Draw,
        read: Option<Rect>,
        regions: &[(Rect, Scissor)],
    ) -> wgpu::BindGroup {
        let under = match read {
            Some(read) => Some(read),
            None => draw.bounds.intersection(draw.clip),
        };
        let area = regions
            .iter()
            .filter_map(|(region, _)| region.intersection(under?))
            .reduce(Rect::union);
        let copied = area.and_then(|area| scissor(area, self.scale, self.width, self.height));
        let target = self.target.clone();
        let (texture, bind) = self.backdrop();
        if let Some((x, y, width, height)) = copied {
            let at = |texture| wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            };
            encoder.copy_texture_to_texture(
                at(&target),
                at(texture),
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        bind.clone()
    }
    /// Draw blended `draw` (a layer's composite) within `region`, reading
    /// the backdrop `bind`: its own pipeline, replacing the target's pixels.
    fn encode_blended(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        region: Rect,
        draw: &Draw,
        bind: &wgpu::BindGroup,
        stats: &mut GpuStats,
    ) {
        let Some(pipeline) = self.fill_pipeline(draw.shading) else {
            return;
        };
        let Some((x, y, w, h)) = region
            .intersection(draw.clip)
            .and_then(|rect| rect.intersection(draw.bounds))
            .and_then(|rect| scissor(rect, self.scale, self.width, self.height))
        else {
            return;
        };
        pass.set_pipeline(pipeline);
        pass.set_scissor_rect(x, y, w, h);
        pass.set_bind_group(0, self.source_bind(draw.source), &[]);
        pass.set_bind_group(2, bind, &[]);
        if !self.chains.is_empty() {
            self.bind_inputs(pass, draw.shading);
        }
        pass.draw(0..4, draw.start..draw.end);
        stats.draw_calls += 1;
        stats.instances += (draw.end - draw.start) as usize;
    }
    #[allow(clippy::too_many_arguments)]
    fn encode_draws<'p>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &dyn Fn(Shading) -> &'p wgpu::RenderPipeline,
        region: Rect,
        draws: &[Draw],
        first: u32,
        kept: Option<u32>,
        stats: &mut GpuStats,
    ) {
        // The pipeline set on `pass`: the caller's, until a batch sets one.
        let mut current = None;
        self.batches(region, draws, first, kept, |batch| {
            self.encode_batch(pass, pipelines, &mut current, batch, stats)
        });
    }
    /// The draws within `region` from instance `first` on, as batches:
    /// consecutive draws sharing a clip, texture source and shading join.
    /// `kept`: the kept frame's `kept_ordered` when these draws are the kept
    /// frame's, now or deferred (`GpuRenderer::kept_now`, `Deferred::kept`):
    /// past it their instances are out of paint order, which the kept rules
    /// allow for.
    fn batches(
        &self,
        region: Rect,
        draws: &[Draw],
        first: u32,
        kept: Option<u32>,
        emit: impl FnMut(Batch),
    ) {
        match kept {
            Some(ordered) => self.batches_of::<true>(region, draws, first, ordered, emit),
            None => self.batches_of::<false>(region, draws, first, 0, emit),
        }
    }
    /// The kept frame's `kept_ordered` while it is drawn from (`batches`).
    pub(crate) fn kept_now(&self) -> Option<u32> {
        self.kept_upload.is_some().then_some(self.kept_ordered)
    }
    /// `batches`, of the kept frame's draws (`KEPT`) or a frame's.
    #[inline(always)]
    fn batches_of<const KEPT: bool>(
        &self,
        region: Rect,
        draws: &[Draw],
        first: u32,
        ordered: u32,
        mut emit: impl FnMut(Batch),
    ) {
        let mut batch: Option<Batch> = None;
        // Skipping blending pays for its extra draws only over many pixels:
        // a small region (a scrolled-in strip) blends plain quads in one draw.
        let pixels = region.width * region.height * self.scale * self.scale;
        let unblended = pixels >= SPLIT_PIXELS;
        for draw in draws {
            // (Kept frames: most draws miss the region, so before the shading.)
            if KEPT && (!region.intersects(draw.bounds) || draw.end <= draw.start.max(first)) {
                continue;
            }
            let shading = match draw.shading {
                Shading::Opaque if !unblended => Shading::Basic,
                // Drawn alone, after their backdrop's copy (`encode_blended`).
                // What comes after them starts anew: joined to what's before,
                // one call would span their quads too.
                Shading::Blend | Shading::FilterBlend(_) => {
                    if let Some(done) = batch.take() {
                        emit(done);
                    }
                    continue;
                }
                shading => shading,
            };
            if !KEPT && (!region.intersects(draw.bounds) || draw.end <= draw.start.max(first)) {
                continue;
            }
            let start = draw.start.max(first);
            let Some(rect) = region.intersection(draw.clip) else {
                if let Some(done) = batch.take() {
                    emit(done);
                }
                continue;
            };
            let source = draw.source;
            let pixels = draw
                .bounds
                .intersection(rect)
                .map_or(0., |covered| covered.width * covered.height)
                * self.scale
                * self.scale;
            if let Some(open) = &mut batch
                && (open.rect == rect
                    || open.shading == shading
                        && start == open.end
                        && scissors_alike(open.rect, rect, draw))
                && open.source == source
                // From the kept frame, the instances it would span to this
                // draw (draws outside `region`, a kept text's spare room),
                // set up by the GPU to be scissored away: under
                // `SKIPPED_SPLIT` (past it, a draw call costs less), and only
                // below those in paint order (past them lie texts moved
                // there, painted elsewhere in order).
                && (!KEPT
                    || start == open.end
                    || start > open.end
                        && start - open.end < SKIPPED_SPLIT
                        && draw.end <= ordered)
            {
                // Another shading joins when what it promotes (quads drawn
                // with a costlier shader than theirs) stays small.
                let merged = match open.shading == shading {
                    true => Some((shading, open.promoted)),
                    false => match merged_shading(open.shading, shading) {
                        Some(merged) => {
                            let promoted = if merged == open.shading {
                                open.promoted
                            } else {
                                open.pixels
                            } + if merged == shading { 0. } else { pixels };
                            let budget = match merged {
                                Shading::Full => PROMOTED_FULL_PIXELS,
                                _ => PROMOTED_PIXELS,
                            };
                            (promoted <= budget).then_some((merged, promoted))
                        }
                        None => None,
                    },
                };
                if let Some((merged, promoted)) = merged {
                    // A batch draws one run of instances: a draw joining it
                    // starts at or past its end (past it: draws between
                    // lay outside the region, scissored away). One placed
                    // before would cut the batch short, and both would draw
                    // nothing.
                    debug_assert!(
                        start >= open.end,
                        "draw {}..{} joins a batch {}..{} out of order",
                        start,
                        draw.end,
                        open.start,
                        open.end
                    );
                    open.shading = merged;
                    open.promoted = promoted;
                    open.pixels += pixels;
                    open.end = draw.end;
                    continue;
                }
            }
            if let Some(done) = batch.take() {
                emit(done);
            }
            batch = Some(Batch {
                rect,
                source,
                shading,
                start,
                end: draw.end,
                pixels,
                promoted: 0.,
            });
        }
        if let Some(done) = batch.take() {
            emit(done);
        }
    }
    fn encode_batch<'p>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &dyn Fn(Shading) -> &'p wgpu::RenderPipeline,
        current: &mut Option<Shading>,
        batch: Batch,
        stats: &mut GpuStats,
    ) {
        let Some((x, y, w, h)) = scissor(batch.rect, self.scale, self.width, self.height) else {
            return;
        };
        if *current != Some(batch.shading) {
            pass.set_pipeline(pipelines(batch.shading));
            *current = Some(batch.shading);
            if !self.chains.is_empty() {
                self.bind_inputs(pass, batch.shading);
            }
        }
        pass.set_scissor_rect(x, y, w, h);
        pass.set_bind_group(0, self.source_bind(batch.source), &[]);
        pass.draw(0..4, batch.start..batch.end);
        stats.draw_calls += 1;
        stats.instances += (batch.end - batch.start) as usize;
    }
    /// The bind group of what `source` samples (the atlas's where it is gone).
    #[inline(always)]
    fn source_bind(&self, source: TextureSource) -> &wgpu::BindGroup {
        match source {
            TextureSource::Atlas => &self.bind,
            TextureSource::Image(id) => self.images.get(&id).map_or(&self.bind, |entry| &entry.0),
            TextureSource::Pixelated(id) => self
                .nearest
                .as_ref()
                .and_then(|nearest| nearest.binds.get(&id))
                .map(|(bind, _)| bind)
                .or_else(|| self.images.get(&id).map(|entry| &entry.0))
                .unwrap_or(&self.bind),
            TextureSource::Layer(id) => {
                self.layers.get(&id).map_or(&self.bind, |layer| &layer.bind)
            }
        }
    }
    /// Plan a batch of backdrop filters (see blur.wgsl): each one's levels
    /// as slots of the atlas, packed, the atlas grown to hold them.
    fn plan_blur(&mut self, filters: &[Filter]) -> Option<BlurPlan> {
        // Mixed over the target by dual-source blending, unless drawn at
        // presentation into two targets at once (which it cannot be): then a
        // copy of each area's pixels to mix with.
        let mixed = self
            .device
            .features()
            .contains(wgpu::Features::DUAL_SOURCE_BLENDING)
            && self.single_pass_format.is_none();
        let scale = self.scale;
        let mut blurs = Vec::new();
        // Each slot with its size in texels and whether it is blurred (else
        // a copy of an area).
        let mut slots: Vec<(SlotRecord, (u32, u32), bool)> = Vec::new();
        let mut draws = Vec::new();
        for filter in filters {
            let Some((ax, ay, aw, ah)) = filter
                .bounds
                .intersection(filter.clip)
                .and_then(|area| scissor(area, scale, self.width, self.height))
            else {
                continue;
            };
            let index = blurs.len() as u32;
            let before = draws.len();
            draws.extend(
                filter
                    .outputs
                    .iter()
                    .filter_map(|output| scissor(*output, scale, self.width, self.height))
                    .map(|output| (index, output)),
            );
            if draws.len() == before {
                continue;
            }
            let sigma = (filter.effects.blur_radius * scale).clamp(0.1, 64.);
            // Finite and non-negative (`Scene::set_effects`).
            let fade = (filter.effects.edge_fade * scale).clamp(0., 1e6);
            // Sigma, and under a fade, each halving of it down to 2 px (below
            // which the backdrop shows through, the blur all but invisible).
            let levels: Vec<f32> = if fade > 0. {
                std::iter::successors(Some(sigma), |s| (s / 2. >= 2.).then_some(s / 2.)).collect()
            } else {
                vec![sigma]
            };
            let area = [ax as f32, ay as f32, (ax + aw) as f32, (ay + ah) as f32];
            let bounds = [
                filter.bounds.x * scale,
                filter.bounds.y * scale,
                filter.bounds.width * scale,
                filter.bounds.height * scale,
            ];
            let edges = [
                bounds[0],
                bounds[1],
                bounds[0] + bounds[2],
                bounds[1] + bounds[3],
            ];
            let first = slots.len() as u32;
            for (k, &level) in levels.iter().enumerate() {
                // Blurred where sigma spans 1.5 to 3 texels: a 16 px blur on a
                // 2x display (sigma 32) runs at an eighth of the size (4
                // texels), sigma 4 at half. Below sigma 3, the full size.
                let factor = (level / 1.5).log2().floor().clamp(0., 3.).exp2();
                // The pixels whose sigma lies within an octave of this
                // level's (see `blurred`), and as far again as its taps reach.
                let region = if levels.len() == 1 {
                    Some(area)
                } else {
                    // Sigma at distance d in: sigma * (d / fade)^2.
                    let at = |octaves: i32| fade * 2_f32.powi(-octaves).sqrt();
                    let near = if k + 1 == levels.len() {
                        0.
                    } else {
                        at(k as i32 + 1)
                    };
                    let far = if k == 0 {
                        f32::INFINITY
                    } else {
                        at(k as i32 - 1)
                    };
                    band(area, edges, near, far)
                };
                let reach = ((3. * level / factor).ceil() + 1.) * factor;
                let [x0, y0, x1, y1] = region.map_or([area[0], area[1], area[0], area[1]], |r| {
                    [
                        (r[0] - reach).max(area[0]).floor(),
                        (r[1] - reach).max(area[1]).floor(),
                        (r[2] + reach).min(area[2]).ceil(),
                        (r[3] + reach).min(area[3]).ceil(),
                    ]
                });
                let size = (
                    ((x1 - x0) / factor).ceil() as u32,
                    ((y1 - y0) / factor).ceil() as u32,
                );
                slots.push((
                    SlotRecord {
                        atlas: [0., 0., size.0 as f32, size.1 as f32],
                        region: [x0, y0, factor, level / factor],
                        owner: [index, 0, 0, 0],
                    },
                    size,
                    true,
                ));
            }
            let original = if mixed {
                u32::MAX
            } else {
                slots.push((
                    SlotRecord {
                        atlas: [0., 0., aw as f32, ah as f32],
                        region: [area[0], area[1], 1., 0.],
                        owner: [index, 0, 0, 0],
                    },
                    (aw, ah),
                    false,
                ));
                slots.len() as u32 - 1
            };
            blurs.push(BlurRecord {
                bounds,
                effect: [
                    filter.effects.opacity,
                    fade,
                    sigma,
                    *levels.last().expect("one level"),
                ],
                area,
                shape: filter.corners.map(|radius| radius * scale),
                slots: [first, levels.len() as u32, original, 0],
                round0: round_box(filter.rounded[0], scale).0,
                radii0: round_box(filter.rounded[0], scale).1,
                round1: round_box(filter.rounded[1], scale).0,
                radii1: round_box(filter.rounded[1], scale).1,
            });
        }
        if blurs.is_empty() {
            return None;
        }
        // Shelves as wide as the target, tallest slots first.
        let mut order: Vec<usize> = (0..slots.len()).collect();
        order.sort_by_key(|&k| std::cmp::Reverse(slots[k].1.1));
        let shelf_width = slots.iter().map(|slot| slot.1.0).fold(self.width, u32::max);
        let (mut x, mut y, mut shelf, mut width) = (0, 0, 0, 1);
        for k in order {
            let (w, h) = slots[k].1;
            if x + w > shelf_width {
                (x, y, shelf) = (0, y + shelf, 0);
            }
            slots[k].0.atlas[0] = x as f32;
            slots[k].0.atlas[1] = y as f32;
            x += w;
            shelf = shelf.max(h);
            width = width.max(x);
        }
        let height = (y + shelf).max(1);
        // Grown in steps so a slowly growing panel does not reallocate
        // every frame.
        let limit = self.device.limits().max_texture_dimension_2d;
        if width > limit || height > limit {
            return None;
        }
        let atlas = self.blur_scratch.take().map(|scratch| scratch.atlas);
        let atlas = [0, 1].map(|i| match &atlas {
            Some(atlas) if atlas[i].width() >= width && atlas[i].height() >= height => {
                atlas[i].clone()
            }
            current => {
                let (current_w, current_h) = current
                    .as_ref()
                    .map_or((0, 0), |a| (a[i].width(), a[i].height()));
                let grow = |want: u32, have: u32| want.max(have).next_multiple_of(64).min(limit);
                atlas_texture(
                    &self.device,
                    grow(width, current_w),
                    grow(height, current_h),
                )
            }
        });
        self.blur_scratch = Some(BlurScratch { atlas });
        Some(BlurPlan {
            blurs,
            slots,
            draws,
            mixed,
        })
    }
    /// Blur a batch of backdrop filters (see `plan_blur`): the reduce,
    /// across and down passes, every slot of the batch drawn in each. Its
    /// records lie in `buffer` (offset and size of the filters', then the
    /// slots').
    fn blur(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        plan: &BlurPlan,
        buffer: &wgpu::Buffer,
        ranges: [(u64, u64); 2],
        stats: &mut GpuStats,
    ) -> Option<VerticalBlur> {
        let BlurPlan {
            slots,
            draws,
            mixed,
            ..
        } = plan;
        let mixed = *mixed;
        if self.blur_pipelines.is_none() {
            self.blur_pipelines = Some(BlurPipelines::new(&self.device));
        }
        let atlas = &self.blur_scratch.as_ref().expect("planned").atlas;
        let views = atlas
            .each_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let records = |(offset, size): (u64, u64)| {
            wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset,
                size: wgpu::BufferSize::new(size),
            })
        };
        let pipelines = self.blur_pipelines.as_ref().expect("created above");
        // Binding 1 goes unread except by the copy mix, but may never be the
        // pass's own target.
        let bind = |source: &wgpu::TextureView, other: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("backdrop blur"),
                layout: &pipelines.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(other),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: records(ranges[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: records(ranges[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&pipelines.linear),
                    },
                ],
            })
        };
        // Whole-area passes: what any output draws depends on the area alone,
        // so a partly damaged filter matches a fresh frame. Copies land in
        // the first atlas with the averages and stay there.
        let copies = slots.iter().any(|slot| !slot.2);
        // A popup blurs its window's frame (`set_backdrop`), but what it mixes
        // the blur over stays its own: past a rounded box's curve, its target
        // (its shadow), not the window's pixels.
        let backdrop = self.backdrop.as_ref().map(|(_, view)| view);
        let source = backdrop.unwrap_or(&self.view);
        let passes = [
            (
                "blur reduce",
                &pipelines.reduce,
                bind(source, &views[1]),
                &views[0],
                backdrop.is_some(),
                false,
            ),
            (
                "blur across",
                &pipelines.across,
                bind(&views[0], &views[0]),
                &views[1],
                true,
                false,
            ),
            (
                "blur down",
                &pipelines.down,
                bind(&views[1], &views[1]),
                &views[0],
                true,
                copies,
            ),
        ];
        for (label, pipeline, group, target, blurred_only, keep) in &passes {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if *keep {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            for (k, (record, (w, h), blurred)) in slots.iter().enumerate() {
                if *w == 0 || *h == 0 || (*blurred_only && !blurred) {
                    continue;
                }
                pass.set_scissor_rect(record.atlas[0] as u32, record.atlas[1] as u32, *w, *h);
                pass.draw(0..3, k as u32..k as u32 + 1);
                stats.draw_calls += 1;
            }
        }
        stats.render_passes += passes.len();
        stats.blur_passes += passes.len();
        if backdrop.is_some() && copies {
            let group = bind(&self.view, &views[1]);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur originals"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[0],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.reduce);
            pass.set_bind_group(0, &group, &[]);
            for (k, (record, (w, h), blurred)) in slots.iter().enumerate() {
                if *w == 0 || *h == 0 || *blurred {
                    continue;
                }
                pass.set_scissor_rect(record.atlas[0] as u32, record.atlas[1] as u32, *w, *h);
                pass.draw(0..3, k as u32..k as u32 + 1);
                stats.draw_calls += 1;
            }
            drop(pass);
            stats.render_passes += 1;
            stats.blur_passes += 1;
        }
        Some(VerticalBlur {
            bind: bind(&views[0], &views[0]),
            draws: draws.clone(),
            mixed,
        })
    }
    /// Draw a batch of filters back over the target, at the start of the
    /// pass that follows their blur passes.
    fn encode_vertical(
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        blur: &VerticalBlur,
        stats: &mut GpuStats,
    ) {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &blur.bind, &[]);
        for &(index, (x, y, width, height)) in &blur.draws {
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..3, index..index + 1);
            stats.draw_calls += 1;
        }
    }
    /// Record `id` as `node`'s raster, dropping the texture it replaces.
    fn retire_node_image(&mut self, node: NodeId, id: u64) {
        if let Some(old) = self.node_images.insert(node, id)
            && old != id
        {
            self.images.remove(&old);
        }
    }
    /// What node `id`'s draw of `image` samples: the image, or, pixelated
    /// (`Scene::image_rendering`), its nearest texel (`Pixelated`), with a
    /// bind group of its own for wgpu (`nearest_bind`). An image of no
    /// texture (a GPU view's frame) samples as it does.
    #[inline(always)]
    fn image_source(&mut self, scene: &Scene, id: NodeId, image: u64) -> TextureSource {
        if scene.has_image_rendering()
            && scene.image_rendering(id) == zgx::image::ImageRendering::Pixelated
            && self.nearest_bind(image)
        {
            return TextureSource::Pixelated(image);
        }
        TextureSource::Image(image)
    }
    /// A bind group sampling `image`'s texture at its nearest texel, made
    /// for it once and again when its texture is replaced; `false` when it
    /// has no texture.
    #[cold]
    #[inline(never)]
    fn nearest_bind(&mut self, image: u64) -> bool {
        let Some(texture) = self.images.get(&image).and_then(|entry| entry.2.clone()) else {
            return false;
        };
        let device = self.device.clone();
        let nearest = self.nearest.get_or_insert_with(|| {
            Box::new(NearestBinds {
                sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("pixelated images"),
                    mag_filter: wgpu::FilterMode::Nearest,
                    min_filter: wgpu::FilterMode::Nearest,
                    ..Default::default()
                }),
                binds: FxHashMap::default(),
            })
        });
        if nearest
            .binds
            .get(&image)
            .is_some_and(|(_, made)| *made == texture)
        {
            return true;
        }
        let view = texture.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pixelated image"),
            layout: &self.context.inner.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&nearest.sampler),
                },
            ],
        });
        nearest.binds.insert(image, (bind, texture));
        true
    }
    /// A dashed or dotted outline (`Scene::outline_style`) of `node`, a box
    /// of `style` at `bounds`: its raster's texture (`OutlineRasters`, apart
    /// from the node's decoration's) and the quad drawing it over the
    /// outline's outer box. Out of line: solid outlines are a ring quad.
    #[cold]
    #[inline(never)]
    fn styled_outline(
        &mut self,
        (node, style, outline): (NodeId, &zgx::scene::QuadStyle, &zgx::scene::Outline),
        outline_style: zgx::decoration::BorderStyle,
        bounds: Rect,
        (opacity, edge_fade): (f32, f32),
        stats: &mut GpuStats,
    ) -> Result<(u64, Quad), GpuError> {
        let outer = outline.outer(bounds);
        let canvas = Arc::new(decoration::outline_canvas(
            style,
            outline,
            outline_style,
            bounds.width,
            bounds.height,
        )?);
        let scale = self.scale;
        let rasters = self.outlines.get_or_insert_default();
        let mut found = rasters
            .cache
            .get(node, &canvas, outer.width, outer.height, scale)?;
        let old = rasters.images.insert(node, found.id);
        if let Some(old) = old.filter(|old| *old != found.id) {
            self.images.remove(&old);
        }
        if !self.images.contains_key(&found.id) {
            if found.pixels.is_none() {
                let rasters = self.outlines.get_or_insert_default();
                rasters.cache.forget(node);
                found = rasters
                    .cache
                    .get(node, &canvas, outer.width, outer.height, scale)?;
            }
            let pixels = found.pixels.expect("fresh raster");
            self.upload_image(&pixels)?;
            stats.image_uploads += 1;
            stats.canvas_rasterizations += 1;
        }
        self.outlines.get_or_insert_default().cache.uploaded(node);
        let rect = [outer.x, outer.y, outer.width, outer.height];
        let quad = Quad {
            rect,
            uv: [0., 0., 1., 1.],
            color: [1., 1., 1., opacity],
            fade: rect,
            options: [edge_fade, 2., 0., 0.],
            shape: [0.; 4],
            border: [0.; 4],
            mask: NO_MASK,
        };
        Ok((found.id, quad))
    }
    /// Gradient text `node`'s gradient image (`Scene::text_gradient_image`),
    /// uploaded if it is new, the one it replaced let go: its id, or `None`
    /// to draw the text as plain text. Out of line: plain text never comes
    /// here.
    #[cold]
    #[inline(never)]
    fn gradient_text_image(
        &mut self,
        scene: &Scene,
        node: NodeId,
        stats: &mut GpuStats,
    ) -> Result<Option<u64>, GpuError> {
        let Some(image) = scene.text_gradient_image(node) else {
            return Ok(None);
        };
        self.retire_node_image(node, image.id());
        Ok(self.ensure_image(image, None, stats)?.then(|| image.id()))
    }
    /// The texture of `node`'s canvas raster, uploaded if it is new; the
    /// cache then keeps no pixels. A lost texture rasterizes again.
    fn canvas_texture(
        &mut self,
        node: NodeId,
        stats: &mut GpuStats,
        mut raster: impl FnMut(&mut canvas::CanvasCache) -> Result<canvas::Raster, GpuError>,
    ) -> Result<u64, GpuError> {
        let mut found = raster(&mut self.canvases)?;
        self.retire_node_image(node, found.id);
        if !self.images.contains_key(&found.id) {
            if found.pixels.is_none() {
                self.canvases.forget(node);
                found = raster(&mut self.canvases)?;
            }
            let pixels = found.pixels.expect("fresh raster");
            self.upload_image(&pixels)?;
            stats.image_uploads += 1;
        }
        self.canvases.uploaded(node);
        Ok(found.id)
    }
    /// `ensure_image` for `image` drawn as what `node` shows, counted while
    /// it shows it (`drew_image`). Out of line: the frame's draw loop calls
    /// it where it called `ensure_image`, and runs as it did.
    #[inline(never)]
    fn ensure_shown_image(
        &mut self,
        node: NodeId,
        image: &zgx::image::ImageData,
        drawn: Option<(f32, f32)>,
        stats: &mut GpuStats,
    ) -> Result<bool, GpuError> {
        self.drew_image(node, image.id());
        self.ensure_image(image, drawn, stats)
    }
    /// `drew_image` for a native surface's frame. Out of line, as
    /// `ensure_shown_image`: the draw loop runs as it did.
    #[cfg(target_os = "macos")]
    #[inline(never)]
    fn drew_surface(&mut self, node: NodeId, frame: u64) {
        self.drew_image(node, frame);
    }
    /// `node` drew `image`: counted while it shows it (`SceneImages`), as a
    /// node appended or given an image isn't named by the scene's changes.
    #[inline]
    fn drew_image(&mut self, node: NodeId, image: u64) {
        if let Some(images) = &mut self.scene_images
            && images.counted
            && images.by_node.get(&node) != Some(&image)
        {
            images.show(node, Some(image));
        }
    }
    /// `image`'s texture for this frame: uploaded if it has none, and
    /// marked drawn now, so it is the last to go past the budget. `false`:
    /// no texture to be had (`IMAGE_MISSES`): this frame goes on without it.
    /// `drawn`: its size where it is drawn now, in device pixels (`None`:
    /// all of it). A texture holds it at that size (`fitted_size`): a photo
    /// in a card takes the card's pixels, not the photo's, and is uploaded
    /// again only as it is drawn larger.
    fn ensure_image(
        &mut self,
        image: &zgx::image::ImageData,
        drawn: Option<(f32, f32)>,
        stats: &mut GpuStats,
    ) -> Result<bool, GpuError> {
        let size = fitted_size(image.width(), image.height(), drawn);
        if let Some(entry) = self.images.get(&image.id())
            && (entry.3 == (0, 0) || (entry.3.0 >= size.0 && entry.3.1 >= size.1))
        {
            if let Some(drawn) = self.image_drawn.get_mut(&image.id()) {
                *drawn = self.image_frame;
            }
            return Ok(true);
        }
        // Held smaller than drawn, the budget's doing: uploaded again only
        // if there's now room for more, not on every frame.
        let outgoing =
            !self.images_outgoing.is_empty() && self.images_outgoing.contains_key(&image.id());
        if let Some(entry) = self.images.get(&image.id())
            && !outgoing
        {
            let held = entry.3;
            let others = (self.images.iter())
                .filter(|(id, _)| **id != image.id())
                .filter(|(id, _)| {
                    self.image_drawn
                        .get(id)
                        .is_none_or(|drawn| *drawn >= self.image_frame)
                })
                .map(|(_, entry)| entry.1)
                .sum::<usize>();
            let fits = self.image_size_within(image, size, others);
            if fits.0 <= held.0 && fits.1 <= held.1 {
                if let Some(drawn) = self.image_drawn.get_mut(&image.id()) {
                    *drawn = self.image_frame;
                }
                return Ok(true);
            }
        }
        match self.upload_image_at(image, size) {
            Ok(()) => {}
            // Its pixels decoding: drawn once they are, or as it was till then.
            Err(error) if error.0 == IMAGE_PENDING => {
                if outgoing && let Some(drawn) = self.image_drawn.get_mut(&image.id()) {
                    *drawn = self.image_frame;
                }
                return Ok(outgoing);
            }
            Err(error) if IMAGE_MISSES.contains(&error.0.as_str()) => {
                self.warn(&format!("{}: not drawn", error.0));
                return Ok(false);
            }
            Err(error) => return Err(error),
        }
        stats.image_uploads += 1;
        Ok(true)
    }
    fn upload_image(&mut self, image: &zgx::image::ImageData) -> Result<(), GpuError> {
        self.upload_image_at(image, (image.width().max(1), image.height().max(1)))
    }
    /// The bytes image textures may hold: four of this window's frames, and
    /// at least `IMAGE_TEXTURES`. With images held at the size they are
    /// drawn, what shows at once is bounded by the window, not the files.
    fn image_budget(&self) -> usize {
        (self.width as usize * self.height as usize * 4 * 4).max(IMAGE_TEXTURES)
    }
    /// Whether a frame's budget has anything to share (`images_to_share`).
    fn images_share(&mut self) {
        self.images_to_share = !self.images_softer.is_empty()
            || !self.images_awaiting.is_empty()
            || !self.images_outgoing.is_empty();
    }
    /// What images drawn last frame, or waiting on their pixels then, leave of
    /// the budget, shared: each takes what it wants up to an even share of what
    /// the smaller leave. They come to this over a frame or two, as they upload
    /// again, and only while some are held softer or waiting: otherwise no image
    /// pays for it. (Left out, those waiting would leave the rest looking as if
    /// they fit: uploaded whole, to go again once those arrive.)
    #[cold]
    fn share_images(&mut self) {
        // On their way out and not asked for last frame (which drew them all:
        // `repaint_all`, as they were marked): nothing to wait for.
        if !self.images_outgoing.is_empty() {
            let asked = self.image_frame - 1;
            let (images, drawn) = (&mut self.images, &self.image_drawn);
            self.images_outgoing.retain(|id, _| {
                let keep = drawn.get(id).is_some_and(|at| *at >= asked);
                if !keep {
                    images.remove(id);
                }
                keep
            });
            let images = &self.images;
            self.image_drawn.retain(|id, _| images.contains_key(id));
        }
        let images = &self.images;
        self.images_softer.retain(|id, _| images.contains_key(id));
        // (The last frame that drew them: one kept draws none anew.)
        let last = self.image_drawn.values().copied().max().unwrap_or(0);
        let awaiting = &self.images_awaiting;
        let mut wants: Vec<usize> = (self.images.iter())
            .filter(|(id, _)| !awaiting.contains_key(*id))
            .filter(|(id, _)| self.image_drawn.get(id).is_some_and(|drawn| *drawn == last))
            .map(|(id, entry)| self.images_softer.get(id).copied().unwrap_or(entry.1))
            .chain(awaiting.values().copied())
            .collect();
        wants.sort_unstable();
        let mut left = self.image_budget();
        let mut share = usize::MAX;
        for (at, want) in wants.iter().enumerate() {
            let even = left / (wants.len() - at);
            if *want > even {
                // (A little under, as sizes round up.)
                share = even / 100 * 97;
                break;
            }
            left -= want;
        }
        let was = self.image_share;
        let moved = match (was, share) {
            (usize::MAX, usize::MAX) => false,
            (usize::MAX, _) | (_, usize::MAX) => true,
            _ => was.abs_diff(share) > was / 20,
        };
        if !moved {
            return;
        }
        self.image_share = share;
        // Those over it upload again as they're drawn, as all may take more:
        // drawn as they are until then (`images_outgoing`). And those it now
        // leaves room for stay as they were.
        let limit = share.saturating_add(share / 20);
        for (id, entry) in &mut self.images {
            if entry.1 > limit && entry.3 != IMAGE_OUTGOING {
                self.images_outgoing.insert(*id, entry.3);
                entry.3 = IMAGE_OUTGOING;
            } else if entry.1 <= limit && entry.3 == IMAGE_OUTGOING {
                entry.3 = self.images_outgoing.remove(id).unwrap_or((0, 0));
            }
        }
        self.repaint_all();
    }
    /// `wanted` under the texture limit, its shape kept.
    fn image_size_limited(&self, image: &zgx::image::ImageData, wanted: (u32, u32)) -> (u32, u32) {
        let limit = self.device.limits().max_texture_dimension_2d;
        let mut scale = 1f32;
        loop {
            let (w, h) = scaled_size(wanted, image, scale);
            if (w <= limit && h <= limit) || (w, h) == (1, 1) {
                return (w, h);
            }
            scale *= 0.75;
        }
    }
    /// The size `image`'s texture takes, wanted at `wanted`, with `held` bytes
    /// of others staying: smaller, its shape kept, while past the texture
    /// limit, its share or the budget; never left out (`IMAGE_LEAST_SIDE`).
    fn image_size_within(
        &self,
        image: &zgx::image::ImageData,
        wanted: (u32, u32),
        held: usize,
    ) -> (u32, u32) {
        let budget = self.image_budget();
        let mut size = self.image_size_limited(image, wanted);
        let bytes = |(w, h): (u32, u32)| w as usize * h as usize * 4;
        if bytes(size) > self.image_share {
            let scale = (self.image_share as f32 / bytes(size) as f32).sqrt();
            size = scaled_size(size, image, scale.min(1.));
        }
        let mut scale = 1f32;
        loop {
            let at = scaled_size(size, image, scale);
            if bytes(at) + held <= budget || at.0.max(at.1) <= IMAGE_LEAST_SIDE {
                return at;
            }
            scale *= 0.75;
        }
    }
    /// The images the frame under way shows, or may: what a texture giving way
    /// must not be. Worked out only when one has to (past a budget), from the
    /// scene: an image's mark says when it last came through an upload, and a
    /// frame paints what it kept without one. Under a layer or a matrix, where
    /// it lands isn't read here, and counts as showing: spared needlessly, a
    /// new image is drawn softer, where evicted, one on screen would go blank.
    #[cold]
    #[inline(never)]
    fn images_showing(&self) -> rustc_hash::FxHashSet<u64> {
        let mut showing = rustc_hash::FxHashSet::default();
        // SAFETY: Set by `render` for as long as it runs, from a scene it
        // borrows throughout; nothing here keeps the reference.
        let Some(scene) = self.frame_scene.map(|scene| unsafe { scene.as_ref() }) else {
            return showing;
        };
        let window = scene.bounds(scene.root());
        for item in scene.paint_items() {
            let image = match item.kind {
                NodeKind::Image(image) => image.id(),
                #[cfg(target_os = "macos")]
                NodeKind::NativeSurface(frame) => frame.id(),
                NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } => {
                    match quad
                        .decoration
                        .as_deref()
                        .and_then(|d| d.background.as_ref())
                    {
                        Some(zgx::decoration::Background::Image { image, .. }) => image.id(),
                        _ => continue,
                    }
                }
                _ => continue,
            };
            let placed_elsewhere = item.projective().is_some()
                || std::iter::successors(scene.parent(item.id), |id| scene.parent(*id))
                    .any(|id| scene.is_layer(id));
            let shown = item.clip.unwrap_or(window).intersection(window);
            if placed_elsewhere
                || scene.is_layer(item.id)
                || shown.is_some_and(|shown| shown.intersection(item.bounds).is_some())
            {
                showing.insert(image);
            }
        }
        // What holds images of its own, as `prune` counts them.
        showing.extend(self.canvases.image_ids());
        showing.extend(self.svgs.image_ids());
        showing.extend(
            (self.node_ramps.values())
                .filter_map(|key| self.ramps.get(key))
                .map(|ramp| ramp.id()),
        );
        showing.insert(effects::EFFECT_ATLAS);
        showing.extend(
            self.layers
                .keys()
                .filter(|id| scene.contains(**id))
                .filter_map(|id| scene.mask(*id).map(|mask| mask.image.id())),
        );
        showing
    }
    /// `upload_image` at `size` (`fitted_size`), or smaller where the budget
    /// or the texture limit says: drawn softer rather than left out.
    fn upload_image_at(
        &mut self,
        image: &zgx::image::ImageData,
        wanted: (u32, u32),
    ) -> Result<(), GpuError> {
        if let Some(frame) = image.gpu_frame() {
            return self.render_view(image, frame);
        }
        let budget = self.image_budget();
        let limit = self.device.limits().max_texture_dimension_2d;
        let (full_w, full_h) = (image.width().max(1), image.height().max(1));
        let bytes = |(w, h): (u32, u32)| w as usize * h as usize * 4;
        // An image it replaces at another size gives its bytes back.
        let replaced = self.images.get(&image.id()).map_or(0, |entry| entry.1);
        // (Nor those on their way out, whose replacements count as they come.)
        let outgoing: usize = (self.images_outgoing.keys())
            .filter(|id| **id != image.id())
            .filter_map(|id| self.images.get(id))
            .map(|entry| entry.1)
            .sum();
        let mut held =
            self.images.values().map(|entry| entry.1).sum::<usize>() - replaced - outgoing
                + self.images_decoding;
        let whole = self.image_size_limited(image, wanted);
        let most = self.image_size_within(image, wanted, 0);
        if bytes(most) + held > budget {
            // Images not drawn this frame give their textures back, least
            // recently drawn first (a long page scrolled through): drawn
            // again, they upload again.
            let showing = self.images_showing();
            let mut idle: Vec<(u64, u64)> = self
                .image_drawn
                .iter()
                .filter(|(id, drawn)| **drawn < self.image_frame && !showing.contains(*id))
                .filter(|(id, _)| !self.images_outgoing.contains_key(*id))
                .map(|(id, drawn)| (*drawn, *id))
                .collect();
            idle.sort_unstable();
            for (_, id) in idle {
                if bytes(most) + held <= budget {
                    break;
                }
                if id == image.id() {
                    continue;
                }
                self.image_drawn.remove(&id);
                if let Some(entry) = self.images.remove(&id) {
                    held -= entry.1;
                }
            }
        }
        // Still past it: drawn smaller rather than left out.
        let (width, height) = self.image_size_within(image, wanted, held);
        if (width, height) != whole {
            let images = &self.images;
            self.images_softer.retain(|id, _| images.contains_key(id));
            self.images_softer.insert(image.id(), bytes(whole));
            self.images_to_share = true;
        } else if !self.images_softer.is_empty() {
            self.images_softer.remove(&image.id());
            self.images_share();
        }
        let wants_whole = bytes(whole);
        let bytes = bytes((width, height));
        if self.images.len() >= MAX_IMAGES && !self.images.contains_key(&image.id()) {
            // Images not drawn this frame give way, the longest gone first;
            // only as many as this drawn at once leave one out.
            let showing = self.images_showing();
            let oldest = (self.image_drawn.iter())
                .filter(|(id, drawn)| **drawn < self.image_frame && !showing.contains(*id))
                .min_by_key(|(_, drawn)| **drawn)
                .map(|(id, _)| *id);
            let Some(oldest) = oldest else {
                return Err(GpuError(IMAGE_MISSES[0].into()));
            };
            self.image_drawn.remove(&oldest);
            self.images.remove(&oldest);
        }
        if width > limit || height > limit {
            return Err(GpuError(IMAGE_MISSES[1].into()));
        }
        // A kept file's pixels at the size it takes, decoded off this thread:
        // until they are, it isn't drawn, and the host draws again once they
        // are (`take_images_awaited`). No larger than its texture will be.
        let source = match image.keeps_file() {
            true => match image.pixels_at((width, height)) {
                Some(source) => Some(source),
                None => {
                    self.images_awaited = true;
                    self.images_decoding += bytes;
                    self.images_awaiting.insert(image.id(), wants_whole);
                    self.images_to_share = true;
                    return Err(GpuError(IMAGE_PENDING.into()));
                }
            },
            false => None,
        };
        let texture = sampled_texture(&self.device, self.unified, width, height, "image");
        let (mut pixels, (mut w, mut h)) = match &source {
            // Premultiplied already, at least its size.
            Some(source) => (source.pixels.to_vec(), (source.width, source.height)),
            None => {
                let mut pixels = image.pixels().to_vec();
                // A frame read back from a renderer is premultiplied already.
                if !image.is_premultiplied() {
                    premultiply(&mut pixels);
                }
                (pixels, (full_w, full_h))
            }
        };
        drop(source);
        // Halved while still at least its size, averaging premultiplied
        // texels four to one (mip levels), then to its size exactly by area:
        // as sharp as it is drawn, without shimmer.
        while w.div_ceil(2) >= width && h.div_ceil(2) >= height && (w, h) != (1, 1) {
            (pixels, w, h) = halve(&pixels, w, h);
        }
        if (w, h) != (width, height) {
            pixels = shrink(&pixels, w, h, (width, height));
        }
        self.write_texels(&texture, (0, 0, width, height), &mut pixels);
        // Its decoded pixels may go now (`take_images_uploaded`).
        self.images_uploaded |= image.keeps_file();
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image"),
            layout: &self.context.inner.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.device.create_sampler(
                        &wgpu::SamplerDescriptor {
                            mag_filter: wgpu::FilterMode::Linear,
                            min_filter: wgpu::FilterMode::Linear,
                            ..Default::default()
                        },
                    )),
                },
            ],
        });
        let held_size = match (width, height) == (full_w, full_h) {
            true => (0, 0),
            false => (width, height),
        };
        self.images_outgoing.remove(&image.id());
        self.images
            .insert(image.id(), (bind, bytes, Some(texture), held_size));
        self.image_holders.insert(image.id(), image.holders());
        self.image_drawn.insert(image.id(), self.image_frame);
        Ok(())
    }
    /// Write premultiplied RGBA texels to `(x, y, width, height)` of a
    /// `sampled_texture`, which no submitted frame samples there.
    fn write_texels(
        &mut self,
        target: &wgpu::Texture,
        (x, y, width, height): (u32, u32, u32, u32),
        rgba: &mut [u8],
    ) {
        #[cfg(target_os = "macos")]
        if self.mapped.is_some() {
            upload::write_texels(target, (x, y, width, height), rgba);
            return;
        }
        // Direct3D 11: into its atlas, or the texture standing for an
        // image's, made at the shell's texture's size.
        #[cfg(d3d11)]
        if let Some(state) = &mut self.d3d11 {
            let size = (target.width(), target.height());
            let atlas = *target == self.atlas;
            let kept = if atlas {
                state.atlas.take()
            } else {
                state.textures.remove(target)
            };
            let surface = match kept.filter(|t| (t.width, t.height) == size) {
                Some(surface) => surface,
                None => match state.device.surface(size.0, size.1, false) {
                    Ok(made) => made,
                    Err(error) => return eprintln!("zgx: no Direct3D 11 texture: {error}"),
                },
            };
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            state.device.write(&surface, (x, y, width, height), rgba);
            if atlas {
                state.atlas = Some(surface);
            } else {
                state.textures.insert(target.clone(), surface);
            }
            return;
        }
        if self.unified {
            self.texture_uploads
                .push(target, (x, y, width, height), rgba);
        } else {
            // A copy keeps bytes as they are: in `FORMAT`'s order, BGRA (an
            // upload pass writes colours, which the target stores in its own).
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: target,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
    /// Before reusing atlas cells: the CPU writes them in place (macOS), so
    /// every frame that sampled the old ones must have finished.
    fn release_atlas_cells(&self) {
        if cfg!(target_os = "macos") && self.mapped.is_some() {
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        }
    }
    /// Start the glyph atlas over in place: every glyph and icon cell
    /// (`entries`, `image_cells`) given up together, so the next to take a
    /// cell never draws over one still listed. (A scale change once kept
    /// the icons' cells: back at a scale, an icon drew the glyphs written
    /// over it meanwhile.)
    fn recycle_atlas(&mut self) {
        self.release_atlas_cells();
        self.entries.clear();
        self.image_cells.clear();
        if let Some(blurred) = &mut self.blurred {
            blurred.glyphs.clear();
        }
        self.atlas_epoch = self.atlas_epoch.wrapping_add(1);
        self.cursor = (0, 0, 0);
    }
    fn reset_atlas(&mut self, size: u32) {
        self.atlas = sampled_texture(&self.device, self.unified, size, size, "glyph atlas");
        self.atlas_size = size;
        self.bind = self.texture_bind(&self.atlas.create_view(&Default::default()), "glyph atlas");
        self.entries.clear();
        self.image_cells.clear();
        if let Some(blurred) = &mut self.blurred {
            blurred.glyphs.clear();
        }
        self.cursor = (0, 0, 0);
        self.atlas_epoch = self.atlas_epoch.wrapping_add(1);
    }
    /// The quads of a run's underline or strikethrough, as its `LineStyle`
    /// says: pieces of the fill's own shapes, and for a wave a tile in the
    /// atlas, repeated, tinted as an icon is. Nothing a frame draws differs
    /// for a solid line, nor any shader for all of them.
    fn line_quads(
        &mut self,
        d: &text::RichDecoration,
        rich: Option<&zgx::rich_text::RichText>,
        out: &mut Vec<TextQuad>,
        stats: &mut GpuStats,
    ) -> Result<(), GpuError> {
        use zgx::rich_text::LineStyle;
        let style = rich.map_or(LineStyle::Solid, |rich| d.line_style(rich));
        if style == LineStyle::Solid || d.bounds.width <= 0. {
            out.push(rich_decoration_quad(d));
            return Ok(());
        }
        let b = d.bounds;
        let thick = b.height.max(0.5);
        let color = rgba(d.color, 1.);
        let piece = |x: f32, y: f32, width: f32, height: f32, radius: f32| TextQuad {
            rect: [x, y, width, height],
            color,
            uv: [0.; 4],
            paint: if radius > 0. {
                [radius, 0., 0., 1.]
            } else {
                [0.; 4]
            },
        };
        let right = b.x + b.width;
        // A row of `length`s, `step` apart, the last cut at the run's end.
        let row = |length: f32, step: f32, radius: f32, out: &mut Vec<TextQuad>| {
            for k in 0..LINE_PIECES {
                let x = b.x + k as f32 * step;
                if x >= right {
                    break;
                }
                out.push(piece(x, b.y, length.min(right - x), thick, radius));
            }
        };
        match style {
            LineStyle::Double => {
                out.push(piece(b.x, b.y, b.width, thick, 0.));
                out.push(piece(b.x, b.y + 2. * thick, b.width, thick, 0.));
            }
            LineStyle::Dotted => row(thick, 2. * thick, thick / 2., out),
            LineStyle::Dashed => row(3. * thick, 5. * thick, 0., out),
            _ => {
                // One period, in pixels: seamless, its gutter wrapping round.
                let line = (thick * self.scale).max(1.);
                let (period, height) = ((line * 6.).round() as u32, (line * 4.).ceil() as u32 + 2);
                let key = WAVE_CELL | (line * 16.).round() as u64;
                let (x, y) = match self.image_cells.get(&key) {
                    Some(&(x, y, _, _)) => (x, y),
                    None => {
                        let cell = self.atlas_cell(period, height, |rgba| {
                            let row = (period + 2) as usize * 4;
                            let (amplitude, middle) = (line, height as f32 / 2. + 0.5);
                            let wave = std::f32::consts::TAU / period as f32;
                            for (k, texel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                                let (cx, cy) = (k * 4 % row / 4, k * 4 / row);
                                let px = cx as f32 - 0.5;
                                let distance = (cy as f32 + 0.5
                                    - middle
                                    - amplitude * (px * wave).sin())
                                .abs()
                                    / (1. + (amplitude * wave * (px * wave).cos()).powi(2)).sqrt();
                                let coverage = (0.5 + line / 2. - distance).clamp(0., 1.);
                                texel.fill((coverage * 255.).round() as u8);
                            }
                        })?;
                        self.image_cells
                            .insert(key, (cell.0, cell.1, period, height));
                        stats.glyph_uploads += 1;
                        cell
                    }
                };
                let (tile, tall) = (period as f32 / self.scale, height as f32 / self.scale);
                let atlas = self.atlas_size as f32;
                let top = b.y + thick / 2. - tall / 2.;
                for k in 0..LINE_PIECES {
                    let left = b.x + k as f32 * tile;
                    if left >= right {
                        break;
                    }
                    let part = ((right - left) / tile).min(1.);
                    out.push(TextQuad {
                        rect: [left, top, tile * part, tall],
                        color,
                        uv: [
                            x as f32 / atlas,
                            y as f32 / atlas,
                            period as f32 * part / atlas,
                            height as f32 / atlas,
                        ],
                        paint: [0., 2., 0., 0.],
                    });
                }
            }
        }
        Ok(())
    }
    /// Make `visible_glyphs` the quads of `shaped`'s lines that can reach
    /// the band `top` to `top + height` (from its origin, logical points),
    /// if that leaves out most of its glyphs; whether it did. Out of line:
    /// what the per-glyph loops of `render_flat` hold stays small.
    #[inline(never)]
    fn visible_lines(
        &mut self,
        shaped: &Shaped,
        top: f32,
        height: f32,
        font_size: f32,
        stats: &mut GpuStats,
    ) -> Result<bool, GpuError> {
        // Lines are laid out downward: glyph baselines (device pixels from
        // its top) never decrease. Glyphs reach past their line by less
        // than `reach`.
        let largest = shaped.rich.as_ref().map_or(font_size, |rich| {
            rich.runs()
                .iter()
                .map(|run| run.font_size)
                .fold(font_size, f32::max)
        });
        let reach = 3. * largest;
        let (top, bottom) = (top - reach, top + height + reach);
        let glyphs = &shaped.glyphs;
        let first = glyphs.partition_point(|g| g.y < top * self.scale);
        let end = glyphs
            .partition_point(|g| g.y <= bottom * self.scale)
            .max(first);
        // Worth it when it leaves out most of its glyphs: a label cut by an
        // edge copies its kept quads, as before.
        if (end - first) * 2 >= glyphs.len() {
            return Ok(false);
        }
        let mut visible = std::mem::take(&mut self.visible_glyphs);
        visible.clear();
        let mut colored = false;
        let lines = |quad: &TextQuad| quad.rect[1] + quad.rect[3] >= top && quad.rect[1] <= bottom;
        let decorations = |background: bool| {
            shaped
                .decorations
                .iter()
                .filter(move |d| d.background == background)
                .map(rich_decoration_quad)
                .filter(lines)
        };
        visible.extend(decorations(true));
        for glyph in &glyphs[first..end] {
            // As `glyph`, through another lookup: `glyph` keeps its one caller.
            let size = f32::from_bits(glyph.key.font_size_bits);
            if !size.is_finite() || size > MAX_ATLAS as f32 {
                self.glyph_too_large(size);
                continue;
            }
            let entry = match self.entries.get_mut(&AtlasKey::from(glyph.key)) {
                Some(entry) => *entry,
                None => self.raster_glyph(glyph.key, stats)?,
            };
            if let Some(entry) = entry {
                let color = shaped
                    .rich
                    .as_ref()
                    .map_or([0.; 4], |rich| rgba(rich.runs()[glyph.run].color, 1.));
                colored |= entry.color;
                visible.push(glyph_quad(glyph, entry, self.scale, self.atlas_size, color));
            }
        }
        let lines_from = visible.len();
        for decoration in shaped.decorations.iter().filter(|d| !d.background) {
            self.line_quads(decoration, shaped.rich.as_deref(), &mut visible, stats)?;
        }
        let mut kept = lines_from;
        for k in lines_from..visible.len() {
            if lines(&visible[k]) {
                visible.swap(kept, k);
                kept += 1;
            }
        }
        visible.truncate(kept);
        self.visible_glyphs = visible;
        self.visible_colored = colored;
        Ok(true)
    }
    #[inline(always)]
    fn glyph<const MODE: u8>(
        &mut self,
        key: CacheKey,
        stats: &mut GpuStats,
    ) -> Result<Option<AtlasEntry>, GpuError> {
        // A resident entry passed the same check when it went in: a patch
        // takes it first. (A full frame keeps the old order.)
        if MODE == PATCH
            && let Some(entry) = get_in::<MODE, _, _>(&mut self.entries, &AtlasKey::from(key))
        {
            return Ok(*entry);
        }
        let size = f32::from_bits(key.font_size_bits);
        if !size.is_finite() || size > MAX_ATLAS as f32 {
            self.glyph_too_large(size);
            return Ok(None);
        }
        if MODE != PATCH
            && let Some(entry) = get_in::<MODE, _, _>(&mut self.entries, &AtlasKey::from(key))
        {
            return Ok(*entry);
        }
        self.raster_glyph(key, stats)
    }
    /// A glyph of `size` pixels left out, too large for the atlas: an issue,
    /// unless its magnified layer draws again coarser (`prepare_layer`).
    #[cold]
    fn glyph_too_large(&mut self, size: f32) {
        let (largest, redrawn) = &mut self.layer_glyph_too_large;
        if size.is_finite() {
            *largest = largest.max(size);
        }
        if !(*redrawn && size.is_finite()) {
            self.warn(GLYPH_TOO_LARGE);
        }
    }
    /// `glyph`'s miss: rasterize `key` into the atlas. Out of line, and
    /// `glyph`, the per-glyph lookup, has one caller: inlined there.
    #[inline(never)]
    fn raster_glyph(
        &mut self,
        key: CacheKey,
        stats: &mut GpuStats,
    ) -> Result<Option<AtlasEntry>, GpuError> {
        if self.entries.len() >= 8192 {
            self.warn("more than 8192 glyphs in a frame: the rest aren't drawn");
            return Ok(None);
        }
        let Some(image) = text::glyph_image(&mut self.swash, &mut self.fonts.borrow_mut(), key)
        else {
            self.entries.insert(AtlasKey::from(key), None);
            return Ok(None);
        };
        let p = image.placement;
        if p.width == 0 || p.height == 0 {
            self.entries.insert(AtlasKey::from(key), None);
            return Ok(None);
        }
        let cell_width = p.width + 2;
        let atlas_full = self.atlas_size >= MAX_ATLAS;
        let cell = self.atlas_cell(p.width, p.height, |rgba| {
            let row_start = |row: u32| ((row + 1) * cell_width + 1) as usize * 4;
            match image.content {
                SwashContent::Mask => {
                    for (row, alphas) in image.data.chunks_exact(p.width as usize).enumerate() {
                        let start = row_start(row as u32);
                        for (texel, alpha) in
                            rgba[start..].as_chunks_mut::<4>().0.iter_mut().zip(alphas)
                        {
                            texel.copy_from_slice(&[255, 255, 255, *alpha]);
                        }
                    }
                }
                SwashContent::Color => {
                    let bytes = p.width as usize * 4;
                    for (row, texels) in image.data.chunks_exact(bytes).enumerate() {
                        let start = row_start(row as u32);
                        rgba[start..start + bytes].copy_from_slice(texels);
                    }
                }
                SwashContent::SubpixelMask => {
                    let bytes = p.width as usize * 4;
                    for (row, pixels) in image.data.chunks_exact(bytes).enumerate() {
                        let start = row_start(row as u32);
                        for (texel, pixel) in rgba[start..]
                            .as_chunks_mut::<4>()
                            .0
                            .iter_mut()
                            .zip(pixels.as_chunks::<4>().0)
                        {
                            texel.copy_from_slice(&[
                                255,
                                255,
                                255,
                                pixel[0].max(pixel[1]).max(pixel[2]),
                            ]);
                        }
                    }
                }
            }
        });
        let (x, y) = match cell {
            // At its largest: nothing left to grow into (`render_inner`).
            Err(error) if atlas_full => {
                self.warn(&format!("{}: the rest aren't drawn", error.0));
                return Ok(None);
            }
            cell => cell?,
        };
        let entry = AtlasEntry {
            x,
            y,
            width: p.width,
            height: p.height,
            left: p.left,
            top: p.top,
            color: matches!(image.content, SwashContent::Color),
        };
        self.entries.insert(AtlasKey::from(key), Some(entry));
        stats.glyph_uploads += 1;
        Ok(Some(entry))
    }
    /// A shadowed text's quads up to its glyphs (its box and marks, from
    /// `start`, under its `own` clip) and its shadows' (`at` to `end`, under
    /// `wide`, which takes them in past its box), culled and drawn as an
    /// item's are: the rest (its glyphs) go on after them, under its own
    /// clip, where they start returned.
    #[cold]
    #[inline(never)]
    fn draw_text_shadows(
        &mut self,
        (quads, draws): (&mut Vec<Quad>, &mut Vec<Draw>),
        (at, end): (u32, u32),
        (start, own, wide, viewport): (u32, Rect, Rect, Rect),
        (regions, region, glyphs): (&[Region], f32, Shading),
    ) -> u32 {
        let mut rest = quads.split_off(end as usize);
        let shadows = quads.split_off(at as usize);
        let own = own.intersection(viewport).unwrap_or_default();
        for (part, clip, shading) in [(None, own, None), (Some(shadows), wide, Some(glyphs))] {
            let from = match part {
                Some(part) => {
                    let from = quads.len();
                    quads.extend(part);
                    from
                }
                None => start as usize,
            };
            let (draw_clip, culled) = match shading {
                Some(_) => cull_glyphs(quads, from, clip, viewport, self.scale),
                None => cull_on_cpu(quads, from, clip, viewport, self.scale),
            };
            if quads.len() <= from {
                continue;
            }
            let drawn = culled.unwrap_or_else(|| quad_bounds(&quads[from..]));
            if region > 0. {
                mark_region(
                    &mut self.region_interiors,
                    regions,
                    region,
                    &mut quads[from..],
                    drawn,
                    false,
                    self.scale,
                );
            }
            let draw = Draw {
                shading: Shading::Full,
                blur: None,
                start: from as u32,
                end: quads.len() as u32,
                clip: draw_clip,
                bounds: drawn,
                source: TextureSource::Atlas,
            };
            let split = self.split_shading;
            push_draw(draws, quads, draw, split, false, self.scale, shading);
        }
        let start = quads.len() as u32;
        quads.append(&mut rest);
        start
    }
    /// A text's shadows when one is blurred, all of them, placed at `at`
    /// (where its glyphs start, so under them): a blurred one from its own
    /// glyphs' blurred coverage (`blurred_glyph`), kept while the text's
    /// shape, the atlas and the scale hold; a sharp one from its glyphs, as
    /// text with no blur places them. Out of line: text with none never
    /// comes here.
    #[cold]
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn place_blurred_shadows(
        &mut self,
        quads: &mut Vec<Quad>,
        at: usize,
        (id, reshaped): (NodeId, bool),
        shadows: &[zgx::scene::TextShadow],
        (x, y, fade): (f32, f32, [f32; 4]),
        (edge_fade, opacity): (f32, f32),
        stats: &mut GpuStats,
    ) -> Result<usize, GpuError> {
        // (A shape past the cache's budget isn't kept: nor its shadows.)
        let Some(shaped) = self.shapes.remove(&id) else {
            return Ok(0);
        };
        let placed = self.blurred_shadow_quads(&shaped, id, reshaped, shadows, stats);
        let blurred = self.blurred.as_ref().and_then(|b| b.texts.get(&id));
        let scale = self.scale;
        let mut under = Vec::new();
        if placed.is_ok() {
            for shadow in shadows.iter().rev() {
                // (At least an eighth of a pixel for colour glyphs: their
                // shadows are their alphas, as blurred ones are.)
                let sigma = shadow_sigma(shadow.blur, scale).max(u32::from(shaped.colored));
                let source = match blurred.filter(|_| sigma > 0) {
                    Some(text) => match text.sigmas.iter().find(|(s, _)| *s == sigma) {
                        Some((_, quads)) => quads.as_slice(),
                        None => continue,
                    },
                    None => &shaped.quads,
                };
                let color = rgba(shadow.color, opacity);
                place_text_quads(
                    &mut under,
                    source,
                    x + (shadow.offset.x * scale).round() / scale,
                    y + (shadow.offset.y * scale).round() / scale,
                    fade,
                    edge_fade,
                    |_| color,
                );
            }
        }
        self.shapes.insert(id, shaped);
        placed?;
        let placed = under.len();
        quads.splice(at..at, under);
        Ok(placed)
    }
    /// The quads of `shaped`'s glyphs blurred at each of `shadows`' sigmas,
    /// made where they aren't kept (`BlurredShadows::texts`).
    fn blurred_shadow_quads(
        &mut self,
        shaped: &Shaped,
        id: NodeId,
        reshaped: bool,
        shadows: &[zgx::scene::TextShadow],
        stats: &mut GpuStats,
    ) -> Result<(), GpuError> {
        let (epoch, scale) = (self.atlas_epoch, self.scale);
        let blurred = self.blurred.get_or_insert_default();
        let text = blurred.texts.entry(id).or_insert_with(|| BlurredText {
            atlas_epoch: epoch,
            scale,
            sigmas: Vec::new(),
        });
        if reshaped || text.atlas_epoch != epoch || text.scale != scale {
            (text.atlas_epoch, text.scale) = (epoch, scale);
            text.sigmas.clear();
        }
        for shadow in shadows {
            let sigma = shadow_sigma(shadow.blur, scale).max(u32::from(shaped.colored));
            let kept = |blurred: &Option<Box<BlurredShadows>>| {
                blurred
                    .as_ref()
                    .and_then(|b| b.texts.get(&id))
                    .is_some_and(|text| text.sigmas.iter().any(|(s, _)| *s == sigma))
            };
            if sigma == 0 || kept(&self.blurred) {
                continue;
            }
            let mut placed = Vec::with_capacity(shaped.glyphs.len());
            for glyph in &shaped.glyphs {
                let Some(b) = self.blurred_glyph(glyph.key, sigma, stats)? else {
                    continue;
                };
                let atlas = self.atlas_size as f32;
                placed.push(TextQuad {
                    rect: [
                        (glyph.x + b.left as f32) / scale,
                        (glyph.y - b.top as f32) / scale,
                        b.size.0 as f32 / scale,
                        b.size.1 as f32 / scale,
                    ],
                    uv: [
                        b.x as f32 / atlas,
                        b.y as f32 / atlas,
                        b.texels.0 as f32 / atlas,
                        b.texels.1 as f32 / atlas,
                    ],
                    color: [0.; 4],
                    paint: [0., 1., 0., 0.],
                });
            }
            // The atlas may have started over meanwhile: these are its own.
            let epoch = self.atlas_epoch;
            let text = self
                .blurred
                .get_or_insert_default()
                .texts
                .entry(id)
                .or_insert_with(|| BlurredText {
                    atlas_epoch: epoch,
                    scale,
                    sigmas: Vec::new(),
                });
            if text.atlas_epoch != epoch {
                (text.atlas_epoch, text.sigmas) = (epoch, Vec::new());
            }
            text.sigmas.push((sigma, placed));
        }
        Ok(())
    }
    /// `key`'s coverage blurred by a Gaussian of `sigma` eighths of a device
    /// pixel, in the atlas: reaching three sigmas past the glyph, at a power
    /// of two fewer texels where sigma spans more than four (CSS blurs a
    /// text shadow's coverage; glyph by glyph, where two glyphs' blurs meet
    /// they combine as paint does, a shade lighter than one blur of both).
    fn blurred_glyph(
        &mut self,
        key: CacheKey,
        sigma: u32,
        stats: &mut GpuStats,
    ) -> Result<Option<BlurredGlyph>, GpuError> {
        let slot = (AtlasKey::from(key), sigma);
        if let Some(entry) = self.blurred.as_ref().and_then(|b| b.glyphs.get(&slot)) {
            return Ok(*entry);
        }
        let image = text::glyph_image(&mut self.swash, &mut self.fonts.borrow_mut(), key);
        let Some(image) = image.filter(|i| i.placement.width > 0 && i.placement.height > 0) else {
            self.blurred
                .get_or_insert_default()
                .glyphs
                .insert(slot, None);
            return Ok(None);
        };
        let p = image.placement;
        let coverage: Vec<f32> = match image.content {
            SwashContent::Mask => image.data.iter().map(|&a| f32::from(a)).collect(),
            SwashContent::Color => (image.data.as_chunks::<4>().0.iter())
                .map(|t| f32::from(t[3]))
                .collect(),
            SwashContent::SubpixelMask => (image.data.as_chunks::<4>().0.iter())
                .map(|t| f32::from(t[0].max(t[1]).max(t[2])))
                .collect(),
        };
        let blurred = blur_coverage(&coverage, (p.width, p.height), sigma as f32 / 8.);
        let (texels, size, reach) = (blurred.texels, blurred.size, blurred.reach);
        let cell_width = texels.0 + 2;
        let (x, y) = self.atlas_cell(texels.0, texels.1, |rgba| {
            for (row, alphas) in blurred.alpha.chunks_exact(texels.0 as usize).enumerate() {
                let start = ((row as u32 + 1) * cell_width + 1) as usize * 4;
                for (texel, alpha) in rgba[start..].as_chunks_mut::<4>().0.iter_mut().zip(alphas) {
                    *texel = [255, 255, 255, *alpha];
                }
            }
        })?;
        let entry = BlurredGlyph {
            x,
            y,
            texels,
            size,
            left: p.left - reach as i32,
            top: p.top + reach as i32,
        };
        self.blurred
            .get_or_insert_default()
            .glyphs
            .insert(slot, Some(entry));
        stats.glyph_uploads += 1;
        Ok(Some(entry))
    }
    /// Reserve a `width`×`height` atlas cell with a transparent 1-texel border
    /// and upload it; `fill` writes the bordered RGBA cell. Returns the
    /// interior's origin. Linear sampling at fractional positions reads one
    /// texel past the content, and a recycled atlas is not cleared, so shared
    /// gutters could otherwise hold stale texels.
    fn atlas_cell(
        &mut self,
        width: u32,
        height: u32,
        fill: impl FnOnce(&mut [u8]),
    ) -> Result<(u32, u32), GpuError> {
        if self.atlas_size == 1 {
            self.reset_atlas(INITIAL_ATLAS);
        }
        let (cell_width, cell_height) = (width + 2, height + 2);
        if self.cursor.0 + cell_width > self.atlas_size {
            self.cursor.0 = 0;
            self.cursor.1 += self.cursor.2;
            self.cursor.2 = 0;
        }
        if self.cursor.1 + cell_height > self.atlas_size || cell_width > self.atlas_size {
            return Err(GpuError(
                "glyph atlas capacity exceeded in one frame".into(),
            ));
        }
        let mut rgba = vec![0; (cell_width * cell_height * 4) as usize];
        fill(&mut rgba);
        let atlas = self.atlas.clone();
        self.write_texels(
            &atlas,
            (self.cursor.0, self.cursor.1, cell_width, cell_height),
            &mut rgba,
        );
        let origin = (self.cursor.0 + 1, self.cursor.1 + 1);
        self.cursor.0 += cell_width;
        self.cursor.2 = self.cursor.2.max(cell_height);
        Ok(origin)
    }
    /// Present the retained target. Use `present_with_status` when scheduling retries.
    pub fn present(&mut self) -> Result<(), GpuError> {
        self.present_with_status().map(|_| ())
    }
    /// Present without discarding whether native acquisition skipped this frame.
    pub fn present_with_status(&mut self) -> Result<PresentationStatus, GpuError> {
        self.present_with_notify(|| {})
    }
    /// The copies bringing the current surface up to date from the one shown
    /// (`buffers[shown]`): the changes since it was last shown, less what `regions`
    /// repaint and `copies` cover this frame, then the scroll `copies`.
    #[cfg(own_surfaces)]
    fn catch_up_draws(
        &mut self,
        shown: usize,
        copies: &[ScrollCopy],
        regions: &[(Rect, Scissor)],
    ) -> Option<(wgpu::BindGroup, Vec<ScrollCopy>)> {
        let mut missing = self
            .layer_present
            .as_mut()
            .map(|present| std::mem::take(&mut present.catch_up))
            .unwrap_or_default();
        let mut rest = Vec::new();
        // Whatever this frame repaints or copies over needs nothing first.
        let cuts = regions
            .iter()
            .map(|(_, s)| *s)
            .chain(copies.iter().map(|(s, _)| *s));
        for cut in cuts {
            for rect in missing.drain(..) {
                scissor_minus(rect, cut, &mut rest);
            }
            std::mem::swap(&mut missing, &mut rest);
        }
        // Frames often change the same places: draw each pixel once where
        // one rectangle holds another.
        missing
            .sort_unstable_by_key(|&(_, _, w, h)| std::cmp::Reverse(u64::from(w) * u64::from(h)));
        let mut kept: Vec<Scissor> = Vec::with_capacity(missing.len());
        for rect in missing {
            if !kept.iter().any(|&outer| scissor_holds(outer, rect)) {
                kept.push(rect);
            }
        }
        if kept.is_empty() && copies.is_empty() {
            return None;
        }
        let pipeline = self.copy_offset.get_or_insert_with(|| {
            let shader = self
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("shifted copy"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("copy_offset.wgsl").into()),
                });
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("shifted copy"),
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs"),
                        compilation_options: Default::default(),
                        // A quad's rect (device pixels) and uv (the shift).
                        buffers: &[wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Quad>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
                        }],
                    },
                    primitive: QUAD_STRIP,
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(FORMAT.into())],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
        });
        // Made once per surface: it is the source every other frame.
        let source = &self.layer_present.as_ref()?.buffers[shown].0;
        let bind = source
            .copy_source
            .get_or_init(|| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("shifted copy source"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&source.view),
                    }],
                })
            })
            .clone();
        let draws = kept
            .into_iter()
            .map(|rect| (rect, (0, 0)))
            .chain(copies.iter().copied())
            .collect();
        Some((bind, draws))
    }
    /// Make a surface the retained target for the frame about to draw,
    /// unless one is: one the window server is done with, with the regions it
    /// lacks of the last frame queued to copy from the shown surface (none
    /// when `whole`: the frame repaints everything).
    #[cfg(target_os = "macos")]
    fn begin_layer_frame(&mut self, whole: bool) -> Result<(), GpuError> {
        let (width, height) = (self.width, self.height);
        let Some(present) = self.layer_present.as_mut() else {
            return Ok(());
        };
        if present.current.is_some() {
            return Ok(());
        }
        if present
            .buffers
            .iter()
            .any(|(b, _)| (b.texture.width(), b.texture.height()) != (width, height))
        {
            // Resized: the layer keeps showing the old surface until the next.
            present.buffers.clear();
            present.shown = None;
            present.history.clear();
        }
        // At full rate through drawables, the next drawable's surface is the
        // target; the renderer's own surfaces go but the one shown (it brings
        // the drawable up to date), and out of full rate, the drawables do.
        let drawable = if present.drawable_frame() {
            present.attach(self.surface.as_ref())?;
            present.retain_buffers(|_, buffer| buffer.drawable);
            // Presented feedback while the trial reads it, or a log does.
            let wants_feedback =
                present_log::enabled() || matches!(present.trial, Trial::Drawables { .. });
            // SAFETY: The raw device is only retained; the guard is dropped
            // before wgpu is called again.
            let device = unsafe { self.device.as_hal::<wgpu::hal::api::Metal>() }
                .map(|hal| hal.raw_device().clone());
            // When the host began this frame, in host seconds (the drawables'
            // presented times' clock).
            let frame_started = self
                .frame_started
                .map(|at| present_log::now_ns() as f64 / 1e9 - at.elapsed().as_secs_f64());
            let next = device.and_then(|device| {
                present
                    .layer
                    .as_mut()
                    .expect("attached above")
                    .next_drawable(&device, width, height, wants_feedback, frame_started)
            });
            next.map(|(texture, surface)| {
                let id = surface.id();
                match present
                    .buffers
                    .iter()
                    .position(|(buffer, _)| buffer.drawable && buffer.surface.id() == id)
                {
                    Some(index) => index,
                    None => {
                        present.buffers.push((
                            layer_present::Buffer::from_drawable(&self.device, surface, texture),
                            None,
                        ));
                        present_log::log(|| format!("new-drawable surface={id}"));
                        present.buffers.len() - 1
                    }
                }
            })
        } else {
            if present.buffers.iter().any(|(buffer, _)| buffer.drawable) {
                present.retain_buffers(|_, buffer| !buffer.drawable);
            }
            // Tiles won the trial: their pool goes once a tile shows.
            if present.verdict == 1
                && present
                    .shown
                    .is_none_or(|shown| !present.buffers[shown].0.drawable)
                && let Some(layer) = present.layer.as_mut()
            {
                layer.release_drawables();
            }
            None
        };
        let index = match drawable {
            Some(index) => index,
            None => {
                // Certainly free: not the latest, and no use beyond the layer's own
                // record of it (one on screen or queued has more). The most recent of
                // them holds the frame nearest this one.
                let idle = |present: &LayerPresent| -> Vec<usize> {
                    let mut idle: Vec<usize> = (0..present.buffers.len())
                        .filter(|&i| {
                            Some(i) != present.shown
                                && present.buffers[i].0.surface.use_count() <= 1
                        })
                        .collect();
                    idle.sort_by_key(|&i| std::cmp::Reverse(present.buffers[i].1));
                    idle
                };
                let budget = if present.full_rate { LAYER_BUFFERS } else { 2 };
                let mut free = idle(present);
                // Slower again: the third surface goes once free.
                if present.buffers.len() > budget
                    && let Some(&spare) = free.last()
                {
                    present.buffers.remove(spare);
                    if let Some(shown) = &mut present.shown
                        && *shown > spare
                    {
                        *shown -= 1;
                    }
                    free = idle(present);
                }
                // None: ask the layer which it no longer shows nor queues (it holds a
                // surface a while after showing the next; that one is free).
                let pick = |present: &LayerPresent| {
                    let busy = present.busy();
                    (0..present.buffers.len())
                        .filter(|&i| Some(i) != present.shown && !busy[i])
                        .max_by_key(|&i| present.buffers[i].1)
                };
                let mut index = free.first().copied().or_else(|| pick(present));
                if index.is_none() && present.buffers.len() < budget {
                    present.buffers.push((
                        layer_present::Buffer::new(&self.device, width, height)?,
                        None,
                    ));
                    index = Some(present.buffers.len() - 1);
                    present_log::log(|| {
                        format!(
                            "new-surface count={} budget={budget}",
                            present.buffers.len()
                        )
                    });
                }
                // Still none: the layer moves on by its next refresh.
                let started = std::time::Instant::now();
                let mut asked = started;
                while index.is_none() && started.elapsed() < std::time::Duration::from_millis(20) {
                    std::thread::sleep(std::time::Duration::from_micros(100));
                    index = idle(present).first().copied();
                    if index.is_none() && asked.elapsed() >= std::time::Duration::from_millis(2) {
                        asked = std::time::Instant::now();
                        index = pick(present);
                    }
                }
                let waited = started.elapsed();
                let forced = index.is_none();
                let index = index.unwrap_or_else(|| {
                    (0..present.buffers.len())
                        .find(|&i| Some(i) != present.shown)
                        .expect("more than one surface")
                });
                if forced || waited.as_micros() > 500 {
                    present_log::log(|| {
                        format!(
                            "{} acquire index={index} waited_us={} forced_busy={forced} buffers={}",
                            present.plane(),
                            waited.as_micros(),
                            present.buffers.len()
                        )
                    });
                }
                index
            }
        };
        let held = present.buffers[index].1;
        let latest = present.shown.and_then(|shown| present.buffers[shown].1);
        present.catch_up = present.catch_up_for(index, whole, width, height);
        // How many areas the surface copies from the one shown before the
        // frame draws (`catch_up_draws` takes them as it renders).
        present_log::log(|| {
            format!(
                "{} draw whole={whole} held={held:?} latest={latest:?} catch_up={}",
                present.plane(),
                present.catch_up.len()
            )
        });
        let buffer = &present.buffers[index].0;
        self.target = buffer.texture.clone();
        self.view = buffer.view.clone();
        present.current = Some(index);
        Ok(())
    }
    /// Show the retained frame (the current surface) through the layer.
    #[cfg(target_os = "macos")]
    fn present_layer(
        &mut self,
        before_present: impl FnOnce(),
    ) -> Result<PresentationStatus, GpuError> {
        let mut present = self.layer_present.take().expect("layer presenting");
        if let Err(error) = present.attach(self.surface.as_ref()) {
            self.layer_present = Some(present);
            return Err(error);
        }
        if let Some(layer) = present.layer.as_mut() {
            layer.set_caret(present.caret);
        }
        let Some(index) = present.current.take() else {
            // Nothing drawn since: the shown frame stands (planes that moved
            // show in a transaction of their own).
            self.layer_present = Some(present);
            if self.views.is_some() {
                self.apply_planes();
            }
            return Ok(PresentationStatus::Presented);
        };
        // A drawable's frame encoded directly presented as it committed.
        let presented = present.buffers[index].0.drawable && self.metal_inflight.borrow().is_some();
        // The window server reads the surface from its next composite: the
        // frame must be complete by then.
        // (A drawable's direct frame is not waited for: it presents itself.)
        if presented || self.finish_direct() {
            // wgpu still frees what was dropped once its own work is done.
            if present.frame.is_multiple_of(64) {
                let _ = self.device.poll(wgpu::PollType::Poll);
            }
        } else {
            let encoder = self
                .pending
                .get_mut()
                .take()
                .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
            let submission = self.queue.submit([encoder.finish()]);
            self.belt.get_mut().recall();
            self.submitted();
            if let Some(mapped) = self.mapped.as_mut() {
                mapped.get_mut().submitted();
            }
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            });
        }
        before_present();
        let plane = present.plane();
        let opaque = self.background.3 == 255;
        let plain = present
            .resized_at
            .is_some_and(|at| at.elapsed() < RESIZE_SETTLE)
            || present.plain_only
            // Every frame is a tile of the plain layer, at the display's
            // full rate too (`layer_present::Layer`), unless the video
            // layer is asked for (`ZGX_PRESENT_VIDEO`).
            || !*VIDEO_AT_FULL_RATE
            // With the video layer: slower frames than the display's (a
            // 30 fps shimmer, a clock) are still the plain layer's, as the
            // video layer queues frames on threads of its own, a cost worth
            // paying only for a full rate. It stays a second after one, so
            // a pause in a scroll does not drop and rebuild it.
            || (!present.full_rate
                && present
                    .video_until
                    .is_none_or(|until| std::time::Instant::now() >= until))
            || present.with_natives
            || present
                .plain_until
                .is_some_and(|until| std::time::Instant::now() < until);
        present_log::log(|| {
            format!(
                "{} present frame={} index={index} held={:?} path={} full_rate={} opaque={opaque}",
                plane,
                present.frame + 1,
                present.buffers[index].1,
                if plain { "plain" } else { "show" },
                present.full_rate,
            )
        });
        let layer = present.layer.as_mut().expect("attached above");
        let buffer = &present.buffers[index].0;
        // A GPU view's plane moving with this frame: the frame's drawable
        // presents in the transaction too, so the two show together.
        let planes_moved = self
            .views
            .as_ref()
            .is_some_and(|views| views.planes_moved());
        let show = |layer: &mut layer_present::Layer| {
            if buffer.drawable {
                // SAFETY: The raw queue is only retained; the guard is dropped at once.
                let queue = unsafe { self.queue.as_hal::<wgpu::hal::api::Metal>() }.map(|hal| {
                    use objc2::Message;
                    hal.as_raw().retain()
                });
                layer.present_drawable(queue.as_deref(), planes_moved);
            } else if plain {
                layer.show_plain(buffer, opaque);
            } else {
                layer.show(buffer, opaque, present.full_rate);
            }
        };
        match self.views.as_deref_mut() {
            // GPU views' planes that moved, showed or hid change with the
            // window's frame, in one transaction: never a frame apart.
            Some(views) => {
                // SAFETY: The raw queue is only retained; the guard is dropped at once.
                let queue = unsafe { self.queue.as_hal::<wgpu::hal::api::Metal>() }.map(|hal| {
                    use objc2::Message;
                    hal.as_raw().retain()
                });
                // Their work (and a drawable frame's) committed and scheduled
                // first: the transaction itself waits for nothing.
                viewport::schedule_planes(views, queue.as_deref());
                layer_present::transaction(|| {
                    show(layer);
                    viewport::apply_planes(views, layer, queue.as_deref());
                });
            }
            None => show(layer),
        }
        layer.keep(
            present
                .buffers
                .iter()
                .map(|(buffer, _)| buffer.surface.id()),
        );
        present.presented(index);
        let frame = present.frame;
        // The machine's verdict, looked up once per key (`verdicts`).
        if *PRESENT_AUTO && present.full_rate {
            // SAFETY: Only the raw device's registry ID is read.
            let gpu = unsafe { self.device.as_hal::<wgpu::hal::api::Metal>() }.map_or(0, |hal| {
                use objc2_metal::MTLDevice;
                hal.raw_device().registryID()
            });
            static OS: std::sync::LazyLock<String> = std::sync::LazyLock::new(verdicts::os);
            let key = format!("{gpu:x}.{}.{}", self.display_key, *OS).replace(' ', "_");
            if key != present.verdict_key {
                present.verdict = verdicts::get(&key);
                present.verdict_key = key;
                if present.verdict == 0 {
                    present.trial = Trial::Untried;
                } else {
                    present.trial = Trial::Done;
                    eprintln!(
                        "zgx: full-rate frames take {} (this machine's stored verdict)",
                        if present.verdict == 2 {
                            "drawables"
                        } else {
                            "tiles"
                        }
                    );
                }
            }
        }
        // The full-rate paths' trial (`Trial`), a frame at a time.
        if *PRESENT_AUTO && present.drawables.is_none() && present.full_rate && present.verdict == 0
        {
            // Where a feedback record stands, and the share of frames lost
            // since then (`Feedback::lost_since`).
            let counts =
                |feedback: Option<std::sync::Arc<layer_present::Feedback>>| -> layer_present::Mark {
                    feedback.map_or_else(Default::default, |feedback| feedback.mark())
                };
            let share = |feedback: Option<std::sync::Arc<layer_present::Feedback>>,
                         from: layer_present::Mark| {
                feedback.map_or(0., |feedback| {
                    let (handed, lost) = feedback.lost_since(from);
                    lost as f64 / handed.max(1) as f64
                })
            };
            present.trial = match present.trial {
                Trial::Untried => {
                    // Tiles judged by the frames they lose need the probe (a
                    // drawable of its own each flip); by energy they run as
                    // they ship, without it.
                    if *TRIAL_RULE == TrialRule::Frames {
                        // SAFETY: The raw device and queue are only retained;
                        // the guards are dropped at once.
                        let device = unsafe { self.device.as_hal::<wgpu::hal::api::Metal>() }
                            .map(|hal| hal.raw_device().clone());
                        let queue =
                            unsafe { self.queue.as_hal::<wgpu::hal::api::Metal>() }.map(|hal| {
                                use objc2::Message;
                                hal.as_raw().retain()
                            });
                        if let (Some(layer), Some(device), Some(queue)) =
                            (present.layer.as_mut(), device, queue)
                        {
                            layer.start_probe(&device, queue);
                        }
                    }
                    let from = counts(
                        present
                            .layer
                            .as_ref()
                            .and_then(|layer| layer.probe_feedback()),
                    );
                    present.trial_started = Some(std::time::Instant::now());
                    present.trial_samples.clear();
                    Trial::Tiles { frames: 0, from }
                }
                Trial::Tiles { frames, from } if frames + 1 >= TRIAL_FRAMES => {
                    let layer = present.layer.as_mut().expect("attached");
                    let tiles = share(layer.probe_feedback(), from);
                    layer.stop_probe();
                    present.trial_samples.extend(energy_sample());
                    let from = counts(layer.drawable_feedback());
                    Trial::Drawables {
                        frames: 0,
                        from,
                        settled: None,
                        tiles,
                        ended: None,
                    }
                }
                Trial::Tiles { frames, from } => {
                    if frames + 1 == TRIAL_WARMUP {
                        present.trial_samples.extend(energy_sample());
                    }
                    Trial::Tiles {
                        frames: frames + 1,
                        from,
                    }
                }
                Trial::Drawables {
                    frames,
                    from,
                    settled,
                    tiles,
                    ended: None,
                } if frames + 1 >= TRIAL_FRAMES => {
                    let feedback = present
                        .layer
                        .as_ref()
                        .and_then(|layer| layer.drawable_feedback());
                    let drawables = share(feedback.clone(), from);
                    // Never settled (fewer shown than the warm-up): all of them.
                    let settled = settled.unwrap_or(from);
                    let drawables_settled = share(feedback.clone(), settled);
                    let drawables_late = feedback.map_or(0., |feedback| {
                        let (handed, _) = feedback.lost_since(settled);
                        feedback.late_since(settled) as f64 / handed.max(1) as f64
                    });
                    present.trial_samples.extend(energy_sample());
                    Trial::Drawables {
                        frames: frames + 1,
                        from,
                        settled: Some(settled),
                        tiles,
                        ended: Some((drawables, drawables_settled, drawables_late)),
                    }
                }
                // Once the last sample is read (off this thread), the verdict.
                Trial::Drawables {
                    tiles,
                    ended: Some((drawables, drawables_settled, drawables_late)),
                    ..
                } if present
                    .trial_samples
                    .iter()
                    .all(|sample| sample.is_finished()) =>
                {
                    let samples: Vec<trial_energy::Sample> = present
                        .trial_samples
                        .drain(..)
                        .filter_map(|sample| sample.join().ok())
                        .collect();
                    let costs = match &samples[..] {
                        [tiles_from, tiles_to, drawables_from, drawables_to] => {
                            Some((tiles_from.to(tiles_to), drawables_from.to(drawables_to)))
                        }
                        _ => None,
                    };
                    let verdict = trial_verdict(
                        *TRIAL_RULE,
                        TrialResult {
                            tiles_lost: tiles,
                            drawables_lost: drawables,
                            drawables_lost_settled: drawables_settled,
                            drawables_late_settled: drawables_late,
                            tiles_watts: costs.map(|(tiles, _)| tiles.watts(TRIAL_GPU_WATTS)),
                            drawables_watts: costs
                                .map(|(_, drawables)| drawables.watts(TRIAL_GPU_WATTS)),
                        },
                    );
                    present.verdict = verdict;
                    verdicts::set(&present.verdict_key, verdict);
                    let elapsed = present
                        .trial_started
                        .map_or(0, |at| at.elapsed().as_millis());
                    let path = if verdict == 2 { "drawables" } else { "tiles" };
                    match (*TRIAL_RULE, costs) {
                        (TrialRule::Energy, Some((on_tiles, on_drawables))) => {
                            let term = |cost: &trial_energy::Cost| {
                                let per_second = |value: f64| value / cost.seconds.max(1e-3) * 1e3;
                                format!(
                                    "{:.0} mW (CPU {:.0} mW, GPU {:.0} and the window server's {:.0} ms a second)",
                                    cost.watts(TRIAL_GPU_WATTS) * 1e3,
                                    per_second(cost.cpu_joules),
                                    per_second(cost.gpu_seconds),
                                    per_second(cost.window_server_gpu_seconds),
                                )
                            };
                            eprintln!(
                                "zgx: full-rate frames take {path} (tiles {}, drawables {}, drawables lost {:.1}%, {:.1}% after warm-up, {:.1}% late; trial {elapsed} ms)",
                                term(&on_tiles),
                                term(&on_drawables),
                                drawables * 100.,
                                drawables_settled * 100.,
                                drawables_late * 100.,
                            );
                        }
                        _ => eprintln!(
                            "zgx: full-rate frames take {path} (lost: tiles {:.1}%, drawables {:.1}% of {} frames each; trial {elapsed} ms)",
                            tiles * 100.,
                            drawables * 100.,
                            TRIAL_FRAMES,
                        ),
                    }
                    Trial::Done
                }
                Trial::Drawables {
                    ended: Some(..), ..
                } => present.trial,
                Trial::Drawables {
                    frames,
                    from,
                    settled,
                    tiles,
                    ended: None,
                } => {
                    let feedback = present
                        .layer
                        .as_ref()
                        .and_then(|layer| layer.drawable_feedback());
                    let settled = match settled {
                        None if feedback.as_ref().is_some_and(|feedback| {
                            feedback.shown_since(from) >= TRIAL_WARMUP as u64
                        }) =>
                        {
                            present.trial_samples.extend(energy_sample());
                            Some(counts(feedback))
                        }
                        settled => settled,
                    };
                    Trial::Drawables {
                        frames: frames + 1,
                        from,
                        settled,
                        tiles,
                        ended: None,
                    }
                }
                Trial::Done => Trial::Done,
            };
        }
        // What the screen did with drawables lately (`layer_present::Feedback`).
        if frame.is_multiple_of(240)
            && let Some(feedback) = present
                .layer
                .as_ref()
                .and_then(|layer| layer.drawable_feedback())
        {
            let line = || {
                let (shown, skipped, median, p99, late) = feedback.summary();
                let (latency, latency_p99) = feedback.latency().unwrap_or_default();
                format!(
                    "drawables shown={shown} skipped={skipped} gap_p50={median:.2}ms gap_p99={p99:.2}ms late={late} start_to_shown_p50={latency:.2}ms p99={latency_p99:.2}ms"
                )
            };
            present_log::log(line);
        }
        self.layer_present = Some(present);
        Ok(PresentationStatus::Presented)
    }
    /// Make the swap chain's current buffer the retained target for the
    /// frame about to draw, unless one is: once the compositor lets a frame
    /// start (the swap chain's waitable), with the regions it lacks of the
    /// last frame queued to copy from the shown buffer (none when `whole`).
    #[cfg(windows)]
    fn begin_layer_frame(&mut self, whole: bool) -> Result<(), GpuError> {
        let (width, height) = (self.width, self.height);
        let Some(present) = self.layer_present.as_ref() else {
            return Ok(());
        };
        if present.current.is_some() {
            return Ok(());
        }
        if !self.background_matched {
            // The first frame, drawn whole: an opaque window whose background
            // lets what is behind show presents with alpha from it. Its
            // buffers may be the target: let go of them first.
            self.target = texture(&self.device, 1, 1, "retained output placeholder");
            self.view = self.target.create_view(&Default::default());
            self.target_copy_bind = None;
            self.match_background()?;
        }
        let Some(present) = self.layer_present.as_ref() else {
            return Ok(());
        };
        if present
            .buffers
            .first()
            .is_none_or(|(b, _)| (b.texture.width(), b.texture.height()) != (width, height))
        {
            // Made or resized: buffers of the window's size. The target may
            // be one of the old ones; it lets go first.
            self.target = texture(&self.device, 1, 1, "retained output placeholder");
            self.view = self.target.create_view(&Default::default());
            self.target_copy_bind = None;
            let present = self.layer_present.as_mut().expect("presenting");
            let chain = present.chain.as_mut().expect("a swap chain");
            chain.release(present.buffers.drain(..).map(|(buffer, _)| buffer));
            present.shown = None;
            present.history.clear();
            if chain.buffers_made() {
                chain.resize(width, height)?;
            }
            present.buffers = chain
                .buffers(&self.device, width, height)?
                .into_iter()
                .map(|buffer| (buffer, None))
                .collect();
        }
        let present = self.layer_present.as_mut().expect("presenting");
        let chain = present.chain.as_ref().expect("a swap chain");
        let started = std::time::Instant::now();
        let ready = chain.wait(1000);
        let index = chain.current();
        if !ready || started.elapsed().as_micros() > 500 {
            present_log::log(|| {
                format!(
                    "dxgi acquire index={index} waited_us={} ready={ready}",
                    started.elapsed().as_micros()
                )
            });
        }
        present.catch_up = present.catch_up_for(index, whole, width, height);
        present_log::log(|| {
            format!(
                "dxgi draw whole={whole} held={:?} catch_up={}",
                present.buffers[index].1,
                present.catch_up.len()
            )
        });
        let buffer = &present.buffers[index].0;
        self.target = buffer.texture.clone();
        self.view = buffer.view.clone();
        present.current = Some(index);
        Ok(())
    }
    /// Show the retained frame (the swap chain's current buffer), the
    /// regions it changed as the present's dirty rectangles.
    #[cfg(windows)]
    fn present_layer(
        &mut self,
        before_present: impl FnOnce(),
    ) -> Result<PresentationStatus, GpuError> {
        let Some(index) = self
            .layer_present
            .as_mut()
            .and_then(|present| present.current.take())
        else {
            // Nothing drawn since: the shown frame stands.
            return Ok(PresentationStatus::Presented);
        };
        // Direct3D 11 submitted the frame as it recorded it (d3d11_frame.rs).
        #[cfg(d3d11)]
        let recorded = self.d3d11.is_some();
        #[cfg(not(d3d11))]
        let recorded = false;
        if !recorded {
            let mut encoder = self
                .pending
                .get_mut()
                .take()
                .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
            {
                let present = self.layer_present.as_ref().expect("presenting");
                // Presented, a buffer is in the common state the compositor reads.
                encoder.transition_resources(
                    std::iter::empty(),
                    std::iter::once(wgpu::TextureTransition {
                        texture: &present.buffers[index].0.texture,
                        selector: None,
                        state: wgpu::TextureUses::PRESENT,
                    }),
                );
            }
            self.queue.submit([encoder.finish()]);
            self.belt.get_mut().recall();
            self.submitted();
            if let Some(mapped) = self.mapped.as_mut() {
                mapped.get_mut().submitted();
            }
        }
        before_present();
        let present = self.layer_present.as_mut().expect("presenting");
        present_log::log(|| {
            format!(
                "dxgi present frame={} index={index} held={:?} dirty={}",
                present.frame + 1,
                present.buffers[index].1,
                present.changed.len()
            )
        });
        let chain = present.chain.as_mut().expect("a swap chain");
        let seen = chain.present(&present.changed)?;
        self.dxgi_stats.presented_chain(chain.swap_chain());
        // The buffer holds the frame either way: the swap chain's index says
        // which the next draws into.
        present.presented(index);
        Ok(if seen {
            PresentationStatus::Presented
        } else {
            PresentationStatus::Occluded
        })
    }
    /// Whether a frame drawn into the renderer's own surfaces waits to be
    /// presented (`present` then shows it; without one it does nothing).
    #[cfg(target_os = "macos")]
    pub fn frame_pending(&self) -> bool {
        self.layer_present
            .as_ref()
            .is_some_and(|present| present.current.is_some())
    }
    /// The frame its layer shows (a popup window's), for an image of it (a
    /// drag image): width and height in pixels, then premultiplied BGRA rows.
    /// Waits for the GPU to finish it.
    #[cfg(target_os = "macos")]
    pub fn shown_pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        let frame = self.shown_frame()?;
        // Direct frames are waited for before they are shown; wgpu's here.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        frame.read(0, 0, u32::MAX, u32::MAX)
    }
    /// The surface its layer shows now, to read a region of later
    /// (`ShownFrame::read`): what the window last showed. Holding it keeps
    /// only that surface, which the renderer does not draw into while it is
    /// shown; take a new one after each frame.
    #[cfg(target_os = "macos")]
    pub fn shown_frame(&self) -> Option<ShownFrame> {
        let present = self.layer_present.as_ref()?;
        let (buffer, _) = present.buffers.get(present.shown?)?;
        Some(ShownFrame {
            surface: buffer.surface.clone(),
            pending: buffer
                .drawable
                .then(|| self.metal_inflight.borrow().clone())
                .flatten(),
            over: self.over_frames(),
        })
    }
    /// Diagnostics: split each frame drawn with Metal directly at its
    /// backdrop blur's passes (reduce, across, down), each committed on its
    /// own, for `debug_take_pass_times`. Off by default; it adds a command
    /// buffer a pass while on. Process-wide.
    #[cfg(target_os = "macos")]
    pub fn debug_time_passes(&self, on: bool) {
        metal_frame::TIME_PASSES.store(on, std::sync::atomic::Ordering::Relaxed);
    }
    /// What `debug_time_passes` split off since last asked, on this thread:
    /// each part's GPU time in microseconds, in the order it ran ("frame
    /// before the blur", "blur reduce", "blur across", "blur down", "frame
    /// after"), waited for.
    #[cfg(target_os = "macos")]
    pub fn debug_take_pass_times(&self) -> Vec<(&'static str, f64)> {
        metal_frame::take_pass_times()
    }
    /// How long the GPU took to run the frame last presented, when it was
    /// encoded with Metal directly: the frame's own work, plus however long
    /// other work on the GPU (the window server's compositing, other apps)
    /// held it up. Taken: `None` until another such frame presents.
    #[cfg(target_os = "macos")]
    pub fn take_gpu_time(&mut self) -> Option<std::time::Duration> {
        self.gpu_time.take()
    }
    /// Whether frames come at the display's full rate (animating or
    /// scrolling). On macOS they may keep a third surface, when the window
    /// server still holds both others as the next frame starts (with
    /// `ZGX_PRESENT_VIDEO`, the video layer's queue does); slower frames make
    /// do with two. Elsewhere ignored.
    pub fn set_full_rate(&mut self, #[allow(unused)] full: bool) {
        #[cfg(target_os = "macos")]
        if let Some(present) = &mut self.layer_present {
            if present.full_rate && !full {
                present.video_until = Some(std::time::Instant::now() + VIDEO_LINGER);
            } else if full {
                present.video_until = None;
            }
            present.full_rate = full;
        }
    }
    /// Whether a scroll is live in the window (a gesture, its momentum or a
    /// fling): on macOS, with `ZGX_SCROLL_DRAWABLES=1`, its full-rate frames
    /// then draw into drawables, which Metal presents once their GPU work is
    /// done, so the main thread never waits for that work. Elsewhere ignored.
    pub fn set_scrolling(&mut self, #[allow(unused)] scrolling: bool) {
        #[cfg(target_os = "macos")]
        if let Some(present) = &mut self.layer_present {
            present.scrolling = scrolling;
        }
    }
    /// Whether the frame just rendered presents itself (macOS): drawn into a
    /// drawable its own command buffer presents, it shows once the GPU is
    /// done whatever the main thread does, so there is nothing to hand over
    /// at a set time.
    pub fn frame_presents_itself(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.layer_present.as_ref().is_some_and(|present| {
                present
                    .current
                    .is_some_and(|index| present.buffers[index].0.drawable)
            }) && self.metal_inflight.borrow().is_some()
        }
        #[cfg(not(target_os = "macos"))]
        false
    }
    /// Encode plain frames with Metal directly (the default on macOS) or
    /// always through wgpu: for comparing the two. Elsewhere ignored.
    #[doc(hidden)]
    pub fn set_direct_metal(&mut self, #[allow(unused)] enabled: bool) {
        #[cfg(target_os = "macos")]
        {
            self.flush_pending();
            self.metal_frame = if enabled {
                Default::default()
            } else {
                std::cell::OnceCell::from(None)
            };
        }
    }
    /// Notify the native host immediately before committing an acquired frame.
    /// Skipped acquisition never invokes the callback, so Wayland hosts do not
    /// wait for a frame callback on a surface commit that never happened.
    pub fn present_with_notify(
        &mut self,
        before_present: impl FnOnce(),
    ) -> Result<PresentationStatus, GpuError> {
        let status = {
            let _span = zgx::profile::span(zgx::profile::Phase::Present);
            let status = self.present_frame(before_present);
            #[cfg(target_os = "macos")]
            if let Some(layer) = self
                .layer_present
                .as_mut()
                .and_then(|present| present.layer.as_mut())
            {
                layer.flush_loops();
            }
            #[cfg(target_os = "macos")]
            self.check_loops(match &status {
                Ok(PresentationStatus::Presented) => "presented",
                Ok(_) => "not presented",
                Err(_) => "present failed",
            });
            #[cfg(target_os = "macos")]
            if status.is_err() {
                self.hide_loops();
            }
            status
        };
        // The frame is done once shown (see `render`).
        if self.presents() {
            zgx::profile::frame();
        }
        status
    }
    /// Copy into `frame`'s image only what it lacks of the retained target
    /// (`SwapHistory::lacks`), through Vulkan itself: wgpu tracks a surface
    /// texture it acquires as uninitialized, and a copy of its own into part
    /// of one clears all of it first. `encoder` (wgpu's) takes the
    /// transitions; the copy is recorded in a command buffer of its own,
    /// returned to submit after it (wgpu takes no encoder holding both).
    /// `None` if it didn't (another backend, an image whose frame isn't
    /// known, most of it changed): the caller copies all of it.
    #[cfg(target_os = "linux")]
    fn copy_lacking(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::SurfaceTexture,
    ) -> Option<Option<wgpu::CommandBuffer>> {
        use ash::vk::{self, Handle as _};
        let size = self.target.size();
        let image = swap_image(&frame.texture)?;
        let rects = self.swap.lacks(image, (size.width, size.height))?;
        let area: u64 = rects.iter().map(|r| u64::from(r.2) * u64::from(r.3)).sum();
        if rects.len() > SWAP_COPIES || area * 2 > u64::from(size.width) * u64::from(size.height) {
            return None;
        }
        let source = unsafe { self.target.as_hal::<wgpu::hal::api::Vulkan>() }
            .map(|texture| unsafe { texture.raw_handle() });
        let device = unsafe { self.device.as_hal::<wgpu::hal::api::Vulkan>() }
            .map(|device| device.raw_device().clone());
        let (Some(source), Some(device)) = (source, device) else {
            return None;
        };
        if rects.is_empty() {
            return Some(None);
        }
        // The target to copy from, the image to copy to, as wgpu tracks them
        // (its barriers; the image's from where it was presented).
        encoder.transition_resources(
            std::iter::empty(),
            [
                wgpu::TextureTransition {
                    texture: &self.target,
                    selector: None,
                    state: wgpu::TextureUses::COPY_SRC,
                },
                wgpu::TextureTransition {
                    texture: &frame.texture,
                    selector: None,
                    state: wgpu::TextureUses::COPY_DST,
                },
            ]
            .into_iter(),
        );
        let layers = vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        };
        let regions: Vec<vk::ImageCopy> = rects
            .iter()
            .map(|&(x, y, width, height)| {
                let offset = vk::Offset3D {
                    x: x as i32,
                    y: y as i32,
                    z: 0,
                };
                vk::ImageCopy {
                    src_subresource: layers,
                    src_offset: offset,
                    dst_subresource: layers,
                    dst_offset: offset,
                    extent: vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    },
                }
            })
            .collect();
        // SAFETY: both images are in the layouts the transitions above left
        // them in (submitted before this command buffer); the regions lie
        // within both (clipped to the target, the image's size); the command
        // buffer is the copy encoder's own, recording.
        let mut copy = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("swap copy"),
            });
        unsafe {
            copy.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|raw| {
                if let Some(raw) = raw {
                    device.cmd_copy_image(
                        raw.raw_handle(),
                        source,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        vk::Image::from_raw(image),
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &regions,
                    );
                }
            });
        }
        present_log::log(|| {
            format!(
                "swap copy rects={} px={area} of {}",
                rects.len(),
                size.width * size.height
            )
        });
        if *SWAP_CHECK {
            self.check_swap(encoder, &mut copy, image, source, &device);
        }
        Some(Some(copy.finish()))
    }
    /// `ZGX_SWAP_CHECK`: after a partial copy, the image and the target read
    /// back (through Vulkan, as the copy was: wgpu would clear the image it
    /// takes for uninitialized) and compared once the frame is submitted
    /// (`swap_check_compare`): an image that kept less than it was taken to
    /// shows where.
    #[cfg(target_os = "linux")]
    fn check_swap(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        copy: &mut wgpu::CommandEncoder,
        image: u64,
        source: ash::vk::Image,
        device: &ash::Device,
    ) {
        use ash::vk::{self, Handle as _};
        let size = self.target.size();
        let bytes = u64::from(size.width) * u64::from(size.height) * 4;
        let buffers = [0, 1].map(|_| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("swap check"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        // Written through wgpu first, so mapping reads what the copies wrote
        // rather than clearing a buffer it holds uninitialized.
        for buffer in &buffers {
            encoder.clear_buffer(buffer, 0, None);
        }
        let raw = buffers.each_ref().map(|buffer| {
            unsafe { buffer.as_hal::<wgpu::hal::api::Vulkan>() }.map(|b| unsafe { b.raw_handle() })
        });
        let [Some(read_image), Some(read_target)] = raw else {
            return;
        };
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let region = vk::BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D::default(),
            image_extent: vk::Extent3D {
                width: size.width,
                height: size.height,
                depth: 1,
            },
        };
        let swap = vk::Image::from_raw(image);
        let barrier = |old, new, from, to| {
            vk::ImageMemoryBarrier::default()
                .image(swap)
                .subresource_range(range)
                .old_layout(old)
                .new_layout(new)
                .src_access_mask(from)
                .dst_access_mask(to)
        };
        // SAFETY: the image is in TRANSFER_DST (the copy's), and goes back
        // there; the target is in TRANSFER_SRC; the buffers hold the whole of
        // either, cleared before (the memory barrier orders it).
        unsafe {
            copy.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|raw| {
                let Some(raw) = raw else { return };
                let cmd = raw.raw_handle();
                let transfer = vk::PipelineStageFlags::TRANSFER;
                device.cmd_pipeline_barrier(
                    cmd,
                    transfer,
                    transfer,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .dst_access_mask(
                            vk::AccessFlags::TRANSFER_WRITE | vk::AccessFlags::TRANSFER_READ,
                        )],
                    &[],
                    &[barrier(
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        vk::AccessFlags::TRANSFER_WRITE,
                        vk::AccessFlags::TRANSFER_READ,
                    )],
                );
                device.cmd_copy_image_to_buffer(
                    cmd,
                    swap,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    read_image,
                    &[region],
                );
                device.cmd_copy_image_to_buffer(
                    cmd,
                    source,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    read_target,
                    &[region],
                );
                device.cmd_pipeline_barrier(
                    cmd,
                    transfer,
                    transfer | vk::PipelineStageFlags::HOST,
                    vk::DependencyFlags::empty(),
                    &[vk::MemoryBarrier::default()
                        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                        .dst_access_mask(vk::AccessFlags::HOST_READ)],
                    &[],
                    &[barrier(
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        vk::AccessFlags::TRANSFER_READ,
                        vk::AccessFlags::TRANSFER_WRITE,
                    )],
                );
            });
        }
        self.swap.checks.push(buffers);
    }
    /// The frame's `ZGX_SWAP_CHECK` reads, now submitted, compared.
    #[cfg(target_os = "linux")]
    fn swap_check_compare(&mut self) {
        let width = self.target.size().width as usize;
        for [image, target] in std::mem::take(&mut self.swap.checks) {
            for buffer in [&image, &target] {
                buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            }
            if self
                .device
                .poll(wgpu::PollType::wait_indefinitely())
                .is_err()
            {
                return;
            }
            let (a, b) = (
                image.slice(..).get_mapped_range(),
                target.slice(..).get_mapped_range(),
            );
            let stale: Vec<usize> = a
                .as_chunks::<4>()
                .0
                .iter()
                .zip(b.as_chunks::<4>().0)
                .enumerate()
                .filter(|(_, (x, y))| x != y)
                .map(|(i, _)| i)
                .collect();
            self.swap.checked += 1;
            if let (Some(first), Some(last)) = (stale.first(), stale.last()) {
                let (x0, y0) = (first % width, first / width);
                let (x1, y1) = (last % width, last / width);
                eprintln!(
                    "ZGX_SWAP_CHECK present {}: {} pixels of the image differ from the frame, ({x0},{y0})..({x1},{y1})",
                    self.swap.checked,
                    stale.len()
                );
            }
        }
    }
    /// What a frame drawn changes in the retained target, for the swap
    /// chain's images to take at the present (`SwapHistory`). Out of line:
    /// every frame's walk (`render_flat_taking`) compiles as it did without
    /// it.
    #[cfg(target_os = "linux")]
    #[cold]
    #[inline(never)]
    fn swap_changes(&mut self, regions: &[(Rect, Scissor)], copies: &[ScrollCopy]) {
        self.swap.changed.extend(regions.iter().map(|(_, s)| *s));
        self.swap.changed.extend(copies.iter().map(|(s, _)| *s));
    }
    /// `frame`'s image now holds the frame drawn; what it changed since the
    /// last present is the next present's regions.
    #[cfg(target_os = "linux")]
    fn swap_presented(&mut self, frame: &wgpu::SurfaceTexture) {
        let Some(image) = swap_image(&frame.texture) else {
            self.swap.changed.clear();
            return;
        };
        if let Some(surface) = &self.surface
            && let Some(hal) = unsafe { surface.as_hal::<wgpu::hal::api::Vulkan>() }
        {
            let size = self.target.size();
            let whole = self
                .swap
                .changed
                .iter()
                .any(|r| r.2 >= size.width && r.3 >= size.height);
            if !whole && self.swap.changed.len() <= SWAP_COPIES {
                hal.set_next_present_regions(
                    self.swap
                        .changed
                        .iter()
                        .map(|&(x, y, width, height)| ash::vk::RectLayerKHR {
                            offset: ash::vk::Offset2D {
                                x: x as i32,
                                y: y as i32,
                            },
                            extent: ash::vk::Extent2D { width, height },
                            layer: 0,
                        })
                        .collect(),
                );
            }
        }
        self.swap.presented(image);
    }
    fn present_frame(
        &mut self,
        before_present: impl FnOnce(),
    ) -> Result<PresentationStatus, GpuError> {
        if let Some(reason) = self.context.lost() {
            return Err(GpuError::device_lost(&reason));
        }
        #[cfg(own_surfaces)]
        if self.layer_present.is_some() {
            return self.present_layer(before_present);
        }
        if self.surface.is_none() {
            return Ok(PresentationStatus::Offscreen);
        }
        let frame = match surface::acquire(self)? {
            surface::Acquired::Frame(frame) => frame,
            surface::Acquired::Skipped(status) => {
                // The retained target must still be complete for the retry.
                self.flush_pending();
                return Ok(status);
            }
        };
        let mut encoder = self
            .pending
            .get_mut()
            .take()
            .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
        // A partial copy's own command buffer, after the frame's (Linux).
        #[allow(unused_mut)]
        let mut raw_copy: Option<wgpu::CommandBuffer> = None;
        if let Some(deferred) = self.deferred.get_mut().take() {
            let format = self
                .single_pass_format
                .expect("deferred only with a view format");
            let raw = frame.texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                ..Default::default()
            });
            self.present_single_pass(&mut encoder, &deferred, &raw, format);
        } else if self.copy_present {
            // Linux, Vulkan: what this image lacks of the frame alone.
            #[cfg(target_os = "linux")]
            let copied = self
                .copy_lacking(&mut encoder, &frame)
                .map(|copy| raw_copy = copy);
            #[cfg(not(target_os = "linux"))]
            let copied: Option<()> = None;
            if copied.is_none() {
                encoder.copy_texture_to_texture(
                    self.target.as_image_copy(),
                    frame.texture.as_image_copy(),
                    self.target.size(),
                );
                present_log::log(|| "swap copy whole".into());
            }
        } else {
            let view = frame.texture.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            pass.set_pipeline(self.blit.as_ref().unwrap());
            pass.set_bind_group(0, self.blit_bind.as_ref().unwrap(), &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue
            .submit(std::iter::once(encoder.finish()).chain(raw_copy));
        self.belt.get_mut().recall();
        self.submitted();
        if let Some(mapped) = self.mapped.as_mut() {
            mapped.get_mut().submitted();
        }
        #[cfg(target_os = "linux")]
        if *SWAP_CHECK {
            self.swap_check_compare();
        }
        // The image holds this frame now; what the frame changed is what
        // the compositor redraws (`VK_KHR_incremental_present`).
        #[cfg(target_os = "linux")]
        self.swap_presented(&frame);
        before_present();
        frame.present();
        #[cfg(windows)]
        if let Some(surface) = &self.surface {
            self.dxgi_stats.presented(surface);
        }
        // Transactional presents block on scheduling; keep them to resizes.
        #[cfg(target_os = "macos")]
        self.set_transaction_present(false);
        #[cfg(target_os = "macos")]
        if self
            .wide_until
            .is_some_and(|until| std::time::Instant::now() >= until)
        {
            self.narrow_drawables();
            // Free the third drawable now rather than when the pool turns over.
            self.release_drawables();
        }
        Ok(PresentationStatus::Presented)
    }
    /// Release memory that only animation needs: filter scratch textures,
    /// idle vertex buffers and, on macOS, Core Animation's pool of window-sized
    /// drawables (two of three, ~13 MB each at 5K). The window keeps showing
    /// its last frame; the next frame recreates whatever it uses. Call when a
    /// window has stopped presenting for a while.
    pub fn trim(&mut self) {
        self.backdrop = None;
        #[cfg(d3d11)]
        if let Some(state) = &mut self.d3d11 {
            state.popup_backdrop = None;
        }
        present_log::log(|| {
            #[cfg(target_os = "macos")]
            let plane = self.layer_present.as_ref().map_or("none", |p| p.plane());
            #[cfg(not(target_os = "macos"))]
            let plane = "none";
            format!("{plane} trim")
        });
        self.flush_pending();
        // Fade bands' textures: a window at rest shows none being drawn.
        self.release_bands();
        // The font cache read at launch, checked as the host first goes idle.
        font_cache_check();
        self.blur_scratch = None;
        // A frame's quads and draws: the next frame makes them again.
        self.frame_quads = Vec::new();
        self.frame_points = Vec::new();
        self.frame_draws = Vec::new();
        self.visible_glyphs = Vec::new();
        self.frame_peaks = (0, 0);
        // Effect slots draw again from their layers when next damaged.
        self.effects = None;
        // Blended layers' backdrops: made again for the next blend.
        self.backdrops = FxHashMap::default();
        self.drop_unheld_shaders();
        self.images.remove(&effects::EFFECT_ATLAS);
        for id in std::mem::take(&mut self.images_gone) {
            self.images.remove(&id);
            self.image_drawn.remove(&id);
        }
        // The ramps whose textures just went: their pixels too, now, not at
        // the next frame's prune.
        let images = &self.images;
        self.ramps.retain(|_, ramp| images.contains_key(&ramp.id()));
        self.spare = None;
        self.layers.retain(|_, cache| !cache.parked);
        // Surfaces the last frame took out: no frame may come to let them go.
        #[cfg(target_os = "macos")]
        if let Some(cache) = &mut self.native_surfaces {
            cache.rest();
        }
        self.retain_views();
        // Idle: only the shown surface is needed; the next frame makes another.
        #[cfg(target_os = "macos")]
        if let Some(present) = &mut self.layer_present
            && present.current.is_none()
            && let Some(shown) = present.shown
        {
            let kept = present.buffers.swap_remove(shown);
            present.buffers = vec![kept];
            present.shown = Some(0);
            if let Some(layer) = &mut present.layer {
                layer.rest(&present.buffers[0].0);
                layer.keep(std::iter::once(present.buffers[0].0.surface.id()));
                // At rest on a tile, the drawables' pool goes too.
                if !present.buffers[0].0.drawable {
                    layer.release_drawables();
                }
            }
        }
        // Vertex memory, from the largest frame drawn (a long list scrolled,
        // a big page): back to a small frame's. The ring drops the buffers
        // the GPU is done with (their mapping callbacks run first); the
        // plain path's buffer goes back to its first size, and its staging
        // belt's chunks go with the belt. The next frame makes what it needs.
        if let Some(mapped) = &mut self.mapped {
            let _ = self.device.poll(wgpu::PollType::Poll);
            let ring = mapped.get_mut();
            ring.trim();
            self.vertex_capacity = ring.bytes();
        } else if self.vertex_capacity > VERTICES_AT_REST {
            self.vertex_capacity = VERTICES_AT_REST;
            self.vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("retained frame vertices"),
                size: VERTICES_AT_REST as u64,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            self.belt = RefCell::new(wgpu::util::StagingBelt::new(
                self.device.clone(),
                256 * 1024,
            ));
        }
        #[cfg(d3d11)]
        if let Some(state) = &mut self.d3d11 {
            state.ring.trim();
            state.spare = None;
        }
        // Word shapes kept for the next frame, and the table they peaked in
        // (streaming fills it): the next frame shapes what it needs again.
        {
            let mut fonts = self.fonts.borrow_mut();
            fonts.shape_run_cache = Default::default();
            fonts.rest();
        }
        self.text_cache
            .borrow_mut()
            .rest(self.shapes.values().filter_map(|shaped| shaped.prepared));
        self.svgs.rest();
        #[cfg(target_os = "macos")]
        {
            self.narrow_drawables();
            self.release_drawables();
            self.idle = true;
        }
    }
    /// Metal's queue let go of, for a window at rest a while (`rest_queue`'s
    /// caller waits `QUEUE_REST_DELAY` past `trim`): the driver's memory for it,
    /// up to 174 MB, given back. A queue idle that long is as slow to start
    /// again as a new one, so the next frame pays nothing more for it. Nothing
    /// elsewhere.
    pub fn rest_queue(&mut self) {
        #[cfg(target_os = "macos")]
        self.renew_queue();
    }
    /// Metal's queue let go of, the driver's memory for it given back
    /// (`Queue::renew`), with all this renderer gave it finished and let go of
    /// (a command buffer holds its queue).
    #[cfg(target_os = "macos")]
    fn renew_queue(&mut self) {
        self.finish_direct();
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        // SAFETY: Only the queue's handle goes; wgpu's work on it has been waited
        // for, and nothing is being encoded (`renew` looks).
        if let Some(queue) = unsafe { self.queue.as_hal::<wgpu::hal::api::Metal>() } {
            queue.renew();
        }
    }
    /// How many effects (layer shaders with inputs) hold textures here.
    #[doc(hidden)]
    pub fn effect_chains(&self) -> usize {
        self.chains.len()
    }
    /// Drop the fill and layer shaders no one holds but this renderer (a
    /// removed node's, a reloaded script's: ids are never reused), with
    /// their pipelines, unless the kept frame still draws them.
    fn drop_unheld_shaders(&mut self) {
        if self.fill_shaders.is_empty() && self.filter_shaders.is_empty() {
            return;
        }
        let drawn: rustc_hash::FxHashSet<u64> = self
            .kept
            .iter()
            .flat_map(|kept| &kept.draws)
            .filter_map(|draw| match draw.shading {
                Shading::Fill(id) | Shading::Filter(id) | Shading::FilterBlend(id) => Some(id),
                _ => None,
            })
            .collect();
        let held = |id: &u64, shader: &mut Arc<zgx::decoration::FillShader>| {
            Arc::strong_count(shader) > 1 || drawn.contains(id)
        };
        self.fill_shaders.retain(held);
        self.filter_shaders.retain(held);
        let (fills, filters) = (&self.fill_shaders, &self.filter_shaders);
        self.fill_pipelines.retain(|id, _| fills.contains_key(id));
        self.filter_pipelines
            .retain(|id, _| filters.contains_key(id));
        self.chains.retain(|id, _| filters.contains_key(id));
        self.blend_pipelines
            .retain(|id, _| *id == BLEND_PLAIN || filters.contains_key(id));
        #[cfg(target_os = "macos")]
        if let Some(Some(frame)) = self.metal_frame.get() {
            frame.retain_fills(|id| fills.contains_key(&id));
        }
        #[cfg(d3d11)]
        if let Some(state) = &self.d3d11 {
            state
                .device
                .retain_app_shaders(|id| fills.contains_key(&id) || filters.contains_key(&id));
        }
    }
    /// Input arrived: if the window idled (`trim`) and has drawn nothing
    /// since, wake the GPU now with a trivial pass, committed without
    /// waiting, so the frame the input makes need not wait for it to wake.
    /// On macOS, unless `ZGX_GPU_WARM=0`; elsewhere nothing.
    #[doc(hidden)]
    pub fn warm_up(&mut self) {
        #[cfg(target_os = "macos")]
        if *WARM_AFTER_IDLE
            && std::mem::take(&mut self.idle)
            && let Some(Some(frame)) = self.metal_frame.get()
        {
            present_log::log(|| "warm".into());
            frame.warm();
            self.warms += 1;
        }
    }
    #[cfg(target_os = "macos")]
    fn release_drawables(&mut self) {
        let Some(surface) = &self.surface else {
            return;
        };
        // SAFETY: Only the surface's layer is used; nothing destroys it.
        let Some(hal) = (unsafe { surface.as_hal::<wgpu::hal::api::Metal>() }) else {
            return;
        };
        let layer = hal.render_layer().lock();
        // Changing drawableSize discards pooled drawables; restoring it at once
        // leaves the configuration and the displayed contents untouched.
        // SAFETY: `drawableSize` is a CGSize property of CAMetalLayer.
        unsafe {
            let size: objc2_core_foundation::CGSize = objc2::msg_send![&**layer, drawableSize];
            let nudged = objc2_core_foundation::CGSize::new(size.width, size.height + 1.);
            let _: () = objc2::msg_send![&**layer, setDrawableSize: nudged];
            let _: () = objc2::msg_send![&**layer, setDrawableSize: size];
        }
    }
    /// A resized layer holds its transactional drawable until the window
    /// server shows the new frame size, up to a second later. With two
    /// drawables the next frame waits that long for one (every step of a
    /// drag resize would stall): keep a third until resizing settles.
    #[cfg(target_os = "macos")]
    fn widen_drawables(&mut self) {
        // Each resize configures the surface, which restores two.
        self.set_drawable_count(3);
        self.wide_until = Some(std::time::Instant::now() + WIDE_DRAWABLES);
    }
    #[cfg(target_os = "macos")]
    fn narrow_drawables(&mut self) {
        if self.wide_until.take().is_some() {
            self.set_drawable_count(2);
        }
    }
    #[cfg(target_os = "macos")]
    fn set_drawable_count(&self, count: usize) {
        let Some(surface) = &self.surface else {
            return;
        };
        // SAFETY: Only the surface's layer is used; nothing destroys it.
        let Some(hal) = (unsafe { surface.as_hal::<wgpu::hal::api::Metal>() }) else {
            return;
        };
        let layer = hal.render_layer().lock();
        // SAFETY: `maximumDrawableCount` is an NSUInteger property of CAMetalLayer.
        let _: () = unsafe { objc2::msg_send![&**layer, setMaximumDrawableCount: count] };
    }
    #[cfg(target_os = "macos")]
    fn set_transaction_present(&mut self, enabled: bool) {
        if self.transaction_present == enabled {
            return;
        }
        let Some(surface) = &self.surface else {
            return;
        };
        // SAFETY: Only the surface's layer is used; nothing destroys it.
        let Some(hal) = (unsafe { surface.as_hal::<wgpu::hal::api::Metal>() }) else {
            return;
        };
        let layer = hal.render_layer().lock();
        // SAFETY: `presentsWithTransaction` is a BOOL property of CAMetalLayer.
        let _: () = unsafe { objc2::msg_send![&**layer, setPresentsWithTransaction: enabled] };
        self.transaction_present = enabled;
    }
    /// Frames whose damage was drawn in the same pass as presentation.
    /// Frames presented to the drawable alone, the retained target untouched.
    pub fn debug_direct_frames(&self) -> u64 {
        self.direct_frames
    }
    /// Frames drawn in one pass with their presentation.
    pub fn debug_single_pass_frames(&self) -> u64 {
        self.single_pass_frames
    }
    /// Test hook: copy scroll moves' pixels offscreen too, where the platform
    /// would repaint them (macOS without a layer of its own), so a test can
    /// check what a copy leaves on screen.
    pub fn debug_allow_scroll_copies(&mut self) {
        self.debug_scroll_copy = true;
    }
    /// How many scroll moves were applied as a copy of their pixels.
    pub fn debug_scroll_copies(&self) -> u64 {
        self.scroll_copied.get()
    }
    /// Test hook: full-rate frames draw into `CAMetalLayer` drawables, or
    /// not (`ZGX_PRESENT_DRAWABLE`); a `debug_for_layer` renderer then
    /// presents as a window's base plane does, not an overlay's.
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn debug_present_drawable(&mut self, drawables: bool) {
        if let Some(present) = &mut self.layer_present {
            present.drawables = Some(drawables);
            present.plain_only = false;
        }
    }
    /// The display the window shows on and its refresh rate
    /// (`"<id>@<hz>"`), part of the full-rate verdict's key.
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn set_display_key(&mut self, key: &str) {
        if self.display_key != key {
            self.display_key = key.to_owned();
        }
    }
    /// The host begins a frame (macOS): its start to its showing is the
    /// drawables' latency (`debug_drawable_latency`, the present log).
    #[cfg(target_os = "macos")]
    pub fn set_frame_start(&mut self, at: std::time::Instant) {
        self.frame_started = Some(at);
    }
    /// Whether full-rate frames present as drawables now (macOS): the host
    /// paces them on the view's own display link then.
    #[cfg(target_os = "macos")]
    pub fn drawables_showing(&self) -> bool {
        self.layer_present
            .as_ref()
            .and_then(|present| present.layer.as_ref())
            .is_some_and(|layer| layer.drawables_shown())
    }
    /// Whether canvases' filled areas draw on the GPU here, or rasterize
    /// (Direct3D 12 compiling with FXC, which can't compile their shader).
    #[doc(hidden)]
    pub fn areas_on_gpu(&self) -> bool {
        self.context.inner.quads.areas
    }
    /// Frames wait for drawables a refresh behind (macOS): the host skips
    /// one tick to put them back in phase. The last wait, once.
    #[cfg(target_os = "macos")]
    pub fn take_rephase(&mut self) -> Option<std::time::Duration> {
        self.layer_present.as_mut()?.layer.as_mut()?.take_rephase()
    }
    /// Test hook: the latest drawables' start (`set_frame_start`) to their
    /// showing, median and 99th percentile in ms (macOS, while feedback is
    /// read: the present log or the trial).
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn debug_drawable_latency(&self) -> Option<(f64, f64)> {
        let present = self.layer_present.as_ref()?;
        present.layer.as_ref()?.drawable_feedback()?.latency()
    }
    /// Test hook: drawables the window server presented and skipped so far,
    /// and the latest gaps' median and 99th percentile in ms (macOS).
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn debug_drawable_feedback(&self) -> Option<(u64, u64, f64, f64, usize)> {
        let present = self.layer_present.as_ref()?;
        Some(present.layer.as_ref()?.drawable_feedback()?.summary())
    }
    /// Frames encoded with Metal directly rather than through wgpu (macOS).
    pub fn debug_metal_frames(&self) -> u64 {
        self.metal_frames
    }
    /// Frames drawn through a stencil of their damage: none, as frames no
    /// longer draw so (kept for 0.1.0's callers).
    #[deprecated(
        since = "0.1.1",
        note = "frames no longer draw through a stencil: always 0"
    )]
    #[doc(hidden)]
    pub fn debug_stamped_frames(&self) -> u64 {
        0
    }
    /// Test hook kept for 0.1.0's callers: frames no longer draw through a
    /// stencil, so it does nothing.
    #[deprecated(
        since = "0.1.1",
        note = "frames no longer draw through a stencil: does nothing"
    )]
    #[doc(hidden)]
    pub fn debug_stencil(&mut self, _stencil: bool) {}
    /// Passes that woke the GPU for input after idling (`warm_up`).
    #[doc(hidden)]
    pub fn debug_warms(&self) -> u64 {
        self.warms
    }
    /// Test hook: a renderer presenting through surfaces of its own, as a
    /// window's does, into `host` (a layer of the caller's), without a
    /// window: `width` by `height` pixels.
    #[cfg(target_os = "macos")]
    #[doc(hidden)]
    pub fn debug_for_layer(
        host: objc2::rc::Retained<objc2_quartz_core::CALayer>,
        width: u32,
        height: u32,
    ) -> Result<Self, GpuError> {
        let context = GpuContext::new()?;
        let mut renderer = Self::create(None, width, height, &context)?;
        if renderer.info.backend != wgpu::Backend::Metal {
            return Err(GpuError(
                "layer presentation needs the Metal backend".into(),
            ));
        }
        renderer.layer_present = Some(LayerPresent {
            host: Some(host),
            plain_only: true,
            ..Default::default()
        });
        Ok(renderer)
    }
    /// Test hook: draw plain quads with the basic shader, or every quad with
    /// the full shader.
    pub fn debug_split_shading(&mut self, split: bool) {
        self.split_shading = split;
    }
    /// Test hook: make quads for the lines of a text partly in its clip
    /// alone (the default), or for all of them.
    pub fn debug_cull_glyphs(&mut self, cull: bool) {
        self.cull_glyphs = cull;
    }
    /// Test hook: treat an offscreen `Bgra8Unorm` texture as the drawable, so
    /// single-pass presentation runs without a window.
    pub fn debug_enable_single_pass(&mut self) {
        self.single_pass_format = Some(wgpu::TextureFormat::Bgra8Unorm);
        self.present_unpremultiplies = !cfg!(target_os = "macos");
    }
    /// Test hook: present into a fresh offscreen drawable configured like a
    /// window (`Bgra8UnormSrgb`, non-opaque) and return its
    /// bytes in RGBA order. Single-pass renderers draw the pending frame
    /// through a non-sRGB view; others use the present blit, as on screen.
    pub fn debug_present_offscreen(&mut self) -> Result<Vec<u8>, GpuError> {
        // Direct3D 11 presents through its own swap chain alone: the no-op
        // shell would read back nothing.
        #[cfg(d3d11)]
        if self.d3d11.is_some() {
            return Err(GpuError(
                "ZGX_D3D11 presents through no wgpu surface, offscreen either".into(),
            ));
        }
        let srgb = wgpu::TextureFormat::Bgra8UnormSrgb;
        let drawable = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen drawable"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: srgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[wgpu::TextureFormat::Bgra8Unorm],
        });
        let mut encoder = self
            .pending
            .get_mut()
            .take()
            .unwrap_or_else(|| self.device.create_command_encoder(&Default::default()));
        if let Some(frame) = self.deferred.get_mut().take() {
            let raw = drawable.create_view(&wgpu::TextureViewDescriptor {
                format: Some(wgpu::TextureFormat::Bgra8Unorm),
                ..Default::default()
            });
            self.present_single_pass(&mut encoder, &frame, &raw, wgpu::TextureFormat::Bgra8Unorm);
        } else {
            let pipeline = blit_pipeline(&self.device, srgb, !cfg!(target_os = "macos"));
            let bind = blit_group(&self.device, &pipeline, &self.view);
            let view = drawable.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.submitted();
        if let Some(mapped) = self.mapped.as_mut() {
            mapped.get_mut().submitted();
        }
        let mut pixels = self.read_texture(&drawable)?;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Ok(pixels)
    }
    /// The target copied on the GPU into a pixel buffer of `staging`'s pool,
    /// whole and finished, for a window to show as it is: no bytes read back.
    /// A buffer of its own while the pool's are all held (shown, or held by
    /// an app), so a frame is never skipped.
    #[cfg(target_os = "macos")]
    pub fn readback_surface(&self, staging: &mut SurfaceReadback) -> Result<PixelBuffer, GpuError> {
        if let Some(reason) = self.context.lost() {
            return Err(GpuError::device_lost(&reason));
        }
        self.flush_pending();
        let size = self.target.size();
        let buffer = staging.buffer(size.width, size.height)?;
        let (texture, wrapper) =
            staging.texture(&self.device, &buffer, (size.width, size.height))?;
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_texture(self.target.as_image_copy(), texture.as_image_copy(), size);
        self.queue.submit([encoder.finish()]);
        // Finished before it's handed over: the window reads it as it is.
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError(e.to_string()))?;
        drop((texture, wrapper));
        staging.flush();
        Ok(PixelBuffer::new(buffer))
    }
    /// The target's pixels, RGBA, a row at a time.
    pub fn readback(&self) -> Result<Vec<u8>, GpuError> {
        let mut pixels = Vec::new();
        self.read_target(&mut pixels, &mut Readback::default(), false)?;
        Ok(pixels)
    }
    /// [`GpuRenderer::readback`] into `pixels`, its allocation reused, through
    /// `staging` (made at the first read, kept for the next): what a video,
    /// reading every frame, calls. The BGRA target is copied to an RGBA
    /// texture on the GPU first (a pass `staging` keeps), so the read is a
    /// copy of rows, with nothing swapped a pixel at a time.
    pub fn readback_into(
        &self,
        pixels: &mut Vec<u8>,
        staging: &mut Readback,
    ) -> Result<(), GpuError> {
        self.read_target(pixels, staging, true)
    }
    fn read_target(
        &self,
        pixels: &mut Vec<u8>,
        staging: &mut Readback,
        swizzle_on_gpu: bool,
    ) -> Result<(), GpuError> {
        if let Some(reason) = self.context.lost() {
            return Err(GpuError::device_lost(&reason));
        }
        // Direct3D 11: its target, the swap chain's buffer drawn last
        // (buffer 0 until presented, then 1), or the renderer's own.
        #[cfg(d3d11)]
        if let Some(state) = &self.d3d11 {
            let present = self.layer_present.as_ref();
            let views = (present.and_then(|present| present.chain.as_ref()))
                .and_then(|chain| chain.d3d11_views());
            let drawing = present.is_some_and(|present| present.current.is_some());
            *pixels = match (&views, &state.target) {
                (Some(views), _) => state.device.read(&views[usize::from(!drawing)])?,
                (None, Some(target)) => state.device.read(target)?,
                // Nothing drawn: what wgpu's target holds, cleared.
                (None, None) => vec![0; (self.width * self.height * 4) as usize],
            };
            return Ok(());
        }
        self.flush_pending();
        self.read_texture_into(&self.target, pixels, staging, swizzle_on_gpu)
    }
    fn read_texture(&self, texture: &wgpu::Texture) -> Result<Vec<u8>, GpuError> {
        let mut pixels = Vec::new();
        self.read_texture_into(texture, &mut pixels, &mut Readback::default(), false)?;
        Ok(pixels)
    }
    fn read_texture_into(
        &self,
        texture: &wgpu::Texture,
        pixels: &mut Vec<u8>,
        staging: &mut Readback,
        swizzle_on_gpu: bool,
    ) -> Result<(), GpuError> {
        let stride = (self.width * 4).div_ceil(256) * 256;
        let size = u64::from(stride) * u64::from(self.height);
        if staging
            .buffer
            .as_ref()
            .is_none_or(|buffer| buffer.size() != size)
        {
            staging.buffer = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
        }
        let buffer = staging.buffer.as_ref().expect("made above");
        let mut encoder = self.device.create_command_encoder(&Default::default());
        // Retained textures are BGRA (see `FORMAT`); callers read RGBA. A
        // reader that keeps its `Readback` has the GPU copy the target to an
        // RGBA texture (the present blit: a `textureLoad` and a store, exact
        // for 8-bit channels) and reads that.
        let swizzled = swizzle_on_gpu && texture.format() == wgpu::TextureFormat::Bgra8Unorm;
        let source = if swizzled {
            let rgba = |device: &wgpu::Device| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("readback rgba"),
                    size: texture.size(),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            // Made again only for another target (resized, or replaced): a
            // video reads the same one frame after frame.
            if staging
                .swizzle
                .as_ref()
                .is_none_or(|swizzle| swizzle.source != *texture)
            {
                let pipeline = match staging.swizzle.take() {
                    Some(swizzle) => swizzle.pipeline,
                    None => blit_pipeline(&self.device, wgpu::TextureFormat::Rgba8Unorm, false),
                };
                let out = rgba(&self.device);
                staging.swizzle = Some(Swizzle {
                    bind: blit_group(
                        &self.device,
                        &pipeline,
                        &texture.create_view(&Default::default()),
                    ),
                    view: out.create_view(&Default::default()),
                    texture: out,
                    source: texture.clone(),
                    pipeline,
                });
            }
            let swizzle = staging.swizzle.as_ref().expect("made above");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &swizzle.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&swizzle.pipeline);
            pass.set_bind_group(0, &swizzle.bind, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            &swizzle.texture
        } else {
            texture
        };
        encoder.copy_texture_to_buffer(
            source.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuError(e.to_string()))?;
        rx.recv()
            .map_err(|e| GpuError(e.to_string()))?
            .map_err(|e| GpuError(e.to_string()))?;
        let mapped = buffer.get_mapped_range(..);
        let row = self.width as usize * 4;
        let length = row * self.height as usize;
        let bgra = !swizzled && texture.format() == wgpu::TextureFormat::Bgra8Unorm;
        let rows = mapped.chunks(stride as usize).map(|read| &read[..row]);
        if pixels.len() == length {
            // A kept buffer (a video's, frame after frame): each row copied
            // and its red and blue swapped in one pass, a pixel a word.
            for (out, read) in pixels.chunks_exact_mut(row).zip(rows) {
                if bgra {
                    for (out, read) in out
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(read.as_chunks::<4>().0)
                    {
                        *out = bgra_to_rgba(u32::from_le_bytes(*read)).to_le_bytes();
                    }
                } else {
                    out.copy_from_slice(read);
                }
            }
        } else {
            // A fresh one (a one-off read): grown a row at a time, nothing
            // zeroed first, then swapped where it lies.
            pixels.clear();
            pixels.reserve_exact(length);
            for read in rows {
                let at = pixels.len();
                pixels.extend_from_slice(read);
                if bgra {
                    for pixel in pixels[at..].as_chunks_mut::<4>().0 {
                        *pixel = bgra_to_rgba(u32::from_le_bytes(*pixel)).to_le_bytes();
                    }
                }
            }
        }
        drop(mapped);
        buffer.unmap();
        Ok(())
    }
}
/// A BGRA pixel, as a little-endian word, swapped to RGBA: a few vector
/// instructions a block of pixels.
#[inline]
pub(crate) fn bgra_to_rgba(word: u32) -> u32 {
    (word & 0xFF00_FF00) | ((word >> 16) & 0xFF) | ((word & 0xFF) << 16)
}

/// The images a scene's nodes show, by node and counted
/// (`GpuRenderer::prune_images`): walked for once (`counted`), then kept as
/// they are drawn (`GpuRenderer::drew_image`), as nodes go
/// (`Scene::resource_changes`), and as the counted ones are looked at again
/// when content changes (`recheck`): a node appended, or given an image,
/// is counted once drawn, and only then has a texture to keep. With
/// the last flush whose changes it holds (`through`), and whether an image
/// went uncounted since textures were last looked over (`dropped`).
#[derive(Default)]
struct SceneImages {
    counted: bool,
    dropped: bool,
    through: u64,
    by_node: FxHashMap<NodeId, u64>,
    users: FxHashMap<u64, u32>,
}
impl SceneImages {
    /// Count the images `scene`'s nodes show, all over again.
    fn walk(&mut self, scene: &Scene) {
        self.by_node.clear();
        self.users.clear();
        for item in scene.paint_items() {
            if let Some(image) = shown_image(item.kind) {
                self.show(item.id, Some(image));
            }
        }
        self.counted = true;
    }
    /// The nodes counted showing an image show it still, or what they show
    /// now is counted instead: one lookup for each, not a walk of the scene.
    fn recheck(&mut self, scene: &Scene) {
        let stale: Vec<(NodeId, Option<u64>)> = (self.by_node.iter())
            .filter_map(|(node, image)| {
                let now = (scene.contains(*node))
                    .then(|| scene.kind(*node))
                    .and_then(shown_image);
                (now != Some(*image)).then_some((*node, now))
            })
            .collect();
        for (node, now) in stale {
            self.show(node, now);
        }
    }
    /// `node` shows `image` now, or none.
    fn show(&mut self, node: NodeId, image: Option<u64>) {
        let old = match image {
            Some(image) => self.by_node.insert(node, image),
            None => self.by_node.remove(&node),
        };
        if old == image {
            return;
        }
        if let Some(old) = old
            && let Some(users) = self.users.get_mut(&old)
        {
            *users -= 1;
            if *users == 0 {
                self.users.remove(&old);
                self.dropped = true;
            }
        }
        if let Some(image) = image {
            *self.users.entry(image).or_default() += 1;
        }
    }
}
/// The image a node of `kind` shows: an image (a native surface's frame
/// too) or a box's background image.
fn shown_image(kind: &NodeKind) -> Option<u64> {
    match kind {
        NodeKind::Image(image) => Some(image.id()),
        #[cfg(target_os = "macos")]
        NodeKind::NativeSurface(frame) => Some(frame.id()),
        NodeKind::Quad(quad) | NodeKind::Panel { quad, .. } => {
            match quad.decoration.as_deref()?.background.as_ref()? {
                zgx::decoration::Background::Image { image, .. } => Some(image.id()),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The images `scene`'s nodes show: images (native surfaces' frames too)
/// and boxes' background images, each id once. Debug builds alone check the
/// counts against it (`prune_images`).
#[cfg(debug_assertions)]
fn scene_image_ids(scene: &Scene) -> FxHashSet<u64> {
    scene
        .paint_items()
        .filter_map(|item| shown_image(item.kind))
        .collect()
}

/// What [`GpuRenderer::readback_into`] reads through, kept by whoever reads
/// every frame (a video's stage) and made at its first read: a staging
/// buffer, and the RGBA texture and pass the target is copied to. An app
/// that never reads back holds none. It belongs to one renderer: its buffer
/// and texture are that renderer's device's, and another's can't use them.
#[derive(Default)]
pub struct Readback {
    buffer: Option<wgpu::Buffer>,
    swizzle: Option<Swizzle>,
}
/// The pass a kept read copies the target to RGBA with, and what it binds:
/// made for one target (`source`), and again for another.
struct Swizzle {
    pipeline: wgpu::RenderPipeline,
    source: wgpu::Texture,
    bind: wgpu::BindGroup,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// A frame a window showed (`GpuRenderer::shown_frame`), its pixels
/// readable while it is held.
#[cfg(target_os = "macos")]
pub struct ShownFrame {
    surface: objc2_core_foundation::CFRetained<objc2_io_surface::IOSurfaceRef>,
    /// The frame's work, if it may still run: a drawable's frame is not
    /// waited for as it presents (`render_direct`), so reading waits.
    pending: Option<metal_frame::CommandBuffer>,
    /// What layers show over it: a read shows what the window does.
    pub(crate) over: Vec<loops::OverFrame>,
}
#[cfg(target_os = "macos")]
impl ShownFrame {
    /// The surface it reads, to tell which layer shows it (a layer's
    /// `contents`); not to be dereferenced.
    #[doc(hidden)]
    pub fn surface_ptr(&self) -> *const std::ffi::c_void {
        (&*self.surface as *const objc2_io_surface::IOSurfaceRef).cast()
    }
    /// The region `x, y, width, height` (pixels, from its top left, clipped
    /// to it) as premultiplied BGRA rows, with its size; `None` for a region
    /// outside it.
    pub fn read(&self, x: u32, y: u32, width: u32, height: u32) -> Option<(u32, u32, Vec<u8>)> {
        if let Some(pending) = &self.pending {
            use objc2_metal::MTLCommandBuffer;
            pending.waitUntilCompleted();
        }
        let surface = objc2_core_foundation::CFRetained::as_ptr(&self.surface)
            .as_ptr()
            .cast::<std::ffi::c_void>()
            .cast_const();
        // SAFETY: a live surface (held), BGRA. It was shown, and its frame's
        // work is complete (waited for above, or before it was shown).
        let read = unsafe {
            iosurface::with_pixels(surface, |bytes, full_w, full_h, stride| {
                let (x, y) = (x as usize, y as usize);
                let w = (width as usize).min(full_w.saturating_sub(x));
                let h = (height as usize).min(full_h.saturating_sub(y));
                (w > 0 && h > 0).then(|| {
                    let mut pixels = Vec::with_capacity(w * h * 4);
                    for row in y..y + h {
                        pixels.extend_from_slice(&bytes[row * stride + x * 4..][..w * 4]);
                    }
                    (w as u32, h as u32, pixels)
                })
            })
            .flatten()
        };
        let (w, h, mut pixels) = read?;
        for over in &self.over {
            over.draw(&mut pixels, (x as i32, y as i32), (w as i32, h as i32));
        }
        Some((w, h, pixels))
    }
}
/// `rect` less `cut`, as up to four rectangles pushed to `out`.
/// The widest a filter of `bounds` and `effects` blurs anywhere in `area`:
/// its radius, less under an edge fade (as blur.wgsl's `sigma`).
fn peak_blur(bounds: Rect, effects: &zgx::scene::Effects, area: Rect) -> f32 {
    if effects.edge_fade <= 0. {
        return effects.blur_radius;
    }
    // How far in from either edge of `lo..hi` the span `from..to` reaches.
    let deepest = |lo: f32, hi: f32, from: f32, to: f32| {
        let at = ((lo + hi) / 2.).clamp(from, to);
        (at - lo).min(hi - at)
    };
    let depth = deepest(
        bounds.x,
        bounds.x + bounds.width,
        area.x,
        area.x + area.width,
    )
    .min(deepest(
        bounds.y,
        bounds.y + bounds.height,
        area.y,
        area.y + area.height,
    ));
    let t = (depth / effects.edge_fade).clamp(0., 1.);
    effects.blur_radius * t * t
}
/// A batch's filters (draws `members`), each with the parts of its output
/// the damaged `regions` reach.
fn batch_filters(draws: &[Draw], regions: &[(Rect, Scissor)], members: &[usize]) -> Vec<Filter> {
    members
        .iter()
        .map(|&i| {
            let draw = &draws[i];
            let &(bounds, effects, corners, rounded) =
                draw.blur.as_deref().expect("batched filter");
            Filter {
                bounds,
                clip: draw.clip,
                outputs: regions
                    .iter()
                    .filter_map(|(region, _)| region.intersection(draw.clip)?.intersection(bounds))
                    .collect(),
                effects,
                corners,
                rounded,
            }
        })
        .collect()
}
/// The bounding box of the part of `area` lying `near` to `far` in from the
/// nearest edge of `edges` (each x0, y0, x1, y1), if any.
fn band(area: [f32; 4], edges: [f32; 4], near: f32, far: f32) -> Option<[f32; 4]> {
    let inset = |by: f32| [edges[0] + by, edges[1] + by, edges[2] - by, edges[3] - by];
    let meet = |a: [f32; 4], b: [f32; 4]| {
        let r = [
            a[0].max(b[0]),
            a[1].max(b[1]),
            a[2].min(b[2]),
            a[3].min(b[3]),
        ];
        (r[0] < r[2] && r[1] < r[3]).then_some(r)
    };
    let within = meet(area, inset(near))?;
    let Some(hole) = far.is_finite().then(|| meet(within, inset(far))).flatten() else {
        return Some(within);
    };
    // `within` less `hole`: the strips above, below and beside it.
    [
        [within[0], within[1], within[2], hole[1]],
        [within[0], hole[3], within[2], within[3]],
        [within[0], hole[1], hole[0], hole[3]],
        [hole[2], hole[1], within[2], hole[3]],
    ]
    .into_iter()
    .filter(|s| s[0] < s[2] && s[1] < s[3])
    .reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}
// Used by surfaces of the renderer's own only.
#[cfg_attr(not(own_surfaces), allow(dead_code))]
fn scissor_minus(rect: Scissor, cut: Scissor, out: &mut Vec<Scissor>) {
    let (x, y, right, bottom) = (rect.0, rect.1, rect.0 + rect.2, rect.1 + rect.3);
    let (cx, cy, cr, cb) = (cut.0, cut.1, cut.0 + cut.2, cut.1 + cut.3);
    if cx >= right || cr <= x || cy >= bottom || cb <= y {
        out.push(rect);
        return;
    }
    // Full-width bands above and below the cut, then its sides between.
    if cy > y {
        out.push((x, y, rect.2, cy - y));
    }
    if cb < bottom {
        out.push((x, cb, rect.2, bottom - cb));
    }
    let (top, low) = (cy.max(y), cb.min(bottom));
    if cx > x {
        out.push((x, top, cx - x, low - top));
    }
    if cr < right {
        out.push((cr, top, right - cr, low - top));
    }
}
/// Whether `outer` contains `inner`.
#[cfg_attr(not(own_surfaces), allow(dead_code))]
fn scissor_holds(outer: Scissor, inner: Scissor) -> bool {
    outer.0 <= inner.0
        && outer.1 <= inner.1
        && outer.0 + outer.2 >= inner.0 + inner.2
        && outer.1 + outer.3 >= inner.1 + inner.3
}
/// A texture draws sample and the CPU writes (glyph cells, images): with
/// `cpu_writes` on macOS, in shared memory written in place
/// (`upload::shared_texture`).
/// Keep `vec`'s room for the frames to come (its length is what this one
/// used), unless a spike left it more than twice what frames have used
/// lately (`peak`, which follows the length up at once and down a 64th a
/// frame) and it holds more than 64 KB: a frame of a long page scrolled past
/// keeps none of it.
fn keep_room<T>(vec: &mut Vec<T>, peak: &mut usize) {
    *peak = vec.len().max(*peak - *peak / 64);
    if vec.capacity() > 2 * *peak && vec.capacity() * std::mem::size_of::<T>() > 64 << 10 {
        vec.shrink_to(*peak);
    }
}
/// `pack_cells`' atlas: its size, each cell's origin, and the packer's
/// cursor after them (`GpuRenderer::cursor`).
struct Packed {
    size: u32,
    origins: Vec<(u32, u32)>,
    cursor: (u32, u32, u32),
}
/// Cells `(width, height)` packed as `atlas_cell` packs them (in shelves,
/// each a texel of gutter round it), tallest first, into the smallest square
/// atlas of a power of two from `INITIAL_ATLAS` and below `below`: its size,
/// each cell's origin in the order given, and the packer's cursor after
/// them. `None` when only `below` or more holds them.
fn pack_cells(cells: &[(u32, u32)], below: u32) -> Option<Packed> {
    let mut order: Vec<usize> = (0..cells.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse((cells[i].1, cells[i].0)));
    let mut size = INITIAL_ATLAS;
    'sizes: while size < below {
        let mut origins = vec![(0, 0); cells.len()];
        let mut cursor = (0, 0, 0);
        for &i in &order {
            let (cell_width, cell_height) = (cells[i].0 + 2, cells[i].1 + 2);
            if cursor.0 + cell_width > size {
                cursor = (0, cursor.1 + cursor.2, 0);
            }
            if cursor.1 + cell_height > size || cell_width > size {
                size *= 2;
                continue 'sizes;
            }
            origins[i] = (cursor.0 + 1, cursor.1 + 1);
            cursor.0 += cell_width;
            cursor.2 = cursor.2.max(cell_height);
        }
        return Some(Packed {
            size,
            origins,
            cursor,
        });
    }
    None
}
fn sampled_texture(
    device: &wgpu::Device,
    #[allow(unused)] cpu_writes: bool,
    width: u32,
    height: u32,
    label: &str,
) -> wgpu::Texture {
    #[cfg(target_os = "macos")]
    if cpu_writes {
        return upload::shared_texture(device, width, height, label);
    }
    texture(device, width, height, label)
}
/// Clear a new target through wgpu, so wgpu knows it is written: frames drawn
/// with Metal directly (`render_direct`) write it behind wgpu's back, and wgpu
/// zeroes a texture it has never written on that texture's first use.
#[cfg(target_os = "macos")]
fn written(device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView) {
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("new target"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    queue.submit([encoder.finish()]);
}
/// A backdrop blur atlas: half floats, so averaging down and blurring in
/// steps adds no 8-bit banding to smooth frost.
fn atlas_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("backdrop blur"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: BLUR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}
fn texture(device: &wgpu::Device, width: u32, height: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}
/// A glyph's quad from its atlas entry, from its text's origin. Inlined
/// into the loops building a text's quads: called, its return copied each
/// glyph's quad again.
#[inline(always)]
fn glyph_quad(
    glyph: &Glyph,
    a: AtlasEntry,
    scale: f32,
    atlas_size: u32,
    color: [f32; 4],
) -> TextQuad {
    TextQuad {
        rect: [
            (glyph.x + a.left as f32) / scale,
            (glyph.y - a.top as f32) / scale,
            a.width as f32 / scale,
            a.height as f32 / scale,
        ],
        uv: [
            a.x as f32 / atlas_size as f32,
            a.y as f32 / atlas_size as f32,
            a.width as f32 / atlas_size as f32,
            a.height as f32 / atlas_size as f32,
        ],
        color,
        paint: [
            0.,
            glyph_texture(a.color, scale),
            if a.color { COLOR_GLYPH } else { 0. },
            0.,
        ],
    }
}
/// `image` in its box `fade` (x, y, width, height) as nine pieces
/// (`zgx::image::ImageSlice`): corners at their widths, edges stretched
/// along their side, the middle both ways, each a quad of its part of the
/// image's texture. Widths past the box shrink together, as CSS's do.
#[cold]
#[inline(never)]
fn push_image_slices(
    quads: &mut Vec<Quad>,
    image: &zgx::image::ImageData,
    slice: zgx::image::ImageSlice,
    fade: [f32; 4],
    (opacity, edge_fade): (f32, f32),
) {
    let [x, y, width, height] = fade;
    let (iw, ih) = (image.width().max(1) as f32, image.height().max(1) as f32);
    let [top, right, bottom, left] = slice.widths;
    let fit = |a: f32, b: f32, room: f32| match a + b > room && a + b > 0. {
        true => room / (a + b),
        false => 1.,
    };
    let (sx, sy) = (fit(left, right, width), fit(top, bottom, height));
    let (left, right, top, bottom) = (left * sx, right * sx, top * sy, bottom * sy);
    let xs = [x, x + left, x + width - right, x + width];
    let ys = [y, y + top, y + height - bottom, y + height];
    let [st, sr, sb, sl] = slice.slice;
    let us = [0., (sl / iw).min(1.), (1. - sr / iw).max(0.), 1.];
    let vs = [0., (st / ih).min(1.), (1. - sb / ih).max(0.), 1.];
    for row in 0..3 {
        for column in 0..3 {
            let (dw, dh) = (xs[column + 1] - xs[column], ys[row + 1] - ys[row]);
            if dw <= 0. || dh <= 0. || us[column + 1] <= us[column] || vs[row + 1] <= vs[row] {
                continue;
            }
            // Half a texel in at the cuts inside the image, so a piece's
            // smoothing never takes in its neighbour's pixels.
            let inner = |at: usize, half: f32| if at == 0 || at == 3 { 0. } else { half };
            let (hu, hv) = (0.5 / iw, 0.5 / ih);
            let u0 = us[column] + inner(column, hu);
            let u1 = (us[column + 1] - inner(column + 1, hu)).max(u0);
            let v0 = vs[row] + inner(row, hv);
            let v1 = (vs[row + 1] - inner(row + 1, hv)).max(v0);
            quads.push(Quad {
                rect: [xs[column], ys[row], dw, dh],
                uv: [u0, v0, u1 - u0, v1 - v0],
                color: [1., 1., 1., opacity],
                fade,
                options: [edge_fade, 2., 0., 0.],
                shape: [0.; 4],
                border: [0.; 4],
                mask: NO_MASK,
            });
        }
    }
}
/// The most pieces one line decoration is drawn in: past that, the rest of a
/// very long run goes without.
const LINE_PIECES: usize = 4096;
/// Where wave tiles sit among `image_cells`, whose ids count up from 1: this
/// bit and the wave's thickness in sixteenths of a pixel.
const WAVE_CELL: u64 = 1 << 63;
fn rich_decoration_quad(d: &text::RichDecoration) -> TextQuad {
    let rect = [d.bounds.x, d.bounds.y, d.bounds.width, d.bounds.height];
    TextQuad {
        rect,
        color: rgba(d.color, 1.),
        uv: [0.; 4],
        // A rounded background (an inline code chip): a rounded rect.
        paint: if d.radius > 0. {
            [d.radius, 0., 0., 1.]
        } else {
            [0.; 4]
        },
    }
}
/// A module made from `source` for one pipeline, as each app shader's is
/// (`app_built`, an effect's pass). With Metal, its naga IR (about 250 KB:
/// the quad shader's whole module) goes once that pipeline is made, where
/// wgpu would keep it with the pipeline (zgx's wgpu-hal patch,
/// `zgx_single_use_next_module`): the module keeps `source`, and a second
/// pipeline from it parses that again.
pub(crate) fn single_use_module(
    device: &wgpu::Device,
    label: &str,
    source: &str,
) -> wgpu::ShaderModule {
    #[cfg(target_os = "macos")]
    wgpu::hal::metal::zgx_single_use_next_module(Some(source.to_owned()));
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    // (Taken back if wgpu made no module of it.)
    #[cfg(target_os = "macos")]
    wgpu::hal::metal::zgx_single_use_next_module(None);
    module
}
/// A shader fill's module: the quad shader (draw.wgsl), the `FillInput` its
/// `fill` takes, the user's source and the fragment entry `zgx_fill`, which
/// shades a quad carrying uniforms (`mask.y`: their record in the region
/// table, plus one) with `fill`'s colour and the rest as `shade` does.
pub(crate) fn fill_source(user: &str) -> String {
    format!(
        "{}\nstruct FillInput {{ p:vec2<f32>, size:vec2<f32>, time:f32, scale:f32, params:array<vec4<f32>,4> }}\n{user}\n{}",
        include_str!("draw.wgsl"),
        "@fragment fn zgx_fill(input:Out)->@location(0) vec4<f32> {
 if input.mask.y<.5 {return shade(input);}
 let u=regions[u32(input.mask.y)-1u]; var v=input;
 let point=select(v.position.xy/viewport.z,v.point,v.options.z>0.5);
 let c=fill(FillInput(point-v.fade.xy,v.fade.zw,u.round0.x,viewport.z,array<vec4<f32>,4>(u.radii0,u.round1,u.radii1,u.fade_box)));
 v.color=vec4(c.rgb,c.a*v.color.a);
 return shade(v);
}"
    )
}
/// Where a layer shader's `zgx_sample` reads: the texture coordinates at
/// the top-left of `node` (its box) and per point from there, for the
/// composite quad `(rect, uv)` drawing the layer's texture (its uv the
/// texture coordinates at `rect`'s top-left and across it). Worked out here
/// rather than from screen derivatives in the shader: those are differences
/// of interpolated coordinates, a little off, and that error times a read
/// a thousand points away lands in a texel beside the one meant.
fn layer_frame((rect, uv): (Rect, [f32; 4]), node: Rect) -> [f32; 4] {
    let per = [uv[2] / rect.width, uv[3] / rect.height];
    [
        uv[0] + (node.x - rect.x) * per[0],
        uv[1] + (node.y - rect.y) * per[1],
        per[0],
        per[1],
    ]
}
/// A layer shader's module (`zgx::scene::LayerShader`): the quad shader,
/// the `LayerInput` its `layer` takes, `zgx_sample`, the user's source and
/// the fragment entry `zgx_filter`, which passes the layer's texel under a
/// layer's composite quad (`mask.y`: its uniforms' record, plus one) through
/// `layer` and shades the result as `shade` does. The quad's `fade` is the
/// node's box and `point` its untransformed position, so `p` is the node's.
pub(crate) fn filter_source(user: &str) -> String {
    format!(
        "{}\n{}\nstruct LayerInput {{ color:vec4<f32>, p:vec2<f32>, size:vec2<f32>, time:f32, scale:f32, params:array<vec4<f32>,4>, radii:vec4<f32> }}\n{}\n{user}\n{}{}",
        include_str!("draw.wgsl"),
        include_str!("blend.wgsl"),
        "// The layer texture's coordinates at the node's top-left, and per point
// (`layer_frame`).
var<private> zgx_frame:vec4<f32>;
// Where the layer's pixels lie, from the node's top-left (its transparent
// border at the edge): past them a sample is clear.
var<private> zgx_painted:vec4<f32>;
// What the backdrop's copy holds, from the node's top-left; and the target's
// pixel at the node's top-left and its pixels per logical point.
var<private> zgx_readable:vec4<f32>;
var<private> zgx_origin:vec2<f32>;
var<private> zgx_pixels:mat2x2<f32>;
// The layer's straight-alpha pixel at logical `p` from the node's top-left.
fn zgx_sample(p:vec2<f32>)->vec4<f32> {
 let uv=zgx_frame.xy+clamp(p,zgx_painted.xy,zgx_painted.zw)*zgx_frame.zw;
 let t=textureSampleLevel(atlas,tex_sampler,uv,0.);
 return select(vec4(0.),vec4(t.rgb/t.a,t.a),t.a>0.);
}
// What lies behind the node at logical `p` from its top-left, straight alpha.
fn zgx_backdrop(p:vec2<f32>)->vec4<f32> {
 let pixel=zgx_origin+zgx_pixels*clamp(p,zgx_readable.xy,zgx_readable.zw);
 let t=textureSampleLevel(zgx_behind,tex_sampler,pixel/vec2<f32>(textureDimensions(zgx_behind)),0.);
 return select(vec4(0.),vec4(t.rgb/t.a,t.a),t.a>0.);
}",
        "fn zgx_filtered(input:Out)->vec4<f32> {
 var v=input;
 let point=select(v.position.xy/viewport.z,v.point,v.options.z>0.5);
 // The target's pixels per logical point of the node, for
 // `zgx_backdrop`: from the screen derivatives of the point, taken before
 // any branch.
 let dp=mat2x2(dpdx(point),dpdy(point));
 let det=dp[0].x*dp[1].y-dp[1].x*dp[0].y;
 let inverse=mat2x2(vec2(dp[1].y,-dp[0].y),vec2(-dp[1].x,dp[0].x))*(1./select(det,1e-6,abs(det)<1e-12));
 if v.mask.y<.5 {return shade(v);}
 let u=regions[u32(v.mask.y)-1u];
 let p=point-v.fade.xy;
 // The shader runs over the node's box and bleed; the layer's margin past
 // them (its resampling edge, overflow) composites as painted.
 let bleed=u.tint.x;
 if any(p<vec2(-bleed))||any(p>=v.fade.zw+vec2(bleed)) {return shade(v);}
 zgx_frame=u.color2;
 zgx_painted=u.color0;
 zgx_readable=u.color1;
 zgx_pixels=inverse;
 zgx_origin=v.position.xy-inverse*p;
 let c=clamp(layer(LayerInput(zgx_sample(p),p,v.fade.zw,u.round0.x,viewport.z,array<vec4<f32>,4>(u.radii0,u.round1,u.radii1,u.fade_box),u.fade)),vec4(0.),vec4(1.));
 return shade_texel(v,vec4(c.rgb*c.a,c.a),true);
}
@fragment fn zgx_filter(input:Out)->@location(0) vec4<f32> { return zgx_filtered(input); }
@fragment fn zgx_filter_blended(input:Out)->@location(0) vec4<f32> {
 let beneath=zgx_beneath(input.position);
 let source=zgx_filtered(input);
 let mode=u32(input.mask.z);
 // Plainly over what's behind it (it reads it): source-over, exact.
 if mode==0u {return source+(1.-source.a)*beneath;}
 return zgx_blend(source,beneath,mode);
}",
        // Where logical `p` from the node's top-left is in its target, in
        // points: the window's own, for a node not inside another's layer.
        // After the user's code (lines count as written), and only where it's
        // called: every other shader parses as before.
        match user.contains("zgx_window") {
            true => "\nfn zgx_window(p:vec2<f32>)->vec2<f32> { return (zgx_origin+zgx_pixels*p)/viewport.z; }",
            false => "",
        }
    )
}
/// For each backdrop-reading layer drawn first in a pass, those after it that
/// can share its copy of the target: no draw before them since then (the
/// others sharing it included) paints anywhere they read or paint, so they
/// read and draw as they would in order (`settle_sharing`). Most frames have
/// none.
fn share_backdrops(draws: &[Draw], reads: &[(usize, Rect)]) -> Vec<(usize, Vec<usize>)> {
    /// How far apart readers share, in draws: a screenful, not a long scan.
    const WITHIN: usize = 512;
    let mut sharing: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut taken = rustc_hash::FxHashSet::default();
    for (index, &(first, _)) in reads.iter().enumerate() {
        if taken.contains(&first) {
            continue;
        }
        let mut shared = Vec::new();
        for &(next, read) in &reads[index + 1..] {
            if next > first + WITHIN {
                break;
            }
            // (Those blurring too run their blur first: a copy of their own.)
            if draws[first].blur.is_some() || draws[next].blur.is_some() {
                continue;
            }
            let area = read.union(draws[next].bounds);
            let clear = draws[first..next].iter().all(|draw| {
                draw.bounds
                    .intersection(draw.clip)
                    .is_none_or(|painted| !painted.intersects(area))
            });
            if clear {
                shared.push(next);
                taken.insert(next);
            }
        }
        if !shared.is_empty() {
            sharing.push((first, shared));
        }
    }
    if !sharing.is_empty() {
        settle_sharing(&mut sharing, draws, reads);
    }
    sharing
}
/// `share_backdrops`' groups, each reader in one at most, the first's to take
/// it (drawn with a second copy, it would read itself), and none drawn at an
/// earlier copy ahead of a reader between that copies later and reads where
/// it paints. One let go draws with a copy of its own, as in order. Out of
/// line: frames sharing no copy never come here.
#[inline(never)]
fn settle_sharing(sharing: &mut Vec<(usize, Vec<usize>)>, draws: &[Draw], reads: &[(usize, Rect)]) {
    // Readers drawn at a copy so far: at an earlier one, or this one's.
    let mut drawn = rustc_hash::FxHashSet::default();
    for (first, shared) in sharing.iter_mut() {
        let first = *first;
        let mut kept: Vec<usize> = Vec::with_capacity(shared.len());
        for &next in shared.iter() {
            if drawn.contains(&next) {
                continue;
            }
            let painted = draws[next].bounds.intersection(draws[next].clip);
            let unread = reads
                .iter()
                .filter(|(reader, _)| (first + 1..next).contains(reader))
                .all(|(reader, theirs)| {
                    drawn.contains(reader)
                        || kept.contains(reader)
                        || painted.is_none_or(|painted| !painted.intersects(*theirs))
                });
            if unread {
                kept.push(next);
            }
        }
        drawn.extend(kept.iter().copied());
        *shared = kept;
    }
    sharing.retain(|(_, shared)| !shared.is_empty());
}
/// A rounded clip in a blur's record, in px: its box and radii, or none's.
fn round_box(clip: Option<zgx::scene::RoundRect>, scale: f32) -> ([f32; 4], [f32; 4]) {
    match clip {
        Some(clip) => (
            [
                clip.rect.x * scale,
                clip.rect.y * scale,
                clip.rect.width * scale,
                clip.rect.height * scale,
            ],
            clip.radii.map(|radius| radius * scale),
        ),
        None => ([0., 0., -1., -1.], [0.; 4]),
    }
}
/// A layer's quad grown to hold `reach` too (its layer shader's bleed): its
/// uv (origin and size) goes on past the layer's at the same rate.
fn bled_quad(bounds: Rect, uv: [f32; 4], reach: Rect) -> (Rect, [f32; 4]) {
    let grown = bounds.union(reach);
    let (du, dv) = (
        uv[2] / bounds.width.max(1e-6),
        uv[3] / bounds.height.max(1e-6),
    );
    (
        grown,
        [
            uv[0] + (grown.x - bounds.x) * du,
            uv[1] + (grown.y - bounds.y) * dv,
            grown.width * du,
            grown.height * dv,
        ],
    )
}
/// The key of the blended layers' plain pipeline in `blend_pipelines`
/// (layer shaders' ids count up from 1).
const BLEND_PLAIN: u64 = 0;
/// The blended layers' module: the quad shader, blend.wgsl, and the
/// fragment entry `zgx_blended`, which shades a layer's composite quad as
/// `shade` does and blends it with the backdrop under it by `mask.z`.
pub(crate) fn blend_source() -> String {
    format!(
        "{}\n{}\n{}",
        include_str!("draw.wgsl"),
        include_str!("blend.wgsl"),
        "@fragment fn zgx_blended(input:Out)->@location(0) vec4<f32> {
 return zgx_blend(shade(input),zgx_beneath(input.position),u32(input.mask.z));
}"
    )
}
/// Checks a layer shader's source (the user's part) as the renderer will
/// compile it, as `check_fill_shader` does a fill's.
pub fn check_layer_shader(user: &str) -> Result<(), String> {
    check_module(user, filter_source)
}
/// Checks a shader fill's source (the user's part, as `FillShader` holds
/// it) as the renderer will compile it: `Err` reads like a compiler
/// message, with lines counted in `user`, for an app or script to show.
pub fn check_fill_shader(user: &str) -> Result<(), String> {
    check_module(user, fill_source)
}
/// Checks `user` in the module `module` builds around it.
fn check_module(user: &str, module: fn(&str) -> String) -> Result<(), String> {
    let source = module(user);
    // Lines before the user's code in the module `module` builds.
    let template = module("\u{1}");
    let before = template[..template.find('\u{1}').unwrap_or(0)]
        .matches('\n')
        .count();
    let place = |location: Option<wgpu::naga::SourceLocation>| match location {
        Some(at) if at.line_number as usize > before => {
            format!("line {}: ", at.line_number as usize - before)
        }
        Some(_) => "in zgx's part of the module (a reserved name?): ".to_owned(),
        None => String::new(),
    };
    let module = wgpu::naga::front::wgsl::parse_str(&source)
        .map_err(|e| format!("{}{}", place(e.location(&source)), e.message()))?;
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map(|_| ())
    .map_err(|e| format!("{}{}", place(e.location(&source)), e.as_inner()))
}
/// Whether `source` (a shader fill's whole module) is valid WGSL.
pub(crate) fn fill_validates(source: &str) -> bool {
    let Ok(module) = wgpu::naga::front::wgsl::parse_str(source) else {
        return false;
    };
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .is_ok()
}
/// A background `image` placed in `bounds` as `fit` has it (centred), the
/// part inside them, cut to `style`'s rounded corners (the quad shader's
/// per-corner shape over the box, whatever part of it the image covers).
/// A gradient ramp: its stops (offsets and colours, as bits) and whether
/// it runs down (a column of texels) or across (a row).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RampKey {
    vertical: bool,
    stops: Vec<(u32, [u8; 4])>,
}
/// Texels in a ramp: smooth to the eye at any size, as the texture is
/// sampled linearly between them.
const RAMP: u32 = 256;
/// `style`'s linear gradient, when it runs straight down or across `bounds`
/// (any other angle rasterizes): its ramp and the texture coordinates that
/// stretch the ramp's ends to the gradient's (padded past them by the
/// sampler's clamping).
#[inline(always)]
fn axis_gradient(style: &zgx::scene::QuadStyle, bounds: Rect) -> Option<(RampKey, [f32; 4])> {
    use zgx::{canvas::Point, decoration::Background};
    let (w, h) = (bounds.width, bounds.height);
    if !(w > 0. && h > 0.) {
        return None;
    }
    let (start, end, stops) = match style.decoration.as_deref()?.background.as_ref()? {
        Background::Linear { angle, stops } => {
            // As `decoration::canvas` places them.
            let (sin, cos) = angle.sin_cos();
            let half = (w * cos.abs() + h * sin.abs()) * 0.5;
            (
                Point::new(w * 0.5 - cos * half, h * 0.5 - sin * half),
                Point::new(w * 0.5 + cos * half, h * 0.5 + sin * half),
                stops,
            )
        }
        Background::Brush(zgx::canvas::Brush::Linear { start, end, stops }) => {
            (*start, *end, stops)
        }
        _ => return None,
    };
    if stops.len() < 2
        || stops.len() > 64
        || stops
            .iter()
            .any(|s| !s.offset.is_finite() || !(0. ..=1.).contains(&s.offset))
        || stops.windows(2).any(|p| p[1].offset < p[0].offset)
    {
        return None;
    }
    let vertical = (start.x - end.x).abs() <= 1e-3;
    let horizontal = (start.y - end.y).abs() <= 1e-3;
    let (from, to, extent) = match (vertical, horizontal) {
        (true, false) => (start.y, end.y, h),
        (false, true) => (start.x, end.x, w),
        _ => return None,
    };
    let span = to - from;
    if !span.is_finite() || span.abs() < 1e-3 {
        return None;
    }
    // Offset 0 at the first texel's centre, 1 at the last's.
    let texel = |t: f32| (0.5 + t * (RAMP - 1) as f32) / RAMP as f32;
    let (a, b) = (texel(-from / span), texel((extent - from) / span));
    let uv = if vertical {
        [0., a, 1., b - a]
    } else {
        [a, 0., b - a, 1.]
    };
    let stops = stops
        .iter()
        .map(|s| {
            (
                s.offset.to_bits(),
                [s.color.0, s.color.1, s.color.2, s.color.3],
            )
        })
        .collect();
    Some((RampKey { vertical, stops }, uv))
}
/// A ramp's texels: the stops interpolated with premultiplied alpha (as
/// CSS gradients are, and rasters, see `canvas::premultiplied_stops`),
/// padded with the end colours.
fn ramp_image(key: &RampKey) -> Result<zgx::image::ImageData, GpuError> {
    let stops: Vec<(f32, [f32; 4])> = key
        .stops
        .iter()
        .map(|&(offset, [r, g, b, a])| {
            let alpha = a as f32 / 255.;
            (
                f32::from_bits(offset),
                [
                    r as f32 * alpha,
                    g as f32 * alpha,
                    b as f32 * alpha,
                    a as f32,
                ],
            )
        })
        .collect();
    let mut pixels = Vec::with_capacity(RAMP as usize * 4);
    for i in 0..RAMP {
        let t = i as f32 / (RAMP - 1) as f32;
        let next = stops.iter().position(|s| s.0 > t).unwrap_or(stops.len());
        let c = match next {
            0 => stops[0].1,
            n if n == stops.len() => stops[n - 1].1,
            n => {
                let ((t0, c0), (t1, c1)) = (stops[n - 1], stops[n]);
                let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1. };
                std::array::from_fn(|k| c0[k] + (c1[k] - c0[k]) * f)
            }
        };
        // Images hold straight colour (their upload premultiplies).
        let alpha = c[3] / 255.;
        let straight = |v: f32| if alpha > 0. { v / alpha } else { 0. };
        pixels.extend(
            [straight(c[0]), straight(c[1]), straight(c[2]), c[3]]
                .map(|v| v.round().clamp(0., 255.) as u8),
        );
    }
    let (width, height) = if key.vertical { (1, RAMP) } else { (RAMP, 1) };
    zgx::image::ImageData::new(width, height, pixels).map_err(|e| GpuError(e.into()))
}
fn background_quad(
    image: &zgx::image::ImageData,
    fit: zgx::style::ObjectFit,
    style: &zgx::scene::QuadStyle,
    bounds: Rect,
    opacity: f32,
    edge_fade: f32,
) -> Option<Quad> {
    use zgx::style::ObjectFit;
    let (source_w, source_h) = (image.width() as f32, image.height() as f32);
    if bounds.width <= 0. || bounds.height <= 0. {
        return None;
    }
    let (horizontal, vertical) = (bounds.width / source_w, bounds.height / source_h);
    let (width, height) = match fit {
        ObjectFit::Fill => (bounds.width, bounds.height),
        ObjectFit::None => (source_w, source_h),
        fit => {
            let scale = match fit {
                ObjectFit::Cover => horizontal.max(vertical),
                ObjectFit::ScaleDown => horizontal.min(vertical).min(1.),
                _ => horizontal.min(vertical),
            };
            (source_w * scale, source_h * scale)
        }
    };
    let placed = Rect::new(
        bounds.x + (bounds.width - width) / 2.,
        bounds.y + (bounds.height - height) / 2.,
        width,
        height,
    );
    let shown = placed.intersection(bounds)?;
    let fade = [bounds.x, bounds.y, bounds.width, bounds.height];
    Some(Quad {
        rect: [shown.x, shown.y, shown.width, shown.height],
        uv: [
            (shown.x - placed.x) / width,
            (shown.y - placed.y) / height,
            shown.width / width,
            shown.height / height,
        ],
        color: [1., 1., 1., opacity],
        fade,
        options: [edge_fade, 2., 0., 1.],
        shape: [0., 0., 0., 1.],
        border: decoration::shadow_corners(style, bounds.width, bounds.height, 0.),
        mask: NO_MASK,
    })
}
/// Pixelated images' sampler and bind groups (`GpuRenderer::nearest_bind`),
/// each with the texture it was made for.
struct NearestBinds {
    sampler: wgpu::Sampler,
    binds: FxHashMap<u64, (wgpu::BindGroup, wgpu::Texture)>,
}
/// Dashed and dotted outlines' rasters (`GpuRenderer::styled_outline`):
/// a cache and textures of their own, by node, apart from its decoration's.
#[derive(Default)]
struct OutlineRasters {
    cache: canvas::CanvasCache,
    images: FxHashMap<NodeId, u64>,
}
/// `outline` around a box of `style` at `bounds`: a ring of its width
/// between the box grown by its offset and by its offset and width, each
/// rounded corner grown alike (square ones stay square).
#[inline(always)]
fn outline_quad(
    outline: &zgx::scene::Outline,
    style: &zgx::scene::QuadStyle,
    bounds: Rect,
    opacity: f32,
    edge_fade: f32,
) -> Quad {
    let outer = outline.outer(bounds);
    let grown = outline.offset + outline.width;
    let corners =
        decoration::shadow_corners(style, bounds.width, bounds.height, 0.).map(|radius| {
            if radius > 0. {
                (radius + grown).max(0.)
            } else {
                0.
            }
        });
    let width = (outline.width.max(0.) * 16.).round().min(4095.);
    let fade = [outer.x, outer.y, outer.width, outer.height];
    Quad {
        rect: fade,
        uv: [0.; 4],
        color: rgba(outline.color, opacity),
        fade,
        options: [edge_fade, 0., 0., 1.],
        shape: [width + 4096. * width, -1., width + 4096. * width, 1.],
        border: corners,
        mask: NO_MASK,
    }
}
/// What a canvas draws as quads (`canvas_quads`): fills (pushed as panels),
/// rings and polyline strokes, in order.
enum CanvasQuad {
    Fill(Quad),
    Ring(Quad),
    Polyline(Quad),
}
/// How many segments a pixel of a polyline stroke may have in its reach
/// (`zgx_polyline`'s loop): denser strokes rasterize.
const POLYLINE_REACH_SEGMENTS: usize = 32;
/// A polyline stroke's quad (polyline.wgsl), its points added to `points`:
/// a subpath of straight lines whose x never decreases (or never
/// increases: drawn the other way), as a sparkline's or a chart's, each
/// pixel finding its segments by a search. `None` for anything else.
fn polyline_quad(
    subpath: &[zgx::canvas::Segment],
    style: &zgx::canvas::Stroke,
    color: Color,
    origin: (f32, f32),
    zoom: f32,
    points: &mut Vec<[f32; 2]>,
) -> Option<Quad> {
    use zgx::canvas::{LineCap, LineJoin, Segment};
    let mut line: Vec<[f32; 2]> = Vec::with_capacity(subpath.len());
    for (k, segment) in subpath.iter().enumerate() {
        let point = match (k, segment) {
            (0, Segment::Move(p)) | (1.., Segment::Line(p)) => p,
            _ => return None,
        };
        let point = [point.x * zoom + origin.0, point.y * zoom + origin.1];
        if !point.iter().all(|v| v.is_finite()) {
            return None;
        }
        // A repeated point adds nothing to the outline but a join's corner.
        if line.last() != Some(&point) {
            line.push(point);
        }
    }
    if line.len() < 2 {
        return None;
    }
    if line.windows(2).all(|w| w[1][0] <= w[0][0]) {
        line.reverse();
    }
    if !line.windows(2).all(|w| w[1][0] >= w[0][0]) {
        return None;
    }
    let half = style.width * zoom / 2.;
    let limit = match style.join {
        LineJoin::Miter => style.miter_limit.max(1.),
        _ => 1.,
    };
    // As far as its pixels lie from its points: a miter's tip, a square
    // cap's corner, and the antialiased edge (as the shader reckons it).
    let reach = half * limit.max(1.42) + 1.;
    // Every pixel searches its reach: the points within any span of twice
    // it, and one segment either side, are at most the loop's.
    if !fits_reach(&line, reach) {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for [x, y] in &line {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    let bounds = Rect::new(x0, y0, x1 - x0, y1 - y0).expand(reach);
    let first = points.len();
    points.extend_from_slice(&line);
    let cap = match style.cap {
        LineCap::Butt => 0.,
        LineCap::Square => 1.,
        LineCap::Round => 2.,
    };
    let join = match style.join {
        LineJoin::Miter => 0.,
        LineJoin::Bevel => 1.,
        LineJoin::Round => 2.,
    };
    let rect = [bounds.x, bounds.y, bounds.width, bounds.height];
    Some(Quad {
        rect,
        uv: [0.; 4],
        color: rgba(color, 1.),
        fade: rect,
        options: [0.; 4],
        shape: [0.; 4],
        border: [half, cap, join, limit],
        mask: [0., first as f32 + 1., line.len() as f32, 0.],
    })
}
/// The most gradient stops an area's shader takes (`zgx_gradient` walks
/// them): more rasterize.
const AREA_STOPS: usize = 16;
/// Whether every span of `2 * reach + 1` along `line` holds no more points
/// than a pixel may search (`POLYLINE_REACH_SEGMENTS`).
fn fits_reach(line: &[[f32; 2]], reach: f32) -> bool {
    let mut end = 0;
    line.iter().enumerate().all(|(k, point)| {
        end = end.max(k);
        while end < line.len() && line[end][0] < point[0] + 2. * reach + 1. {
            end += 1;
        }
        end - k < POLYLINE_REACH_SEGMENTS
    })
}
/// The y of `chain` (x never decreasing) at `x`, within it.
fn chain_y(chain: &[[f32; 2]], x: f32) -> f32 {
    let k = chain.partition_point(|p| p[0] < x);
    match k {
        0 => chain[0][1],
        k if k >= chain.len() => chain[chain.len() - 1][1],
        k => {
            let (a, b) = (chain[k - 1], chain[k]);
            let width = b[0] - a[0];
            if width > 0. {
                a[1] + (b[1] - a[1]) * (x - a[0]) / width
            } else {
                b[1]
            }
        }
    }
}
/// Its points along each chain, left to right (`area_chains`).
type AreaChains = (Vec<[f32; 2]>, Vec<[f32; 2]>);
/// A closed polygon's two chains from its left end to its right, as
/// `area_quad` takes them: one subpath of straight lines, its points on two
/// runs with x never decreasing, one above the other all along (they never
/// cross, so which fill rule doesn't matter). A chart's area, a band, a
/// triangle. `None` for anything else.
fn area_chains(
    segments: &[zgx::canvas::Segment],
    origin: (f32, f32),
    zoom: f32,
) -> Option<AreaChains> {
    use zgx::canvas::Segment;
    let mut ring: Vec<[f32; 2]> = Vec::with_capacity(segments.len());
    for (k, segment) in segments.iter().enumerate() {
        let point = match (k, segment) {
            (0, Segment::Move(p)) | (1.., Segment::Line(p)) => p,
            (1.., Segment::Close) if k + 1 == segments.len() => continue,
            _ => return None,
        };
        let point = [point.x * zoom + origin.0, point.y * zoom + origin.1];
        if !point.iter().all(|v| v.is_finite()) {
            return None;
        }
        if ring.last() != Some(&point) {
            ring.push(point);
        }
    }
    if ring.len() > 1 && ring.first() == ring.last() {
        ring.pop();
    }
    let n = ring.len();
    if n < 3 {
        return None;
    }
    let x_at = |k: usize| ring[k % n][0];
    let (left, right) = ring
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, r), p| (l.min(p[0]), r.max(p[0])));
    if left >= right {
        return None;
    }
    // Where the ring leaves its left edge, going each way round: at most
    // two points stand on it.
    let forward = (0..n).find(|&k| x_at(k) == left && x_at(k + 1) > left)?;
    let backward = (0..n).find(|&k| x_at(k) == left && x_at(k + n - 1) > left)?;
    let walk = |start: usize, step: usize| {
        let mut chain = vec![ring[start]];
        let mut k = start;
        while chain[chain.len() - 1][0] < right {
            k = (k + step) % n;
            chain.push(ring[k]);
            if chain.len() > n {
                return None;
            }
        }
        Some(chain)
    };
    let top = walk(forward, 1)?;
    let bottom = walk(backward, n - 1)?;
    // Every point on one chain or the other, but for one more on the right
    // edge at most (between their ends, a straight line).
    if top.len() + bottom.len() > n + 2 || top.len() + bottom.len() < n {
        return None;
    }
    if !(top.windows(2).all(|w| w[1][0] >= w[0][0])
        && bottom.windows(2).all(|w| w[1][0] >= w[0][0]))
    {
        return None;
    }
    // One above the other at every point of either.
    let side = |x: f32| chain_y(&top, x) - chain_y(&bottom, x);
    let (mut above, mut below) = (false, false);
    for [x, _] in top.iter().chain(&bottom) {
        let gap = side(*x);
        above |= gap > 0.;
        below |= gap < 0.;
    }
    (!(above && below)).then_some((top, bottom))
}
/// A filled area's quad (polyline.wgsl's `zgx_area`): what `area_chains`
/// takes, in a colour or a linear gradient, its points and stops added to
/// `points`. It draws with the polyline strokes, which share its search and
/// its points; `None` rasterizes it.
fn area_quad(
    path: &zgx::canvas::Path,
    brush: &zgx::canvas::Brush,
    origin: (f32, f32),
    zoom: f32,
    points: &mut Vec<[f32; 2]>,
) -> Option<Quad> {
    use zgx::canvas::Brush;
    let (top, bottom) = area_chains(path.segments(), origin, zoom)?;
    // A pixel's search reaches its antialiased edge (`zgx_area`).
    const REACH: f32 = 2.;
    if !fits_reach(&top, REACH) || !fits_reach(&bottom, REACH) {
        return None;
    }
    let (color, along, stops) = match brush {
        Brush::Solid(color) => (*color, [0.; 4], None),
        Brush::Linear { start, end, stops } => {
            let (from, to) = (
                [start.x * zoom + origin.0, start.y * zoom + origin.1],
                [end.x * zoom + origin.0, end.y * zoom + origin.1],
            );
            let way = [to[0] - from[0], to[1] - from[1]];
            let length = way[0] * way[0] + way[1] * way[1];
            if !(2..=AREA_STOPS).contains(&stops.len())
                || !length.is_finite()
                || length <= 0.
                || !stops.iter().all(|s| (0. ..=1.).contains(&s.offset))
                || !stops.windows(2).all(|w| w[0].offset <= w[1].offset)
            {
                return None;
            }
            (
                Color(0, 0, 0, 0),
                [from[0], from[1], way[0], way[1]],
                Some(stops),
            )
        }
        Brush::Radial { .. } | Brush::Conic { .. } | Brush::Slash { .. } => return None,
    };
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for [x, y] in top.iter().chain(&bottom) {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    let bounds = Rect::new(x0, y0, x1 - x0, y1 - y0).expand(REACH);
    let first = points.len();
    points.extend_from_slice(&top);
    points.extend_from_slice(&bottom);
    let stop_count = stops.map_or(0, |stops| stops.len());
    for stop in stops.into_iter().flat_map(|stops| stops.iter()) {
        let [r, g, b, a] = rgba(stop.color, 1.);
        points.push([stop.offset, r * a]);
        points.push([g * a, b * a]);
        points.push([a, 0.]);
    }
    let rect = [bounds.x, bounds.y, bounds.width, bounds.height];
    Some(Quad {
        rect,
        uv: [0.; 4],
        color: rgba(color, 1.),
        fade: rect,
        options: [0.; 4],
        shape: [1., bottom.len() as f32, stop_count as f32, 0.],
        border: along,
        mask: [0., first as f32 + 1., top.len() as f32, 0.],
    })
}
/// A grid fill's marks over a canvas's box (`bounds`), as plain quads.
fn grid_quads(grid: &zgx::scene::GridFill, bounds: Rect, opacity: f32, quads: &mut Vec<Quad>) {
    let color = rgba(grid.color, opacity);
    quads.extend(grid.marks(bounds).into_iter().map(|r| {
        let rect = [r.x, r.y, r.width, r.height];
        Quad {
            rect,
            uv: [0.; 4],
            color,
            fade: rect,
            options: [0.; 4],
            shape: [0.; 4],
            border: [0.; 4],
            mask: NO_MASK,
        }
    }));
}
/// A canvas at `origin`, under `view` (its pan and zoom), drawn as quads,
/// when each of its commands is a solid
/// fill or stroke of a simple shape (`Path::shape`) the quad shader draws
/// as a rasterizer would: a rectangle, its corners rounded or not, a circle,
/// a line along an axis. No raster is made or uploaded, and moving or
/// scaling what it draws costs its quads alone. `None` for anything else
/// (curves, ellipses that aren't circles, gradients, patterns, dashes,
/// clips), which rasterizes as before.
fn canvas_quads(
    canvas: &zgx::canvas::Canvas,
    origin: (f32, f32),
    view: zgx::scene::CanvasView,
    areas: bool,
    points: &mut Vec<[f32; 2]>,
) -> Option<Vec<CanvasQuad>> {
    // Its text drawn into its raster: rasterized, words and all.
    if canvas.rasters_text() && !canvas.texts().is_empty() {
        return None;
    }
    let mark = points.len();
    let quads = canvas_quads_of(canvas, origin, view, areas, points);
    if quads.is_none() {
        points.truncate(mark);
    }
    quads
}
fn canvas_quads_of(
    canvas: &zgx::canvas::Canvas,
    origin: (f32, f32),
    view: zgx::scene::CanvasView,
    areas: bool,
    points: &mut Vec<[f32; 2]>,
) -> Option<Vec<CanvasQuad>> {
    use zgx::canvas::{Brush, Draw, LineCap, LineJoin, Segment, Shape};
    let mut out = Vec::with_capacity(canvas.commands().len());
    // The view scales what it draws about the box's origin, then moves it:
    // shapes stay shapes, only placed and sized as they show.
    let zoom = view.scale;
    let origin = (origin.0 + view.x, origin.1 + view.y);
    let at = |r: Rect| {
        Rect::new(
            r.x * zoom + origin.0,
            r.y * zoom + origin.1,
            r.width * zoom,
            r.height * zoom,
        )
    };
    // A box given by a corner and a negative size, the other way round.
    let positive = |r: Rect| {
        Rect::new(
            r.x.min(r.x + r.width),
            r.y.min(r.y + r.height),
            r.width.abs(),
            r.height.abs(),
        )
    };
    // Its pixels reach past its box by its antialiased edge: a quad shades
    // the pixels whose centres it covers, and a shape starting a quarter of
    // a pixel in would lose the quarter-covered pixel before it.
    let reach = |r: Rect| {
        let r = r.expand(1.);
        [r.x, r.y, r.width, r.height]
    };
    // A filled box with corners rounded by `radius`: the panel fill.
    let fill = |r: Rect, radius: f32, color: Color| {
        CanvasQuad::Fill(Quad {
            rect: reach(r),
            uv: [0.; 4],
            color: rgba(color, 1.),
            fade: [r.x, r.y, r.width, r.height],
            options: [0., 0., 0., 1.],
            shape: [0., 0., 0., 1.],
            border: [radius; 4],
            mask: NO_MASK,
        })
    };
    for command in canvas.commands() {
        match command {
            // Neither box nor circle: an area, if `zgx_area` draws it.
            Draw::Fill { path, brush, .. } if path.shape().is_none() => {
                if !areas {
                    return None;
                }
                let quad = area_quad(path, brush, origin, zoom, points)?;
                out.push(CanvasQuad::Polyline(quad));
            }
            Draw::Fill {
                path,
                brush: Brush::Solid(color),
                ..
            } => match path.shape()? {
                Shape::Rect { rect, radius } => {
                    let rect = positive(rect);
                    out.push(fill(at(rect), radius * zoom, *color));
                }
                Shape::Ellipse { rect } if rect.width == rect.height => {
                    let rect = positive(rect);
                    out.push(fill(at(rect), rect.width * zoom / 2., *color));
                }
                Shape::Ellipse { .. } => return None,
                // A line has no inside.
                Shape::Line { .. } => {}
            },
            Draw::Stroke {
                path,
                brush: Brush::Solid(color),
                style,
            } if style.dash.is_empty() && style.width > 0. && style.width * zoom < 255. => {
                let half = style.width / 2.;
                // As it shows: the stroke scales with what it outlines.
                let (width, shown_half) = (style.width * zoom, style.width * zoom / 2.);
                let Some(shape) = path.shape() else {
                    // Each subpath a polyline stroke.
                    let segments = path.segments();
                    let mut start = 0;
                    while start < segments.len() {
                        let end = segments[start + 1..]
                            .iter()
                            .position(|s| matches!(s, Segment::Move(_)))
                            .map_or(segments.len(), |at| start + 1 + at);
                        let quad = polyline_quad(
                            &segments[start..end],
                            style,
                            *color,
                            origin,
                            zoom,
                            points,
                        )?;
                        out.push(CanvasQuad::Polyline(quad));
                        start = end;
                    }
                    continue;
                };
                match shape {
                    // A ring as wide as the line, half outside the outline.
                    shape @ (Shape::Rect { .. } | Shape::Ellipse { .. }) => {
                        let (rect, radius) = match shape {
                            Shape::Rect { rect, radius } => (positive(rect), radius),
                            Shape::Ellipse { rect } if rect.width == rect.height => {
                                (positive(rect), rect.width.abs() / 2.)
                            }
                            _ => return None,
                        };
                        // Square corners join as the stroke asks: a miter
                        // (within its limit) stays square, a round join
                        // rounds by half the width, a bevel cuts them.
                        let outer = match (radius > 0., style.join) {
                            (true, _) => radius * zoom + shown_half,
                            (false, LineJoin::Round) => shown_half,
                            (false, LineJoin::Miter)
                                if style.miter_limit >= std::f32::consts::SQRT_2 =>
                            {
                                0.
                            }
                            (false, _) => return None,
                        };
                        let box_ = at(rect).expand(shown_half);
                        let side = (width * 16.).round();
                        let fade = [box_.x, box_.y, box_.width, box_.height];
                        out.push(CanvasQuad::Ring(Quad {
                            rect: reach(box_),
                            uv: [0.; 4],
                            color: rgba(*color, 1.),
                            fade,
                            options: [0., 0., 0., 1.],
                            shape: [side + 4096. * side, -1., side + 4096. * side, 1.],
                            border: [outer; 4],
                            mask: NO_MASK,
                        }));
                    }
                    // Along an axis, a line is a box as wide as it is, its
                    // ends as its caps say.
                    Shape::Line { from, to } if from.x == to.x || from.y == to.y => {
                        let reach = match style.cap {
                            LineCap::Butt => 0.,
                            LineCap::Square | LineCap::Round => half,
                        };
                        let (x0, x1) = (from.x.min(to.x), from.x.max(to.x));
                        let (y0, y1) = (from.y.min(to.y), from.y.max(to.y));
                        let rect = if from.y == to.y {
                            Rect::new(x0 - reach, y0 - half, x1 - x0 + 2. * reach, style.width)
                        } else {
                            Rect::new(x0 - half, y0 - reach, style.width, y1 - y0 + 2. * reach)
                        };
                        if rect.width <= 0. || rect.height <= 0. {
                            continue;
                        }
                        let radius = if style.cap == LineCap::Round {
                            shown_half
                        } else {
                            0.
                        };
                        out.push(fill(at(rect), radius, *color));
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    Some(out)
}
/// `style`'s border alone at `bounds`: a ring of its side widths (top,
/// right, bottom, left) inside its rounded box (`draw.wgsl`'s ring mode,
/// two widths a lane in 16ths of a pixel, up to 255 px).
/// A rounded selection's pieces (`TextHighlight::pieces`): each run a box
/// with its own corners, as a rounded run background is; each curve the ring
/// between its circle and a wider one, cut to its square by the quad. Both
/// are shapes every backend's shader has.
#[cold]
#[inline(never)]
fn push_highlight(
    quads: &mut Vec<Quad>,
    highlight: &zgx::scene::TextHighlight,
    (x, y): (f32, f32),
    scale: f32,
    opacity: f32,
    edge_fade: f32,
) {
    let color = rgba(highlight.color, opacity);
    let pieces = highlight.pieces(x, y, scale);
    quads.extend(
        pieces
            .into_iter()
            .map(|piece| piece_quad(piece, color, edge_fade)),
    );
}
/// A text node's marks: as shapes, as a selection is, if they were set so
/// (`Scene::set_text_mark_shapes`); otherwise each rounded a little, as a
/// browser's find marks are.
#[cold]
#[inline(never)]
fn push_marks(
    quads: &mut Vec<Quad>,
    scene: &zgx::scene::Scene,
    item: &zgx::scene::PaintItem<'_>,
    bounds: Rect,
    (x, y): (f32, f32),
    scale: f32,
) {
    let (opacity, edge_fade) = (item.effects.opacity, item.effects.edge_fade);
    if let Some(pieces) = scene.text_mark_pieces(item.id, x, y, scale) {
        quads.extend(
            (pieces.into_iter())
                .map(|(piece, color)| piece_quad(piece, rgba(color, opacity), edge_fade)),
        );
        return;
    }
    quads.extend(scene.text_marks(item.id).iter().map(|m| {
        let rect = [
            bounds.x + m.rect.x,
            bounds.y + m.rect.y,
            m.rect.width,
            m.rect.height,
        ];
        Quad {
            rect,
            uv: [0.; 4],
            color: rgba(m.color, opacity),
            fade: rect,
            options: [edge_fade, 0., 0., 0.],
            shape: [MARK_RADIUS, 0., 0., 1.],
            border: [0.; 4],
            mask: NO_MASK,
        }
    }));
}
/// A piece of a shape (`TextHighlight::pieces`) as a quad.
fn piece_quad(piece: zgx::scene::HighlightPiece, color: [f32; 4], edge_fade: f32) -> Quad {
    use zgx::scene::HighlightPiece;
    {
        match piece {
            HighlightPiece::Run { rect, radii } => {
                let rect = [rect.x, rect.y, rect.width, rect.height];
                Quad {
                    rect,
                    uv: [0.; 4],
                    color,
                    fade: rect,
                    options: [edge_fade, 0., 0., 1.],
                    shape: [0., 0., 0., 1.],
                    border: radii,
                    mask: NO_MASK,
                }
            }
            HighlightPiece::Fillet {
                rect,
                centre,
                radius,
            } => {
                // Far enough out that the wider circle's edge, and its
                // antialiasing, miss the square's far corner (1.41 × `radius`).
                let width = ((radius + 2.) * 16.).ceil();
                let outer = radius + width / 16.;
                Quad {
                    rect: [rect.x, rect.y, rect.width, rect.height],
                    uv: [0.; 4],
                    color,
                    fade: [centre.0 - outer, centre.1 - outer, outer * 2., outer * 2.],
                    options: [edge_fade, 0., 0., 1.],
                    shape: [width + 4096. * width, -1., width + 4096. * width, 1.],
                    border: [outer; 4],
                    mask: NO_MASK,
                }
            }
        }
    }
}
fn ring_quad(style: &zgx::scene::QuadStyle, bounds: Rect, opacity: f32, edge_fade: f32) -> Quad {
    let sides = style.border_sides();
    let width = |value: f32| match value.is_finite() {
        true => (value.max(0.) * 16.).round().min(4095.),
        false => 0.,
    };
    let fade = [bounds.x, bounds.y, bounds.width, bounds.height];
    Quad {
        rect: fade,
        uv: [0.; 4],
        color: rgba(style.border_color, opacity),
        fade,
        options: [edge_fade, 0., 0., 1.],
        shape: [
            width(sides.top) + 4096. * width(sides.right),
            -1.,
            width(sides.bottom) + 4096. * width(sides.left),
            1.,
        ],
        border: decoration::shadow_corners(style, bounds.width, bounds.height, 0.),
        mask: NO_MASK,
    }
}
/// A box's inset shadows (`Decoration::inset_shadows`): a sharp one a ring
/// inside it as wide on each side as the shadow's offset pushes it (and its
/// spread all round), in the shadow's colour, cut to the box's corners; a
/// blurred one a quad of the polyline strokes' module (`zgx_inset`), with its
/// offset, blur and spread in `points`. Whether any was that.
fn push_insets(
    quads: &mut Vec<Quad>,
    points: &mut Vec<[f32; 2]>,
    style: &zgx::scene::QuadStyle,
    bounds: Rect,
    opacity: f32,
    edge_fade: f32,
) -> bool {
    let shadows = style
        .decoration
        .as_deref()
        .and_then(|d| d.inset_shadows.as_deref())
        .unwrap_or(&[]);
    let mut blurred = false;
    for s in shadows.iter().filter(|s| s.color.3 > 0) {
        let spread = s.spread.max(0.);
        let fade = [bounds.x, bounds.y, bounds.width, bounds.height];
        let corners = decoration::shadow_corners(style, bounds.width, bounds.height, 0.);
        if s.blur_radius > 0. && s.blur_radius.is_finite() {
            let first = points.len();
            points.push([s.offset.x, s.offset.y]);
            points.push([s.blur_radius, spread]);
            quads.push(Quad {
                rect: fade,
                uv: [0.; 4],
                color: rgba(s.color, opacity),
                fade,
                options: [edge_fade, 0., 0., 1.],
                shape: [2., 0., 0., 0.],
                border: corners,
                mask: [0., first as f32 + 1., 1., 0.],
            });
            blurred = true;
            continue;
        }
        let width = |value: f32| match value.is_finite() {
            true => ((value.max(0.) + spread) * 16.).round().min(4095.),
            false => 0.,
        };
        let (x, y) = (s.offset.x, s.offset.y);
        quads.push(Quad {
            rect: fade,
            uv: [0.; 4],
            color: rgba(s.color, opacity),
            fade,
            options: [edge_fade, 0., 0., 1.],
            shape: [
                width(y) + 4096. * width(-x),
                -1.,
                width(-y) + 4096. * width(x),
                1.,
            ],
            border: corners,
            mask: NO_MASK,
        });
    }
    blurred
}
fn rgba(c: Color, opacity: f32) -> [f32; 4] {
    [
        c.0 as f32 / 255.,
        c.1 as f32 / 255.,
        c.2 as f32 / 255.,
        c.3 as f32 / 255. * opacity,
    ]
}
/// Quads into the retained target, or with `drawable` also into a second,
/// presentation target in the same pass (`fs_both`).
/// The full quad shader drawing into a drawable of `format` alone
/// (`Deferred::direct`).
fn drawable_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    blend: bool,
    format: wgpu::TextureFormat,
    basic: bool,
) -> wgpu::RenderPipeline {
    let all = wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4];
    let basic_attributes = [all[0], all[1], all[2], all[4]];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quads to the drawable"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if basic { "vs_basic" } else { "vs" }),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: 128,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: if basic { &basic_attributes } else { &all },
            }],
        },
        primitive: QUAD_STRIP,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if basic { "fs_basic" } else { "fs" }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
/// Premultiplied colour over the target: every quad's but gradient text's.
const OVER: wgpu::BlendState = wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING;
/// The colour where the target has some, as much as it has (source-in).
const SOURCE_IN: wgpu::BlendState = {
    let component = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::DstAlpha,
        dst_factor: wgpu::BlendFactor::Zero,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component,
        alpha: component,
    }
};
/// The colour under the target's (destination-over).
const BEHIND: wgpu::BlendState = {
    let component = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState {
        color: component,
        alpha: component,
    }
};
fn quad_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    blend: Option<wgpu::BlendState>,
    drawable: Option<wgpu::TextureFormat>,
    basic: bool,
) -> wgpu::RenderPipeline {
    let all = wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4];
    // `basic` reads rect, uv, colour and options only.
    let basic_attributes = [all[0], all[1], all[2], all[4]];
    let target = |format| {
        Some(wgpu::ColorTargetState {
            format,
            blend,
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let targets = [target(FORMAT), drawable.and_then(target)];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quads"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if basic { "vs_basic" } else { "vs" }),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: 128,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: if basic { &basic_attributes } else { &all },
            }],
        },
        primitive: QUAD_STRIP,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            // Single-pass presentation draws with the full shader alone.
            entry_point: Some(match (basic, drawable.is_some()) {
                (true, true) => "fs_basic_both",
                (true, false) => "fs_basic",
                (false, true) => "fs_both",
                (false, false) => "fs",
            }),
            compilation_options: Default::default(),
            targets: if drawable.is_some() {
                &targets
            } else {
                &targets[..1]
            },
        }),
        multiview_mask: None,
        cache: None,
    })
}
/// A shader fill's pipeline, or the image effects': quads blended over the
/// target, `module`'s `entry` shading them (`zgx_fill`, see `fill_source`;
/// `zgx_image_effect`, see `image_effect_source`).
/// Whether Direct3D 12 compiles with FXC, and with its debug flags (the
/// instance's `DEBUG`): an app's shaders then compile in a child.
#[cfg(windows)]
static APP_SHADER_FXC: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
/// Tests: an app's shaders built in the process (wgpu's naga and FXC) when
/// set, through a child (`app_fragment`) when not.
static APP_SHADERS_IN_PROCESS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// Tests: build an app's shaders in the process (`true`) or through a child
/// where one can (`false`, the default); for `app_shaders_d3d12`'s parity.
#[doc(hidden)]
pub fn set_app_shaders_in_process(in_process: bool) {
    APP_SHADERS_IN_PROCESS.store(in_process, std::sync::atomic::Ordering::Relaxed);
}

/// The wgpu-hal whose Direct3D 12 register assignment `dx12_fill::options`
/// follows: a wgpu update fails
/// `the_dx12_fill_register_map_was_checked_against_this_wgpu_hal` (on every
/// platform) until the map is checked again and `app_shaders_d3d12` passes.
#[cfg(test)]
const DX12_FILL_CHECKED_WGPU_HAL: &str = "29.0.4";

/// The pipeline layout an app's shader draws with (`dx12_fill::Layout`).
#[derive(Clone, Copy, Debug)]
enum AppLayout {
    Quads,
    Blend,
    #[cfg_attr(not(windows), allow(dead_code))]
    Inputs {
        blend: bool,
        textures: u32,
    },
}
/// Tests: end the session's FXC child, so the next app shader starts one.
#[doc(hidden)]
pub fn stop_app_shader_child() {
    #[cfg(windows)]
    fxc_child::stop();
}
/// Whether Direct3D 12's app shaders are compiled by FXC (in a child, as
/// the first context made in this process chose), not DXC: `None` before
/// any context (tests).
#[doc(hidden)]
pub fn app_shaders_compile_with_fxc() -> Option<bool> {
    #[cfg(windows)]
    return APP_SHADER_FXC.get().map(|&(fxc, _)| fxc);
    #[cfg(not(windows))]
    None
}
/// Compiles handed to a child so far in this process (tests).
#[doc(hidden)]
pub fn app_shaders_compiled_in_child() -> usize {
    #[cfg(windows)]
    return fxc_child::compiled();
    #[cfg(not(windows))]
    0
}

/// A shader fill's or layer shader's pipeline, or none if it fails to
/// build (a GPU error, not a panic in debug builds).
fn shader_pipeline(
    device: &wgpu::Device,
    make: impl FnOnce() -> wgpu::RenderPipeline,
) -> Option<wgpu::RenderPipeline> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let pipeline = make();
    match pollster::block_on(scope.pop()) {
        None => Some(pipeline),
        Some(error) => {
            eprintln!("zgx: a shader's pipeline doesn't build, so it draws nothing: {error}");
            None
        }
    }
}

fn fill_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    fragment: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    entry: &str,
) -> wgpu::RenderPipeline {
    let all = wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shader fill"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: 128,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &all,
            }],
        },
        primitive: QUAD_STRIP,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: fragment,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
/// A blended layer's pipeline (`prepare_blend`): its quad replaces the
/// target's pixels with the blend of the layer and the backdrop it read.
fn blend_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    fragment: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    entry: &str,
) -> wgpu::RenderPipeline {
    let all = wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blended layer"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: 128,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &all,
            }],
        },
        primitive: QUAD_STRIP,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: fragment,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
/// An image's quad drawn with its effect (image_effects.wgsl): the image's
/// lanes, its effect's size in the texture lane (premultiplied, `2 + size`),
/// characters theirs in `options.w` (-1: its positive values are per-corner
/// radii) and their reveal (their node's shader parameters) in the colour,
/// white for any image, and `border.zw`: no lane more for any quad.
#[cold]
#[inline(never)]
fn image_effect_quad(
    image: &zgx::image::ImageData,
    transform: zgx::affine::Affine,
    opacity: f32,
    edge_fade: f32,
    fade: [f32; 4],
    reveal: [[f32; 4]; 4],
) -> Quad {
    let transformed = image.transform() != zgx::affine::Affine::IDENTITY;
    let characters = matches!(image.effect(), zgx::image::ImageEffect::Characters(_));
    let ([front, band, ragged, _], [dx, dy, _, _]) = match characters {
        true => (reveal[0], reveal[1]),
        false => ([1.; 4], [0.; 4]),
    };
    Quad {
        rect: fade,
        uv: [0., 0., 1., 1.],
        color: [front, band, ragged, opacity],
        fade,
        options: [
            edge_fade,
            2. + image.effect().size(),
            if transformed { 1. } else { 0. },
            if characters { -1. } else { 0. },
        ],
        shape: match transformed {
            true => [transform.a, transform.c, transform.b, transform.d],
            false => [0.; 4],
        },
        border: match transformed {
            true => [transform.tx, transform.ty, dx, dy],
            false => [0., 0., dx, dy],
        },
        mask: NO_MASK,
    }
}
/// The image effects' module: the quad shader (draw.wgsl), whose vertex
/// stage, clip regions and shading they share, and image_effects.wgsl.
pub(crate) fn image_effect_source() -> String {
    format!(
        "{}\n{}",
        include_str!("draw.wgsl"),
        include_str!("image_effects.wgsl")
    )
}
/// The blends quads draw with besides premultiplied alpha, by name: gradient
/// text's (`Shading::SourceIn`, `Shading::Behind`), for `tests/every_shader.rs`
/// to make draw.wgsl's pipelines with.
#[doc(hidden)]
pub fn quad_blends() -> [(&'static str, wgpu::BlendState); 2] {
    [("source-in", SOURCE_IN), ("behind", BEHIND)]
}
/// Every shader module the renderer compiles, named, composed as it
/// composes them (a fill and a layer shader around a plain user part), for
/// checks that each compiles on a backend: `tests/every_shader.rs`, which
/// Windows CI runs through FXC with nothing cached.
#[doc(hidden)]
pub fn shader_modules() -> Vec<(&'static str, String)> {
    let present = |srgb: bool, post_alpha: bool| {
        format!(
            "const OUTPUT_SRGB:bool={srgb};\nconst POST_ALPHA:bool={post_alpha};\n{}",
            include_str!("blit.wgsl")
        )
    };
    [
        ("draw", include_str!("draw.wgsl").to_owned()),
        ("polyline", polyline_source(false)),
        ("polyline with areas", polyline_source(true)),
        ("image effects", image_effect_source()),
        ("blend", blend_source()),
        (
            "fill",
            fill_source(
                "fn fill(input:FillInput)->vec4<f32> { return vec4(input.p/input.size,input.time,1.); }",
            ),
        ),
        (
            "layer shader",
            filter_source("fn layer(input:LayerInput)->vec4<f32> { return input.color; }"),
        ),
        (
            "layer shader reading the backdrop",
            filter_source(
                "fn layer(input:LayerInput)->vec4<f32> { return zgx_backdrop(input.p+vec2(4.,0.)); }",
            ),
        ),
        ("blur", include_str!("blur.wgsl").to_owned()),
        (
            "blur, dual-source",
            format!(
                "enable dual_source_blending;\n{}\n{BLUR_MIX}",
                include_str!("blur.wgsl")
            ),
        ),
        ("effects", include_str!("effects.wgsl").to_owned()),
        ("copy both", include_str!("copy_both.wgsl").to_owned()),
        ("copy offset", include_str!("copy_offset.wgsl").to_owned()),
        (
            "texture upload",
            include_str!("texture_upload.wgsl").to_owned(),
        ),
        (
            "native surface",
            include_str!("native_surface.wgsl").to_owned(),
        ),
        ("present", present(false, false)),
        ("present, sRGB, alpha after", present(true, true)),
    ]
    .into_iter()
    // The glass material: an app's shader to FXC, as any of theirs.
    .chain([(
        "glass material",
        filter_source(zgx::glass::GlassStyle::material_source()),
    )])
    .collect()
}
/// The polyline strokes' module: the quad shader (draw.wgsl), whose vertex
/// stage and clip regions they share, polyline.wgsl, and its filled areas
/// (area.wgsl) where they draw on the GPU (`areas_on_gpu`).
fn polyline_source(areas: bool) -> String {
    let strokes = include_str!("polyline.wgsl");
    if areas {
        return format!(
            "{}\n{strokes}\n{}",
            include_str!("draw.wgsl"),
            include_str!("area.wgsl")
        );
    }
    // Without the branch to areas, not only their shader: FXC's optimizer
    // still overflowed on a branch to a stub returning nothing, where the
    // module without it compiles.
    assert!(
        strokes.contains(AREA_BRANCH),
        "polyline.wgsl's branch to areas"
    );
    format!(
        "{}\n{}",
        include_str!("draw.wgsl"),
        strokes.replace(AREA_BRANCH, "")
    )
}
/// The line of polyline.wgsl's `zgx_polyline` that hands an area to
/// area.wgsl's `zgx_area`.
const AREA_BRANCH: &str = " if v.shape.x>.5 {return zgx_area(v);}\n";
/// Whether a canvas's filled areas draw as quads (area.wgsl), for a device
/// of `backends` compiling shaders with `compiler`. Not where FXC compiles
/// them (Direct3D 12 without DXC): its optimizer recursed on area.wgsl until
/// it overflowed any stack, which crashed an app at its first canvas stroke.
/// A canvas holding an area rasterizes there, as one too dense for a quad
/// does. Not on the no-op backend either (Direct3D 11 draws no polylines).
fn areas_on_gpu(backends: wgpu::Backends, compiler: &wgpu::Dx12Compiler) -> bool {
    match backends {
        wgpu::Backends::DX12 => matches!(
            compiler,
            wgpu::Dx12Compiler::DynamicDxc { .. } | wgpu::Dx12Compiler::StaticDxc
        ),
        wgpu::Backends::NOOP => false,
        _ => true,
    }
}
/// The polyline strokes' pipeline: into the retained target, with
/// `drawable` also into that in the same pass (`fs_polyline_both`), or into
/// it `alone` (`Deferred::direct`).
fn polyline_pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    drawable: Option<wgpu::TextureFormat>,
    alone: bool,
) -> wgpu::RenderPipeline {
    let all = wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Float32x4,7=>Float32x4];
    let target = |format| {
        Some(wgpu::ColorTargetState {
            format,
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let targets = match (drawable, alone) {
        (Some(format), true) => vec![target(format)],
        (Some(format), false) => vec![target(FORMAT), target(format)],
        (None, _) => vec![target(FORMAT)],
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("polylines"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: 128,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &all,
            }],
        },
        primitive: QUAD_STRIP,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(if targets.len() == 2 {
                "fs_polyline_both"
            } else {
                "fs_polyline"
            }),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}
/// Whether presentation must convert premultiplied pixels to straight alpha.
/// wgpu reports Metal layers as "post-multiplied" merely because they are not
/// opaque, but Core Animation composites `CAMetalLayer` contents as
/// premultiplied (measured: straight 50% red over black shows twice as bright).
fn unpremultiplies(config: &wgpu::SurfaceConfiguration) -> bool {
    config.alpha_mode == wgpu::CompositeAlphaMode::PostMultiplied && !cfg!(target_os = "macos")
}
fn blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    post_alpha: bool,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("present"),
        source: wgpu::ShaderSource::Wgsl(
            (format!(
                "const OUTPUT_SRGB:bool={};\nconst POST_ALPHA:bool={};\n{}",
                format.is_srgb(),
                post_alpha,
                include_str!("blit.wgsl")
            ))
            .into(),
        ),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("present"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
fn blit_group(
    device: &wgpu::Device,
    pipeline: &wgpu::RenderPipeline,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("present"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(view),
        }],
    })
}
/// `r` grown to whole physical pixels, as `scissor` rounds it.
fn snap_out(r: Rect, scale: f32) -> Rect {
    let x = (r.x * scale).floor() / scale;
    let y = (r.y * scale).floor() / scale;
    let right = ((r.x + r.width) * scale).ceil() / scale;
    let bottom = ((r.y + r.height) * scale).ceil() / scale;
    Rect::new(x, y, right - x, bottom - y)
}
/// Whether `quad` lies entirely within `clip`.
fn inside(quad: &Quad, clip: Rect) -> bool {
    rect_inside(quad_rect(quad), clip)
}
/// Whether a quad covering `rect` lies entirely within `clip`.
fn rect_inside(rect: Rect, clip: Rect) -> bool {
    let Rect {
        x,
        y,
        width: w,
        height: h,
    } = rect;
    x >= clip.x && y >= clip.y && x + w <= clip.x + clip.width && y + h <= clip.y + clip.height
}
/// Whether a quad covering `rect` lies entirely outside `clip`.
fn rect_outside(rect: Rect, clip: Rect) -> bool {
    let Rect {
        x,
        y,
        width: w,
        height: h,
    } = rect;
    x + w <= clip.x || y + h <= clip.y || x >= clip.x + clip.width || y >= clip.y + clip.height
}
/// A backdrop filter's corner radii (top-left, top-right, bottom-right,
/// bottom-left): a rounded panel blurs only inside its own curve.
fn blur_corners(kind: &NodeKind, bounds: Rect) -> [f32; 4] {
    match kind {
        NodeKind::Panel { quad, .. } | NodeKind::Quad(quad) => {
            decoration::shadow_corners(quad, bounds.width, bounds.height, 0.)
        }
        _ => [0.; 4],
    }
}
fn scissor(r: Rect, scale: f32, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    // (A cast floors what is past zero and saturates the rest.)
    let x = (r.x * scale) as u32;
    let y = (r.y * scale) as u32;
    let right = ((r.x + r.width) * scale).ceil().min(width as f32) as u32;
    let bottom = ((r.y + r.height) * scale).ceil().min(height as f32) as u32;
    (right > x && bottom > y).then(|| (x, y, right - x, bottom - y))
}
// Each activated filter contributes its complete clipped output and sampling
// halo. Bounding merges may touch previously unrelated filters; rescan until
// all such dependencies are included. Each filter activates at most once.
fn blur_damage(damage: &[Rect], filters: &[(Rect, f32)], viewport: Rect, scale: f32) -> Vec<Rect> {
    let mut regions = merge_damage(damage, viewport, scale);
    let mut dependencies: Vec<_> = filters
        .iter()
        .filter_map(|(output, _radius)| {
            let width = (viewport.width * scale).round() as u32;
            let height = (viewport.height * scale).round() as u32;
            let output = output.intersection(viewport)?;
            // A filter samples only its own visible pixels, so damage
            // anywhere in them re-blurs all of them, and nothing beyond.
            let (x, y, w, h) = scissor(output, scale, width, height)?;
            let (left, top, right, bottom) = (x, y, x + w, y + h);
            let dependency = Rect::new(
                left as f32 / scale,
                top as f32 / scale,
                (right - left) as f32 / scale,
                (bottom - top) as f32 / scale,
            );
            Some((dependency, false))
        })
        .collect();
    // Each pass takes every dependency the regions meet (those it took
    // among them, so one meeting another just taken joins too), then merges
    // once: not once a dependency, which made a frame of many readers
    // quadratic.
    loop {
        let before = regions.len();
        for (dependency, processed) in &mut dependencies {
            if !*processed && regions.iter().any(|region| region.intersects(*dependency)) {
                regions.push(*dependency);
                *processed = true;
            }
        }
        if regions.len() == before {
            break;
        }
        regions = merge_damage(&regions, viewport, scale);
    }
    regions
}
fn rounded_damage(rect: Rect, viewport: Rect, scale: f32) -> Option<Rect> {
    let rect = rect.intersection(viewport)?;
    let x = (rect.x * scale).floor() / scale;
    let y = (rect.y * scale).floor() / scale;
    Some(Rect::new(
        x,
        y,
        ((rect.x + rect.width) * scale).ceil() / scale - x,
        ((rect.y + rect.height) * scale).ceil() / scale - y,
    ))
}
/// Push a panel quad. An opaque one with rounded corners or a border paints
/// exactly its fill colour away from its edges (in `shade`: past the corner
/// radius, the border and the antialiasing band of at most two device
/// pixels): that interior goes as a flat opaque quad, drawn unblended so the
/// GPU skips shading whatever it covers, and only the four bands around it
/// need the distance field and blending. Every band keeps the
/// panel's own outer edges (the right and bottom ones measured back from the
/// far edge), so the same pixels are covered and antialiased; the seams
/// between pieces lie on whole device pixels, where rounding cannot move a
/// pixel centre across them.
fn push_panel(quads: &mut Vec<Quad>, quad: Quad, scale: f32, split: bool) {
    if panel_whole(&quad, split) {
        quads.push(quad);
        return;
    }
    let radius = if quad.options[3] > 0.5 {
        quad.border.iter().copied().fold(0., f32::max)
    } else {
        quad.shape[0]
    };
    let band = radius.max(quad.shape[1] + 1.).max(0.) + 1.;
    let [x, y, width, height] = quad.rect;
    let (right, bottom) = (x + width, y + height);
    // The interior, on device pixels inside the bands.
    let (left, top) = (
        ((x + band) * scale).ceil() / scale,
        ((y + band) * scale).ceil() / scale,
    );
    let (inner_right, inner_bottom) = (
        ((right - band) * scale).floor() / scale,
        ((bottom - band) * scale).floor() / scale,
    );
    let (inner_width, inner_height) = (inner_right - left, inner_bottom - top);
    if inner_width <= 0. || inner_height <= 0. || inner_width * inner_height < PANEL_INTERIOR {
        quads.push(quad);
        return;
    }
    for rect in [
        [x, y, width, top - y],
        [x, bottom, width, inner_bottom - bottom],
        [x, top, left - x, inner_height],
        [right, top, inner_right - right, inner_height],
    ] {
        quads.push(Quad { rect, ..quad });
    }
    // Last, and opaque: drawn without blending, it hides what lies beneath
    // (the GPU skips shading fragments an opaque one covers), as the whole
    // panel's blended shader could not.
    let interior = [left, top, inner_width, inner_height];
    quads.push(Quad {
        rect: interior,
        fade: interior,
        options: [0.; 4],
        shape: [0.; 4],
        border: [0.; 4],
        ..quad
    });
}
/// Whether `push_panel` draws `quad` whole, known before its bands are
/// worked out: the interior lies inside the box, so a box under
/// `PANEL_INTERIOR` has none large enough.
#[inline(always)]
fn panel_whole(quad: &Quad, split: bool) -> bool {
    !split
        || quad.color[3] < 1.
        || quad.options[..3] != [0.; 3]
        || quad.shape[2] > 0.
        || quad.shape[3] <= 0.5
        || quad.rect != quad.fade
        || quad.rect[2] * quad.rect[3] < PANEL_INTERIOR
}
/// Nodes `shaped_like` remembers at most by what they show: past it, it
/// starts again.
const SHAPED_LIKE: usize = 4096;

/// Instances between draws for a region (of draws outside it, or empty
/// spare room) past which a batch from the kept frame ends rather than
/// spanning them (`batches`): holding the window's every draw, a batch
/// would otherwise take blocks of rows the damage misses. (A frame drawn
/// as usual holds what its damage reaches alone.)
const SKIPPED_SPLIT: u32 = 32;
/// Panels split only with an interior of at least this many logical pixels:
/// below, five quads cost more than shading one.
const PANEL_INTERIOR: f32 = 1024.;
/// Quads draw as four-vertex triangle strips, one per instance.
pub(crate) const QUAD_STRIP: wgpu::PrimitiveState = wgpu::PrimitiveState {
    topology: wgpu::PrimitiveTopology::TriangleStrip,
    strip_index_format: None,
    front_face: wgpu::FrontFace::Ccw,
    cull_mode: None,
    unclipped_depth: false,
    polygon_mode: wgpu::PolygonMode::Fill,
    conservative: false,
};
/// Regions a frame draws at most: each draws every batch it touches again.
const MAX_REGIONS: usize = 16;
/// Damage in at most `MAX_REGIONS` groups, each the bounds of its members:
/// rectangles sharing a cell of a grid, the finest of cells doubling from 16
/// that leaves few enough.
// Out of line: inlined, it grew `merge_damage`'s other paths (scroll's).
#[inline(never)]
fn cluster_damage(damage: &[Rect], viewport: Rect) -> Vec<Rect> {
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    // A grid's groups, or `None` if it leaves too many (counted as they
    // join, so a grid the search discards makes no bounds).
    let groups = |cell: f32| -> Option<Vec<Rect>> {
        let columns = (viewport.width / cell).ceil().max(1.) as usize;
        let rows = (viewport.height / cell).ceil().max(1.) as usize;
        let mut owner = vec![usize::MAX; columns * rows];
        let mut parent: Vec<usize> = (0..damage.len()).collect();
        let mut count = damage.len();
        for (i, r) in damage.iter().enumerate() {
            // The root of `i`'s group: `i` until a cell joins it to another,
            // then that one's root (each join keeps it a root).
            let mut group = i;
            // (A cast floors what is past zero and saturates the rest.)
            let first = |value: f32, count: usize| ((value / cell) as usize).min(count);
            let end =
                |value: f32, count: usize| ((value / cell).ceil().max(0.) as usize).min(count);
            for row in first(r.y, rows)..end(r.y + r.height, rows) {
                for column in first(r.x, columns)..end(r.x + r.width, columns) {
                    let slot = &mut owner[row * columns + column];
                    if *slot == usize::MAX {
                        *slot = i;
                    } else {
                        let other = root(&mut parent, *slot);
                        count -= usize::from(group != other);
                        parent[group] = other;
                        group = other;
                    }
                }
            }
        }
        if count > MAX_REGIONS && cell < viewport.width.max(viewport.height) {
            return None;
        }
        let mut bounds: Vec<Option<Rect>> = vec![None; damage.len()];
        for (i, r) in damage.iter().enumerate() {
            let group = &mut bounds[root(&mut parent, i)];
            *group = Some(group.map_or(*r, |g| g.union(*r)));
        }
        Some(bounds.into_iter().flatten().collect())
    };
    // Coarser cells only merge groups, so the answer is the finest cell that
    // fits. A frame's damage is much like the last one's: search from the
    // cell that fitted last time instead of from 16 (a dashboard changing
    // everywhere doubled five or six times every frame).
    thread_local! {
        static LAST: std::cell::Cell<f32> = const { std::cell::Cell::new(16.) };
    }
    let mut cell = LAST.get().max(16.);
    let mut found = groups(cell);
    if found.is_some() {
        while cell > 16. {
            let finer = groups(cell / 2.);
            if finer.is_none() {
                break;
            }
            cell /= 2.;
            found = finer;
        }
    } else {
        loop {
            cell *= 2.;
            found = groups(cell);
            if found.is_some() {
                break;
            }
        }
    }
    LAST.set(cell);
    // A cell as large as the viewport always fits.
    found.expect("a grid fits")
}
fn merge_damage(damage: &[Rect], viewport: Rect, scale: f32) -> Vec<Rect> {
    // Each region redraws every batch it touches: nearby small ones (the
    // dots of a spinner) are fewer draws as one, for a few more pixels. The
    // pixels gained are bounded by the smaller region, so a large region
    // doesn't swallow every small one in reach, growing as it goes.
    let area = |r: Rect| r.width * r.height * scale * scale;
    let close = |a: Rect, b: Rect| {
        let (a, b) = (area(a), area(b));
        a + b + a.min(b) * 0.5 + NEAR_PIXELS
    };
    let close = |a: Rect, b: Rect| a.intersects(b) || area(a.union(b)) <= close(a, b);
    let mut regions: Vec<Rect> = Vec::new();
    let rounded: Vec<Rect>;
    let damage = if damage.len() > MAX_REGIONS {
        rounded = cluster_damage(damage, viewport);
        &rounded[..]
    } else {
        damage
    };
    for r in damage {
        let Some(mut r) = rounded_damage(*r, viewport, scale) else {
            continue;
        };
        let mut i = 0;
        while i < regions.len() {
            if close(regions[i], r) {
                r = r.union(regions.swap_remove(i));
                i = 0;
            } else {
                i += 1;
            }
        }
        regions.push(r);
    }
    regions
}

pub mod assets;

fn create_quad_pipelines(
    device: &wgpu::Device,
) -> (
    wgpu::BindGroupLayout,
    wgpu::BindGroupLayout,
    wgpu::PipelineLayout,
) {
    let group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("quads"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    // The viewport, and the frame's clip regions (`Region`).
    let viewport_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("viewport"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quads"),
        bind_group_layouts: &[Some(&group_layout), Some(&viewport_layout)],
        immediate_size: 0,
    });
    (group_layout, viewport_layout, layout)
}

impl surface::SurfaceSource for GpuRenderer {
    type Frame = wgpu::SurfaceTexture;
    fn acquire(&mut self) -> Result<surface::Acquisition<Self::Frame>, GpuError> {
        use surface::Acquisition;
        #[cfg(windows)]
        self.match_background()?;
        Ok(match self.surface.as_ref().unwrap().get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Acquisition::Frame(frame),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // The swap chain is about to change: what its images hold
                // isn't trusted (each present copies all of the frame).
                #[cfg(target_os = "linux")]
                self.swap.forget();
                Acquisition::Frame(frame)
            }
            wgpu::CurrentSurfaceTexture::Timeout => Acquisition::Timeout,
            wgpu::CurrentSurfaceTexture::Occluded => Acquisition::Occluded,
            wgpu::CurrentSurfaceTexture::Lost => Acquisition::Lost,
            wgpu::CurrentSurfaceTexture::Outdated => Acquisition::Outdated,
            other => return Err(GpuError(format!("surface acquisition: {other:?}"))),
        })
    }
    fn recreate(&mut self) -> Result<(), GpuError> {
        #[cfg(all(windows, feature = "composition"))]
        if let Some(target) = &self.composition {
            self.surface = Some(target.surface(&self.instance)?);
            return Ok(());
        }
        if let Some((display, window)) = self.raw {
            // SAFETY: as `for_raw_surface`'s caller promised.
            self.surface = Some(
                unsafe {
                    self.instance
                        .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                            raw_display_handle: Some(display),
                            raw_window_handle: window,
                        })
                }
                .map_err(|e| GpuError(e.to_string()))?,
            );
            return Ok(());
        }
        self.surface = Some(
            self.instance
                .create_surface(self.window.as_ref().unwrap().clone())
                .map_err(|e| GpuError(e.to_string()))?,
        );
        #[cfg(target_os = "macos")]
        {
            self.transaction_present = false;
        }
        Ok(())
    }
    fn configure(&mut self) {
        self.surface
            .as_ref()
            .unwrap()
            .configure(&self.device, self.config.as_ref().unwrap());
        #[cfg(target_os = "linux")]
        self.swap.forget();
        // A new swap chain shows in its visual once the tree commits.
        #[cfg(all(windows, feature = "composition"))]
        if let Some(target) = &self.composition {
            target.commit();
        }
        // Configuring restores the default count.
        #[cfg(target_os = "macos")]
        if self.wide_until.is_some() {
            self.set_drawable_count(3);
        }
    }
}

/// The most rectangles a present copies apart, or names to the compositor;
/// past it, all of the window.
#[cfg(target_os = "linux")]
const SWAP_COPIES: usize = 64;
/// How many recent frames' changes `SwapHistory` keeps: an image presented
/// further back takes all of the frame.
#[cfg(target_os = "linux")]
const SWAP_HISTORY: usize = 8;
/// The swap chain's images on Linux's Vulkan surface (`copy_present`): the
/// frame each holds, by its image (a `VkImage`), and what recent frames
/// changed in the retained target, so a present copies what the image it
/// acquired lacks rather than all of it. Presentation leaves an image's
/// contents as they were (`vkQueuePresentKHR`).
#[cfg(target_os = "linux")]
#[derive(Default)]
struct SwapHistory {
    held: Vec<(u64, u64)>,
    frame: u64,
    history: std::collections::VecDeque<(u64, Vec<Scissor>)>,
    /// What the frames drawn since the last present changed.
    changed: Vec<Scissor>,
    /// `ZGX_SWAP_CHECK`'s reads of the frame being presented (image, target).
    checks: Vec<[wgpu::Buffer; 2]>,
    /// Presents checked so far.
    checked: u64,
}
/// `ZGX_SWAP_CHECK=1`: each partial copy into a swap chain image is read
/// back and compared with the frame (`check_swap`).
#[cfg(target_os = "linux")]
static SWAP_CHECK: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| zgx::env::var_os("ZGX_SWAP_CHECK").is_some_and(|v| v != "0"));
#[cfg(target_os = "linux")]
impl SwapHistory {
    /// A new swap chain: its images hold nothing known.
    fn forget(&mut self) {
        self.held.clear();
        self.history.clear();
    }
    /// What `image` lacks of the frame drawn, within `size`: the changes of
    /// every frame since the one it holds, and the frame's own; `None`
    /// (all of it) when what it holds isn't known, or is older than the
    /// history kept.
    fn lacks(&self, image: u64, (width, height): (u32, u32)) -> Option<Vec<Scissor>> {
        let held = self
            .held
            .iter()
            .find(|(i, _)| *i == image)
            .map(|(_, f)| *f)?;
        let mut rects: Vec<Scissor> = if held == self.frame {
            Vec::new()
        } else if self.history.front().is_some_and(|(f, _)| *f <= held + 1) {
            self.history
                .iter()
                .filter(|(frame, _)| *frame > held)
                .flat_map(|(_, changed)| changed.iter().copied())
                .collect()
        } else {
            return None;
        };
        rects.extend(self.changed.iter().copied());
        Some(
            rects
                .into_iter()
                .filter_map(|(x, y, w, h)| {
                    let (right, bottom) = ((x + w).min(width), (y + h).min(height));
                    (x < right && y < bottom).then(|| (x, y, right - x, bottom - y))
                })
                .collect(),
        )
    }
    /// `image` was presented: it holds the newest frame, whose changes join
    /// the history.
    fn presented(&mut self, image: u64) {
        self.frame += 1;
        let changed = std::mem::take(&mut self.changed);
        self.history.push_back((self.frame, changed));
        if self.history.len() > SWAP_HISTORY {
            self.history.pop_front();
        }
        match self.held.iter_mut().find(|(i, _)| *i == image) {
            Some(held) => held.1 = self.frame,
            None => self.held.push((image, self.frame)),
        }
    }
}
#[cfg(all(test, target_os = "linux"))]
mod swap_history_tests {
    use super::{SWAP_HISTORY, SwapHistory};
    /// Three images taking turns: each lacks the changes of the frames it
    /// missed and the frame's own, all of it while unknown or too old, and
    /// nothing once a new swap chain forgets them.
    #[test]
    fn an_image_lacks_what_changed_since_the_frame_it_holds() {
        let size = (100, 50);
        let mut swap = SwapHistory::default();
        // Unknown images: all of it.
        assert_eq!(swap.lacks(1, size), None);
        for (image, cell) in [(1, 0), (2, 10), (3, 20)] {
            swap.changed.push((cell, 0, 4, 4));
            swap.presented(image);
        }
        // Image 1 holds frame 1; frames 2 and 3 changed (10,0) and (20,0),
        // and the frame drawn now changes (30,0).
        swap.changed.push((30, 0, 4, 4));
        assert_eq!(
            swap.lacks(1, size),
            Some(vec![(10, 0, 4, 4), (20, 0, 4, 4), (30, 0, 4, 4)])
        );
        // Image 3 holds the last frame presented: the frame's own alone.
        assert_eq!(swap.lacks(3, size), Some(vec![(30, 0, 4, 4)]));
        // Clipped to a window that shrank.
        swap.changed.push((95, 45, 10, 10));
        assert_eq!(swap.lacks(3, size).unwrap().last(), Some(&(95, 45, 5, 5)));
        // An image last given a frame older than the history kept: all of it.
        for _ in 0..SWAP_HISTORY + 1 {
            swap.presented(2);
        }
        assert_eq!(swap.lacks(1, size), None);
        // A new swap chain: nothing is known.
        swap.forget();
        assert_eq!(swap.lacks(2, size), None);
    }
}
/// A Vulkan swap chain image's handle, `None` on another backend.
#[cfg(target_os = "linux")]
fn swap_image(texture: &wgpu::Texture) -> Option<u64> {
    use ash::vk::Handle;
    unsafe { texture.as_hal::<wgpu::hal::api::Vulkan>() }
        .map(|t| unsafe { t.raw_handle() }.as_raw())
}

#[cfg(test)]
mod raster_step_tests {
    use super::{RASTER_STEPS, step_scale};
    /// Every whole step of two is that power of two exactly, so a layer
    /// magnified 2 or 4 times, or shrunk as many, places its texel centres
    /// on the halves its reads ask for; the steps between are those times √2.
    /// (`SQRT_2.powi(2)` is a ULP under 2: a layer shader's read of its own
    /// far texels at 2x came out 253 of 255 on an RTX 4090 and on WARP.)
    #[test]
    fn whole_steps_of_two_are_exact() {
        for step in RASTER_STEPS {
            let scale = step_scale(step);
            match step % 2 == 0 {
                true => assert_eq!(scale, 2_f32.powi(step / 2), "step {step}"),
                false => {
                    let between = 2_f64.powf(f64::from(step) / 2.);
                    assert!(
                        (f64::from(scale) - between).abs() <= between * 1e-7,
                        "step {step}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod dx12_fill_pin {
    /// A wgpu update fails this until `dx12_fill::options`' register map is
    /// checked against its wgpu-hal again (and `app_shaders_d3d12` passes).
    #[test]
    fn the_dx12_fill_register_map_was_checked_against_this_wgpu_hal() {
        let lock = include_str!("../../../Cargo.lock");
        let version = lock
            .split("[[package]]")
            .find(|package| package.contains("name = \"wgpu-hal\"\n"))
            .and_then(|package| package.split("version = \"").nth(1))
            .and_then(|rest| rest.split('"').next())
            .expect("wgpu-hal in Cargo.lock");
        assert_eq!(
            version,
            super::DX12_FILL_CHECKED_WGPU_HAL,
            "wgpu-hal is {version}: check dx12_fill::options against its \
             create_pipeline_layout, run app_shaders_d3d12, then update \
             DX12_FILL_CHECKED_WGPU_HAL"
        );
    }
}

#[cfg(test)]
mod shadow_blur_tests {
    use super::{blur_coverage, shadow_sigma};
    /// A blur spreads a glyph's coverage three sigmas out and keeps all of
    /// it, at its own size and averaged down for a wide one; its middle is
    /// lighter, its reach has some.
    #[test]
    fn a_blurred_glyph_keeps_its_coverage_and_reaches_three_sigmas() {
        // (A glyph about as large as its blur: a far wider one spreads its
        // coverage thinner than an 8-bit alpha keeps.)
        for (sigma, (width, height)) in [(1.5f32, (4u32, 6u32)), (3., (6, 9)), (20., (24, 32))] {
            let coverage = vec![255f32; (width * height) as usize];
            let blurred = blur_coverage(&coverage, (width, height), sigma);
            let step = blurred.size.0 / blurred.texels.0;
            assert_eq!(step, blurred.size.1 / blurred.texels.1);
            assert!(blurred.reach as f32 >= sigma * 3., "{sigma}");
            assert!(blurred.size.0 >= width + 2 * blurred.reach);
            let total: f32 = (blurred.alpha.iter())
                .map(|&a| f32::from(a) * (step * step) as f32)
                .sum();
            let kept = total / (255. * (width * height) as f32);
            assert!((0.97..1.03).contains(&kept), "sigma {sigma}: {kept} of it");
            let middle = blurred.alpha
                [(blurred.texels.1 / 2 * blurred.texels.0 + blurred.texels.0 / 2) as usize];
            assert!(middle < 255, "sigma {sigma}: lighter in the middle");
            assert!(sigma < 4. || step > 1, "a wide blur is averaged down");
        }
    }
    /// Sigma is keyed in eighths of a device pixel, at most 128 pixels; none,
    /// negative or not a number is sharp.
    #[test]
    fn shadow_sigmas_key_in_eighths_of_a_device_pixel() {
        assert_eq!(shadow_sigma(0., 2.), 0);
        assert_eq!(shadow_sigma(-3., 2.), 0);
        assert_eq!(shadow_sigma(f32::NAN, 2.), 0);
        assert_eq!(shadow_sigma(2., 2.), 32);
        assert_eq!(shadow_sigma(1e6, 1.), 128 * 8);
    }
}
#[cfg(test)]
mod image_level_tests {
    /// The level an image needs is its coarsest halving still at least as
    /// large as it is drawn, and a halving averages texels four to one.
    #[test]
    fn images_are_held_at_the_size_they_are_drawn() {
        assert_eq!(
            super::fitted_size(4000, 3000, Some((1120., 840.))),
            (1120, 840)
        );
        assert_eq!(
            super::fitted_size(4000, 3000, Some((600., 840.))),
            (1120, 840)
        );
        assert_eq!(
            super::fitted_size(4000, 3000, Some((9000., 1.))),
            (4000, 3000)
        );
        assert_eq!(super::fitted_size(4000, 3000, None), (4000, 3000));
        assert_eq!(
            super::fitted_size(4000, 3000, Some((f32::NAN, 1.))),
            (4000, 3000)
        );
        assert_eq!(super::fitted_size(5, 3, Some((0., 0.))), (1, 1));
        let (out, w, h) = super::halve(&[0, 0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 255], 3, 1);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out, [128, 128, 128, 128, 255, 0, 0, 255]);
        // Three to two by area: the middle texel shared half and half.
        let row = [0, 0, 0, 255, 120, 120, 120, 255, 240, 240, 240, 255];
        assert_eq!(
            super::shrink(&row, 3, 1, (2, 1)),
            [40, 40, 40, 255, 200, 200, 200, 255]
        );
    }
}
#[cfg(test)]
mod blur_damage_tests {
    use super::*;

    /// A filter samples only its own pixels: damage beside it stays local,
    /// damage inside it re-blurs the whole filter and nothing more.
    #[test]
    fn damage_outside_a_filter_is_local_and_inside_rebuilds_the_filter() {
        let viewport = Rect::new(0., 0., 500., 300.);
        let filter = (Rect::new(100., 100., 40., 30.), 2.);
        let distant = Rect::new(10., 10., 5., 5.);
        assert_eq!(
            blur_damage(&[distant], &[filter], viewport, 1.),
            vec![distant]
        );
        let beside = Rect::new(95., 110., 1., 1.);
        assert_eq!(
            blur_damage(&[beside], &[filter], viewport, 1.),
            vec![beside]
        );
        assert_eq!(
            blur_damage(&[Rect::new(101., 110., 1., 1.)], &[filter], viewport, 1.),
            vec![Rect::new(100., 100., 40., 30.)]
        );
        assert!(blur_damage(&[], &[filter], viewport, 1.).is_empty());
    }

    #[test]
    fn chained_filters_expand_to_fixed_point_independent_of_order() {
        let viewport = Rect::new(0., 0., 500., 300.);
        // Overlapping filters: rebuilding one damages the other.
        let filters = [
            (Rect::new(135., 100., 20., 20.), 2.),
            (Rect::new(120., 100., 20., 20.), 2.),
        ];
        let damage = [Rect::new(121., 105., 1., 1.)];
        let expected = vec![Rect::new(120., 100., 35., 20.)];
        assert_eq!(blur_damage(&damage, &filters, viewport, 1.), expected);
        assert_eq!(
            blur_damage(&damage, &[filters[1], filters[0]], viewport, 1.),
            expected
        );
    }

    #[test]
    fn reconstruction_covers_the_filter_pixels_across_fractional_scales() {
        for scale in [0.7, 1., 1.25, 1.3, 1.5, 1.7, 2.3] {
            let viewport = Rect::new(0., 0., 701. / scale, 503. / scale);
            for index in 0..127 {
                let output = Rect::new(
                    (index * 37 % 680) as f32 / scale + 0.017,
                    (index * 19 % 480) as f32 / scale + 0.031,
                    7.71,
                    8.13,
                );
                for radius in [0.0001, 0.33334, 1.17, 11.31, 1000.] {
                    let (x, y, w, h) = scissor(output, scale, 701, 503).unwrap();
                    let regions = blur_damage(&[output], &[(output, radius)], viewport, scale);
                    assert_eq!(regions.len(), 1);
                    let (dx, dy, dw, dh) = scissor(regions[0], scale, 701, 503).unwrap();
                    assert!(dx <= x && dy <= y);
                    assert!(dx + dw >= x + w && dy + dh >= y + h);
                }
            }
        }
    }

    #[test]
    fn fractional_output_rounds_to_whole_pixels_whatever_the_radius() {
        let viewport = Rect::new(0., 0., 400., 300.);
        // At 1.25x the output spans physical [12, 26).
        let regions = blur_damage(
            &[Rect::new(10.2, 10.2, 1., 1.)],
            &[(Rect::new(10.1, 10.1, 10.1, 10.1), 0.25)],
            viewport,
            1.25,
        );
        let (x, y, width, height) = scissor(regions[0], 1.25, 500, 375).unwrap();
        assert!(x <= 12 && y <= 12 && x + width >= 26 && y + height >= 26);
        assert!(width < 20 && height < 20);
        let huge = blur_damage(
            &[Rect::new(0., 0., 1., 1.)],
            &[(Rect::new(0., 0., 1., 1.), 10000.)],
            viewport,
            1.,
        );
        assert_eq!(huge, vec![Rect::new(0., 0., 1., 1.)]);
    }
}

pub mod canvas;

pub mod decoration;

pub mod svg;

#[cfg(test)]
mod batch_order_tests {
    use super::*;
    fn draw(start: u32, end: u32, x: f32) -> Draw {
        Draw {
            shading: Shading::Basic,
            blur: None,
            start,
            end,
            clip: Rect::new(0., 0., 40., 20.),
            bounds: Rect::new(x, 0., 4., 4.),
            source: TextureSource::Atlas,
        }
    }
    fn batches(draws: &[Draw]) -> Vec<(u32, u32)> {
        let gpu = GpuRenderer::new(40, 20).unwrap();
        let mut out = Vec::new();
        gpu.batches(Rect::new(0., 0., 40., 20.), draws, 0, None, |batch| {
            out.push((batch.start, batch.end))
        });
        out
    }
    /// Draws in instance order join one batch, across a gap (instances of
    /// draws outside the region, scissored away) too.
    #[test]
    fn draws_in_instance_order_join_one_batch() {
        assert_eq!(batches(&[draw(0, 2, 0.), draw(2, 3, 8.)]), [(0, 3)]);
        assert_eq!(batches(&[draw(0, 2, 0.), draw(5, 6, 8.)]), [(0, 6)]);
    }
    /// A draw whose instances lie before the batch's would cut it short:
    /// both would draw nothing. That fails loudly.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "out of order")]
    fn a_draw_before_the_batch_it_joins_fails_loudly() {
        batches(&[draw(4, 6, 0.), draw(0, 2, 8.)]);
    }
}

#[cfg(test)]
mod native_surface_shader_tests {
    #[test]
    fn nv12_compute_shader_validates_on_the_available_gpu_backend() {
        let gpu = super::GpuRenderer::new(4, 4).unwrap();
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("NV12 portable validation"),
                source: wgpu::ShaderSource::Wgsl(include_str!("native_surface.wgsl").into()),
            });
        let _pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("NV12 validation"),
                layout: None,
                module: &shader,
                entry_point: Some("convert"),
                compilation_options: Default::default(),
                cache: None,
            });
    }
}

/// A single-use module (`single_use_module`; with Metal, its IR goes once
/// its pipeline is made) makes a second pipeline from its source again,
/// drawing as the first does, and as a module made otherwise does.
#[cfg(all(test, target_os = "macos"))]
mod single_use_module_tests {
    const SOURCE: &str =
        "@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
 return vec4(f32(i & 1u) * 4. - 1., f32(i >> 1u) * 4. - 1., 0., 1.);
}
@fragment fn fs()->@location(0) vec4<f32> { return vec4(1., .5, .25, 1.); }";

    /// Two pipelines from one module, each drawing a 4x4 target: whether
    /// each built (no error), and what each drew.
    fn two_pipelines_from_one_module(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        single_use: bool,
    ) -> Vec<(bool, Vec<u8>)> {
        let module = match single_use {
            true => super::single_use_module(device, "single use", SOURCE),
            false => device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("as wgpu makes it"),
                source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
            }),
        };
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let made = || {
            // (The hal's link failure reaches wgpu as an internal error.)
            let scope = device.push_error_scope(wgpu::ErrorFilter::Internal);
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("single use"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            let built = pollster::block_on(scope.pop()).is_none();
            (built, drawn(device, queue, &pipeline, format))
        };
        vec![made(), made()]
    }

    fn drawn(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipeline: &wgpu::RenderPipeline,
        format: wgpu::TextureFormat,
    ) -> Vec<u8> {
        let size = wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let view = target.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            pass.set_pipeline(pipeline);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &out,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: None,
                },
            },
            size,
        );
        queue.submit([encoder.finish()]);
        out.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = out.slice(..).get_mapped_range();
        mapped.to_vec()
    }

    #[test]
    fn a_single_use_modules_second_pipeline_is_built_again_and_draws_the_same() {
        let gpu = super::GpuRenderer::new(4, 4).unwrap();
        let single_use = two_pipelines_from_one_module(&gpu.device, &gpu.queue, true);
        let elsewhere = two_pipelines_from_one_module(&gpu.device, &gpu.queue, false);
        let first = &elsewhere[0].1[..4];
        assert_eq!((first[2], first[3]), (255, 255), "drawn (BGRA): {first:?}");
        for (built, pixels) in single_use.iter().chain(&elsewhere) {
            assert!(built);
            assert_eq!(pixels, &elsewhere[0].1);
        }
    }
}

// macOS writes shared textures directly; the paths tested are the others'.
#[cfg(all(test, not(target_os = "macos")))]
mod texel_upload_tests {
    use super::*;
    use zgx::scene::{Color, Layout, Style};

    /// Images keep their channels whichever way texels reach the GPU: the
    /// adapter's own (an upload pass where CPU and GPU share memory) and the
    /// queue copy every other adapter takes, forced here.
    #[test]
    fn images_keep_red_and_blue_through_either_upload_path() {
        let image = Arc::new(
            zgx::image::ImageData::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap(),
        );
        for copied in [false, true] {
            let mut gpu = GpuRenderer::new(40, 20).unwrap();
            if copied {
                gpu.mapped = None;
            }
            let mut scene = Scene::new(40., 20.);
            scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
            scene.append(
                scene.root(),
                NodeKind::Rect(Color(0, 0, 0, 255)),
                Style {
                    width: Some(40.),
                    height: Some(20.),
                    ..Default::default()
                },
            );
            scene.append(
                scene.root(),
                NodeKind::Image(image.clone()),
                Style {
                    width: Some(40.),
                    height: Some(20.),
                    ..Default::default()
                },
            );
            let report = scene.flush();
            gpu.render(&scene, &report.damage).unwrap();
            let pixels = gpu.readback().unwrap();
            let pixel = |x: usize| &pixels[(10 * 40 + x) * 4..(10 * 40 + x) * 4 + 4];
            assert_eq!(pixel(2), &[255, 0, 0, 255], "left, copied={copied}");
            assert_eq!(pixel(37), &[0, 0, 255, 255], "right, copied={copied}");
        }
    }
}
/// wgpu's instance flags for the build, as zgx sets them: no indirect-call
/// validation (zgx draws nothing indirect; it would compile a compute shader
/// while creating the device, a few milliseconds of startup), and in release
/// builds no names for the driver. wgpu names every command list and object
/// it makes otherwise, several a frame (Direct3D 12's `SetName`: 16 K cycles
/// a frame of `animation_workload` on Windows). `WGPU_DISCARD_HAL_LABELS=0`
/// brings them back for a capture (`with_env`, applied after).
fn instance_flags(build: wgpu::InstanceFlags, release: bool) -> wgpu::InstanceFlags {
    let mut flags = build - wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL;
    flags.set(wgpu::InstanceFlags::DISCARD_HAL_LABELS, release);
    flags
}

#[cfg(test)]
mod instance_flags_tests {
    use super::instance_flags;
    use wgpu::InstanceFlags as F;

    #[test]
    fn release_builds_give_the_driver_no_names() {
        for build in [F::from_build_config(), F::debugging(), F::empty()] {
            let release = instance_flags(build, true);
            assert!(release.contains(F::DISCARD_HAL_LABELS));
            assert!(!release.contains(F::VALIDATION_INDIRECT_CALL));
            let debug = instance_flags(build, false);
            assert!(!debug.contains(F::DISCARD_HAL_LABELS));
            assert!(!debug.contains(F::VALIDATION_INDIRECT_CALL));
            // Everything else is wgpu's for the build.
            let rest = !(F::DISCARD_HAL_LABELS | F::VALIDATION_INDIRECT_CALL);
            assert_eq!(release & rest, build & rest);
        }
    }
}

#[cfg(test)]
mod keep_room_tests {
    use super::keep_room;

    #[test]
    fn a_spike_of_quads_is_given_back_and_steady_frames_keep_their_room() {
        let mut quads: Vec<[u8; 128]> = Vec::new();
        let mut peak = 0;
        let frame = |quads: &mut Vec<[u8; 128]>, peak: &mut usize, used: usize| {
            quads.clear();
            quads.resize(used, [0; 128]);
            keep_room(quads, peak);
        };
        // Steady frames, a little above and below each other: no shrinking.
        for i in 0..200 {
            frame(&mut quads, &mut peak, 3000 + (i % 7) * 100);
        }
        let steady = quads.capacity();
        for i in 0..200 {
            frame(&mut quads, &mut peak, 3000 + (i % 7) * 100);
            assert_eq!(quads.capacity(), steady, "steady frames keep their room");
        }
        // One frame of a long page: the room grows, then goes back.
        frame(&mut quads, &mut peak, 60_000);
        assert!(quads.capacity() >= 60_000);
        for _ in 0..300 {
            frame(&mut quads, &mut peak, 3000);
        }
        assert!(
            quads.capacity() <= 2 * 3000,
            "a spike's room is given back: {}",
            quads.capacity()
        );
        // Small vectors are left alone.
        let mut small: Vec<[u8; 128]> = Vec::with_capacity(400);
        let mut small_peak = 0;
        for _ in 0..200 {
            small.clear();
            small.push([0; 128]);
            keep_room(&mut small, &mut small_peak);
        }
        assert_eq!(small.capacity(), 400);
    }
}

#[cfg(test)]
mod copy_blocker_tests {
    use super::*;
    use zgx::{
        compose::prelude::{rgba, *},
        widgets::Ui,
    };

    /// Which paint items lie within a node, asked in paint order as the copy
    /// asks of its content's, say what `Scene::is_within` says, for every
    /// item and whatever node, with raised rows and border passes among them;
    /// and within a scroll view's content none is walked up the tree.
    #[test]
    fn items_within_a_node_in_paint_order_without_a_walk_each() {
        let mut ui = Ui::new(200., 300.);
        let offset = ui.signal(0_f32);
        let view = ui.mount(
            column()
                .w(200.)
                .h(300.)
                .child(div().id("header").w(200.).h(20.).bg(rgb(0x111111)))
                .child(
                    scroll(offset.clone())
                        .id("list")
                        .w(200.)
                        .h(260.)
                        .bg(rgb(0x202020))
                        .children((0..12).map(|i| {
                            let mut row = row()
                                .id(format!("row-{i}"))
                                .w(200.)
                                .h(24.)
                                .bg(rgb(0x303030))
                                .child(div().w(20.).h(20.).bg(rgb(0x404040)))
                                .child(
                                    column()
                                        .id(format!("parts-{i}"))
                                        .child(div().w(100.).h(10.).bg(rgb(0x505050)))
                                        .child(div().w(100.).h(10.).bg(rgb(0x606060))),
                                );
                            if i % 4 == 1 {
                                // A border pass: its outline after what it holds.
                                row = row.outline(2., rgb(0xffffff));
                            }
                            row
                        })),
                )
                .child(div().id("footer").w(200.).h(20.).bg(rgb(0x111111))),
        );
        ui.prepare_frame();
        let raised = view.find("row-6").unwrap();
        ui.scene.borrow_mut().set_z(raised, 1);
        ui.prepare_frame();
        let scene = ui.scene.borrow();
        let items = scene.layer_items(None);
        assert!(items.iter().any(|item| item.border_pass), "a border pass");
        let list = view.find("list").unwrap();
        let roots = [
            list,
            view.find("row-1").unwrap(),
            view.find("parts-6").unwrap(),
            view.find("header").unwrap(),
            scene.root(),
        ];
        for root in roots {
            let mut within = Within::default();
            for item in &items {
                assert_eq!(
                    within.holds(&scene, item.id, root),
                    scene.is_within(item.id, root),
                    "@{} within @{}",
                    item.id.number(),
                    root.number()
                );
            }
        }
        // The content's run, as the copy asks: not one walked up.
        let first = items.iter().position(|item| item.id == list).unwrap();
        let last = items
            .iter()
            .rposition(|item| scene.is_within(item.id, list))
            .unwrap();
        let mut within = Within::default();
        for item in &items[first..=last] {
            assert!(within.holds(&scene, item.id, list));
        }
        assert_eq!(within.1, 0, "items of the content walked up the tree");
    }

    /// Whether `carried` repaints the point `(x, y)`.
    fn repaints(carried: &[Rect], x: f32, y: f32) -> bool {
        carried
            .iter()
            .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
    }

    /// A node that stays put over a scroll view no longer keeps its scroll
    /// from copying the pixels drawn: it repaints where it is and where the
    /// copy put it. A blurred one repaints with what's behind it, and the
    /// list copies under it too; with nothing over it, the same scroll copies
    /// with nothing to repaint.
    #[test]
    fn a_node_over_a_scroll_view_repaints_itself_and_the_view_copies() {
        for cover in [None, Some(false), Some(true)] {
            let mut ui = Ui::new(160., 120.);
            let offset = ui.signal(0_f32);
            let rows = virtual_list(
                offset.clone(),
                24.,
                2,
                || 1000,
                |i| i,
                |_, key, _| {
                    div().size(144., 24.).bg(if key % 2 == 0 {
                        rgb(0x223344)
                    } else {
                        rgb(0x334455)
                    })
                },
            )
            .w(160.)
            .h(100.)
            .p(8.)
            .bg(rgb(0x101820));
            let mut root = overlay().w(160.).h(120.).bg(rgb(0x000000)).child(rows);
            if let Some(blurred) = cover {
                let mut square = div()
                    .absolute()
                    .left(40.)
                    .top(40.)
                    .size(30., 30.)
                    .bg(rgba(0xffffff80));
                if blurred {
                    square = square.backdrop_blur(6.);
                }
                root = root.child(square);
            }
            ui.mount(root);
            ui.prepare_frame();
            ui.scene.borrow_mut().flush();
            offset.set(14.);
            ui.prepare_frame();
            let damage = ui.scene.borrow_mut().flush().damage;
            let scene = ui.scene.borrow();
            let scroll = scene.scroll_moves().last().expect("it moved").clone();
            let mut carried = Vec::new();
            let blocker = copy_blocker(&scene, &scroll, &damage, 1., &mut carried, &mut None);
            match cover {
                None => {
                    assert_eq!(blocker, None);
                    assert!(carried.is_empty(), "{carried:?}");
                }
                Some(false) => {
                    assert_eq!(
                        blocker, None,
                        "the square over the list repaints, the list copies"
                    );
                    assert!(repaints(&carried, 55., 55.), "where it is: {carried:?}");
                    assert!(
                        repaints(&carried, 55., 55. + scroll.dy),
                        "where the copy put it: {carried:?}"
                    );
                }
                Some(true) => {
                    // (A blurred one repaints with what's behind it: it's in
                    // the frame's damage, and the list copies under it.)
                    assert_eq!(
                        blocker, None,
                        "the blurred square repaints, the list copies"
                    );
                    assert!(
                        damage.iter().any(|d| d.x <= 40.
                            && d.y <= 40.
                            && d.x + d.width >= 70.
                            && d.y + d.height >= 70.),
                        "the blurred square is in the damage: {damage:?}"
                    );
                }
            }
        }
    }

    /// A clear square, or a clear rectangle (an overlay scrollbar's track
    /// away from the pointer), over a scroll view paints nothing: the
    /// scroll copies, as with nothing over it (main's case, kept).
    #[test]
    fn a_clear_node_over_a_scroll_view_lets_it_copy() {
        for rect in [false, true] {
            let mut ui = Ui::new(160., 120.);
            let offset = ui.signal(0_f32);
            let rows = virtual_list(
                offset.clone(),
                24.,
                2,
                || 1000,
                |i| i,
                |_, key, _| {
                    div().size(144., 24.).bg(if key % 2 == 0 {
                        rgb(0x223344)
                    } else {
                        rgb(0x334455)
                    })
                },
            )
            .w(160.)
            .h(100.)
            .p(8.)
            .bg(rgb(0x101820));
            let square = div()
                .absolute()
                .left(40.)
                .top(40.)
                .size(30., 30.)
                .bg(rgba(0xffffff00));
            ui.mount(
                overlay()
                    .w(160.)
                    .h(120.)
                    .bg(rgb(0x000000))
                    .child(rows)
                    .child(square),
            );
            ui.prepare_frame();
            if rect {
                let mut scene = ui.scene.borrow_mut();
                let square = *scene.children(scene.root()).last().unwrap();
                scene.set_kind(square, NodeKind::Rect(zgx::scene::Color(255, 255, 255, 0)));
            }
            ui.scene.borrow_mut().flush();
            offset.set(14.);
            ui.prepare_frame();
            let damage = ui.scene.borrow_mut().flush().damage;
            let scene = ui.scene.borrow();
            let scroll = scene.scroll_moves().last().expect("it moved").clone();
            let blocker = copy_blocker(&scene, &scroll, &damage, 1., &mut Vec::new(), &mut None);
            assert_eq!(
                blocker,
                None,
                "a clear {} paints nothing",
                if rect { "rect" } else { "square" }
            );
        }
    }

    /// A list with no fill of its own on a see-through rounded card with an
    /// edge copies: the card is one colour across the list's view but for its
    /// corners, which repaint. Something of its own under the list (a square
    /// on the card) still keeps it from copying.
    #[test]
    fn a_see_through_card_under_a_list_lets_it_copy() {
        for marked in [false, true] {
            let mut ui = Ui::new(200., 160.);
            let offset = ui.signal(0_f32);
            let rows = virtual_list(
                offset.clone(),
                24.,
                2,
                || 1000,
                |i| i,
                |_, key, _| {
                    div().size(150., 12.).bg(if key % 2 == 0 {
                        rgb(0x223344)
                    } else {
                        rgb(0x334455)
                    })
                },
            )
            .absolute()
            .left(21.)
            .top(21.)
            .w(158.)
            .h(118.);
            let card = div()
                .absolute()
                .left(20.)
                .top(20.)
                .size(160., 120.)
                .bg(rgba(0x2a2a2acc))
                .border(1.)
                .border_color(rgba(0xffffff20))
                .rounded(12.);
            let mut root = overlay().w(200.).h(160.).child(card);
            if marked {
                root = root.child(
                    div()
                        .absolute()
                        .left(60.)
                        .top(60.)
                        .size(10., 10.)
                        .bg(rgb(0xff0000)),
                );
            }
            ui.mount(root.child(rows));
            ui.prepare_frame();
            ui.scene.borrow_mut().flush();
            offset.set(14.);
            ui.prepare_frame();
            let damage = ui.scene.borrow_mut().flush().damage;
            let scene = ui.scene.borrow();
            let scroll = scene.scroll_moves().last().expect("it moved").clone();
            let mut carried = Vec::new();
            let blocker = copy_blocker(&scene, &scroll, &damage, 1., &mut carried, &mut None);
            if marked {
                assert!(
                    matches!(blocker, Some(CopyBlocker::Under(_))),
                    "the square under the list keeps it from copying: {blocker:?}"
                );
            } else {
                assert_eq!(blocker, None, "the card is one colour under the list");
                assert!(
                    repaints(&carried, 22., 22.),
                    "a corner repaints: {carried:?}"
                );
                assert!(
                    !repaints(&carried, 100., 80.),
                    "the middle copies: {carried:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod raised_copy_tests {
    use super::*;
    use zgx::{
        compose::prelude::{rgba, *},
        widgets::Ui,
    };

    /// A node raised over a scroll view repaints over it, as a later sibling
    /// does, though it comes first in the document; not raised it lies under
    /// the list, hidden by its fill. The scroll copies either way.
    #[test]
    fn a_raised_node_over_a_scroll_view_repaints_over_it() {
        for raised in [false, true] {
            let mut ui = Ui::new(160., 120.);
            let offset = ui.signal(0_f32);
            let rows = virtual_list(
                offset.clone(),
                24.,
                2,
                || 1000,
                |i| i,
                |_, key, _| {
                    div().size(144., 24.).bg(if key % 2 == 0 {
                        rgb(0x223344)
                    } else {
                        rgb(0x334455)
                    })
                },
            )
            .w(160.)
            .h(100.)
            .p(8.)
            .bg(rgb(0x101820));
            let mut badge = div()
                .absolute()
                .left(40.)
                .top(40.)
                .size(30., 30.)
                .bg(rgba(0xffffff80));
            if raised {
                badge = badge.z(1);
            }
            ui.mount(
                overlay()
                    .w(160.)
                    .h(120.)
                    .bg(rgb(0x000000))
                    .child(badge)
                    .child(rows),
            );
            ui.prepare_frame();
            ui.scene.borrow_mut().flush();
            offset.set(14.);
            ui.prepare_frame();
            let damage = ui.scene.borrow_mut().flush().damage;
            let scene = ui.scene.borrow();
            let scroll = scene.scroll_moves().last().expect("it moved").clone();
            let mut carried = Vec::new();
            let blocker = copy_blocker(&scene, &scroll, &damage, 1., &mut carried, &mut None);
            assert_eq!(blocker, None, "the list copies either way");
            let square =
                |r: &Rect| r.x <= 55. && 55. < r.x + r.width && r.y <= 55. && 55. < r.y + r.height;
            assert_eq!(
                carried.iter().any(square),
                raised,
                "raised, the square repaints over the list: {carried:?}"
            );
        }
    }
}

#[cfg(test)]
mod kept_index_tests {
    use super::*;

    fn scene_ids(count: usize) -> (Scene, Vec<NodeId>, zgx::scene::Inherited) {
        let mut scene = Scene::new(100., 100.);
        let mut ids = vec![scene.root()];
        for _ in 1..count {
            ids.push(scene.append(
                scene.root(),
                NodeKind::Rect(Color(20, 30, 40, 255)),
                zgx::scene::Style::default(),
            ));
        }
        scene.flush();
        let mut inherited = Vec::new();
        scene.layer_items_recorded(&mut inherited);
        (scene, ids, inherited.remove(0))
    }

    fn items(ids: &[NodeId], inherited: &zgx::scene::Inherited) -> Vec<KeptItem> {
        ids.iter()
            .enumerate()
            .map(|(at, &id)| KeptItem {
                id,
                inherited: inherited.clone(),
                start: 0,
                len: 0,
                room: 0,
                draws: 0..0,
                boxed: None,
                end: (ids[at..].iter().rposition(|&other| other == id).unwrap() + at + 1) as u32,
            })
            .collect()
    }

    fn agrees(index: &KeptIndex, kept: &[KeptItem], ids: &[NodeId]) {
        for &id in ids {
            assert_eq!(
                index.get(id, kept),
                kept.iter().position(|item| item.id == id)
            );
        }
    }

    #[test]
    fn subtree_ends_count_unique_nodes_through_nested_border_passes() {
        let (mut scene, ids, _) = scene_ids(2);
        let quad = zgx::scene::QuadStyle {
            border_over: true,
            border_width: 1.,
            border_color: Color(255, 255, 255, 255),
            ..Default::default()
        };
        scene.set_kind(ids[1], NodeKind::Quad(quad.clone()));
        let style = zgx::scene::Style {
            width: Some(80.),
            height: Some(80.),
            ..Default::default()
        };
        scene.set_style(ids[1], style.clone());
        let child = scene.append(ids[1], NodeKind::Quad(quad), style.clone());
        scene.append(child, NodeKind::Rect(Color(1, 2, 3, 255)), style);
        scene.flush();
        let mut inherited = Vec::new();
        let painted = scene.layer_items_recorded(&mut inherited);
        let expected = painted.iter().filter(|item| !item.border_pass).count();
        let ids: Vec<_> = painted.iter().map(|item| item.id).collect();
        let mut kept = items(&ids, &inherited[0]);
        subtree_ends(&scene, &mut kept, 0);
        assert_eq!(KeptIndex::nodes(&kept), expected);
        assert!(
            kept.len() > expected,
            "the test must include border duplicates"
        );
        let mut sparse = FxHashMap::default();
        sparse.reserve(KeptIndex::nodes(&kept));
        let room = sparse.capacity();
        KeptIndex::fill(&mut sparse, &kept);
        assert_eq!(
            sparse.capacity(),
            room,
            "fallback fills without another allocation"
        );
    }

    #[test]
    fn dense_index_rejects_recycled_generations_and_keeps_the_first_border_item() {
        let (mut scene, ids, inherited) = scene_ids(3);
        let root = ids[0];
        let mut live = ids[1];
        let spare = ids[2];
        let mut kept = items(&[root, live, spare, live], &inherited);
        let mut index = KeptIndex::new(&kept);
        assert!(matches!(index, KeptIndex::Dense(_)));
        let mut history = ids.clone();
        for _ in 0..32 {
            let old = live;
            scene.remove(old);
            live = scene.append(
                root,
                NodeKind::Rect(Color(0, 0, 0, 255)),
                zgx::scene::Style::default(),
            );
            assert_eq!(live.number(), old.number());
            assert_ne!(live, old);
            kept = items(&[root, live, spare, live], &inherited);
            index.patch(&kept, false, vec![old], &[(1, 4, root)]);
            history.push(live);
            agrees(&index, &kept, &history);
            assert_eq!(index.get(live, &kept), Some(1));
        }
    }

    #[test]
    fn stale_dense_offsets_are_checked_after_removal_reorder_and_growth() {
        let (_, ids, inherited) = scene_ids(10);
        let mut kept = items(&ids[..6], &inherited);
        let mut index = KeptIndex::new(&kept);
        for order in [vec![0, 2, 4], vec![4, 0, 2, 6, 8, 9], vec![0], vec![]] {
            let selected: Vec<_> = order.into_iter().map(|at| ids[at]).collect();
            kept = items(&selected, &inherited);
            index.patch(&kept, true, Vec::new(), &[]);
            agrees(&index, &kept, &ids);
        }
    }

    #[test]
    fn sparse_slots_fall_back_and_keep_generation_and_duplicate_semantics() {
        let (_, ids, inherited) = scene_ids(100);
        let small = items(&ids[..4], &inherited);
        let mut index = KeptIndex::new(&small);
        assert!(matches!(index, KeptIndex::Dense(_)));
        let far = items(&[ids[0], ids[99], ids[99]], &inherited);
        index.patch(&far, true, Vec::new(), &[]);
        assert!(matches!(index, KeptIndex::Sparse(_)));
        agrees(&index, &far, &ids);
        let direct = KeptIndex::new(&far);
        assert!(matches!(direct, KeptIndex::Sparse(_)));
        agrees(&direct, &far, &ids);
        let next = items(&[ids[0], ids[98], ids[98]], &inherited);
        index.patch(&next, false, vec![ids[99]], &[(1, 3, ids[0])]);
        agrees(&index, &next, &ids);
        assert_eq!(
            std::mem::size_of::<KeptIndex>(),
            std::mem::size_of::<FxHashMap<NodeId, u32>>()
        );
    }
}

#[cfg(test)]
mod region_index_tests {
    use super::*;

    #[test]
    fn regions_share_an_index_when_their_bits_match() {
        let region = |tint: f32| Region {
            round0: [1., 2., 30., 40.],
            radii0: [4.; 4],
            round1: [0., 0., -1., -1.],
            radii1: [0.; 4],
            fade_box: [0.; 4],
            fade: [0.; 4],
            color: [[0.; 4]; 3],
            tint: [0., 0., 0., tint],
        };
        let mut regions = Vec::new();
        assert_eq!(region_index(&mut regions, None), 0.);
        assert_eq!(region_index(&mut regions, Some(region(0.))), 1.);
        assert_eq!(region_index(&mut regions, Some(region(1.))), 2.);
        // Equal bits, even behind another: the same index.
        assert_eq!(region_index(&mut regions, Some(region(0.))), 1.);
        // The last word differs, or only its zero's sign: another.
        assert_eq!(region_index(&mut regions, Some(region(2.))), 3.);
        assert_eq!(region_index(&mut regions, Some(region(-0.))), 4.);
        // NaN bits equal NaN bits: one entry, which the shader reads alike
        // whichever index it is drawn through.
        assert_eq!(region_index(&mut regions, Some(region(f32::NAN))), 5.);
        assert_eq!(region_index(&mut regions, Some(region(f32::NAN))), 5.);
        // Only the last eight are looked through.
        for tint in 3..11 {
            region_index(&mut regions, Some(region(tint as f32)));
        }
        assert_eq!(region_index(&mut regions, Some(region(0.))), 14.);
    }
}

#[cfg(test)]
mod moved_box_clip_tests {
    use super::*;

    #[test]
    fn a_viewport_clip_prepares_and_scissors_as_before() {
        fn bits(r: Rect) -> [u32; 4] {
            [
                r.x.to_bits(),
                r.y.to_bits(),
                r.width.to_bits(),
                r.height.to_bits(),
            ]
        }
        for scale in [0.1, 0.5, 1., 1.25, 2., 3.1, 30., f32::MAX] {
            for pixels in [(1u32, 1u32), (701, 503), (16384, 8192)] {
                let viewport = Rect::new(0., 0., pixels.0 as f32 / scale, pixels.1 as f32 / scale);
                let clips = [
                    None,
                    Some(viewport),
                    Some(Rect::new(-0., 0., viewport.width, viewport.height)),
                    Some(Rect::new(-13.5, 9.2, 300., 80.1)),
                    Some(Rect::new(500., 10., 2., 3.)),
                    Some(Rect::new(0., 0., -2., 3.)),
                    Some(Rect::new(f32::NAN, 2., 4., 5.)),
                    Some(Rect::new(2., 3., f32::INFINITY, 5.)),
                ];
                let boxes = [
                    Rect::new(0., 0., viewport.width, viewport.height),
                    Rect::new(10., 10., 15., 16.),
                    Rect::new(-1., -1., 3., 4.),
                    Rect::new(500., 0., 20., 20.),
                    Rect::new(0., 0., 0., 0.),
                    Rect::new(f32::NAN, 10., 20., 20.),
                    Rect::new(-f32::INFINITY, 0., f32::INFINITY, 1.),
                ];
                for source in clips {
                    for bounds in boxes {
                        let before =
                            source
                                .unwrap_or(viewport)
                                .intersection(viewport)
                                .filter(|clip| {
                                    viewport.intersects(*clip) && viewport.intersects(bounds)
                                });
                        let now = moved_box_clip(source, viewport, bounds);
                        assert_eq!(now.map(|(clip, _)| bits(clip)), before.map(bits));
                        if let (Some((clip, whole)), Some(old)) = (now, before) {
                            let quad = Quad {
                                rect: [bounds.x, bounds.y, bounds.width, bounds.height],
                                ..bytemuck::Zeroable::zeroed()
                            };
                            let rect = quad_rect(&quad);
                            let limit = snap_out(old, scale);
                            let scissor_before = match rect_inside(rect, limit) {
                                true => viewport,
                                false => old,
                            };
                            let scissor = match whole || rect_inside(rect, limit) {
                                true => viewport,
                                false => clip,
                            };
                            assert_eq!(bits(scissor_before), bits(scissor));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod region_cover_tests {
    use super::*;

    /// draw.wgsl's `rounded`, at logical point `p`.
    fn rounded(b: [f32; 4], r: [f32; 4], feather: f32, p: [f32; 2], scale: f32) -> f32 {
        if b[2] < 0. {
            return 1.;
        }
        let half = [b[2] * 0.5, b[3] * 0.5];
        let q = [p[0] - b[0] - half[0], p[1] - b[1] - half[1]];
        let radius = match (q[1] < 0., q[0] > 0.) {
            (true, false) => r[0],
            (true, true) => r[1],
            (false, true) => r[2],
            (false, false) => r[3],
        };
        let d = [q[0].abs() - half[0] + radius, q[1].abs() - half[1] + radius];
        let sdf = d[0].max(0.).hypot(d[1].max(0.)) + d[0].max(d[1]).min(0.) - radius;
        if feather > 0. {
            let t = (-sdf / feather).clamp(0., 1.);
            return t * t * (3. - 2. * t);
        }
        (0.5 - sdf * scale).clamp(0., 1.)
    }

    fn region(round: [f32; 4], radii: [f32; 4], feather: f32) -> Region {
        Region {
            round0: round,
            radii0: radii,
            round1: [0., 0., -1., -1.],
            radii1: [0.; 4],
            fade_box: [0.; 4],
            fade: [0.; 4],
            color: [[0.; 4]; 3],
            tint: [0., feather, 0., 0.],
        }
    }

    /// The least coverage at the pixel centres a quad on `bounds` draws.
    fn least(region: &Region, bounds: Rect, scale: f32) -> f32 {
        let span = |from: f32, to: f32| {
            ((from * scale - 0.5).ceil() as i32)..((to * scale - 0.5).ceil() as i32)
        };
        let mut least = 1f32;
        for y in span(bounds.y, bounds.y + bounds.height) {
            for x in span(bounds.x, bounds.x + bounds.width) {
                let p = [(x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale];
                least = least.min(rounded(
                    region.round0,
                    region.radii0,
                    region.tint[1],
                    p,
                    scale,
                ));
            }
        }
        least
    }

    #[test]
    fn rows_flush_with_a_cards_straight_edges_leave_its_region() {
        // A card's zebra rows (heavy's), right edges on the card's.
        let card = region([793., 121., 470., 550.], [7.; 4], 0.);
        for y in (131..640).step_by(24) {
            let row = Rect::new(795., y as f32, 468., 20.);
            assert!(card.covers(row, 1.), "row at {y}");
            assert_eq!(least(&card, row, 1.), 1.);
        }
        // Into a corner's square, or a fractional edge's last pixel: kept.
        assert!(!card.covers(Rect::new(795., 123., 468., 24.), 1.));
        assert!(!card.covers(Rect::new(795., 659., 468., 20.), 1.));
        let fractional = region([793., 121.4, 470.3, 550.], [7.; 4], 0.);
        assert!(!fractional.covers(Rect::new(795., 131., 468.3, 20.), 1.));
        assert!(fractional.covers(Rect::new(795., 131., 468., 20.), 1.));
    }

    #[test]
    fn a_covered_quad_draws_every_pixel_at_full_coverage() {
        let mut seed = 0x2545_f491_u32;
        let mut next = |steps: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % steps
        };
        let mut covered = 0;
        for _ in 0..100_000 {
            let scale = [1., 1.25, 1.5, 2.][next(4) as usize];
            let quarter = |n: u32| n as f32 * 0.25;
            let round = [
                quarter(next(80)),
                quarter(next(80)),
                8. + quarter(next(160)),
                8. + quarter(next(160)),
            ];
            let radii = [0, 1, 2, 3].map(|_| [0., 2., 4., 7., 12.][next(5) as usize]);
            let feather = [0., 0., 0., 1.5][next(4) as usize];
            let region = region(round, radii, feather);
            // Bounds near the box's edges, often on them.
            let edge = |at: f32, size: f32, far: bool, n: u32| {
                let base = if far { at + size } else { at };
                match n % 3 {
                    0 => base,
                    _ => base + if far { -1. } else { 1. } * quarter(n % 48),
                }
            };
            let x0 = edge(round[0], round[2], false, next(1000));
            let y0 = edge(round[1], round[3], false, next(1000));
            let x1 = edge(round[0], round[2], true, next(1000));
            let y1 = edge(round[1], round[3], true, next(1000));
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let bounds = Rect::new(x0, y0, x1 - x0, y1 - y0);
            if region.covers(bounds, scale) {
                covered += 1;
                let least = least(&region, bounds, scale);
                assert!(
                    least >= 1. - 1e-5,
                    "{bounds:?} in {round:?} radii {radii:?} feather {feather} at {scale}: {least}"
                );
            }
        }
        assert!(covered > 2_500, "only {covered} covered");
    }
}

#[cfg(test)]
mod atlas_glyph_errors_tests {
    use super::*;

    #[test]
    fn cached_empty_glyphs_and_invalid_misses_keep_error_precedence() {
        let mut gpu = GpuRenderer::new(4, 4).unwrap();
        let key = |glyph: u16, size: f32| {
            CacheKey::new(
                Default::default(),
                glyph,
                size,
                (0., 0.),
                cosmic_text::fontdb::Weight::NORMAL,
                0,
                cosmic_text::CacheKeyFlags::empty(),
            )
            .0
        };
        for glyph in 0..8192 {
            gpu.entries.insert(AtlasKey::from(key(glyph, 12.)), None);
        }
        let mut shaped = Shaped {
            rich: None,
            text_options: Default::default(),
            decorations: Vec::new(),
            font: Default::default(),
            text: Arc::from(""),
            size: 12.,
            width: 4.,
            height: 201.,
            scale: 1.,
            glyphs: [0., 100., 200.]
                .into_iter()
                .map(|y| Glyph {
                    key: key(7, 12.),
                    run: 0,
                    x: 0.,
                    y,
                })
                .collect(),
            quads: Vec::new(),
            colored: false,
            atlas_epoch: 0,
            last_used: 0,
            prepared: None,
        };
        let mut stats = GpuStats::default();
        let before = (gpu.entries.len(), gpu.entries.capacity(), gpu.atlas_epoch);
        // These paths finish before touching the font system.
        let fonts = gpu.fonts.clone();
        let _borrow = fonts.borrow_mut();
        assert!(
            gpu.glyph::<PATCH>(key(7, 12.), &mut stats)
                .unwrap()
                .is_none()
        );
        assert!(
            gpu.glyph::<FRAME>(key(7, 12.), &mut stats)
                .unwrap()
                .is_none()
        );
        assert!(gpu.visible_lines(&shaped, 0., 1., 12., &mut stats).unwrap());
        assert!(gpu.visible_glyphs.is_empty());
        for size in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            MAX_ATLAS as f32 + 1.,
        ] {
            shaped.glyphs[0].key = key(u16::MAX, size);
            let result = gpu.visible_lines(&shaped, 0., 1., 12., &mut stats);
            assert!(result.unwrap(), "the frame goes on");
            assert!(gpu.visible_glyphs.is_empty(), "without it");
            assert_eq!(
                (gpu.entries.len(), gpu.entries.capacity(), gpu.atlas_epoch),
                before
            );
            for patch in [false, true] {
                let result = if patch {
                    gpu.glyph::<PATCH>(key(u16::MAX, size), &mut stats)
                } else {
                    gpu.glyph::<FRAME>(key(u16::MAX, size), &mut stats)
                };
                assert!(result.unwrap().is_none(), "the frame goes on without it");
                assert_eq!(
                    (gpu.entries.len(), gpu.entries.capacity(), gpu.atlas_epoch),
                    before
                );
            }
        }
        // Every finite size up to the limit passes the check, negatives
        // included; the full table's error comes next.
        for size in [-1., -0., 0., 12., MAX_ATLAS as f32] {
            let result = gpu.glyph::<PATCH>(key(u16::MAX, size), &mut stats);
            assert!(result.unwrap().is_none(), "the frame goes on without it");
        }
        assert_eq!(
            (gpu.entries.len(), gpu.entries.capacity(), gpu.atlas_epoch),
            before
        );
        assert_eq!(stats.glyph_uploads, 0);
        assert_eq!(stats.geometry_rebuilds, 0);
    }
}

#[cfg(test)]
mod kept_text_growth_tests {
    use super::*;
    use zgx::scene::{Layout, Style};

    // Both initial positions, repeated room growth and shorter values; the
    // kept ranges are inspected, so the pixels can't pass only because
    // every item was made again.
    #[test]
    fn growing_kept_text_matches_fresh_and_clears_shorter_values() {
        let lengths = [
            12, 12, 12, 12, 12, 12, 30, 50, 80, 130, 200, 320, 80, 12, 380, 20,
        ];
        for scale in [1., 1.5, 2.] {
            for trailing in [false, true] {
                let width = (320. * scale) as u32;
                let height = (200. * scale) as u32;
                let mut gpu = GpuRenderer::new(width, height).unwrap();
                gpu.set_scale_factor(scale);
                let mut scene = Scene::new(320., 200.);
                scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
                let fixed = |w, h| Style {
                    width: Some(w),
                    height: Some(h),
                    ..Default::default()
                };
                scene.append(
                    scene.root(),
                    NodeKind::Rect(Color(10, 20, 30, 255)),
                    fixed(320., 200.),
                );
                let text = scene.append(
                    scene.root(),
                    NodeKind::Text {
                        text: "".into(),
                        font_size: 12.,
                        color: Color(220, 230, 240, 255),
                    },
                    fixed(320., 200.),
                );
                if trailing {
                    scene.append(
                        scene.root(),
                        NodeKind::Rect(Color(120, 25, 90, 140)),
                        fixed(80., 20.),
                    );
                }
                let (mut tails, mut moves, mut shrinks) = (0, 0, 0);
                for (step, &length) in lengths.iter().enumerate() {
                    let value: String = (0..length)
                        .map(|at| match at % 8 {
                            7 => ' ',
                            _ => char::from(b'a' + ((at + step) % 20) as u8),
                        })
                        .collect();
                    let before = gpu.kept.as_ref().and_then(|kept| {
                        kept.items
                            .iter()
                            .find(|item| item.id == text)
                            .map(|item| (item.start, item.len, item.room, kept.quads.len()))
                    });
                    scene.set_kind(
                        text,
                        NodeKind::Text {
                            text: value.into(),
                            font_size: 12.,
                            color: Color(220, 230, 240, 255),
                        },
                    );
                    scene.flush();
                    let full = [Rect::new(0., 0., 320., 200.)];
                    let stats = gpu.render(&scene, &full).unwrap();
                    if let (Some((start, len, room, total)), Some(kept)) = (before, &gpu.kept) {
                        let item = kept.items.iter().find(|item| item.id == text).unwrap();
                        if stats.patched_items > 0 && item.room > room {
                            if start as usize + room as usize == total {
                                tails += 1;
                                assert_eq!(item.start, start, "a text at the tail grows in place");
                                assert_eq!(kept.quads.len(), start as usize + item.room as usize);
                            } else {
                                moves += 1;
                                assert_eq!(item.start as usize, total, "another moves to the end");
                            }
                        }
                        if stats.patched_items > 0 && item.len < len {
                            shrinks += 1;
                            let spare = &kept.quads[(item.start + item.len) as usize
                                ..(item.start + item.room) as usize];
                            let bytes: &[u8] = bytemuck::cast_slice(spare);
                            assert!(
                                bytes.iter().all(|&byte| byte == 0),
                                "no glyphs left in its spare room"
                            );
                        }
                    }
                    let actual = gpu.readback().unwrap();
                    let mut fresh =
                        GpuRenderer::new_with_context(width, height, &gpu.context()).unwrap();
                    fresh.set_scale_factor(scale);
                    fresh.render(&scene, &full).unwrap();
                    assert!(
                        actual == fresh.readback().unwrap(),
                        "scale {scale}, trailing {trailing}, step {step}"
                    );
                }
                assert!(tails >= 2, "repeated growth at the tail, got {tails}");
                assert!(shrinks >= 2, "shorter values, got {shrinks}");
                if trailing {
                    assert!(moves >= 1, "a text before another moves");
                }
            }
        }
    }
}

#[cfg(test)]
mod blend_backdrop_tests {
    use super::*;
    use zgx::scene::{BlendMode, Layout, Style};

    #[test]
    fn a_resized_window_keeps_no_backdrop_for_its_former_sizes() {
        let mut gpu = GpuRenderer::new(96, 80).unwrap();
        let mut scene = Scene::new(96., 80.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        scene.append(
            scene.root(),
            NodeKind::Rect(Color(40, 90, 200, 255)),
            Style {
                width: Some(96.),
                height: Some(80.),
                ..Style::default()
            },
        );
        let layer = scene.append(
            scene.root(),
            NodeKind::Rect(Color(230, 102, 26, 255)),
            Style {
                width: Some(40.),
                height: Some(40.),
                ..Style::default()
            },
        );
        scene.set_blend(layer, BlendMode::Multiply);
        for delta in 0..32u32 {
            let (width, height) = (96 + delta, 80 + delta);
            gpu.resize(width, height);
            scene.resize(width as f32, height as f32);
            let report = scene.flush();
            gpu.render(&scene, &report.damage).unwrap();
            let pixels = gpu.readback().unwrap();
            assert!(
                gpu.backdrops.len() <= 1 + gpu.layers.len(),
                "{} backdrops held at {width}×{height}",
                gpu.backdrops.len()
            );
            // Still blended as a renderer new at this size blends.
            let mut fresh = GpuRenderer::new_with_context(width, height, &gpu.context()).unwrap();
            fresh
                .render(&scene, &[Rect::new(0., 0., width as f32, height as f32)])
                .unwrap();
            assert!(pixels == fresh.readback().unwrap(), "{width}×{height}");
        }
        gpu.resize(96, 80);
        scene.resize(96., 80.);
        scene.remove(layer);
        let report = scene.flush();
        gpu.render(&scene, &report.damage).unwrap();
        gpu.readback().unwrap();
        gpu.trim();
        assert!(
            gpu.backdrops.is_empty(),
            "{} backdrops after trim",
            gpu.backdrops.len()
        );
    }
}

#[cfg(test)]
mod highlight_shape_tests {
    use super::*;
    use zgx::scene::{Color, Layout, Style, TextHighlight, Transform};

    /// A translucent selection over lines of different lengths, placed off
    /// the pixel grid: `(pixels, width in pixels)`.
    fn draw(scale: f32) -> (Vec<u8>, usize) {
        let (width, height) = (80., 60.);
        let pixels = (
            (width * scale).ceil() as u32,
            (height * scale).ceil() as u32,
        );
        let mut gpu = GpuRenderer::new(pixels.0, pixels.1).unwrap();
        gpu.set_scale_factor(scale);
        let mut scene = Scene::new(width, height);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let size = Style {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        };
        scene.append(
            scene.root(),
            NodeKind::Rect(Color(0, 0, 0, 255)),
            size.clone(),
        );
        let text = NodeKind::Text {
            text: "".into(),
            color: Color(0, 0, 0, 0),
            font_size: 1.,
        };
        let node = scene.append(scene.root(), text, size);
        scene.set_transform(node, Transform { x: 3.3, y: 3.3 });
        scene.flush();
        let highlight = TextHighlight {
            color: Color(255, 255, 255, 96),
            rects: vec![
                Rect::new(20., 0., 40., 13.3),
                Rect::new(0., 13.3, 60., 13.3),
                Rect::new(0., 26.6, 30., 13.3),
            ],
            text_color: None,
            radius: 3.,
        };
        scene.set_text_highlight(node, Some(highlight));
        scene.flush();
        gpu.render(&scene, &[]).unwrap();
        (gpu.readback().unwrap(), pixels.0 as usize)
    }

    /// One colour from line to line at any scale: no seam where they meet,
    /// no darker band where they would overlap.
    #[test]
    fn a_translucent_selection_is_even_across_its_lines() {
        for scale in [1., 1.25, 1.5, 2.] {
            let (pixels, width) = draw(scale);
            let at = |x: f32, y: f32| {
                let (x, y) = ((x * scale) as usize, (y * scale) as usize);
                pixels[(y * width + x) * 4]
            };
            let full = at(10., 20.);
            assert!(full > 20, "the selection shows at {scale}");
            let column = 28.;
            let mut y = 6.;
            while y < 40. {
                assert_eq!(at(column, y), full, "at {y}, scale {scale}");
                y += 1. / scale;
            }
        }
    }

    /// The inner corner where a line meets a longer one curves: filled at
    /// the corner itself, and not at the far side of the curve.
    #[test]
    fn an_inner_corner_curves() {
        let (pixels, width) = draw(2.);
        let at = |x: f32, y: f32| pixels[((y * 2.) as usize * width + (x * 2.) as usize) * 4];
        let full = at(10., 20.);
        // The first line starts at 23.5 and meets the second at 16.5, on
        // whole pixels: the curve is 3 around (20.5, 13.5).
        assert_eq!(at(23.1, 16.2), full, "in the corner");
        assert_eq!(at(20.6, 13.6), 0, "past the curve");
    }
}

#[cfg(test)]
mod shader_compile_tests {
    use super::*;
    use zgx::decoration::{Background, Decoration, FillShader};
    use zgx::scene::{Layout, QuadStyle, Style};

    /// A 40-point box filled with `shader` over a red background colour.
    fn filled(scene: &mut Scene, shader: &Arc<FillShader>) {
        let kind = NodeKind::Quad(QuadStyle {
            fill: Color(255, 0, 0, 255),
            decoration: Some(Arc::new(Decoration {
                background: Some(Background::Shader(shader.clone())),
                ..Default::default()
            })),
            ..Default::default()
        });
        let style = Style {
            width: Some(40.),
            height: Some(40.),
            ..Style::default()
        };
        scene.append(scene.root(), kind, style);
    }
    fn green() -> Arc<FillShader> {
        FillShader::new(
            "fn fill(input: FillInput) -> vec4<f32> { return vec4<f32>(0.0, 1.0, 0.0, 1.0); }",
            None,
        )
    }
    /// The box's middle pixel after a frame, and whether the frame went
    /// without something still on its way.
    fn frame(gpu: &mut GpuRenderer, scene: &mut Scene) -> ([u8; 4], bool) {
        let report = scene.flush();
        gpu.render(scene, &report.damage).unwrap();
        let pixels = gpu.readback().unwrap();
        let at = (20 * 64 + 20) * 4;
        (pixels[at..at + 4].try_into().unwrap(), gpu.images_awaited)
    }
    fn compiled(gpu: &GpuRenderer, shader: &FillShader) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            if gpu
                .fill_pipelines
                .get(&shader.id())
                .is_some_and(|c| c.get().is_some())
            {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }
    fn renderer() -> (GpuRenderer, Scene) {
        let mut gpu = GpuRenderer::new(64, 64).unwrap();
        // As a window's frames: they never wait for a compile.
        gpu.offscreen_waits = false;
        let mut scene = Scene::new(64., 64.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        (gpu, scene)
    }

    /// The frame that first meets a shader fill doesn't wait for its
    /// pipeline: it draws the box's colour, says it went without, and the
    /// fill draws once compiled.
    #[test]
    fn a_window_draws_on_while_a_fill_compiles() {
        let (mut gpu, mut scene) = renderer();
        let shader = green();
        filled(&mut scene, &shader);
        let (pixel, awaited) = frame(&mut gpu, &mut scene);
        assert_eq!(pixel, [255, 0, 0, 255], "the box's colour meanwhile");
        assert!(awaited, "a frame went without it");
        assert!(compiled(&gpu, &shader));
        // The host, woken, draws again (`images_decoded`).
        assert!(gpu.images_decoded());
        let (pixel, awaited) = frame(&mut gpu, &mut scene);
        assert_eq!(pixel, [0, 255, 0, 255]);
        assert!(!awaited);
    }

    /// A fill a script declared (`warm_fill_shader`) compiles from the next
    /// frame on, before anything draws it: the first frame that does has it.
    #[test]
    fn a_declared_fill_is_compiled_before_it_draws() {
        let (mut gpu, mut scene) = renderer();
        let shader = green();
        warm_fill_shader(&shader);
        frame(&mut gpu, &mut scene);
        assert!(compiled(&gpu, &shader));
        filled(&mut scene, &shader);
        let (pixel, awaited) = frame(&mut gpu, &mut scene);
        assert_eq!(pixel, [0, 255, 0, 255]);
        assert!(!awaited);
    }

    /// Offscreen, where frames wait for their shaders anyway, a fill
    /// compiles on the renderer's own thread: compiled as it is started,
    /// with no second thread allocating beside the frame (which made the
    /// gate's fx cells count differently run to run).
    #[test]
    fn offscreen_a_fill_is_compiled_as_it_starts() {
        let (mut gpu, _) = renderer();
        gpu.offscreen_waits = true;
        let shader = green();
        gpu.start_fill(&shader);
        assert!(
            gpu.fill_pipelines
                .get(&shader.id())
                .is_some_and(|c| c.get().is_some()),
            "compiled when start_fill returned"
        );
    }

    /// Offscreen, nothing wakes a host: a frame waits for the compile.
    #[test]
    fn offscreen_a_frame_waits_for_its_fill() {
        let (mut gpu, mut scene) = renderer();
        gpu.offscreen_waits = true;
        let shader = green();
        filled(&mut scene, &shader);
        assert_eq!(frame(&mut gpu, &mut scene), ([0, 255, 0, 255], false));
    }
}

#[cfg(test)]
mod shader_retention_tests {
    use super::*;
    use zgx::decoration::{Background, Decoration, FillShader};
    use zgx::scene::{BlendMode, Layout, QuadStyle, Style};

    fn sized(side: f32) -> Style {
        Style {
            width: Some(side),
            height: Some(side),
            ..Style::default()
        }
    }
    fn frame(gpu: &mut GpuRenderer, scene: &mut Scene) {
        let report = scene.flush();
        gpu.render(scene, &report.damage).unwrap();
        gpu.readback().unwrap();
    }

    #[test]
    fn removed_shaders_go_at_trim_and_held_ones_stay() {
        let mut gpu = GpuRenderer::new(64, 64).unwrap();
        let mut scene = Scene::new(64., 64.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        let fill = |i: u32| {
            FillShader::new(
                format!(
                    "fn fill(input: FillInput) -> vec4<f32> {{ return vec4<f32>(0.{i}, 0.3, 0.5, 1.0); }}"
                ),
                None,
            )
        };
        let filled = |shader: &Arc<FillShader>| {
            NodeKind::Quad(QuadStyle {
                decoration: Some(Arc::new(Decoration {
                    background: Some(Background::Shader(shader.clone())),
                    ..Default::default()
                })),
                ..Default::default()
            })
        };
        // One shader the scene keeps all along.
        let held = fill(9);
        scene.append(scene.root(), filled(&held), sized(10.));
        let (mut gone, mut gone_fills) = (Vec::new(), Vec::new());
        for i in 0..4 {
            // A shader fill, and a layer shader blended over the backdrop.
            let shader = fill(i);
            gone_fills.push(shader.id());
            gone.push(Arc::downgrade(&shader));
            let node = scene.append(scene.root(), filled(&shader), sized(40.));
            let layer = scene.append(
                scene.root(),
                NodeKind::Container(Layout::Overlay),
                sized(30.),
            );
            scene.append(layer, NodeKind::Rect(Color(200, 120, 40, 255)), sized(30.));
            let invert = FillShader::new(
                "fn layer(input: LayerInput) -> vec4<f32> { return vec4(1.0 - input.color.rgb, input.color.a); }",
                None,
            );
            gone.push(Arc::downgrade(&invert));
            scene.set_layer_shader(layer, Some(invert));
            scene.set_blend(layer, BlendMode::Multiply);
            frame(&mut gpu, &mut scene);
            scene.remove(node);
            scene.remove(layer);
            frame(&mut gpu, &mut scene);
            gpu.trim();
        }
        // The scene lets go of removed nodes over the frames after (lazy
        // death): one more, then the renderer lets go at its trim.
        frame(&mut gpu, &mut scene);
        gpu.trim();
        let alive = gone
            .iter()
            .filter(|shader| shader.upgrade().is_some())
            .count();
        assert_eq!(alive, 0, "removed shaders kept alive");
        // (Of this test's: fills other tests declare compile here too.)
        assert!(
            gpu.fill_pipelines.contains_key(&held.id())
                && !gone_fills
                    .iter()
                    .any(|id| gpu.fill_pipelines.contains_key(id)),
            "the held fill's pipeline alone"
        );
        assert!(gpu.fill_shaders.contains_key(&held.id()));
        assert!(gpu.filter_pipelines.is_empty() && gpu.filter_shaders.is_empty());
        assert!(gpu.blend_pipelines.keys().all(|&id| id == BLEND_PLAIN));
        // The held shader still draws after the trim, as a fresh renderer
        // draws it.
        frame(&mut gpu, &mut scene);
        let mut fresh = GpuRenderer::new_with_context(64, 64, &gpu.context()).unwrap();
        fresh
            .render(&scene, &[Rect::new(0., 0., 64., 64.)])
            .unwrap();
        gpu.render_uncounted(&scene, &[Rect::new(0., 0., 64., 64.)])
            .unwrap();
        assert!(gpu.readback().unwrap() == fresh.readback().unwrap());
    }
}

#[cfg(test)]
mod thread_safety_tests {
    fn send_sync<T: Send + Sync>() {}

    /// What could move between threads, or be shared by them, still can: a
    /// type that stops being `Send` or `Sync` breaks the programs that move
    /// it, which a patch release may not (word shapes are shared by `Arc`).
    #[test]
    fn its_thread_safe_types_stay_send_and_sync() {
        send_sync::<crate::prepared::TextCache>();
        send_sync::<crate::prepared::Laid<'static>>();
        #[cfg(target_os = "macos")]
        send_sync::<crate::Caret>();
        send_sync::<crate::GpuError>();
        send_sync::<crate::GpuStats>();
        send_sync::<crate::DebugCacheStats>();
        send_sync::<crate::ShaderCacheCounts>();
        send_sync::<crate::PendingGpuContext>();
        send_sync::<crate::PresentationStatus>();
        send_sync::<crate::text::FontData>();
        send_sync::<crate::text::TextCursor>();
        send_sync::<crate::text::ShapedText>();
        send_sync::<crate::text::CpuTextRaster>();
        send_sync::<crate::text::RichDecoration>();
        send_sync::<crate::canvas::Raster>();
        send_sync::<crate::canvas::CanvasCache>();
        send_sync::<crate::canvas::HeadlessCanvases>();
        send_sync::<crate::svg::SvgCache>();
        send_sync::<crate::viewport::ViewportFrame<'static>>();
    }
}

#[cfg(test)]
mod rest_tests {
    use super::*;
    use zgx::scene::{Color, Layout, QuadStyle, Style, Transform};

    fn fixed(width: f32, height: f32) -> Style {
        Style {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        }
    }
    /// A scene of `count` small boxes: one frame of that many quads.
    fn boxes(count: usize) -> Scene {
        let mut scene = Scene::new(400., 300.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        for index in 0..count {
            let node = scene.append(
                scene.root(),
                NodeKind::Rect(Color(40, (index % 200) as u8, 90, 255)),
                fixed(3., 3.),
            );
            let (x, y) = ((index % 130) as f32 * 3., (index / 130 % 100) as f32 * 3.);
            scene.set_transform(node, Transform { x, y });
        }
        scene
    }
    fn draw(gpu: &mut GpuRenderer, scene: &mut Scene) {
        let report = scene.flush();
        gpu.render(scene, &report.damage).unwrap();
    }

    /// What wgpu holds now: textures, samplers and bind groups, as its HAL
    /// counts them (each made, less each freed).
    fn held(gpu: &GpuRenderer) -> (isize, isize, isize) {
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        let counters = gpu.device.get_internal_counters();
        (
            counters.hal.textures.read(),
            counters.hal.samplers.read(),
            counters.hal.bind_groups.read(),
        )
    }

    /// A page pushed and popped (a navigation stack's) shows an image, the
    /// same one each time as a symbol is, or a new one, or a gradient along
    /// one axis (the soft edge of a page coming over another), then leaves
    /// it: the textures, samplers and bind groups that took come back to
    /// where they were, and the same picture isn't uploaded again each time.
    #[test]
    fn a_page_pushed_and_popped_again_and_again_gives_its_image_textures_back() {
        use zgx::decoration::{Background, Decoration};
        let stops: Arc<[zgx::canvas::GradientStop]> = Arc::from(vec![
            zgx::canvas::GradientStop {
                offset: 0.,
                color: Color(0, 0, 0, 24),
            },
            zgx::canvas::GradientStop {
                offset: 1.,
                color: Color(0, 0, 0, 0),
            },
        ]);
        let symbol = Arc::new(zgx::image::ImageData::new(24, 24, vec![200; 24 * 24 * 4]).unwrap());
        for what in ["symbol", "fresh", "gradient"] {
            let mut gpu = GpuRenderer::new(400, 300).unwrap();
            let mut scene = boxes(10);
            let mut uploads = 0;
            let push = |gpu: &mut GpuRenderer, scene: &mut Scene, cycle: usize| {
                let kind = match what {
                    "symbol" => NodeKind::Image(symbol.clone()),
                    "fresh" => NodeKind::Image(Arc::new(
                        zgx::image::ImageData::new(24, 24, vec![(cycle % 250) as u8; 24 * 24 * 4])
                            .unwrap(),
                    )),
                    _ => NodeKind::Quad(QuadStyle {
                        decoration: Some(Arc::new(Decoration {
                            background: Some(Background::Linear {
                                angle: std::f32::consts::PI,
                                stops: stops.clone(),
                            }),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                };
                let page = scene.append(scene.root(), kind, fixed(9., 300.));
                let report = scene.flush();
                let stats = gpu.render(scene, &report.damage).unwrap();
                scene.remove(page);
                draw(gpu, scene);
                stats.image_uploads
            };
            for cycle in 0..10 {
                push(&mut gpu, &mut scene, cycle);
            }
            let warm = held(&gpu);
            // Past the textures kept of images no node shows (256): what it
            // keeps must stop somewhere, and give all back at rest.
            for cycle in 10..400 {
                uploads += push(&mut gpu, &mut scene, cycle);
            }
            let after = held(&gpu);
            let kept = if what == "fresh" {
                IMAGES_GONE_COUNT as isize
            } else {
                0
            };
            assert!(
                after.0 <= warm.0 + kept && after.1 <= warm.1 + kept && after.2 <= warm.2 + kept,
                "{what}: textures, samplers, bind groups {warm:?} warm, {after:?} after 390 pushes"
            );
            if what != "fresh" {
                assert_eq!(
                    uploads, 0,
                    "{what}: the same picture uploaded again on pushes"
                );
            }
            gpu.trim();
            let rest = held(&gpu);
            assert!(
                rest.0 <= warm.0 && rest.1 <= warm.1 && rest.2 <= warm.2,
                "{what}: {rest:?} at rest, {warm:?} warm"
            );
            // No box shows a gradient now: at rest no ramp is kept, pixels
            // included.
            assert!(
                gpu.ramps.is_empty(),
                "{what}: {} ramps kept at rest",
                gpu.ramps.len()
            );
        }
    }

    /// A window that drew one big frame (a long list scrolled, a big page)
    /// and went back to a small one gives its vertex memory back at rest, as
    /// its other caches: on either upload path.
    #[test]
    fn vertex_memory_returns_to_a_small_frames_at_rest() {
        for ring in [true, false] {
            let mut gpu = GpuRenderer::new(400, 300).unwrap();
            if !ring {
                gpu.mapped = None;
            }
            let mut small = boxes(20);
            draw(&mut gpu, &mut small);
            gpu.trim();
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            gpu.trim();
            let baseline = gpu.debug_cache_stats().vertex_buffer_bytes;
            let mut big = boxes(40_000);
            draw(&mut gpu, &mut big);
            let peak = gpu.debug_cache_stats().vertex_buffer_bytes;
            assert!(
                peak > 4 * 1024 * 1024,
                "ring {ring}: {peak} bytes for the big frame"
            );
            // Back to the small page, then at rest.
            let mut small = boxes(20);
            draw(&mut gpu, &mut small);
            let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
            gpu.trim();
            let rest = gpu.debug_cache_stats().vertex_buffer_bytes;
            assert!(
                rest <= baseline.max(4096),
                "ring {ring}: {rest} bytes at rest, {baseline} before"
            );
            // And the next frame draws.
            draw(&mut gpu, &mut small);
        }
    }
    /// A big page: `count` rows of text in many sizes, pictures, layers with
    /// an opacity, and a layer shader. Every renderer cache fills.
    fn big_page(count: usize) -> Scene {
        let mut scene = Scene::new(400., 300.);
        scene.set_kind(scene.root(), NodeKind::Container(Layout::Overlay));
        for index in 0..count {
            let y = (index % 60) as f32 * 5.;
            let text = scene.append(
                scene.root(),
                NodeKind::Text {
                    text: format!(
                        "Row {index}: {}",
                        "the quick brown fox jumps ".repeat(1 + index % 3)
                    )
                    .into(),
                    color: Color(220, 220, 230, 255),
                    font_size: 9. + (index % 23) as f32,
                },
                Style::default(),
            );
            scene.set_transform(text, Transform { x: 0., y });
            if index % 4 == 0 {
                let pixels: Vec<u8> = (0..32 * 32)
                    .flat_map(|p| [(p % 255) as u8, index as u8, 90, 255])
                    .collect();
                let image = Arc::new(zgx::image::ImageData::new(32, 32, pixels).unwrap());
                let node = scene.append(scene.root(), NodeKind::Image(image), fixed(16., 16.));
                scene.set_transform(
                    node,
                    Transform {
                        x: (index % 20) as f32 * 18.,
                        y,
                    },
                );
            }
            if index % 10 == 0 {
                let layer = scene.append(
                    scene.root(),
                    NodeKind::Container(Layout::Overlay),
                    fixed(60., 30.),
                );
                scene.set_effects(
                    layer,
                    zgx::scene::Effects {
                        opacity: 0.8,
                        ..Default::default()
                    },
                );
                scene.append(
                    layer,
                    NodeKind::Rect(Color(90, 40, 200, 255)),
                    fixed(60., 30.),
                );
                scene.set_transform(
                    layer,
                    Transform {
                        x: (index % 6) as f32 * 60.,
                        y,
                    },
                );
                if index % 20 == 0 {
                    scene.set_layer_shader(
                        layer,
                        Some(zgx::decoration::FillShader::new(
                            format!("fn layer(input: LayerInput) -> vec4<f32> {{ return input.color * {}.0 / 100.0; }}", 50 + index % 40),
                            None,
                        )),
                    );
                }
            }
        }
        scene
    }
    /// Its caches at rest. Its shader pipelines count only those whose
    /// shader the renderer alone holds: a fill another test declares
    /// meanwhile (`compile::warm_fill_shader`, process-wide) every renderer
    /// starts, and keeps while that test holds it.
    fn rest(gpu: &mut GpuRenderer) -> DebugCacheStats {
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        gpu.trim();
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        gpu.trim();
        let held_elsewhere = gpu
            .fill_shaders
            .values()
            .chain(gpu.filter_shaders.values())
            .filter(|shader| Arc::strong_count(shader) > 1 && compile::is_declared(shader))
            .count();
        let mut stats = gpu.debug_cache_stats();
        stats.shader_pipelines -= held_elsewhere;
        stats
    }

    /// After one big page, a window back on a small one and at rest holds
    /// what it held for the small one alone: every cache, the renderer made
    /// as the adapter has it (CI runs it on each platform's). The glyph atlas
    /// keeps its size until a long idle (`trim_atlas`, which the host calls
    /// then), and gives it back there.
    #[test]
    fn every_cache_returns_to_a_small_frames_at_rest() {
        let mut gpu = GpuRenderer::new(400, 300).unwrap();
        gpu.debug_wait_for_shaders();
        let mut small = boxes(20);
        draw(&mut gpu, &mut small);
        let baseline = rest(&mut gpu);
        let mut big = big_page(600);
        // A fill declared elsewhere in the process meanwhile (a script's,
        // or another test's): every renderer starts it, and keeps it while
        // its declarer holds it, which isn't this renderer's to give back.
        let declared = zgx::decoration::FillShader::new(
            "fn fill(input: FillInput) -> vec4<f32> { return vec4<f32>(0.2, 0.4, 0.6, 1.0); }",
            None,
        );
        compile::warm_fill_shader(&declared);
        draw(&mut gpu, &mut big);
        draw(&mut gpu, &mut big);
        let peak = gpu.debug_cache_stats();
        // The big page gone (its nodes, its shaders), the small one again.
        drop(big);
        let mut small = boxes(20);
        draw(&mut gpu, &mut small);
        let at_rest = rest(&mut gpu);
        // The big page did fill every cache.
        assert!(peak.shaped_nodes > 0 && peak.atlas_entries > 0, "{peak:?}");
        assert!(
            peak.image_textures > 0 && peak.layer_textures > 0,
            "{peak:?}"
        );
        assert!(
            peak.shader_pipelines > 0 && peak.vertex_buffer_bytes > 1 << 20,
            "{peak:?}"
        );
        let held = |stats: &DebugCacheStats| {
            [
                stats.shaped_nodes,
                stats.shaped_bytes,
                stats.cached_quads,
                stats.atlas_entries,
                stats.swash_images,
                stats.swash_outlines,
                stats.image_textures,
                stats.image_bytes,
                stats.canvas_raster_bytes,
                stats.svg_raster_bytes,
                stats.layer_textures,
                stats.layer_bytes,
                stats.shader_pipelines,
            ]
        };
        // Which pipelines, if any stay: each shader's id, who holds it and
        // its source's start.
        let kept = |gpu: &GpuRenderer| {
            let shader = |id: &u64| {
                let held = gpu
                    .fill_shaders
                    .get(id)
                    .or_else(|| gpu.filter_shaders.get(id));
                held.map(|s| {
                    (
                        Arc::strong_count(s),
                        s.source().chars().take(60).collect::<String>(),
                    )
                })
            };
            let fills: Vec<_> = gpu
                .fill_pipelines
                .keys()
                .map(|id| (*id, shader(id)))
                .collect();
            let filters: Vec<_> = gpu
                .filter_pipelines
                .keys()
                .map(|id| (*id, shader(id)))
                .collect();
            let blends: Vec<_> = gpu
                .blend_pipelines
                .keys()
                .map(|id| (*id, shader(id)))
                .collect();
            format!("fills {fills:?}, filters {filters:?}, blends {blends:?}")
        };
        assert_eq!(
            held(&at_rest),
            held(&baseline),
            "{at_rest:?}; {}",
            kept(&gpu)
        );
        assert!(
            at_rest.vertex_buffer_bytes <= baseline.vertex_buffer_bytes.max(4096),
            "{at_rest:?}"
        );
        // A long idle: the atlas too, back to its first size (a page of text
        // grows it to that at once).
        gpu.trim_atlas();
        let first = (INITIAL_ATLAS * INITIAL_ATLAS * 4) as usize;
        assert!(gpu.debug_cache_stats().atlas_bytes <= first.max(baseline.atlas_bytes));
        // And the next frame draws.
        draw(&mut gpu, &mut small);
    }

    /// A shrunk atlas's packing: every cell inside it with its gutter, none
    /// overlapping, in the smallest size that holds them; none when only
    /// the size it has does.
    #[test]
    fn shrunk_atlas_cells_fit_and_never_overlap() {
        let cells: Vec<(u32, u32)> = (0..300).map(|i| (8 + i % 23, 10 + i % 17)).collect();
        let Packed {
            size,
            origins,
            cursor,
        } = pack_cells(&cells, MAX_ATLAS).unwrap();
        assert_eq!(size, INITIAL_ATLAS);
        let boxes: Vec<(u32, u32, u32, u32)> = origins
            .iter()
            .zip(&cells)
            .map(|((x, y), (w, h))| (x - 1, y - 1, x + w + 1, y + h + 1))
            .collect();
        for (i, a) in boxes.iter().enumerate() {
            assert!(a.2 <= size && a.3 <= size, "inside: {a:?}");
            assert!(
                a.3 <= cursor.1 + cursor.2,
                "under the cursor's shelf: {a:?}"
            );
            for b in &boxes[i + 1..] {
                assert!(
                    a.2 <= b.0 || b.2 <= a.0 || a.3 <= b.1 || b.3 <= a.1,
                    "{a:?} {b:?}"
                );
            }
        }
        // Too many for the first size: the next.
        let many: Vec<(u32, u32)> = vec![(40, 40); 400];
        assert_eq!(
            pack_cells(&many, MAX_ATLAS).unwrap().size,
            INITIAL_ATLAS * 2
        );
        // Only the size it has holds them: nothing smaller.
        assert!(pack_cells(&many, INITIAL_ATLAS * 2).is_none());
    }
}

#[cfg(test)]
mod premultiply_tests {
    fn exact(pixels: &[u8]) -> Vec<u8> {
        let mut out = pixels.to_vec();
        for pixel in out.as_chunks_mut::<4>().0 {
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        out
    }

    #[test]
    fn premultiplying_is_exact_for_every_colour_and_alpha() {
        // Every colour at every alpha, in blocks that mix them.
        let mut pixels: Vec<u8> = (0..=255u8)
            .flat_map(|alpha| (0..=255u8).flat_map(move |c| [c, 255 - c, c / 2, alpha]))
            .collect();
        let expected = exact(&pixels);
        super::premultiply(&mut pixels);
        assert!(pixels == expected, "differs from the division");
    }

    #[test]
    fn opaque_and_transparent_blocks_premultiply_as_any_other() {
        // Sixteen opaque pixels, sixteen transparent with colour, sixteen of
        // one alpha in between, and a short block left over: each kind of
        // block, and the tail.
        let mut pixels = Vec::new();
        for i in 0..16u8 {
            pixels.extend([i * 13, 200 - i, 7, 255]);
        }
        for i in 0..16u8 {
            pixels.extend([i * 13, 200 - i, 7, 0]);
        }
        for i in 0..16u8 {
            pixels.extend([i * 13, 200 - i, 7, 255 - i]);
        }
        for i in 0..5u8 {
            pixels.extend([i * 40, 9, 250, if i % 2 == 0 { 0 } else { 255 }]);
        }
        let expected = exact(&pixels);
        super::premultiply(&mut pixels);
        assert!(pixels == expected, "a block differs from the division");
    }
}
#[cfg(test)]
mod copy_refusal_tests {
    use super::*;
    use zgx::{compose::prelude::*, widgets::Ui};

    /// A view holding two stripes, wrapped by `around`, moved once: what
    /// `copy_blocker` makes of it after `prepare` has seen its content.
    fn refusal(
        around: impl FnOnce(View) -> View,
        prepare: impl FnOnce(&mut Scene, NodeId),
    ) -> Option<(CopyBlocker, String)> {
        let mut ui = Ui::new(240., 180.);
        let offset = ui.signal(0_f32);
        let page = column()
            .w(240.)
            .child(div().w(240.).h(400.).bg(rgb(0x5a7a9a)))
            .child(div().w(240.).h(400.).bg(rgb(0x9a5a7a)));
        let view = scroll(offset.clone())
            .w(240.)
            .h(180.)
            .bg(rgb(0x101820))
            .child(page);
        ui.mount(around(view));
        let step = |ui: &mut Ui| {
            ui.prepare_frame();
            ui.scene.borrow_mut().flush().damage
        };
        step(&mut ui);
        offset.set(8.);
        step(&mut ui);
        let content = ui.scene.borrow().scroll_moves()[0].node;
        prepare(&mut ui.scene.borrow_mut(), content);
        offset.set(16.);
        let damage = step(&mut ui);
        let scene = ui.scene.borrow();
        let moved = scene.scroll_moves()[0].clone();
        copy_blocker(&scene, &moved, &damage, 1., &mut Vec::new(), &mut None).map(|blocker| {
            let note = blocker.describe(&scene);
            (blocker, note)
        })
    }

    #[test]
    fn a_plain_view_has_no_refusal() {
        assert_eq!(refusal(|view| view, |_, _| {}), None);
    }

    #[test]
    fn a_view_under_two_fades_says_so() {
        let (blocker, note) = refusal(
            |view| {
                div()
                    .w(240.)
                    .h(180.)
                    .overflow_hidden()
                    .fade_edges(10., 10.)
                    .child(view.fade_edges(20., 20.))
            },
            |_, _| {},
        )
        .expect("refused");
        assert_eq!(blocker, CopyBlocker::TwoFades);
        assert!(note.contains("two fades"), "{note}");
    }

    /// A loop in the content is no refusal (`baked_loops.rs` has it scrolling).
    #[cfg(target_os = "macos")]
    #[test]
    fn a_loop_in_the_content_is_no_refusal() {
        let refused = refusal(
            |view| view,
            |scene, content| {
                let node = scene.children(content)[0];
                let baked =
                    zgx::scene::BakedLoop::new(vec![(1., 0.), (0.5, 0.)], 30., Box::new(|| {}));
                scene.set_baked_loop(node, Some(std::rc::Rc::new(baked)));
            },
        );
        assert_eq!(refused, None);
    }
}
