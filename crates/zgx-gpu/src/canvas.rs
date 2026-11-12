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
    /// Where it lies, if drawn to be panned (`CanvasCache::get_panned`).
    panned: Option<Panned>,
}
/// A raster drawn to be panned: a region of the drawing under one zoom, in
/// the box's device pixels under the view it was first drawn under.
#[derive(Clone, Copy)]
struct Panned {
    /// The region drawn (x0, y0, x1, y1).
    region: [i32; 4],
    /// What the drawing paints within, there: no pan shows more.
    painted: [i32; 4],
}
/// Where a panned raster shows in its canvas's box: `rect` (x, y, width,
/// height, in points from the box's corner; empty for nothing) and the part
/// of the raster drawn there, `uv` (x, y, width, height, from 0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Where in the box, in points.
    pub rect: [f32; 4],
    /// Which part of the raster.
    pub uv: [f32; 4],
}
/// The most pixels a panned raster's region takes (its bytes the budget).
const REGION_PIXELS: i64 = BUDGET as i64 / 4;
/// The longest side of a panned raster's region, unless the viewport's is
/// longer: within any GPU's textures.
const REGION_SIDE: i64 = 4096;
impl Panned {
    /// Where it shows panned by `shift` device pixels from where it was
    /// drawn, in a box of `size` device pixels (`width` points across, at
    /// `scale`): `None` if part of the drawing that would show isn't in it.
    fn place(
        &self,
        shift: [i32; 2],
        size: [i32; 2],
        width: f32,
        height: f32,
        scale: f32,
    ) -> Option<Placement> {
        let [rx0, ry0, rx1, ry1] = self.region;
        // The viewport, where the raster was drawn.
        let seen = [-shift[0], -shift[1], size[0] - shift[0], size[1] - shift[1]];
        let needed = intersect(seen, self.painted);
        if needed[0] < needed[2]
            && needed[1] < needed[3]
            && (needed[0] < rx0 || needed[1] < ry0 || needed[2] > rx1 || needed[3] > ry1)
        {
            return None;
        }
        let [x0, y0, x1, y1] = intersect(seen, self.region);
        if x0 >= x1 || y0 >= y1 {
            return Some(Placement {
                rect: [0.; 4],
                uv: [0.; 4],
            });
        }
        let (x, y) = (
            (x0 + shift[0]) as f32 / scale,
            (y0 + shift[1]) as f32 / scale,
        );
        // The last pixel, cut where the box ends.
        let w = ((x1 - x0) as f32 / scale).min(width - x);
        let h = ((y1 - y0) as f32 / scale).min(height - y);
        let (rw, rh) = ((rx1 - rx0) as f32, (ry1 - ry0) as f32);
        Some(Placement {
            rect: [x, y, w, h],
            uv: [
                (x0 - rx0) as f32 / rw,
                (y0 - ry0) as f32 / rh,
                w * scale / rw,
                h * scale / rh,
            ],
        })
    }
}
fn intersect(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}
/// What `canvas` paints within (x0, y0, x1, y1, in its points): its fills
/// and strokes' reach, a curve's by its control points. `None` for nothing.
fn painted_bounds(canvas: &Canvas) -> Option<[f32; 4]> {
    let mut out: Option<[f32; 4]> = None;
    // A path's points, painted `reach` around and moved by `(dx, dy)`.
    let mut within = |path: &Path, reach: f32, (dx, dy): (f32, f32)| {
        let reach = if reach.is_finite() { reach.max(0.) } else { 0. };
        for segment in path.segments() {
            let points: &[Point] = match segment {
                Segment::Move(p) | Segment::Line(p) => std::slice::from_ref(p),
                Segment::Quadratic(a, b) => &[*a, *b],
                Segment::Cubic(a, b, c) => &[*a, *b, *c],
                Segment::Close => &[],
            };
            for p in points {
                let (x, y) = (p.x + dx, p.y + dy);
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                let [x0, y0, x1, y1] = out.get_or_insert([x, y, x, y]);
                *x0 = x0.min(x - reach);
                *y0 = y0.min(y - reach);
                *x1 = x1.max(x + reach);
                *y1 = y1.max(y + reach);
            }
        }
    };
    for command in canvas.commands() {
        match command {
            Draw::Fill { path, .. } => within(path, 0., (0., 0.)),
            // A miter juts out at most its limit times half the width; a
            // square cap, half the width's diagonal.
            Draw::Stroke { path, style, .. } => within(
                path,
                style.width / 2. * style.miter_limit.max(std::f32::consts::SQRT_2),
                (0., 0.),
            ),
            Draw::Clip { .. } | Draw::Restore => {}
        }
    }
    out
}
/// The first region of `canvas` drawn under `view` for a box of `size`
/// device pixels at `scale`: what of it shows, as a raster of the box drew,
/// and what it paints within (both in the box's device pixels).
fn panned_region(
    canvas: &Canvas,
    view: zgx::scene::CanvasView,
    size: [i32; 2],
    scale: f32,
) -> Panned {
    // Two pixels more each way, for antialiasing.
    let painted = painted_bounds(canvas).map_or([0; 4], |[x0, y0, x1, y1]| {
        let at = |v: f32, offset: f32| ((v * view.scale + offset) * scale) as f64;
        let clamp = |v: f64| v.clamp(i32::MIN as f64 / 2., i32::MAX as f64 / 2.) as i32;
        [
            clamp(at(x0, view.x).floor() - 2.),
            clamp(at(y0, view.y).floor() - 2.),
            clamp(at(x1, view.x).ceil() + 2.),
            clamp(at(y1, view.y).ceil() + 2.),
        ]
    });
    Panned {
        region: intersect([0, 0, size[0], size[1]], painted),
        painted,
    }
}
fn is_empty([x0, y0, x1, y1]: [i32; 4]) -> bool {
    x0 >= x1 || y0 >= y1
}
fn area([x0, y0, x1, y1]: [i32; 4]) -> i64 {
    (i64::from(x1) - i64::from(x0)) * (i64::from(y1) - i64::from(y0))
}
impl Panned {
    /// It grown for the viewport panned by `shift` past it: on to an
    /// eighth of the viewport beyond, the way the view went, added to what
    /// it holds while within the limits (or in its place).
    fn extended(&self, shift: [i32; 2], size: [i32; 2]) -> Panned {
        let seen = [-shift[0], -shift[1], size[0] - shift[0], size[1] - shift[1]];
        let (mx, my) = (size[0] / 8, size[1] / 8);
        let [x0, y0, x1, y1] = self.region;
        let ahead = [
            if seen[0] < x0 { seen[0] - mx } else { seen[0] },
            if seen[1] < y0 { seen[1] - my } else { seen[1] },
            if seen[2] > x1 { seen[2] + mx } else { seen[2] },
            if seen[3] > y1 { seen[3] + my } else { seen[3] },
        ];
        let grown = intersect(ahead, self.painted);
        let both = match is_empty(self.region) {
            true => grown,
            false => [
                x0.min(grown[0]),
                y0.min(grown[1]),
                x1.max(grown[2]),
                y1.max(grown[3]),
            ],
        };
        let fits = |[x0, y0, x1, y1]: [i32; 4]| {
            i64::from(x1 - x0) <= REGION_SIDE.max(i64::from(size[0] + 2 * mx))
                && i64::from(y1 - y0) <= REGION_SIDE.max(i64::from(size[1] + 2 * my))
                && area([x0, y0, x1, y1]) <= REGION_PIXELS
        };
        Panned {
            region: if fits(both) {
                both
            } else if area(grown) <= REGION_PIXELS {
                grown
            } else {
                intersect(seen, self.painted)
            },
            painted: self.painted,
        }
    }
}
/// The `region` (x0, y0, x1, y1, in its box's device pixels) of `canvas`
/// under `view` at `scale`, its pixels where `old` (the region `pixels`
/// hold) has them taken from there: only what is new is rasterized.
fn rasterize_region(
    canvas: &Canvas,
    view: zgx::scene::CanvasView,
    scale: f32,
    region: [i32; 4],
    old: Option<([i32; 4], &ImageData)>,
) -> Result<ImageData, GpuError> {
    if is_empty(region) {
        // Nothing shows: a pixel, never drawn.
        return ImageData::new(1, 1, vec![0; 4]).map_err(|e| GpuError(e.into()));
    }
    if area(region) > REGION_PIXELS {
        return Err(GpuError("canvas raster exceeds 32 MiB".into()));
    }
    let [x0, y0, x1, y1] = region;
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let drawn = |[ax, ay, bx, by]: [i32; 4]| {
        let transform = sk::Transform::from_translate(-ax as f32, -ay as f32).pre_concat(
            sk::Transform::from_row(
                view.scale * scale,
                0.,
                0.,
                view.scale * scale,
                view.x * scale,
                view.y * scale,
            ),
        );
        rasterize_at(canvas, (bx - ax) as u32, (by - ay) as u32, transform)
    };
    let kept = old
        .map(|(at, pixels)| (intersect(at, region), at, pixels))
        .filter(|(both, ..)| !is_empty(*both));
    let Some((both, at, pixels)) = kept else {
        return drawn(region);
    };
    let mut out = vec![0u8; w * h * 4];
    // Each part's rows into place.
    let mut put =
        |[ax, ay, bx, by]: [i32; 4], from: &[u8], from_x: i32, from_y: i32, stride: usize| {
            let len = (bx - ax) as usize * 4;
            for y in ay..by {
                let src = ((y - from_y) as usize * stride + (ax - from_x) as usize) * 4;
                let dst = ((y - y0) as usize * w + (ax - x0) as usize) * 4;
                out[dst..dst + len].copy_from_slice(&from[src..src + len]);
            }
        };
    put(both, pixels.pixels(), at[0], at[1], pixels.width() as usize);
    // What is new: the bands above and below what was kept, and beside it.
    for part in [
        [x0, y0, x1, both[1]],
        [x0, both[3], x1, y1],
        [x0, both[1], both[0], both[3]],
        [both[2], both[1], x1, both[3]],
    ] {
        if !is_empty(part) {
            let image = drawn(part)?;
            put(
                part,
                image.pixels(),
                part[0],
                part[1],
                image.width() as usize,
            );
        }
    }
    ImageData::new(w as u32, h as u32, out).map_err(|e| GpuError(e.into()))
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
        // A panned raster's are kept: a pan past it draws what is new
        // beside them.
        if let Some(entry) = self.entries.get_mut(&node)
            && entry.panned.is_none()
        {
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
            && entry.panned.is_none()
        {
            entry.used = self.clock;
            return Ok(entry.raster());
        }
        let image = Arc::new(rasterize_viewed(canvas, view, width, height, scale)?);
        Ok(self.store(node, canvas, image, (width, height, scale), view, None))
    }
    /// `canvas` rasterized for `node` under `view`, to be drawn where the
    /// `Placement` says in its `width` by `height` point box at `scale`. A
    /// pan moves the raster, in whole device pixels, rasterizing nothing;
    /// one onto more of the drawing rasterizes what it shows anew alone,
    /// kept beside the rest (to the limits). A zoom or a change rasterizes
    /// what shows, as `get_viewed` does.
    pub fn get_panned(
        &mut self,
        node: NodeId,
        canvas: &Arc<Canvas>,
        view: zgx::scene::CanvasView,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<(Raster, Placement), GpuError> {
        if ![width, height, scale, view.x, view.y, view.scale]
            .iter()
            .all(|v| v.is_finite())
            || width <= 0.
            || height <= 0.
            || scale <= 0.
            || view.scale <= 0.
            || canvas.commands().len() > 65536
        {
            return Err(GpuError(
                "invalid canvas dimensions or command count".into(),
            ));
        }
        let size = [
            (width * scale).ceil() as i32,
            (height * scale).ceil() as i32,
        ];
        self.clock = self.clock.wrapping_add(1);
        // The raster at this zoom, and how far the view moved since.
        let mut kept = None;
        if let Some(entry) = self.entries.get_mut(&node)
            && let Some(panned) = entry.panned
            && entry
                .canvas
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, canvas) || old == *canvas)
            && entry.width == width
            && entry.height == height
            && entry.scale == scale
            && entry.view.scale == view.scale
        {
            let shift = [
                ((view.x - entry.view.x) * scale).round() as i32,
                ((view.y - entry.view.y) * scale).round() as i32,
            ];
            if let Some(placement) = panned.place(shift, size, width, height, scale) {
                entry.used = self.clock;
                return Ok((entry.raster(), placement));
            }
            kept = entry
                .pixels
                .clone()
                .map(|pixels| (entry.view, panned, pixels, shift));
        }
        // Panned past what it holds: what is new drawn beside what it
        // holds. Else (a zoom, a change) what shows, as a box's raster.
        let (drawn, panned, image, shift) = match &kept {
            Some((drawn, old, pixels, shift)) => {
                let panned = old.extended(*shift, size);
                let image = rasterize_region(
                    canvas,
                    *drawn,
                    scale,
                    panned.region,
                    Some((old.region, pixels)),
                )?;
                (*drawn, panned, image, *shift)
            }
            None => {
                let panned = panned_region(canvas, view, size, scale);
                let image = rasterize_region(canvas, view, scale, panned.region, None)?;
                (view, panned, image, [0, 0])
            }
        };
        let placement = panned
            .place(shift, size, width, height, scale)
            .expect("drawn where it shows");
        let raster = self.store(
            node,
            canvas,
            Arc::new(image),
            (width, height, scale),
            drawn,
            Some(panned),
        );
        Ok((raster, placement))
    }
    /// Keep `image`, `canvas` rasterized for `node`, within the budget.
    fn store(
        &mut self,
        node: NodeId,
        canvas: &Arc<Canvas>,
        image: Arc<ImageData>,
        (width, height, scale): (f32, f32, f32),
        view: zgx::scene::CanvasView,
        panned: Option<Panned>,
    ) -> Raster {
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
                panned,
            },
        );
        self.rasterizations += 1;
        Raster {
            id: image.id(),
            pixels: Some(image),
        }
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
                .get_panned(item.id, canvas, view, bounds.width, bounds.height, scale)
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
    // Exact scaling to rounded texture extents avoids a fractional final-pixel seam.
    let transform = sk::Transform::from_scale(w as f32 / width, h as f32 / height).pre_concat(
        sk::Transform::from_row(view.scale, 0., 0., view.scale, view.x, view.y),
    );
    rasterize_at(canvas, w, h, transform)
}
/// `canvas` drawn by `transform` (its points to pixels) into a `w` by `h`
/// pixel image.
fn rasterize_at(
    canvas: &Canvas,
    w: u32,
    h: u32,
    transform: sk::Transform,
) -> Result<ImageData, GpuError> {
    let mut pixmap =
        sk::Pixmap::new(w, h).ok_or_else(|| GpuError("canvas allocation failed".into()))?;
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
