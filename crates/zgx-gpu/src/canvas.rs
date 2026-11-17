//! Shared bounded vector raster cache. Rasterization only follows changed content,
//! allocated size or device scale; GPU upload uses ordinary retained image identity.
use crate::GpuError;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};
use tiny_skia as sk;
use zgx::{
    canvas::*,
    image::ImageData,
    scene::{NodeId, NodeKind, Scene},
};
const BUDGET: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 1024;
struct Entry {
    canvas: Weak<Canvas>,
    decoration: Option<zgx::scene::QuadStyle>,
    width: f32,
    height: f32,
    scale: f32,
    /// The canvas's view it was drawn under (`Scene::canvas_view`).
    view: zgx::scene::CanvasView,
    /// The raster's image id, which its GPU texture is keyed by.
    id: u64,
    /// Its pixels, until uploaded (see `CanvasCache::uploaded`).
    pixels: Option<Arc<ImageData>>,
    /// Its size in bytes, uploaded or not: what the budget counts.
    bytes: usize,
    used: u64,
}
/// A cached raster: its image id, and its pixels if not uploaded yet.
pub struct Raster {
    /// The image id its GPU texture is keyed by.
    pub id: u64,
    /// Its pixels, until they are uploaded.
    pub pixels: Option<Arc<ImageData>>,
}
impl Entry {
    fn raster(&self) -> Raster {
        Raster {
            id: self.id,
            pixels: self.pixels.clone(),
        }
    }
}
/// Rasterized canvases and detailed decorations by node, kept while they
/// are unchanged and within a byte budget.
#[derive(Default)]
pub struct CanvasCache {
    entries: HashMap<NodeId, Entry>,
    bytes: usize,
    clock: u64,
    rasterizations: u64,
}
impl CanvasCache {
    /// Forget the rasters of nodes that are gone or no longer draw one.
    pub fn retain(&mut self, scene: &Scene) {
        self.entries.retain(|id, _| draws_raster(scene, *id));
        self.bytes = self.entries.values().map(|e| e.bytes).sum();
    }
    /// Whether it holds no raster.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// `retain` for the nodes `changed` alone (`Scene::resource_changes`).
    pub fn retain_changed(&mut self, scene: &Scene, changed: &[NodeId]) {
        for &node in changed {
            if self.entries.contains_key(&node) && !draws_raster(scene, node) {
                self.forget(node);
            }
        }
    }
    /// Forget the rasters of nodes `keep` says no to.
    pub fn retain_nodes(&mut self, keep: impl Fn(NodeId) -> bool) {
        self.entries.retain(|id, _| keep(*id));
        self.bytes = self.entries.values().map(|e| e.bytes).sum();
    }
    /// The image ids of the rasters it holds.
    pub fn image_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries.values().map(|e| e.id)
    }
    /// `node`'s raster is on the GPU: its pixels need not stay in memory.
    pub fn uploaded(&mut self, node: NodeId) {
        if let Some(entry) = self.entries.get_mut(&node) {
            entry.pixels = None;
        }
    }
    /// Rasterize `node` again on its next request (its texture was lost).
    pub fn forget(&mut self, node: NodeId) {
        if let Some(entry) = self.entries.remove(&node) {
            self.bytes -= entry.bytes;
        }
    }
    /// How many rasterizations it has done.
    pub fn rasterizations(&self) -> u64 {
        self.rasterizations
    }
    /// The bytes its rasters take, uploaded or not.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    /// `node`'s decoration (`style`) rasterized at `width` by `height` logical
    /// points and `scale`, from the cache while unchanged. More than 32
    /// shadows is an error.
    pub fn get_decoration(
        &mut self,
        node: NodeId,
        style: &zgx::scene::QuadStyle,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Raster, GpuError> {
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&node)
            && entry.decoration.as_ref() == Some(style)
            && entry.width == width
            && entry.height == height
            && entry.scale == scale
        {
            entry.used = self.clock;
            return Ok(entry.raster());
        }
        let canvas = Arc::new(crate::decoration::canvas(style, width, height)?);
        let image = self.get(node, &canvas, width, height, scale)?;
        if let Some(entry) = self.entries.get_mut(&node) {
            entry.decoration = Some(style.clone());
        }
        Ok(image)
    }
    /// `canvas` rasterized for `node` at `width` by `height` logical points and
    /// `scale`, from the cache while unchanged.
    pub fn get(
        &mut self,
        node: NodeId,
        canvas: &Arc<Canvas>,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Raster, GpuError> {
        self.get_viewed(node, canvas, Default::default(), width, height, scale)
    }
    /// `canvas` rasterized under `view` (its pan and zoom) for `node` at
    /// `width` by `height` logical points and `scale`, from the cache while
    /// neither changed.
    pub fn get_viewed(
        &mut self,
        node: NodeId,
        canvas: &Arc<Canvas>,
        view: zgx::scene::CanvasView,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Raster, GpuError> {
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&node)
            && entry
                .canvas
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, canvas) || old == *canvas)
            && entry.width == width
            && entry.height == height
            && entry.scale == scale
            && entry.view == view
        {
            entry.used = self.clock;
            return Ok(entry.raster());
        }
        let image = Arc::new(rasterize_viewed(canvas, view, width, height, scale)?);
        if let Some(old) = self.entries.remove(&node) {
            self.bytes -= old.bytes;
        }
        let bytes = image.pixels().len();
        while (self.bytes + bytes > BUDGET || self.entries.len() >= MAX_ENTRIES)
            && !self.entries.is_empty()
        {
            let victim = *self.entries.iter().min_by_key(|(_, e)| e.used).unwrap().0;
            self.bytes -= self.entries.remove(&victim).unwrap().bytes;
        }
        self.bytes += bytes;
        self.entries.insert(
            node,
            Entry {
                canvas: Arc::downgrade(canvas),
                decoration: None,
                width,
                height,
                scale,
                view,
                id: image.id(),
                pixels: Some(image.clone()),
                bytes,
                used: self.clock,
            },
        );
        self.rasterizations += 1;
        Ok(Raster {
            id: image.id(),
            pixels: Some(image),
        })
    }
}
/// Whether `node` is in the scene, a canvas or a decorated box.
fn draws_raster(scene: &Scene, node: NodeId) -> bool {
    scene.contains(node)
        && match scene.kind(node) {
            NodeKind::Canvas(_) => true,
            NodeKind::Quad(style) | NodeKind::Panel { quad: style, .. } => {
                style.decoration.is_some()
            }
            _ => false,
        }
}
fn color(color: zgx::scene::Color) -> sk::Color {
    sk::Color::from_rgba8(color.0, color.1, color.2, color.3)
}
/// Stops that make tiny-skia, which interpolates straight colours, draw
/// what interpolating premultiplied ones gives (as CSS and the GPU ramps
/// do): a span between stops of differing alpha is cut into steps whose
/// colours are the premultiplied mix, so a fade to transparent stays its
/// colour instead of greying.
fn premultiplied_stops(stops: &[zgx::canvas::GradientStop]) -> Vec<sk::GradientStop> {
    const STEPS: usize = 16;
    let mut out = Vec::with_capacity(stops.len());
    for (i, stop) in stops.iter().enumerate() {
        out.push(sk::GradientStop::new(stop.offset, color(stop.color)));
        let Some(next) = stops.get(i + 1) else { break };
        let (a, b) = (stop.color, next.color);
        if a.3 == b.3 || next.offset <= stop.offset {
            continue;
        }
        let premultiplied = |c: zgx::scene::Color| {
            let alpha = c.3 as f32 / 255.;
            [
                c.0 as f32 * alpha,
                c.1 as f32 * alpha,
                c.2 as f32 * alpha,
                alpha,
            ]
        };
        let (pa, pb) = (premultiplied(a), premultiplied(b));
        for step in 1..STEPS {
            let f = step as f32 / STEPS as f32;
            let mix: [f32; 4] = std::array::from_fn(|k| pa[k] + (pb[k] - pa[k]) * f);
            let straight = |v: f32| {
                if mix[3] > 0. {
                    (v / mix[3] / 255.).clamp(0., 1.)
                } else {
                    0.
                }
            };
            out.push(sk::GradientStop::new(
                stop.offset + (next.offset - stop.offset) * f,
                sk::Color::from_rgba(straight(mix[0]), straight(mix[1]), straight(mix[2]), mix[3])
                    .unwrap_or(sk::Color::TRANSPARENT),
            ));
        }
    }
    out
}
fn point(point: Point) -> sk::Point {
    sk::Point::from_xy(point.x, point.y)
}
fn path(path: &Path) -> Option<sk::Path> {
    let mut builder = sk::PathBuilder::new();
    for segment in path.segments() {
        match *segment {
            Segment::Move(p) => builder.move_to(p.x, p.y),
            Segment::Line(p) => builder.line_to(p.x, p.y),
            Segment::Quadratic(a, b) => builder.quad_to(a.x, a.y, b.x, b.y),
            Segment::Cubic(a, b, c) => builder.cubic_to(a.x, a.y, b.x, b.y, c.x, c.y),
            Segment::Close => builder.close(),
        }
    }
    builder.finish()
}
/// A renderer's canvas work on the CPU, with no GPU: for benches that count
/// instructions and draw nothing (the CPU gate), so a canvas that stops
/// drawing as shapes shows there. Each canvas a frame damages is found as
/// the quads and polyline strokes `GpuRenderer` draws, or rasterized (from
/// the cache while unchanged) where it cannot be.
#[derive(Default)]
pub struct HeadlessCanvases {
    cache: CanvasCache,
    points: Vec<[f32; 2]>,
}
impl HeadlessCanvases {
    /// The canvases of `scene` within `damage` at `scale`, as a renderer
    /// would draw them: how many drew as shapes, and how many rasterized.
    pub fn paint(
        &mut self,
        scene: &Scene,
        damage: &[zgx::scene::Rect],
        scale: f32,
    ) -> (usize, usize) {
        let mut counts = (0, 0);
        for item in scene.layer_items_within(None, Some(damage)) {
            let (NodeKind::Canvas(canvas), bounds) = (item.kind, item.bounds) else {
                continue;
            };
            if bounds.width <= 0. || bounds.height <= 0. {
                continue;
            }
            self.points.clear();
            let view = scene.canvas_view(item.id);
            if scene.has_grid_fills()
                && let Some(grid) = scene.grid_fill(item.id)
            {
                let mut marks = Vec::new();
                crate::grid_quads(grid, bounds, 1., &mut marks);
                std::hint::black_box(marks);
            }
            if item.effects.opacity >= 1.
                && item.effects.edge_fade <= 0.
                && let Some(quads) = crate::canvas_quads(
                    canvas,
                    (bounds.x, bounds.y),
                    view,
                    !cfg!(windows),
                    &mut self.points,
                )
            {
                std::hint::black_box((quads, &self.points));
                self.cache.forget(item.id);
                counts.0 += 1;
            } else if self
                .cache
                .get_viewed(item.id, canvas, view, bounds.width, bounds.height, scale)
                .is_ok()
            {
                counts.1 += 1;
            }
        }
        self.cache.retain(scene);
        counts
    }
}
/// The bytes a pixel of `canvas`'s raster takes at most while it draws: its
/// pixmap's 4, or its deepest clip stack's masks (a byte each), whichever
/// is more (what `rasterize_viewed` checks against `BUDGET`).
fn bytes_per_pixel(canvas: &Canvas) -> u64 {
    let (mut depth, mut deepest) = (0u64, 0u64);
    for command in canvas.commands() {
        match command {
            Draw::Clip { .. } => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            Draw::Restore => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest.clamp(4, 16)
}
/// The pixel size of a raster of `width` by `height` logical points at
/// `scale`: at `scale` while `bytes_per_pixel` of each fit `BUDGET`, and
/// otherwise at the largest scale below it that fits. `None`: not even one
/// pixel fits.
/// How much coarser than the display a canvas past the budget may draw.
const MOST_COARSE: f32 = 8.;
fn raster_size(width: f32, height: f32, scale: f32, bytes_per_pixel: u64) -> Option<(u32, u32)> {
    let most = BUDGET as u64 / bytes_per_pixel.max(1);
    let size = |scale: f32| {
        let w = ((width * scale).ceil() as u32).max(1);
        let h = ((height * scale).ceil() as u32).max(1);
        (w, h, u64::from(w) * u64::from(h))
    };
    // Coarser than an eighth of the display's scale it would draw nothing
    // worth seeing: refused (that canvas alone, not the frame).
    let least = scale / MOST_COARSE;
    let mut scale = scale;
    for _ in 0..64 {
        let (w, h, pixels) = size(scale);
        if pixels <= most {
            return Some((w, h));
        }
        if (w == 1 && h == 1) || scale < least {
            return None;
        }
        // Rounding up (and a side held at one pixel) can leave it just
        // past: a little less each time, which a few tries settle.
        scale *= (most as f32 / pixels as f32).sqrt().min(0.999);
    }
    None
}
/// Rasterize `canvas` at `width` by `height` logical points and `scale`
/// into an image.
pub fn rasterize(
    canvas: &Canvas,
    width: f32,
    height: f32,
    scale: f32,
) -> Result<ImageData, GpuError> {
    rasterize_viewed(canvas, Default::default(), width, height, scale)
}
/// Rasterize `canvas` under `view` (its drawing scaled and moved, as
/// `Scene::canvas_view` places it) at `width` by `height` logical points
/// and `scale` into an image.
pub fn rasterize_viewed(
    canvas: &Canvas,
    view: zgx::scene::CanvasView,
    width: f32,
    height: f32,
    scale: f32,
) -> Result<ImageData, GpuError> {
    if ![width, height, scale]
        .iter()
        .all(|v| v.is_finite() && *v > 0.)
        || canvas.commands().len() > 65536
    {
        return Err(GpuError(
            "invalid canvas dimensions or command count".into(),
        ));
    }
    // A raster past the budget is drawn coarser (fewer pixels, stretched over
    // its box) rather than not at all: a large canvas (a long map) must not
    // cost the window its frame.
    let (w, h) = raster_size(width, height, scale, bytes_per_pixel(canvas))
        .ok_or_else(|| GpuError("canvas raster exceeds 32 MiB".into()))?;
    let mut pixmap =
        sk::Pixmap::new(w, h).ok_or_else(|| GpuError("canvas allocation failed".into()))?;
    // Exact scaling to rounded texture extents avoids a fractional final-pixel seam.
    let transform = sk::Transform::from_scale(w as f32 / width, h as f32 / height).pre_concat(
        sk::Transform::from_row(view.scale, 0., 0., view.scale, view.x, view.y),
    );
    let mut masks: Vec<sk::Mask> = Vec::new();
    for command in canvas.commands() {
        match command {
            Draw::Clip { path: source, rule } => {
                if masks.len() >= 16
                    || (u64::from(w) * u64::from(h))
                        .checked_mul((masks.len() + 1) as u64)
                        .is_none_or(|bytes| bytes > BUDGET as u64)
                {
                    return Err(GpuError("canvas clip stack exceeds budget".into()));
                }
                let mut mask = masks.last().cloned().unwrap_or_else(|| {
                    let mut mask = sk::Mask::new(w, h).unwrap();
                    mask.data_mut().fill(255);
                    mask
                });
                if let Some(path) = path(source) {
                    mask.intersect_path(
                        &path,
                        match rule {
                            FillRule::Winding => sk::FillRule::Winding,
                            FillRule::EvenOdd => sk::FillRule::EvenOdd,
                        },
                        true,
                        transform,
                    );
                } else {
                    mask.clear();
                }
                masks.push(mask);
                continue;
            }
            Draw::Restore => {
                if masks.pop().is_none() {
                    return Err(GpuError("unbalanced canvas restore".into()));
                }
                continue;
            }
            _ => {}
        }
        let (source, brush) = match command {
            Draw::Fill { path, brush, .. } | Draw::Stroke { path, brush, .. } => (path, brush),
            _ => unreachable!(),
        };
        let Some(path) = path(source) else {
            continue;
        };
        let mut paint = sk::Paint::default();
        let tile;
        paint.shader = match brush {
            Brush::Solid(c) => sk::Shader::SolidColor(color(*c)),
            Brush::Linear { start, end, stops } => {
                if stops.len() < 2
                    || stops.len() > 1024
                    || stops
                        .iter()
                        .any(|s| !s.offset.is_finite() || !(0. ..=1.).contains(&s.offset))
                {
                    return Err(GpuError("invalid gradient stops".into()));
                }
                sk::LinearGradient::new(
                    point(*start),
                    point(*end),
                    premultiplied_stops(stops),
                    sk::SpreadMode::Pad,
                    sk::Transform::identity(),
                )
                .ok_or_else(|| GpuError("invalid gradient geometry".into()))?
            }
            Brush::Slash {
                background,
                foreground,
                spacing,
                width,
            } => {
                if !spacing.is_finite()
                    || !width.is_finite()
                    || *spacing < 1.
                    || *spacing > 1024.
                    || *width < 0.
                    || *width > *spacing
                {
                    return Err(GpuError("invalid slash pattern".into()));
                }
                let side = spacing.ceil() as u32;
                let mut pixels = sk::Pixmap::new(side, side).unwrap();
                for y in 0..side {
                    for x in 0..side {
                        let stripe = ((x + y) as f32 % side as f32) < width / spacing * side as f32;
                        pixels.pixels_mut()[(y * side + x) as usize] =
                            color(if stripe { *foreground } else { *background })
                                .premultiply()
                                .to_color_u8();
                    }
                }
                tile = pixels;
                sk::Pattern::new(
                    tile.as_ref(),
                    sk::SpreadMode::Repeat,
                    sk::FilterQuality::Bilinear,
                    1.,
                    sk::Transform::from_scale(spacing / side as f32, spacing / side as f32),
                )
            }
        };
        match command {
            Draw::Fill { rule, .. } => pixmap.fill_path(
                &path,
                &paint,
                match rule {
                    FillRule::Winding => sk::FillRule::Winding,
                    FillRule::EvenOdd => sk::FillRule::EvenOdd,
                },
                transform,
                masks.last(),
            ),
            Draw::Stroke { style, .. } => {
                if !style.width.is_finite()
                    || style.width <= 0.
                    || !style.miter_limit.is_finite()
                    || style.miter_limit < 1.
                    || !style.dash_offset.is_finite()
                    || style.dash.len() > 1024
                    // A dash may be 0 long (a dot, with round caps, as an
                    // HTML canvas's `setLineDash` takes), not the pattern.
                    || style.dash.iter().any(|d| !d.is_finite() || *d < 0.)
                    || (!style.dash.is_empty() && style.dash.iter().sum::<f32>() <= 0.)
                {
                    return Err(GpuError("invalid stroke style".into()));
                }
                let dash = if style.dash.is_empty() {
                    None
                } else {
                    Some(
                        sk::StrokeDash::new(style.dash.clone(), style.dash_offset)
                            .ok_or_else(|| GpuError("invalid dash pattern".into()))?,
                    )
                };
                let stroke = sk::Stroke {
                    width: style.width,
                    miter_limit: style.miter_limit,
                    line_cap: match style.cap {
                        LineCap::Butt => sk::LineCap::Butt,
                        LineCap::Round => sk::LineCap::Round,
                        LineCap::Square => sk::LineCap::Square,
                    },
                    line_join: match style.join {
                        LineJoin::Miter => sk::LineJoin::Miter,
                        LineJoin::Round => sk::LineJoin::Round,
                        LineJoin::Bevel => sk::LineJoin::Bevel,
                    },
                    dash,
                };
                pixmap.stroke_path(&path, &paint, &stroke, transform, masks.last());
            }
            _ => unreachable!(),
        }
    }
    if !masks.is_empty() {
        return Err(GpuError("unbalanced canvas clip".into()));
    }
    let mut pixels = pixmap.take();
    for p in pixels.as_chunks_mut::<4>().0 {
        let alpha = u32::from(p[3]);
        if alpha > 0 {
            for channel in &mut p[..3] {
                *channel = (u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
        }
    }
    ImageData::new(w, h, pixels).map_err(|e| GpuError(e.into()))
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    #[test]
    fn removed_canvas_drops_commands_before_next_paint() {
        let mut scene = Scene::new(100., 100.);
        let mut canvas = Canvas::new();
        canvas.fill(
            Path::rectangle(zgx::scene::Rect::new(0., 0., 10., 10.)),
            zgx::scene::Color(255, 0, 0, 255),
        );
        let canvas = Arc::new(canvas);
        let weak = Arc::downgrade(&canvas);
        let node = scene.append(
            scene.root(),
            NodeKind::Canvas(canvas.clone()),
            Default::default(),
        );
        let mut cache = CanvasCache::default();
        let first = cache.get(node, &canvas, 20., 20., 1.).unwrap();
        cache.uploaded(node);
        let second = cache.get(node, &canvas, 20., 20., 1.).unwrap();
        assert_eq!(first.id, second.id);
        assert!(second.pixels.is_none(), "uploaded rasters keep no pixels");
        assert_eq!(cache.rasterizations(), 1);
        scene.remove(node);
        drop(canvas);
        assert!(weak.upgrade().is_none());
        assert!(
            cache.bytes() > 0,
            "test runs before next render retention pass"
        );
    }
    #[test]
    fn tiny_canvas_rasters_obey_entry_count_and_lru_limit() {
        let mut scene = Scene::new(100., 100.);
        let canvas = Arc::new(Canvas::new());
        let mut cache = CanvasCache::default();
        let mut first = None;
        for _ in 0..=MAX_ENTRIES {
            let node = scene.append(
                scene.root(),
                NodeKind::Canvas(canvas.clone()),
                Default::default(),
            );
            first.get_or_insert(node);
            cache.get(node, &canvas, 1., 1., 1.).unwrap();
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert_eq!(cache.bytes(), MAX_ENTRIES * 4);
        assert!(!cache.entries.contains_key(&first.unwrap()));
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    fn filled() -> Canvas {
        let mut canvas = Canvas::new();
        canvas.fill(
            Path::rectangle(zgx::scene::Rect::new(0., 0., 10., 10.)),
            zgx::scene::Color(255, 0, 0, 255),
        );
        canvas
    }
    #[test]
    fn raster_within_budget_keeps_its_scale() {
        assert_eq!(raster_size(100., 50., 2., 4), Some((200, 100)));
        assert_eq!(raster_size(100.2, 50., 1., 4), Some((101, 50)));
    }
    #[test]
    fn raster_past_budget_is_drawn_coarser_not_refused() {
        // A long map (the canvas that skipped a window's frames): 1044 by
        // 6356 points at 2x is 101 MiB of pixels.
        let (w, h) = raster_size(1044., 6356., 2., 4).unwrap();
        assert!(u64::from(w) * u64::from(h) * 4 <= BUDGET as u64);
        assert!(w > 1044 && h > 6356, "as fine as fits: {w}x{h}");
        let ratio = (w as f32 / h as f32) / (1044. / 6356.);
        assert!((ratio - 1.).abs() < 0.01, "keeps its shape: {w}x{h}");
        // Clip masks take a byte a pixel each.
        let (w, h) = raster_size(1044., 6356., 2., 8).unwrap();
        assert!(u64::from(w) * u64::from(h) * 8 <= BUDGET as u64);
        // Thin and long: a side held at one pixel.
        let (w, h) = raster_size(0.5, 4e6, 1., 4).unwrap();
        assert_eq!(w, 1);
        assert!(u64::from(h) * 4 <= BUDGET as u64);
        // Coarser than an eighth of the display's scale: refused, that
        // canvas alone (the frame goes on without it).
        assert_eq!(raster_size(0.5, 1e9, 1., 4), None);
        assert_eq!(raster_size(100_000., 100_000., 1., 4), None);

        let image = rasterize(&filled(), 1044., 6356., 2.).unwrap();
        assert!(image.pixels().len() <= BUDGET);
        let mut cache = CanvasCache::default();
        let mut scene = Scene::new(100., 100.);
        let canvas = Arc::new(filled());
        let node = scene.append(
            scene.root(),
            NodeKind::Canvas(canvas.clone()),
            Default::default(),
        );
        let raster = cache.get(node, &canvas, 1044., 6356., 2.).unwrap();
        assert!(raster.pixels.is_some());
        assert!(cache.bytes() <= BUDGET);
    }
    #[test]
    fn clip_depth_sets_bytes_per_pixel() {
        assert_eq!(bytes_per_pixel(&filled()), 4);
        let mut canvas = filled();
        let rect = || Path::rectangle(zgx::scene::Rect::new(0., 0., 10., 10.));
        for _ in 0..6 {
            canvas.clip(rect(), FillRule::Winding);
        }
        for _ in 0..6 {
            canvas.restore();
        }
        assert_eq!(bytes_per_pixel(&canvas), 6);
        // Six masks of a raster as large as fits.
        rasterize(&canvas, 1044., 6356., 2.).unwrap();
    }
}
