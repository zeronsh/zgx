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
/// 2 to 1,024 stops with offsets from 0 to 1, or an error.
fn check_stops(stops: &[zgx::canvas::GradientStop]) -> Result<(), GpuError> {
    if stops.len() < 2
        || stops.len() > 1024
        || stops
            .iter()
            .any(|s| !s.offset.is_finite() || !(0. ..=1.).contains(&s.offset))
    {
        return Err(GpuError("invalid gradient stops".into()));
    }
    Ok(())
}
fn affine(m: zgx::affine::Affine) -> sk::Transform {
    sk::Transform::from_row(m.a, m.b, m.c, m.d, m.tx, m.ty)
}
/// The colour at each of `N` offsets evenly from 0 to 1 along `stops`,
/// premultiplied and mixed premultiplied, as `premultiplied_stops` draws a
/// linear or radial one.
fn ramp<const N: usize>(stops: &[zgx::canvas::GradientStop]) -> [sk::PremultipliedColorU8; N] {
    let premultiplied = |c: zgx::scene::Color| {
        let alpha = f32::from(c.3) / 255.;
        [
            f32::from(c.0) * alpha,
            f32::from(c.1) * alpha,
            f32::from(c.2) * alpha,
            alpha * 255.,
        ]
    };
    std::array::from_fn(|i| {
        let t = i as f32 / (N - 1) as f32;
        let after = stops.iter().position(|s| s.offset > t);
        let mix = match after {
            None => premultiplied(stops[stops.len() - 1].color),
            Some(0) => premultiplied(stops[0].color),
            Some(next) => {
                let (a, b) = (&stops[next - 1], &stops[next]);
                let f = (t - a.offset) / (b.offset - a.offset);
                let (pa, pb) = (premultiplied(a.color), premultiplied(b.color));
                std::array::from_fn(|k| pa[k] + (pb[k] - pa[k]) * f)
            }
        };
        let [r, g, b, a] = mix.map(|v| v.round().clamp(0., 255.) as u8);
        sk::PremultipliedColorU8::from_rgba(r.min(a), g.min(a), b.min(a), a)
            .unwrap_or(sk::PremultipliedColorU8::TRANSPARENT)
    })
}
/// Pixels to draw from a device pixel: where their top left falls, and them.
type Tile = ((i32, i32), sk::Pixmap);
/// A conic gradient's pixels over the device pixels `path`, drawn under
/// `transform` and reaching `reach` past its outline, can cover within a
/// `size` raster: where they start and the pixels, or `None` where it
/// covers none.
fn conic_tile(
    path: &sk::Path,
    reach: f32,
    transform: sk::Transform,
    placed: zgx::affine::Affine,
    stops: &[zgx::canvas::GradientStop],
    (width, height): (u32, u32),
) -> Result<Option<Tile>, GpuError> {
    let bounds = path.bounds();
    let Some(device) = sk::Rect::from_ltrb(
        bounds.left() - reach,
        bounds.top() - reach,
        bounds.right() + reach,
        bounds.bottom() + reach,
    )
    .and_then(|r| r.transform(transform)) else {
        return Ok(None);
    };
    let x0 = (device.left().floor().max(0.)) as i32;
    let y0 = (device.top().floor().max(0.)) as i32;
    let x1 = (device.right().ceil().min(width as f32)) as i32;
    let y1 = (device.bottom().ceil().min(height as f32)) as i32;
    if x1 <= x0 || y1 <= y0 {
        return Ok(None);
    }
    // Device pixels back to the gradient's own coordinates.
    let to_gradient = transform
        .pre_concat(affine(placed))
        .invert()
        .ok_or_else(|| GpuError("invalid gradient geometry".into()))?;
    const STEPS: usize = 1024;
    let colors = ramp::<STEPS>(stops);
    let (tw, th) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let mut tile =
        sk::Pixmap::new(tw, th).ok_or_else(|| GpuError("canvas allocation failed".into()))?;
    let pixels = tile.pixels_mut();
    for y in 0..th {
        for x in 0..tw {
            let mut at = [sk::Point::from_xy(
                x0 as f32 + x as f32 + 0.5,
                y0 as f32 + y as f32 + 0.5,
            )];
            to_gradient.map_points(&mut at);
            let turn = at[0].y.atan2(at[0].x) / std::f32::consts::TAU;
            let offset = turn.rem_euclid(1.);
            pixels[(y * tw + x) as usize] =
                colors[((offset * (STEPS - 1) as f32).round() as usize).min(STEPS - 1)];
        }
    }
    Ok(Some(((x0, y0), tile)))
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
    let w = (width * scale).ceil() as u32;
    let h = (height * scale).ceil() as u32;
    if w == 0
        || h == 0
        || (u64::from(w) * u64::from(h))
            .checked_mul(4)
            .is_none_or(|bytes| bytes > BUDGET as u64)
    {
        return Err(GpuError("canvas raster exceeds 32 MiB".into()));
    }
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
                check_stops(stops)?;
                sk::LinearGradient::new(
                    point(*start),
                    point(*end),
                    premultiplied_stops(stops),
                    sk::SpreadMode::Pad,
                    sk::Transform::identity(),
                )
                .ok_or_else(|| GpuError("invalid gradient geometry".into()))?
            }
            Brush::Radial {
                transform: placed,
                stops,
            } => {
                check_stops(stops)?;
                sk::RadialGradient::new(
                    sk::Point::zero(),
                    sk::Point::zero(),
                    1.,
                    premultiplied_stops(stops),
                    sk::SpreadMode::Pad,
                    affine(*placed),
                )
                .ok_or_else(|| GpuError("invalid gradient geometry".into()))?
            }
            Brush::Conic {
                transform: placed,
                stops,
            } => {
                check_stops(stops)?;
                // tiny-skia has no sweep gradient: its pixels are worked out
                // over the device pixels the path can reach, and drawn as a
                // pattern there.
                let reach = match command {
                    Draw::Stroke { style, .. } => style.width.max(0.) * style.miter_limit.max(1.),
                    _ => 0.,
                };
                let Some((tile_at, pixels)) =
                    conic_tile(&path, reach, transform, *placed, stops, (w, h))?
                else {
                    continue;
                };
                tile = pixels;
                let to_user = transform
                    .invert()
                    .ok_or_else(|| GpuError("invalid canvas transform".into()))?;
                sk::Pattern::new(
                    tile.as_ref(),
                    sk::SpreadMode::Pad,
                    sk::FilterQuality::Nearest,
                    1.,
                    to_user.pre_translate(tile_at.0 as f32, tile_at.1 as f32),
                )
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
mod gradient_tests {
    use super::*;
    use zgx::{affine::Affine, scene::Color};
    const RED: Color = Color(255, 0, 0, 255);
    const BLUE: Color = Color(0, 0, 255, 255);
    fn stops(colors: &[Color]) -> Vec<GradientStop> {
        let n = (colors.len() - 1) as f32;
        colors
            .iter()
            .enumerate()
            .map(|(i, &color)| GradientStop {
                offset: i as f32 / n,
                color,
            })
            .collect()
    }
    /// A 100 by 100 square filled with `brush`, rasterized at 1x.
    fn filled(brush: Brush) -> ImageData {
        let mut canvas = Canvas::new();
        canvas.fill(
            Path::rectangle(zgx::scene::Rect::new(0., 0., 100., 100.)),
            brush,
        );
        rasterize(&canvas, 100., 100., 1.).unwrap()
    }
    fn at(image: &ImageData, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * image.width() + x) * 4) as usize;
        image.pixels()[i..i + 4].try_into().unwrap()
    }
    /// Within 3 of `expected` in each channel.
    fn close(found: [u8; 4], expected: [u8; 4]) -> bool {
        found.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 3)
    }

    #[test]
    fn a_radial_gradient_runs_out_from_its_centre_and_holds_its_last_colour() {
        let circle = Affine::scale(40., 40.).then(Affine::translation(50., 50.));
        let image = filled(Brush::radial(circle, stops(&[RED, BLUE])).unwrap());
        // Pixel centres: (50.5, 50.5) is half a pixel out.
        assert!(
            close(at(&image, 50, 50), [253, 0, 2, 255]),
            "{:?}",
            at(&image, 50, 50)
        );
        let half = at(&image, 70, 50);
        assert!(close(half, [125, 0, 130, 255]), "halfway out: {half:?}");
        assert_eq!(at(&image, 50, 70), half, "the same distance down");
        assert_eq!(at(&image, 29, 50), half, "or back");
        assert_eq!(at(&image, 95, 50), [0, 0, 255, 255], "past its edge");
        assert_eq!(at(&image, 0, 0), [0, 0, 255, 255]);
        // An ellipse half as tall: half its height down is halfway out.
        let ellipse = Affine::scale(40., 20.).then(Affine::translation(50., 50.));
        let image = filled(Brush::radial(ellipse, stops(&[RED, BLUE])).unwrap());
        assert!(close(at(&image, 50, 60), half), "{:?}", at(&image, 50, 60));
        assert!(close(at(&image, 70, 50), half), "{:?}", at(&image, 70, 50));
        assert_eq!(at(&image, 50, 75), [0, 0, 255, 255]);
    }

    #[test]
    fn a_conic_gradient_turns_clockwise_from_its_angle() {
        let centre = Affine::translation(50., 50.);
        let image = filled(Brush::conic(centre, stops(&[RED, BLUE])).unwrap());
        // A quarter turn clockwise from +x is straight down (y grows down).
        let down = at(&image, 50, 90);
        assert!(close(down, [191, 0, 64, 255]), "a quarter round: {down:?}");
        let left = at(&image, 10, 50);
        assert!(close(left, [128, 0, 128, 255]), "half round: {left:?}");
        let up = at(&image, 50, 10);
        assert!(close(up, [64, 0, 191, 255]), "three quarters: {up:?}");
        // Turned a quarter, it starts straight down.
        let turned = Affine::rotation(std::f32::consts::FRAC_PI_2).then(centre);
        let image = filled(Brush::conic(turned, stops(&[RED, BLUE])).unwrap());
        // Just clockwise of straight down (left of it): just past its start.
        let down = at(&image, 47, 90);
        assert!(down[0] > 240 && down[2] < 15, "{down:?}");
        let left = at(&image, 10, 50);
        assert!(close(left, [191, 0, 64, 255]), "{left:?}");
        // Mirrored top to bottom, it runs anticlockwise: straight down is
        // three quarters round.
        let mirrored = Affine::scale(1., -1.).then(centre);
        let image = filled(Brush::conic(mirrored, stops(&[RED, BLUE])).unwrap());
        let down = at(&image, 50, 90);
        assert!(close(down, [64, 0, 191, 255]), "{down:?}");
    }

    #[test]
    fn a_conic_gradient_mixes_alpha_premultiplied_and_keeps_to_its_clip_and_shape() {
        let centre = Affine::translation(50., 50.);
        let clear_red = Color(255, 0, 0, 0);
        let mut canvas = Canvas::new();
        canvas
            .clip(
                Path::rectangle(zgx::scene::Rect::new(0., 0., 100., 50.)),
                FillRule::Winding,
            )
            .fill(
                Path::ellipse(zgx::scene::Rect::new(10., 10., 80., 80.)).unwrap(),
                Brush::conic(centre, stops(&[RED, clear_red])).unwrap(),
            )
            .restore();
        let image = rasterize(&canvas, 100., 100., 1.).unwrap();
        // Up and left (offset about 0.625): its colour stays red as it fades.
        let faded = at(&image, 30, 30);
        assert!(
            faded[0] == 255 && faded[1] == 0 && (90..=100).contains(&faded[3]),
            "{faded:?}"
        );
        assert_eq!(at(&image, 50, 70)[3], 0, "below the clip");
        assert_eq!(at(&image, 2, 2)[3], 0, "outside the ellipse");
        // A stroke of it.
        let mut canvas = Canvas::new();
        canvas.stroke(
            Path::ellipse(zgx::scene::Rect::new(10., 10., 80., 80.)).unwrap(),
            Brush::conic(centre, stops(&[RED, BLUE])).unwrap(),
            Stroke {
                width: 6.,
                ..Default::default()
            },
        );
        let image = rasterize(&canvas, 100., 100., 1.).unwrap();
        assert!(
            close(at(&image, 50, 89), [191, 0, 64, 255]),
            "{:?}",
            at(&image, 50, 89)
        );
        assert_eq!(at(&image, 50, 50)[3], 0, "inside the ring");
    }

    #[test]
    fn gradients_that_place_nothing_are_errors() {
        let flat = Affine::scale(0., 1.);
        assert!(Brush::radial(flat, stops(&[RED, BLUE])).is_err());
        assert!(Brush::conic(flat, stops(&[RED, BLUE])).is_err());
        let nan = Affine::translation(f32::NAN, 0.);
        assert!(Brush::conic(nan, stops(&[RED, BLUE])).is_err());
        assert!(Brush::radial(Affine::IDENTITY, stops(&[RED])).is_err());
    }
}
