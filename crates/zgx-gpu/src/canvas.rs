//! Shared bounded vector raster cache. Rasterization only follows changed content,
//! allocated size or device scale; GPU upload uses ordinary retained image identity.
use crate::GpuError;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
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
    /// Whether it is a raster of another zoom, scaled, while this zoom's is
    /// made off the frame: the host is woken when it is
    /// (`zgx::image::wake_waiting`), to draw again.
    pub scaled: bool,
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
                scaled: false,
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
            scaled: false,
        })
    }
    /// Where it shows, drawn under `drawn`, scaled and moved to stand for
    /// the drawing under `view` (another zoom), in a box `width` by
    /// `height` points at `scale`.
    fn scaled(
        &self,
        drawn: zgx::scene::CanvasView,
        view: zgx::scene::CanvasView,
        width: f32,
        height: f32,
        scale: f32,
    ) -> Placement {
        let k = view.scale / drawn.scale;
        let (ox, oy) = (
            (view.x - drawn.x * k) * scale,
            (view.y - drawn.y * k) * scale,
        );
        let [x0, y0, x1, y1] = self.region.map(|v| v as f32);
        let at = [x0 * k + ox, y0 * k + oy, x1 * k + ox, y1 * k + oy];
        let shown = [
            at[0].max(0.),
            at[1].max(0.),
            at[2].min(width * scale),
            at[3].min(height * scale),
        ];
        if is_empty(self.region) || shown[0] >= shown[2] || shown[1] >= shown[3] {
            return Placement {
                rect: [0.; 4],
                uv: [0.; 4],
                scaled: true,
            };
        }
        let (w, h) = (at[2] - at[0], at[3] - at[1]);
        Placement {
            rect: [
                shown[0] / scale,
                shown[1] / scale,
                (shown[2] - shown[0]) / scale,
                (shown[3] - shown[1]) / scale,
            ],
            uv: [
                (shown[0] - at[0]) / w,
                (shown[1] - at[1]) / h,
                (shown[2] - shown[0]) / w,
                (shown[3] - shown[1]) / h,
            ],
            scaled: true,
        }
    }
}
/// A raster for a new zoom being made off the frame (`get_panned`).
struct Zoom {
    canvas: Arc<Canvas>,
    view: zgx::scene::CanvasView,
    /// The box's width and height, in points, and its scale.
    size: (f32, f32, f32),
    /// The region and its pixels, once made.
    done: Arc<Mutex<Option<Made>>>,
}
type Made = Result<(Panned, ImageData), GpuError>;
type Job = Box<dyn FnOnce() + Send>;
/// Run `job` on the thread rasterizing zooms, one at a time.
fn off_frame(job: Job) {
    static WORKER: OnceLock<Mutex<std::sync::mpsc::Sender<Job>>> = OnceLock::new();
    let sender = WORKER.get_or_init(|| {
        let (send, receive) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("zgx canvas zoom".into())
            .spawn(move || {
                for job in receive {
                    job();
                }
            })
            .expect("a canvas rasterizing thread");
        Mutex::new(send)
    });
    let _ = sender.lock().unwrap_or_else(|p| p.into_inner()).send(job);
}
/// The fonts a window's text is shaped in, as its context was given them
/// (`GpuContext::add_font`, and `replace_font`, which replaces its
/// families' faces), and how many there have been: what a canvas drawing
/// its text into its raster (`Canvas::raster_text`) shapes it with, as the
/// window shapes its text nodes.
#[derive(Clone, Default)]
pub struct CanvasFonts(Arc<Mutex<FontsGiven>>);
/// How many fonts have been given, and each with whether it replaces its
/// families' faces.
type FontsGiven = (u64, Vec<(crate::text::FontData, bool)>);
impl CanvasFonts {
    pub(crate) fn add(&self, font: &crate::text::FontData, replace: bool) {
        let mut held = self.0.lock().unwrap_or_else(|p| p.into_inner());
        held.0 += 1;
        held.1.push((font.clone(), replace));
    }
}
/// Where each of a drawing's texts lays out (`label_box`).
type Boxes = Arc<[(f32, f32, (f32, f32))]>;
/// What a canvas's text is drawn into its raster with: the CPU's text
/// rasterizer, in the system's fonts and the window's (made again when
/// those change), and each drawing's text boxes, measured once. Shared with
/// the thread making zooms off the frame.
#[derive(Default)]
struct TextRaster {
    fonts: CanvasFonts,
    /// The fonts it was made in, and how many had been given then: made
    /// again when either changes (another window's context gives others).
    loaded: Option<(CanvasFonts, u64)>,
    raster: Option<crate::text::CpuTextRaster>,
    boxes: HashMap<usize, (Weak<Canvas>, Boxes)>,
}
type SharedText = Arc<Mutex<TextRaster>>;
impl TextRaster {
    fn raster(&mut self) -> &mut crate::text::CpuTextRaster {
        let (serial, fonts) = {
            let held = self.fonts.0.lock().unwrap_or_else(|p| p.into_inner());
            (held.0, held.1.clone())
        };
        let current = self
            .loaded
            .as_ref()
            .is_some_and(|(made, at)| Arc::ptr_eq(&made.0, &self.fonts.0) && *at == serial);
        if self.raster.is_none() || !current {
            let mut text = crate::text::CpuTextRaster::default();
            for (font, replace) in &fonts {
                match replace {
                    true => text.replace_font(font),
                    false => text.register_font(font),
                }
            }
            self.raster = Some(text);
            self.loaded = Some((self.fonts.clone(), serial));
            self.boxes.clear();
        }
        self.raster.as_mut().expect("made just now")
    }
    /// `canvas`'s text boxes, measured the first time it is asked.
    fn boxes(&mut self, canvas: &Arc<Canvas>) -> Boxes {
        self.raster();
        let key = Arc::as_ptr(canvas) as usize;
        if let Some((drawing, boxes)) = self.boxes.get(&key)
            && drawing.upgrade().is_some_and(|d| Arc::ptr_eq(&d, canvas))
        {
            return boxes.clone();
        }
        self.boxes
            .retain(|_, (drawing, _)| drawing.strong_count() > 0);
        let text = self.raster.as_mut().expect("made above");
        let boxes: Boxes = canvas
            .texts()
            .iter()
            .map(|label| label_box(label, text))
            .collect();
        self.boxes
            .insert(key, (Arc::downgrade(canvas), boxes.clone()));
        boxes
    }
}
/// What `canvas`'s text covers (x0, y0, x1, y1, in its points), a line's
/// size around its box for what its glyphs reach past it: `None` for none,
/// or if it is not drawn into the raster.
fn text_bounds(canvas: &Arc<Canvas>, text: &SharedText) -> Option<[f32; 4]> {
    if !canvas.rasters_text() || canvas.texts().is_empty() {
        return None;
    }
    let boxes = text.lock().unwrap_or_else(|p| p.into_inner()).boxes(canvas);
    let mut out: Option<[f32; 4]> = None;
    for (index, (x, y, (w, h))) in boxes.iter().enumerate() {
        let reach = canvas.texts()[index].size;
        let t = canvas.text_transform(index);
        for (px, py) in [(*x, *y), (x + w, *y), (*x, y + h), (x + w, y + h)] {
            let (px, py) = t.point(px, py);
            if !px.is_finite() || !py.is_finite() {
                continue;
            }
            let [x0, y0, x1, y1] = out.get_or_insert([px, py, px, py]);
            *x0 = x0.min(px - reach);
            *y0 = y0.min(py - reach);
            *x1 = x1.max(px + reach);
            *y1 = y1.max(py + reach);
        }
    }
    out
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
            Draw::Image { rect, .. } => within(&Path::rectangle(*rect), 0., (0., 0.)),
            // Grown by its spread and blurred three sigmas out.
            Draw::Shadow {
                path,
                offset,
                blur,
                spread,
                ..
            } => within(
                path,
                spread.max(0.) + blur.max(0.) * 3.,
                (offset.x, offset.y),
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
    text: Option<[f32; 4]>,
) -> Panned {
    let bounds = match (painted_bounds(canvas), text) {
        (Some(a), Some(b)) => Some([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]),
        (a, b) => a.or(b),
    };
    // Two pixels more each way, for antialiasing.
    let painted = bounds.map_or([0; 4], |[x0, y0, x1, y1]| {
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
    canvas: &Arc<Canvas>,
    view: zgx::scene::CanvasView,
    scale: f32,
    region: [i32; 4],
    old: Option<([i32; 4], &ImageData)>,
    text: &SharedText,
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
    // Its text in its raster: drawn with its shapes, in their layers.
    let mut labels = (canvas.rasters_text() && !canvas.texts().is_empty()).then(|| {
        let mut held = text.lock().unwrap_or_else(|p| p.into_inner());
        let boxes = held.boxes(canvas);
        (held, boxes)
    });
    let mut drawn = |[ax, ay, bx, by]: [i32; 4]| {
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
        let (w, h) = ((bx - ax) as u32, (by - ay) as u32);
        match &mut labels {
            None => rasterize_at(canvas, w, h, transform),
            Some((held, boxes)) => {
                let pixmap = sk::Pixmap::new(w, h)
                    .ok_or_else(|| GpuError("canvas allocation failed".into()))?;
                let at = Placed {
                    view,
                    scale,
                    origin: (ax, ay),
                    transform,
                };
                straight(paint_with_text(canvas, pixmap, &at, boxes, held.raster())?)
            }
        }
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
    /// Rasters for new zooms being made off the frame, by node.
    zooms: HashMap<NodeId, Zoom>,
    bytes: usize,
    clock: u64,
    rasterizations: u64,
    /// What canvases drawing their text into their raster draw it with.
    text: SharedText,
}
impl CanvasCache {
    /// Draw the text of canvases that draw it into their raster
    /// (`Canvas::raster_text`) in `fonts` (and the system's), as the window
    /// shapes its text.
    pub fn set_fonts(&mut self, fonts: &CanvasFonts) {
        let mut held = self.text.lock().unwrap_or_else(|p| p.into_inner());
        if !Arc::ptr_eq(&held.fonts.0, &fonts.0) {
            held.fonts = fonts.clone();
        }
    }
    /// Forget the rasters of nodes that are gone or no longer draw one.
    pub fn retain(&mut self, scene: &Scene) {
        self.entries.retain(|id, _| draws_raster(scene, *id));
        self.zooms.retain(|id, _| draws_raster(scene, *id));
        self.bytes = self.entries.values().map(|e| e.bytes).sum();
    }
    /// Whether it holds no raster.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// `retain` for the nodes `changed` alone (`Scene::resource_changes`).
    pub fn retain_changed(&mut self, scene: &Scene, changed: &[NodeId]) {
        for &node in changed {
            if (self.entries.contains_key(&node) || self.zooms.contains_key(&node))
                && !draws_raster(scene, node)
            {
                self.forget(node);
            }
        }
    }
    /// Forget the rasters of nodes `keep` says no to.
    pub fn retain_nodes(&mut self, keep: impl Fn(NodeId) -> bool) {
        self.entries.retain(|id, _| keep(*id));
        self.zooms.retain(|id, _| keep(*id));
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
        self.zooms.remove(&node);
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
    /// kept beside the rest (to the limits). A change rasterizes what
    /// shows, as `get_viewed` does. So does a zoom if `wait`; else the
    /// raster of the zoom before shows scaled (`Placement::scaled`) while
    /// this zoom's is made off the frame, and is swapped for it once made.
    #[allow(clippy::too_many_arguments)]
    pub fn get_panned(
        &mut self,
        node: NodeId,
        canvas: &Arc<Canvas>,
        view: zgx::scene::CanvasView,
        width: f32,
        height: f32,
        scale: f32,
        wait: bool,
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
        let same = |drawing: &Arc<Canvas>| Arc::ptr_eq(drawing, canvas) || **drawing == **canvas;
        // A zoom's raster made off the frame: the raster from now on, if
        // still of this drawing (else dropped). One that failed is made
        // here instead, its error returned.
        let mut wait = wait;
        let landed = self
            .zooms
            .get(&node)
            .and_then(|zoom| zoom.done.lock().unwrap_or_else(|p| p.into_inner()).take());
        if let Some(made) = landed {
            let zoom = self.zooms.remove(&node).expect("in flight");
            match made {
                Ok((panned, image))
                    if zoom.size == (width, height, scale) && same(&zoom.canvas) =>
                {
                    self.store(
                        node,
                        canvas,
                        Arc::new(image),
                        zoom.size,
                        zoom.view,
                        Some(panned),
                    );
                }
                Ok(_) => {}
                Err(_) => wait = true,
            }
        }
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
        // Zoomed: the raster of the zoom before, scaled, while this one's
        // is made off the frame (unless one is already being made).
        if !wait
            && let Some(entry) = self.entries.get_mut(&node)
            && let Some(panned) = entry.panned
            && entry.view.scale != view.scale
            && entry.width == width
            && entry.height == height
            && entry.scale == scale
            && entry.canvas.upgrade().is_some_and(|old| same(&old))
        {
            entry.used = self.clock;
            let shown = (
                entry.raster(),
                panned.scaled(entry.view, view, width, height, scale),
            );
            if let std::collections::hash_map::Entry::Vacant(slot) = self.zooms.entry(node) {
                let done: Arc<Mutex<Option<Made>>> = Arc::default();
                let (filled, drawing) = (done.clone(), canvas.clone());
                // The thread's own rasterizer, in the same fonts: a frame
                // drawing text meanwhile never waits on it.
                let fonts = self
                    .text
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .fonts
                    .clone();
                off_frame(Box::new(move || {
                    static TEXT: OnceLock<SharedText> = OnceLock::new();
                    let text = TEXT.get_or_init(Default::default);
                    text.lock().unwrap_or_else(|p| p.into_inner()).fonts = fonts;
                    let bounds = text_bounds(&drawing, text);
                    let panned = panned_region(&drawing, view, size, scale, bounds);
                    let made = rasterize_region(&drawing, view, scale, panned.region, None, text)
                        .map(|image| (panned, image));
                    *filled.lock().unwrap_or_else(|p| p.into_inner()) = Some(made);
                    zgx::image::wake_waiting();
                }));
                let zoom = Zoom {
                    canvas: canvas.clone(),
                    view,
                    size: (width, height, scale),
                    done,
                };
                slot.insert(zoom);
            }
            return Ok(shown);
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
                    &self.text,
                )?;
                (*drawn, panned, image, *shift)
            }
            None => {
                let bounds = text_bounds(canvas, &self.text);
                let panned = panned_region(canvas, view, size, scale, bounds);
                let image = rasterize_region(canvas, view, scale, panned.region, None, &self.text)?;
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
/// `image` drawn in `rect` as `fit` fills it, centred, cut to `rect`
/// rounded by `radius`, at `opacity` (`Draw::Image`). A picture with no
/// pixels to read (a GPU view's frame) draws nothing.
fn draw_image(
    pixmap: &mut sk::Pixmap,
    image: &ImageData,
    (rect, fit, radius, opacity): (zgx::scene::Rect, zgx::style::ObjectFit, f32, f32),
    transform: sk::Transform,
    clip: Option<&sk::Mask>,
) -> Result<(), GpuError> {
    use zgx::style::ObjectFit;
    if ![rect.x, rect.y, rect.width, rect.height, radius, opacity]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(GpuError("invalid canvas image".into()));
    }
    let (source_w, source_h) = (image.width() as f32, image.height() as f32);
    let pixels = image.pixels();
    if rect.width <= 0.
        || rect.height <= 0.
        || opacity <= 0.
        || pixels.len() != image.width() as usize * image.height() as usize * 4
    {
        return Ok(());
    }
    // As `ui.image` fits its pixels (a pixel a point at `None`).
    let (horizontal, vertical) = (rect.width / source_w, rect.height / source_h);
    let (width, height) = match fit {
        ObjectFit::Fill => (rect.width, rect.height),
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
    let placed = zgx::scene::Rect::new(
        rect.x + (rect.width - width) / 2.,
        rect.y + (rect.height - height) / 2.,
        width,
        height,
    );
    let Some(shown) = placed.intersection(rect) else {
        return Ok(());
    };
    let mut premultiplied = pixels.to_vec();
    if !image.is_premultiplied() {
        for p in premultiplied.as_chunks_mut::<4>().0 {
            let alpha = u16::from(p[3]);
            for channel in &mut p[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
    }
    let size = sk::IntSize::from_wh(image.width(), image.height())
        .ok_or_else(|| GpuError("invalid canvas image".into()))?;
    let picture = sk::Pixmap::from_vec(premultiplied, size)
        .ok_or_else(|| GpuError("invalid canvas image".into()))?;
    // Sampled linearly, as the GPU samples `ui.image`'s pixels (bicubic took
    // three times as long here).
    let paint = sk::Paint {
        shader: sk::Pattern::new(
            picture.as_ref(),
            sk::SpreadMode::Pad,
            sk::FilterQuality::Bilinear,
            opacity.min(1.),
            sk::Transform::from_row(
                width / source_w,
                0.,
                0.,
                height / source_h,
                placed.x,
                placed.y,
            ),
        ),
        ..Default::default()
    };
    let shown_path = sk::PathBuilder::from_rect(
        sk::Rect::from_xywh(shown.x, shown.y, shown.width, shown.height)
            .ok_or_else(|| GpuError("invalid canvas image".into()))?,
    );
    if radius <= 0. {
        pixmap.fill_path(&shown_path, &paint, sk::FillRule::Winding, transform, clip);
        return Ok(());
    }
    let Some(corners) = Path::rounded_rect(rect, radius)
        .ok()
        .as_ref()
        .and_then(path)
    else {
        return Ok(());
    };
    if shown == rect {
        pixmap.fill_path(&corners, &paint, sk::FillRule::Winding, transform, clip);
        return Ok(());
    }
    // Smaller than its box (`Contain`): the picture, cut by the box's corners.
    let mut mask = match clip {
        Some(clip) => clip.clone(),
        None => {
            let mut mask = sk::Mask::new(pixmap.width(), pixmap.height())
                .ok_or_else(|| GpuError("canvas allocation failed".into()))?;
            mask.data_mut().fill(255);
            mask
        }
    };
    mask.intersect_path(&corners, sk::FillRule::Winding, true, transform);
    pixmap.fill_path(
        &shown_path,
        &paint,
        sk::FillRule::Winding,
        transform,
        Some(&mask),
    );
    Ok(())
}
/// The shadow `source` casts (`Draw::Shadow`), worked out in device
/// pixels: its coverage grown (or shrunk) by `spread`, blurred as a text
/// shadow is (`blur_coverage`), moved by `offset` and painted in `color`
/// through `clip`. Only what can reach the raster is worked out.
fn draw_shadow(
    pixmap: &mut sk::Pixmap,
    source: &Path,
    (color, offset, blur, spread): (zgx::scene::Color, Point, f32, f32),
    transform: sk::Transform,
    clip: Option<&sk::Mask>,
) -> Result<(), GpuError> {
    if ![offset.x, offset.y, blur, spread]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(GpuError("invalid canvas shadow".into()));
    }
    let Some(shape) = path(source).and_then(|shape| shape.transform(transform)) else {
        return Ok(());
    };
    if color.3 == 0 {
        return Ok(());
    }
    // Points to pixels: the raster's scale and the view's zoom.
    let scale = (transform.sx + transform.sy) / 2.;
    let (spread, sigma) = (spread * scale, blur.max(0.) * scale);
    let sigma = if sigma < 0.01 { 0. } else { sigma };
    let reach = (sigma * 3.).ceil();
    let (dx, dy) = (offset.x * transform.sx, offset.y * transform.sy);
    let bounds = shape.bounds();
    let grown = spread.max(0.) + 1.;
    let x0 = (bounds.left() + dx - grown).max(-reach).floor();
    let y0 = (bounds.top() + dy - grown).max(-reach).floor();
    let x1 = (bounds.right() + dx + grown)
        .min(pixmap.width() as f32 + reach)
        .ceil();
    let y1 = (bounds.bottom() + dy + grown)
        .min(pixmap.height() as f32 + reach)
        .ceil();
    if x1 <= x0 || y1 <= y0 {
        return Ok(());
    }
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    // BUDGET, or as much as the picture it falls on when that's bigger.
    let budget = (BUDGET as u64).max(u64::from(pixmap.width()) * u64::from(pixmap.height()) * 4);
    if (u64::from(w) * u64::from(h))
        .checked_mul(4)
        .is_none_or(|bytes| bytes > budget)
    {
        return Err(GpuError("canvas shadow exceeds its budget".into()));
    }
    let mut cast =
        sk::Pixmap::new(w, h).ok_or_else(|| GpuError("canvas allocation failed".into()))?;
    let at = sk::Transform::from_translate(dx - x0, dy - y0);
    let mut ink = sk::Paint::default();
    ink.set_color_rgba8(0, 0, 0, 255);
    cast.fill_path(&shape, &ink, sk::FillRule::Winding, at, None);
    // Grown by a band `spread` wide round its edge, or shrunk by clearing it.
    if spread != 0. {
        if spread < 0. {
            ink.blend_mode = sk::BlendMode::Clear;
        }
        let band = sk::Stroke {
            width: spread.abs() * 2.,
            line_join: sk::LineJoin::Round,
            ..Default::default()
        };
        cast.stroke_path(&shape, &ink, &band, at, None);
    }
    let (alpha, texels, origin, step) = if sigma > 0. {
        let coverage: Vec<f32> = cast.pixels().iter().map(|p| f32::from(p.alpha())).collect();
        let blurred = crate::blur_coverage(&coverage, (w, h), sigma);
        let reach = blurred.reach as f32;
        let step = blurred.size.0 / blurred.texels.0;
        (
            blurred.alpha,
            blurred.texels,
            (x0 - reach, y0 - reach),
            step as f32,
        )
    } else {
        let alpha = cast.pixels().iter().map(|p| p.alpha()).collect();
        (alpha, (w, h), (x0, y0), 1.)
    };
    // Over the part of the raster it reaches, each pixel's coverage read
    // between the texels (bilinearly, in 256ths), tinted, cut by the clip
    // and blended over what is there, in integers and one pass: through a
    // pixmap and `draw_pixmap` it took four times as long.
    let (tw, th) = (texels.0 as usize, texels.1 as usize);
    let (left, top) = (origin.0.max(0.) as u32, origin.1.max(0.) as u32);
    let right = (origin.0 + texels.0 as f32 * step).clamp(0., pixmap.width() as f32) as u32;
    let bottom = (origin.1 + texels.1 as f32 * step).clamp(0., pixmap.height() as f32) as u32;
    if right <= left || bottom <= top {
        return Ok(());
    }
    // Each pixel's centre in texels along an axis: the two texels around it
    // and how far it is from the first, in 256ths.
    let between = |from: u32, to: u32, origin: f32, last: usize| -> Vec<(usize, usize, u32)> {
        (from..to)
            .map(|p| {
                let t = ((p as f32 + 0.5 - origin) / step - 0.5).clamp(0., last as f32);
                let i = t as usize;
                (i, (i + 1).min(last), ((t - i as f32) * 256.) as u32)
            })
            .collect()
    };
    let columns = between(left, right, origin.0, tw - 1);
    let rows = between(top, bottom, origin.1, th - 1);
    let zgx::scene::Color(r, g, b, a) = color;
    let tint = [r, g, b].map(u32::from);
    let stride = pixmap.width() as usize;
    let (left, span) = (left as usize, (right - left) as usize);
    let clip = clip.map(|clip| clip.data());
    let data = pixmap.data_mut();
    // A row's coverage, 0 to 255 × 256.
    let mut line = vec![0u32; span];
    for (y, &(above, below, down)) in (top as usize..).zip(&rows) {
        let (above, below) = (&alpha[above * tw..][..tw], &alpha[below * tw..][..tw]);
        for (out, &(first, next, across)) in line.iter_mut().zip(&columns) {
            let mix = |texels: &[u8]| {
                u32::from(texels[first]) * (256 - across) + u32::from(texels[next]) * across
            };
            *out = (mix(above) * (256 - down) + mix(below) * down) >> 8;
        }
        let start = y * stride + left;
        let clipped = clip.map(|clip| &clip[start..start + span]);
        let pixels = &mut data[start * 4..(start + span) * 4];
        for (x, (pixel, &coverage)) in pixels
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(&line)
            .enumerate()
        {
            let coverage = match clipped {
                Some(clipped) => coverage * u32::from(clipped[x]) / 255,
                None => coverage,
            };
            let alpha = (u32::from(a) * coverage + 255 * 128) / (255 * 256);
            if alpha == 0 {
                continue;
            }
            // Source over, premultiplied.
            let under = 255 - alpha;
            for (k, channel) in pixel.iter_mut().enumerate() {
                let source = match tint.get(k) {
                    Some(c) => (c * alpha + 127) / 255,
                    None => alpha,
                };
                *channel = (source + (u32::from(*channel) * under + 127) / 255) as u8;
            }
        }
    }
    Ok(())
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
                .get_panned(
                    item.id,
                    canvas,
                    view,
                    bounds.width,
                    bounds.height,
                    scale,
                    true,
                )
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
    straight(paint(canvas, view, width, height, scale, None)?)
}
/// `canvas` as a `ui.canvas` shows it, alone: its shapes under `view` and
/// its text (as `CanvasLabels` lays it out) in the same layers a window
/// draws them in (`Canvas::layers`), each text cut by its clip's box
/// (`Canvas::text_clips`) and mapped by its transform, at `width` by
/// `height` logical points and `scale`, on `background` (transparent if
/// none). A picture of a drawing with no window or frame:
/// `ui.render_canvas`.
pub fn picture(
    canvas: &Canvas,
    view: zgx::scene::CanvasView,
    width: f32,
    height: f32,
    scale: f32,
    background: Option<zgx::scene::Color>,
    text: &mut crate::text::CpuTextRaster,
) -> Result<ImageData, GpuError> {
    picture_up_to(canvas, view, width, height, scale, background, text, BUDGET)
}
/// The most a picture made for a file may take (`picture_up_to`): 256 MiB of
/// pixels, 8192 by 8192 or a 1440 by 900 screen at 4x.
pub const FILE_BUDGET: usize = 256 * 1024 * 1024;
/// `picture` with its own cap on the raster's bytes: a picture written to a
/// file (`FILE_BUDGET`) isn't one a window keeps.
#[allow(clippy::too_many_arguments)]
pub fn picture_up_to(
    canvas: &Canvas,
    view: zgx::scene::CanvasView,
    width: f32,
    height: f32,
    scale: f32,
    background: Option<zgx::scene::Color>,
    text: &mut crate::text::CpuTextRaster,
    budget: usize,
) -> Result<ImageData, GpuError> {
    let mut pixmap = paint_up_to(
        &Canvas::new(),
        view,
        width,
        height,
        scale,
        background,
        budget,
    )?;
    if canvas.commands().len() > 65536 {
        return Err(GpuError(
            "invalid canvas dimensions or command count".into(),
        ));
    }
    let transform = device(&pixmap, width, height, view);
    let boxes: Vec<_> = canvas
        .texts()
        .iter()
        .map(|label| label_box(label, text))
        .collect();
    // `layers` asks the texts' sizes in their order.
    let mut sizes = boxes.iter().map(|(_, _, size)| *size);
    let layers = canvas.layers(|_| sizes.next().unwrap_or_default());
    let clips = canvas.text_clips();
    for layer in &layers {
        pixmap = match layers.len() {
            1 => paint_into(canvas, pixmap, transform)?,
            _ => paint_into(&canvas.layer(layer), pixmap, transform)?,
        };
        for index in layer.texts.clone() {
            let clip = clips.get(index).copied().flatten();
            let label = Label {
                text: &canvas.texts()[index],
                at: boxes[index],
                transform: canvas.text_transform(index),
            };
            let at = Placed {
                view,
                scale,
                origin: (0, 0),
                transform,
            };
            draw_label(&mut pixmap, &label, clip, &at, text);
        }
    }
    straight(pixmap)
}
/// A canvas's text to draw: it, its box's top left and size, and what it
/// is mapped by.
struct Label<'a> {
    text: &'a CanvasText,
    at: (f32, f32, (f32, f32)),
    transform: zgx::affine::Affine,
}
/// Where a drawing lands in a pixmap: under `view` at `scale`, the
/// pixmap's top left `origin` pixels into that, `transform` mapping the
/// drawing's points to the pixmap's pixels.
#[derive(Clone, Copy)]
struct Placed {
    view: zgx::scene::CanvasView,
    scale: f32,
    origin: (i32, i32),
    transform: sk::Transform,
}
/// `canvas`'s shapes and text in their layers (`Canvas::layers`), as
/// `picture` draws them, into `pixmap` placed `at`: each text cut by its
/// clip's box and mapped by its transform. A text that can't reach the
/// pixmap is left out (a strip a pan uncovers draws what falls in it).
fn paint_with_text(
    canvas: &Canvas,
    mut pixmap: sk::Pixmap,
    at: &Placed,
    boxes: &[(f32, f32, (f32, f32))],
    text: &mut crate::text::CpuTextRaster,
) -> Result<sk::Pixmap, GpuError> {
    let mut sizes = boxes.iter().map(|(_, _, size)| *size);
    let layers = canvas.layers(|_| sizes.next().unwrap_or_default());
    let clips = canvas.text_clips();
    let (view, scale) = (at.view, at.scale);
    let zoom = scale * view.scale;
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    for layer in &layers {
        pixmap = match layers.len() {
            1 => paint_into(canvas, pixmap, at.transform)?,
            _ => paint_into(&canvas.layer(layer), pixmap, at.transform)?,
        };
        for index in layer.texts.clone() {
            let label = Label {
                text: &canvas.texts()[index],
                at: boxes[index],
                transform: canvas.text_transform(index),
            };
            if label.transform == zgx::affine::Affine::IDENTITY {
                let (x, y, (tw, th)) = label.at;
                let reach = label.text.size * zoom + 2.;
                let left = (view.x + x * view.scale) * scale - at.origin.0 as f32;
                let top = (view.y + y * view.scale) * scale - at.origin.1 as f32;
                if left + tw * zoom + reach < 0.
                    || top + th * zoom + reach < 0.
                    || left - reach > w
                    || top - reach > h
                {
                    continue;
                }
            }
            let clip = clips.get(index).copied().flatten();
            draw_label(&mut pixmap, &label, clip, at, text);
        }
    }
    Ok(pixmap)
}
/// `label` drawn over `pixmap` (whose pixels `transform` maps the canvas's
/// points to), cut by `clip`'s box, as a window shows it: its glyphs placed
/// as the GPU places them, on whole pixels below 2x.
fn draw_label(
    pixmap: &mut sk::Pixmap,
    label: &Label,
    clip: Option<zgx::canvas::TextClip>,
    at: &Placed,
    text: &mut crate::text::CpuTextRaster,
) {
    let Placed {
        view,
        scale,
        origin,
        transform,
    } = *at;
    let (x, y, size) = label.at;
    let zoom = scale * view.scale;
    let glyphs = rasterize_scaled(
        text,
        &label.text.text,
        label.text.size,
        size.0,
        size.1,
        &label.text.font,
        zoom,
    );
    let mask = clip.and_then(|clip| {
        let mut mask = sk::Mask::new(pixmap.width(), pixmap.height())?;
        let r = clip.rect;
        if r.width > 0. && r.height > 0. {
            let shape = Path::rounded_rect(r, clip.radius).ok()?;
            mask.fill_path(&path(&shape)?, sk::FillRule::Winding, true, transform);
        }
        Some(mask)
    });
    if label.transform == zgx::affine::Affine::IDENTITY {
        let left = ((view.x + x * view.scale) * scale).round() as i32 - origin.0;
        let top = ((view.y + y * view.scale) * scale).round() as i32 - origin.1;
        blend_glyphs(
            pixmap,
            (left, top),
            &glyphs,
            label.text,
            zoom,
            mask.as_ref(),
        );
        return;
    }
    // Turned: drawn upright into pixels of its own, then those mapped.
    let pad = (label.text.size * zoom).ceil() as i32 + 2;
    let (tw, th) = (
        (size.0 * zoom).ceil() as i32 + 2 * pad,
        (size.1 * zoom).ceil() as i32 + 2 * pad,
    );
    let Some(mut own) = sk::Pixmap::new(tw.max(1) as u32, th.max(1) as u32) else {
        return;
    };
    blend_glyphs(&mut own, (pad, pad), &glyphs, label.text, zoom, None);
    let t = label.transform;
    let placed = transform
        .pre_concat(sk::Transform::from_row(t.a, t.b, t.c, t.d, t.tx, t.ty))
        .pre_translate(x, y)
        .pre_scale(1. / zoom, 1. / zoom)
        .pre_translate(-pad as f32, -pad as f32);
    pixmap.draw_pixmap(
        0,
        0,
        own.as_ref(),
        &sk::PixmapPaint {
            quality: sk::FilterQuality::Bilinear,
            ..Default::default()
        },
        placed,
        mask.as_ref(),
    );
}
/// `glyphs` of `label`, its top left at `at` in `pixmap`'s pixels, blended
/// over what is there (premultiplied, as the pixmap is), each pixel's
/// coverage times `mask`'s where there is one.
fn blend_glyphs(
    pixmap: &mut sk::Pixmap,
    (left, top): (i32, i32),
    glyphs: &[(i32, i32, cosmic_text::SwashImage)],
    label: &CanvasText,
    zoom: f32,
    mask: Option<&sk::Mask>,
) {
    // A mask's coverage firmed below 2x by how light the text is, as
    // `draw.wgsl`'s `firm` does.
    let (r, g, b) = (label.color.0, label.color.1, label.color.2);
    let light = (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.;
    let firmness = if zoom < crate::SHARP_TEXT_BELOW {
        (light * 2.).clamp(0., 1.)
    } else {
        0.
    };
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    let pixels = pixmap.pixels_mut();
    for (gx, gy, image) in glyphs {
        let gx = left + gx + image.placement.left;
        let gy = top + gy - image.placement.top;
        for by in 0..image.placement.height as i32 {
            let py = gy + by;
            if py < 0 || py >= h {
                continue;
            }
            for bx in 0..image.placement.width as i32 {
                let px = gx + bx;
                if px < 0 || px >= w {
                    continue;
                }
                let i = (by * image.placement.width as i32 + bx) as usize;
                let (ink, coverage) = match image.content {
                    cosmic_text::SwashContent::Mask => {
                        let a = f32::from(image.data[i]) / 255.;
                        ((r, g, b), a + firmness * 0.67 * a * (1. - a))
                    }
                    cosmic_text::SwashContent::Color => (
                        (
                            image.data[i * 4],
                            image.data[i * 4 + 1],
                            image.data[i * 4 + 2],
                        ),
                        f32::from(image.data[i * 4 + 3]) / 255.,
                    ),
                    cosmic_text::SwashContent::SubpixelMask => {
                        ((r, g, b), f32::from(image.data[i * 4]) / 255.)
                    }
                };
                let cut = mask.map_or(1., |m| f32::from(m.data()[(py * w + px) as usize]) / 255.);
                let alpha = coverage * cut * f32::from(label.color.3) / 255.;
                if alpha <= 0. {
                    continue;
                }
                let under = pixels[(py * w + px) as usize];
                // Over what is there, premultiplied as the pixmap is.
                let over = |ink: u8, under: u8| {
                    (f32::from(ink) * alpha + f32::from(under) * (1. - alpha)).round() as u8
                };
                let a = over(255, under.alpha());
                pixels[(py * w + px) as usize] = sk::PremultipliedColorU8::from_rgba(
                    over(ink.0, under.red()).min(a),
                    over(ink.1, under.green()).min(a),
                    over(ink.2, under.blue()).min(a),
                    a,
                )
                .unwrap_or(under);
            }
        }
    }
}
/// Where `label` lays out, as `CanvasLabels` places its text node: its
/// box's top left and size. Its lines wrap at the box's width: a
/// paragraph's (`width`), else the text's own.
fn label_box(label: &CanvasText, text: &mut crate::text::CpuTextRaster) -> (f32, f32, (f32, f32)) {
    let wrap = label.width.filter(|w| w.is_finite() && *w > 0.);
    let (width, height) = text.measure(&label.text, label.size, wrap, &label.font);
    let size = (wrap.unwrap_or(width), height);
    let (x, y) = label.origin(size);
    (x, y, size)
}
/// `text` rasterized by `raster` at `scale` device pixels a point, each
/// glyph placed as the GPU renderer places it (`styled_glyphs`: below 2x on
/// whole pixels): its position and image in pixels.
fn rasterize_scaled(
    raster: &mut crate::text::CpuTextRaster,
    text: &str,
    size: f32,
    width: f32,
    height: f32,
    font: &zgx::text_layout::FontStyle,
    scale: f32,
) -> Vec<(i32, i32, cosmic_text::SwashImage)> {
    use crate::text::{glyph_image, set_buffer_text, wrap_width};
    use cosmic_text::{Buffer, Metrics};
    let (fonts, swash) = raster.parts();
    let size = size.max(1.);
    let mut buffer = Buffer::new_empty(Metrics::new(size, font.line_height.resolve(size)));
    buffer.set_size(wrap_width(Some(width)), Some(height));
    set_buffer_text(fonts, &mut buffer, text, font, size);
    buffer.shape_until_scroll(fonts, false);
    let glyphs = crate::styled_glyphs(Vec::new(), buffer.layout_runs(), scale, |_, _| 0);
    glyphs
        .into_iter()
        .filter_map(|glyph| {
            let image = glyph_image(swash, fonts, glyph.key)?;
            Some((glyph.x as i32, glyph.y as i32, image))
        })
        .collect()
}
/// `image` as a JPEG file's bytes at `quality` (1 to 100). JPEG keeps no
/// transparency: what's see-through is laid over white.
#[cfg(feature = "jpeg")]
pub fn jpeg(image: &ImageData, quality: u8) -> Result<Vec<u8>, GpuError> {
    let mut rgb = Vec::with_capacity(image.pixels().len() / 4 * 3);
    for px in image.pixels().as_chunks::<4>().0 {
        let a = u32::from(px[3]);
        for &c in &px[..3] {
            rgb.push(((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8);
        }
    }
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality.clamp(1, 100))
        .encode(
            &rgb,
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|e| GpuError(e.to_string()))?;
    Ok(bytes)
}
/// `image` as a WebP file's bytes, lossless (8-bit RGBA).
#[cfg(feature = "webp")]
pub fn webp(image: &ImageData) -> Result<Vec<u8>, GpuError> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
        .write_image(
            image.pixels(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| GpuError(e.to_string()))?;
    Ok(bytes)
}
/// `image` as a PNG file's bytes (8-bit RGBA).
#[cfg(feature = "png")]
pub fn png(image: &ImageData) -> Result<Vec<u8>, GpuError> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.pixels(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| GpuError(e.to_string()))?;
    Ok(bytes)
}
/// `canvas`'s shapes drawn into premultiplied pixels over `background`.
fn paint(
    canvas: &Canvas,
    view: zgx::scene::CanvasView,
    width: f32,
    height: f32,
    scale: f32,
    background: Option<zgx::scene::Color>,
) -> Result<sk::Pixmap, GpuError> {
    paint_up_to(canvas, view, width, height, scale, background, BUDGET)
}
/// `paint` with its own cap on the raster's bytes (`picture_up_to`).
fn paint_up_to(
    canvas: &Canvas,
    view: zgx::scene::CanvasView,
    width: f32,
    height: f32,
    scale: f32,
    background: Option<zgx::scene::Color>,
    budget: usize,
) -> Result<sk::Pixmap, GpuError> {
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
            .is_none_or(|bytes| bytes > budget as u64)
    {
        return Err(GpuError(format!(
            "canvas raster exceeds {} MiB",
            budget >> 20
        )));
    }
    let mut pixmap =
        sk::Pixmap::new(w, h).ok_or_else(|| GpuError("canvas allocation failed".into()))?;
    if let Some(background) = background {
        pixmap.fill(color(background));
    }
    // Exact scaling to rounded texture extents avoids a fractional final-pixel seam.
    let transform = sk::Transform::from_scale(w as f32 / width, h as f32 / height).pre_concat(
        sk::Transform::from_row(view.scale, 0., 0., view.scale, view.x, view.y),
    );
    paint_into(canvas, pixmap, transform)
}
/// What maps a canvas's points under `view` to `pixmap`'s pixels, for a
/// drawing `width` by `height` points: exact scaling to the rounded pixel
/// extents avoids a fractional final-pixel seam.
fn device(
    pixmap: &sk::Pixmap,
    width: f32,
    height: f32,
    view: zgx::scene::CanvasView,
) -> sk::Transform {
    sk::Transform::from_scale(
        pixmap.width() as f32 / width,
        pixmap.height() as f32 / height,
    )
    .pre_concat(sk::Transform::from_row(
        view.scale, 0., 0., view.scale, view.x, view.y,
    ))
}
/// `canvas` drawn by `transform` (its points to pixels) into a `w` by `h`
/// pixel image.
fn rasterize_at(
    canvas: &Canvas,
    w: u32,
    h: u32,
    transform: sk::Transform,
) -> Result<ImageData, GpuError> {
    let pixmap =
        sk::Pixmap::new(w, h).ok_or_else(|| GpuError("canvas allocation failed".into()))?;
    straight(paint_into(canvas, pixmap, transform)?)
}
/// `canvas`'s shapes drawn by `transform` into `pixmap`.
fn paint_into(
    canvas: &Canvas,
    mut pixmap: sk::Pixmap,
    transform: sk::Transform,
) -> Result<sk::Pixmap, GpuError> {
    let (w, h) = (pixmap.width(), pixmap.height());
    // Clips' masks: BUDGET in all, or for a picture bigger than that (one
    // for a file, `picture_up_to`) twice its pixels' bytes.
    let pixels = u64::from(w) * u64::from(h) * 4;
    let mask_budget = if pixels > BUDGET as u64 {
        pixels * 2
    } else {
        BUDGET as u64
    };
    let mut masks: Vec<sk::Mask> = Vec::new();
    for command in canvas.commands() {
        match command {
            Draw::Clip { path: source, rule } => {
                if masks.len() >= 16
                    || (u64::from(w) * u64::from(h))
                        .checked_mul((masks.len() + 1) as u64)
                        .is_none_or(|bytes| bytes > mask_budget)
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
            Draw::Image {
                image,
                rect,
                fit,
                radius,
                opacity,
            } => {
                draw_image(
                    &mut pixmap,
                    image,
                    (*rect, *fit, *radius, *opacity),
                    transform,
                    masks.last(),
                )?;
                continue;
            }
            Draw::Shadow {
                path,
                color,
                offset,
                blur,
                spread,
            } => {
                draw_shadow(
                    &mut pixmap,
                    path,
                    (*color, *offset, *blur, *spread),
                    transform,
                    masks.last(),
                )?;
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
    Ok(pixmap)
}
/// Premultiplied pixels as an image's straight ones.
fn straight(pixmap: sk::Pixmap) -> Result<ImageData, GpuError> {
    let (w, h) = (pixmap.width(), pixmap.height());
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
mod image_tests {
    use super::*;
    use zgx::{scene::Rect, style::ObjectFit};
    /// An 8×4 picture, its left half red and its right half blue: sampled
    /// a few pixels from the middle, each colour alone.
    fn picture() -> Arc<ImageData> {
        let pixels = (0..32)
            .flat_map(|i| match i % 8 < 4 {
                true => [255, 0, 0, 255],
                false => [0, 0, 255, 255],
            })
            .collect::<Vec<u8>>();
        Arc::new(ImageData::new(8, 4, pixels).unwrap())
    }
    /// `draw` rasterized at 20 by 10 points, one pixel a point.
    fn drawn(draw: impl FnOnce(&mut Canvas)) -> ImageData {
        let mut canvas = Canvas::new();
        draw(&mut canvas);
        rasterize(&canvas, 20., 10., 1.).unwrap()
    }
    fn at(image: &ImageData, x: u32, y: u32) -> [u8; 4] {
        let i = (y * image.width() + x) as usize * 4;
        image.pixels()[i..i + 4].try_into().unwrap()
    }
    #[test]
    fn a_picture_fills_its_box_as_its_fit_says() {
        let fill = drawn(|c| {
            c.image(
                picture(),
                Rect::new(0., 0., 20., 10.),
                ObjectFit::Fill,
                0.,
                1.,
            );
        });
        assert_eq!(at(&fill, 2, 5), [255, 0, 0, 255]);
        assert_eq!(at(&fill, 17, 5), [0, 0, 255, 255]);
        // Cover: 20 by 10 in a 10 by 10 box, its middle shown, cut to it.
        let cover = drawn(|c| {
            c.image(
                picture(),
                Rect::new(0., 0., 10., 10.),
                ObjectFit::Cover,
                0.,
                1.,
            );
        });
        assert_eq!(at(&cover, 1, 5), [255, 0, 0, 255]);
        assert_eq!(at(&cover, 9, 5), [0, 0, 255, 255]);
        assert_eq!(at(&cover, 15, 5)[3], 0, "nothing past the box");
        // Contain: 10 by 5, centred down the box, nothing above or below.
        let contain = drawn(|c| {
            c.image(
                picture(),
                Rect::new(0., 0., 10., 10.),
                ObjectFit::Contain,
                0.,
                1.,
            );
        });
        assert_eq!(at(&contain, 2, 5), [255, 0, 0, 255]);
        assert_eq!(at(&contain, 2, 1)[3], 0);
        assert_eq!(at(&contain, 2, 8)[3], 0);
    }
    #[test]
    fn a_picture_is_cut_to_its_corners_faded_and_clipped() {
        let rounded = drawn(|c| {
            c.image(
                picture(),
                Rect::new(0., 0., 20., 10.),
                ObjectFit::Fill,
                5.,
                1.,
            );
        });
        assert_eq!(at(&rounded, 0, 0)[3], 0, "its corner cut");
        assert_eq!(at(&rounded, 10, 5)[3], 255);
        // Smaller than its box, still cut where it reaches a corner.
        let contained = drawn(|c| {
            c.image(
                picture(),
                Rect::new(0., 0., 20., 10.),
                ObjectFit::Contain,
                5.,
                1.,
            );
        });
        assert_eq!(at(&contained, 0, 0)[3], 0);
        assert_eq!(at(&contained, 10, 5)[3], 255);
        let faded = drawn(|c| {
            c.image(
                picture(),
                Rect::new(0., 0., 20., 10.),
                ObjectFit::Fill,
                0.,
                0.5,
            );
        });
        let alpha = at(&faded, 2, 5)[3];
        assert!((126..=129).contains(&alpha), "{alpha}");
        assert_eq!(at(&faded, 2, 5)[..3], [255, 0, 0], "its colour kept");
        let clipped = drawn(|c| {
            c.clip(
                Path::rectangle(Rect::new(0., 0., 10., 10.)),
                FillRule::Winding,
            );
            c.image(
                picture(),
                Rect::new(0., 0., 20., 10.),
                ObjectFit::Fill,
                0.,
                1.,
            );
            c.restore();
        });
        assert_eq!(at(&clipped, 2, 5), [255, 0, 0, 255]);
        assert_eq!(at(&clipped, 17, 5)[3], 0, "outside the clip");
    }
}

#[cfg(test)]
mod shadow_tests {
    use super::*;
    use zgx::scene::{Color, Rect};
    const RED: Color = Color(255, 0, 0, 255);
    /// The shadow of a box at `rect`, `w` by `h` points, one pixel a point.
    fn cast(
        (w, h): (f32, f32),
        rect: Rect,
        (color, offset, blur, spread): (Color, Point, f32, f32),
    ) -> ImageData {
        let mut canvas = Canvas::new();
        canvas.shadow(Path::rectangle(rect), color, offset, blur, spread);
        rasterize(&canvas, w, h, 1.).unwrap()
    }
    fn at(image: &ImageData, x: u32, y: u32) -> [u8; 4] {
        let i = (y * image.width() + x) as usize * 4;
        image.pixels()[i..i + 4].try_into().unwrap()
    }
    #[test]
    fn a_sharp_shadow_is_the_shape_moved_grown_or_shrunk() {
        let rect = Rect::new(5., 2., 6., 4.);
        let moved = cast((20., 10.), rect, (RED, Point::new(4., 3.), 0., 0.));
        assert_eq!(at(&moved, 12, 7), [255, 0, 0, 255], "where it fell");
        assert_eq!(at(&moved, 6, 3)[3], 0, "not where the shape is");
        let rect = Rect::new(8., 3., 4., 4.);
        let grown = cast((20., 10.), rect, (RED, Point::new(0., 0.), 0., 2.));
        assert_eq!(at(&grown, 6, 5)[3], 255, "two points out");
        assert_eq!(at(&grown, 5, 5)[3], 0);
        let shrunk = cast((20., 10.), rect, (RED, Point::new(0., 0.), 0., -1.));
        assert_eq!(at(&shrunk, 8, 5)[3], 0, "a point in");
        assert_eq!(at(&shrunk, 10, 5)[3], 255);
        let faint = cast(
            (20., 10.),
            rect,
            (Color(0, 0, 0, 128), Point::new(0., 0.), 0., 0.),
        );
        assert_eq!(at(&faint, 10, 5), [0, 0, 0, 128], "its colour's alpha");
    }
    #[test]
    fn a_blurred_shadow_fades_past_its_edge_as_a_gaussian() {
        // Sigma 2: half at the edge, most of it inside, a little beyond.
        let rect = Rect::new(5., 2., 10., 6.);
        let soft = cast((20., 10.), rect, (RED, Point::new(0., 0.), 2., 0.));
        let alpha = |x, y| at(&soft, x, y)[3];
        assert!(alpha(10, 5) > 180, "{}", alpha(10, 5));
        assert!((90..170).contains(&alpha(5, 5)), "{}", alpha(5, 5));
        assert!((1..60).contains(&alpha(3, 5)), "{}", alpha(3, 5));
        assert!(alpha(0, 5) < 10, "{}", alpha(0, 5));
        assert_eq!(at(&soft, 10, 5)[..3], [255, 0, 0], "its colour kept");
        // Sigma 10, worked out at a quarter size, the same shape of fade.
        let rect = Rect::new(25., 25., 50., 50.);
        let wide = cast((100., 100.), rect, (RED, Point::new(0., 0.), 10., 0.));
        let alpha = |x, y| at(&wide, x, y)[3];
        assert!(alpha(50, 50) > 230, "{}", alpha(50, 50));
        assert!((100..160).contains(&alpha(25, 50)), "{}", alpha(25, 50));
        assert!(alpha(0, 0) < 10, "{}", alpha(0, 0));
        assert!(
            alpha(25, 50).abs_diff(alpha(74, 50)) < 12,
            "even on both sides: {} {}",
            alpha(25, 50),
            alpha(74, 50)
        );
    }
    #[test]
    fn a_shadow_is_clipped_and_drawn_under_what_follows() {
        let mut canvas = Canvas::new();
        canvas.clip(
            Path::rectangle(Rect::new(0., 0., 10., 10.)),
            FillRule::Winding,
        );
        canvas.shadow(
            Path::rectangle(Rect::new(2., 2., 16., 6.)),
            RED,
            Point::new(0., 0.),
            0.,
            0.,
        );
        canvas.restore();
        canvas.fill(
            Path::rectangle(Rect::new(4., 4., 2., 2.)),
            Color(0, 0, 255, 255),
        );
        let drawn = rasterize(&canvas, 20., 10., 1.).unwrap();
        assert_eq!(at(&drawn, 3, 3), [255, 0, 0, 255]);
        assert_eq!(at(&drawn, 15, 5)[3], 0, "outside the clip");
        assert_eq!(at(&drawn, 4, 4), [0, 0, 255, 255], "the shape over it");
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
