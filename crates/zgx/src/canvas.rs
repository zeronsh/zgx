//! Retained, backend-independent vector drawing in local logical coordinates.
use crate::{
    affine::Affine,
    scene::{Color, Rect},
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, OnceLock},
};

/// A point in the canvas's local logical coordinates, y down.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Across.
    pub x: f32,
    /// Down.
    pub y: f32,
}
impl Point {
    /// The point at `x`, `y`.
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}
/// One step of a path.
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    /// Start a new subpath here.
    Move(Point),
    /// A straight line to here.
    Line(Point),
    /// A quadratic Bézier curve through a control point to an end point.
    Quadratic(Point, Point),
    /// A cubic Bézier curve through two control points to an end point.
    Cubic(Point, Point, Point),
    /// Close the subpath with a line back to its start.
    Close,
}
/// A path known as a simple shape (made by its constructor, or a builder
/// given nothing else): a renderer may draw it as that shape, without
/// rasterizing its segments, which stay what it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A rectangle, its corners rounded by `radius` (0: square; at most
    /// half its shorter side).
    Rect {
        /// The rectangle.
        rect: Rect,
        /// Its corners' radius.
        radius: f32,
    },
    /// The ellipse inscribed in `rect` (a circle when it is square).
    Ellipse {
        /// Its bounds.
        rect: Rect,
    },
    /// A straight line from `from` to `to`.
    Line {
        /// Where it starts.
        from: Point,
        /// Where it ends.
        to: Point,
    },
}

/// An immutable vector path, cheap to clone. Its coordinates are finite.
#[derive(Clone, Debug, Default)]
pub struct Path {
    /// A shape's are made when first asked for: a renderer drawing it as
    /// the shape never needs them (a canvas of thousands of rectangles
    /// allocates none).
    segments: OnceLock<Arc<[Segment]>>,
    shape: Option<Shape>,
}
impl PartialEq for Path {
    fn eq(&self, other: &Self) -> bool {
        match (self.shape, other.shape) {
            // A shape's segments are its shape's.
            (Some(a), Some(b)) => a == b,
            _ => self.shape == other.shape && self.segments() == other.segments(),
        }
    }
}
impl Path {
    /// The path that is `shape` alone, if its coordinates are finite.
    fn of_shape(shape: Shape) -> Result<Self, &'static str> {
        let finite = |values: &[f32]| values.iter().all(|v| v.is_finite());
        let ok = match shape {
            Shape::Rect { rect, .. } | Shape::Ellipse { rect } => {
                finite(&[rect.x, rect.y, rect.x + rect.width, rect.y + rect.height])
            }
            Shape::Line { from, to } => finite(&[from.x, from.y, to.x, to.y]),
        };
        if !ok {
            return Err("path coordinates must be finite");
        }
        Ok(Self {
            segments: OnceLock::new(),
            shape: Some(shape),
        })
    }
    /// Start building a path.
    pub fn builder() -> PathBuilder {
        PathBuilder::default()
    }
    /// Its segments, in order.
    pub fn segments(&self) -> &[Segment] {
        self.segments.get_or_init(|| {
            let mut builder = PathBuilder::default();
            match self.shape {
                Some(Shape::Rect { rect, radius }) => {
                    builder.rounded_rect(rect, radius);
                }
                Some(Shape::Ellipse { rect }) => {
                    builder.ellipse(rect);
                }
                Some(Shape::Line { from, to }) => {
                    builder.move_to(from.x, from.y).line_to(to.x, to.y);
                }
                // Made with its segments.
                None => {}
            }
            builder.segments.into()
        })
    }
    /// The simple shape it is, if it was made as one.
    pub fn shape(&self) -> Option<Shape> {
        self.shape
    }
    /// The outline of `rect`.
    pub fn rectangle(rect: Rect) -> Self {
        Self::rounded_rect(rect, 0.).expect("finite rectangle")
    }
    /// The outline of `rect` with corners rounded by `radius` (at most half
    /// its shorter side), or an error unless both are finite.
    pub fn rounded_rect(rect: Rect, radius: f32) -> Result<Self, &'static str> {
        Self::of_shape(Shape::Rect {
            rect,
            radius: clamp_radius(rect, radius),
        })
    }
    /// The ellipse inscribed in `rect`, or an error unless it is finite.
    pub fn ellipse(rect: Rect) -> Result<Self, &'static str> {
        Self::of_shape(Shape::Ellipse { rect })
    }
    /// A straight line from `from` to `to`, or an error unless they are
    /// finite.
    pub fn line(from: Point, to: Point) -> Result<Self, &'static str> {
        Self::of_shape(Shape::Line { from, to })
    }
}
/// A corner radius as a rectangle takes it: at least 0, at most half its
/// shorter side.
fn clamp_radius(rect: Rect, radius: f32) -> f32 {
    radius
        .max(0.)
        .min(rect.width.abs() / 2.)
        .min(rect.height.abs() / 2.)
}
/// Builds a [`Path`] segment by segment; [`PathBuilder::build`] checks it.
#[derive(Default)]
pub struct PathBuilder {
    segments: Vec<Segment>,
    transform: Affine,
    /// The simple shape the path is, while it is nothing else.
    shape: Option<Shape>,
}
impl PathBuilder {
    /// A segment added: the path is no longer just its shape.
    fn push(&mut self, segment: Segment) -> &mut Self {
        self.segments.push(segment);
        self.shape = None;
        self
    }
    /// Start a new subpath at `x`, `y`.
    pub fn move_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.push(Segment::Move(Point::new(x, y)))
    }
    /// A straight line to `x`, `y`.
    pub fn line_to(&mut self, x: f32, y: f32) -> &mut Self {
        self.push(Segment::Line(Point::new(x, y)))
    }
    /// A quadratic curve through `cx`, `cy` to `x`, `y`.
    pub fn quadratic_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) -> &mut Self {
        self.push(Segment::Quadratic(Point::new(cx, cy), Point::new(x, y)))
    }
    /// A cubic curve through `c1` and `c2` to `end`.
    pub fn cubic_to(&mut self, c1: Point, c2: Point, end: Point) -> &mut Self {
        self.push(Segment::Cubic(c1, c2, end))
    }
    /// Add `path`'s segments.
    pub fn append(&mut self, path: &Path) -> &mut Self {
        let first = self.segments.is_empty();
        self.segments.extend_from_slice(path.segments());
        self.shape = if first { path.shape() } else { None };
        self
    }
    /// Close the subpath.
    pub fn close(&mut self) -> &mut Self {
        self.push(Segment::Close)
    }
    /// A rectangle's outline with corners rounded by `radius` (at most half
    /// its shorter side), as a closed subpath: on an empty builder, the path
    /// is that shape (`Path::shape`).
    pub fn rounded_rect(&mut self, rect: Rect, radius: f32) -> &mut Self {
        let first = self.segments.is_empty();
        let radius = clamp_radius(rect, radius);
        let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
        if radius > 0. {
            use std::f32::consts::{FRAC_PI_2, PI};
            self.move_to(x + radius, y).line_to(x + w - radius, y);
            self.arc(
                Point::new(x + w - radius, y + radius),
                radius,
                -FRAC_PI_2,
                FRAC_PI_2,
            );
            self.line_to(x + w, y + h - radius);
            self.arc(
                Point::new(x + w - radius, y + h - radius),
                radius,
                0.,
                FRAC_PI_2,
            );
            self.line_to(x + radius, y + h);
            self.arc(
                Point::new(x + radius, y + h - radius),
                radius,
                FRAC_PI_2,
                FRAC_PI_2,
            );
            self.line_to(x, y + radius);
            self.arc(Point::new(x + radius, y + radius), radius, PI, FRAC_PI_2);
        } else {
            self.move_to(x, y)
                .line_to(x + w, y)
                .line_to(x + w, y + h)
                .line_to(x, y + h);
        }
        self.close();
        if first {
            self.shape = Some(Shape::Rect { rect, radius });
        }
        self
    }
    /// The ellipse inscribed in `rect`, as a closed subpath of four cubic
    /// arcs: on an empty builder, the path is that shape.
    pub fn ellipse(&mut self, rect: Rect) -> &mut Self {
        let first = self.segments.is_empty();
        let k = 0.552_284_8;
        let (rx, ry) = (rect.width / 2., rect.height / 2.);
        let (cx, cy) = (rect.x + rx, rect.y + ry);
        self.move_to(cx + rx, cy);
        self.cubic_to(
            Point::new(cx + rx, cy + ry * k),
            Point::new(cx + rx * k, cy + ry),
            Point::new(cx, cy + ry),
        );
        self.cubic_to(
            Point::new(cx - rx * k, cy + ry),
            Point::new(cx - rx, cy + ry * k),
            Point::new(cx - rx, cy),
        );
        self.cubic_to(
            Point::new(cx - rx, cy - ry * k),
            Point::new(cx - rx * k, cy - ry),
            Point::new(cx, cy - ry),
        );
        self.cubic_to(
            Point::new(cx + rx * k, cy - ry),
            Point::new(cx + rx, cy - ry * k),
            Point::new(cx + rx, cy),
        );
        self.close();
        if first {
            self.shape = Some(Shape::Ellipse { rect });
        }
        self
    }
    /// Append a circular arc, connecting to its start with a line. Angles are
    /// clockwise radians; sweeps are bounded to one full turn in either direction.
    pub fn arc(&mut self, center: Point, radius: f32, start: f32, sweep: f32) -> &mut Self {
        if ![center.x, center.y, radius, start, sweep]
            .iter()
            .all(|v| v.is_finite())
            || radius < 0.
        {
            return self.line_to(f32::NAN, f32::NAN);
        }
        let sweep = sweep.clamp(-std::f32::consts::TAU, std::f32::consts::TAU);
        let steps = (sweep.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.) as usize;
        let delta = sweep / steps as f32;
        let first = Point::new(
            center.x + radius * start.cos(),
            center.y + radius * start.sin(),
        );
        if self.segments.is_empty() {
            self.move_to(first.x, first.y);
        } else {
            self.line_to(first.x, first.y);
        }
        for index in 0..steps {
            let a = start + index as f32 * delta;
            let b = a + delta;
            let k = 4. / 3. * (delta / 4.).tan();
            self.cubic_to(
                Point::new(
                    center.x + radius * (a.cos() - k * a.sin()),
                    center.y + radius * (a.sin() + k * a.cos()),
                ),
                Point::new(
                    center.x + radius * (b.cos() + k * b.sin()),
                    center.y + radius * (b.sin() - k * b.cos()),
                ),
                Point::new(center.x + radius * b.cos(), center.y + radius * b.sin()),
            );
        }
        self
    }
    /// Transform every point by `transform` when built, after any transform
    /// already set.
    pub fn transform(&mut self, transform: Affine) -> &mut Self {
        self.transform = self.transform.then(transform);
        self
    }
    /// The path, or an error if it has more than 65,536 segments or a point
    /// isn't finite after transforming.
    pub fn build(self) -> Result<Path, &'static str> {
        if self.segments.len() > 65536 {
            return Err("path exceeds 65536 segments");
        }
        let transform = self.transform;
        // Moved or scaled, a shape is that shape no longer: its points are.
        let shape = self.shape.filter(|_| transform == Affine::IDENTITY);
        let point = |p: Point| -> Result<Point, &'static str> {
            let (x, y) = transform.point(p.x, p.y);
            if x.is_finite() && y.is_finite() {
                Ok(Point::new(x, y))
            } else {
                Err("path coordinates must be finite")
            }
        };
        // A shape alone is kept as the shape, its segments made if asked.
        if let Some(shape) = shape {
            return Path::of_shape(shape);
        }
        let segments = self
            .segments
            .into_iter()
            .map(|s| {
                Ok(match s {
                    Segment::Move(p) => Segment::Move(point(p)?),
                    Segment::Line(p) => Segment::Line(point(p)?),
                    Segment::Quadratic(a, b) => Segment::Quadratic(point(a)?, point(b)?),
                    Segment::Cubic(a, b, c) => Segment::Cubic(point(a)?, point(b)?, point(c)?),
                    Segment::Close => Segment::Close,
                })
            })
            .collect::<Result<Vec<_>, &'static str>>()?;
        Ok(Path {
            segments: OnceLock::from(Arc::<[Segment]>::from(segments)),
            shape,
        })
    }
}
/// A colour at a point along a gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// Where, from 0 at the start to 1 at the end.
    pub offset: f32,
    /// The colour there.
    pub color: Color,
}
/// What fills or strokes a path.
#[derive(Clone, Debug, PartialEq)]
pub enum Brush {
    /// One colour.
    Solid(Color),
    /// A linear gradient from `start` to `end` (see [`Brush::linear`]).
    Linear {
        /// Where offset 0 lies.
        start: Point,
        /// Where offset 1 lies.
        end: Point,
        /// The colours along it, in offset order.
        stops: Arc<[GradientStop]>,
    },
    /// A radial gradient (see [`Brush::radial`]): offset 0 at the centre, 1
    /// on the circle of radius 1 about the origin, both placed by
    /// `transform`, and the last colour past it.
    Radial {
        /// Where the unit circle lies: an ellipse where it scales unevenly.
        transform: Affine,
        /// The colours out from the centre, in offset order.
        stops: Arc<[GradientStop]>,
    },
    /// A conic gradient (see [`Brush::conic`]): offset 0 along +x from the
    /// origin, rising clockwise to 1 a full turn round, both placed by
    /// `transform`.
    Conic {
        /// Where its centre and its first colour's direction lie.
        transform: Affine,
        /// The colours round it, in offset order.
        stops: Arc<[GradientStop]>,
    },
    /// Diagonal parallel stripes in logical coordinates.
    Slash {
        /// The colour between the stripes.
        background: Color,
        /// The stripes' colour.
        foreground: Color,
        /// The distance from one stripe to the next.
        spacing: f32,
        /// Each stripe's width.
        width: f32,
    },
}
impl From<Color> for Brush {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}
impl Brush {
    /// A linear gradient from `start` to `end`, or an error unless the points
    /// are finite and distinct and there are 2 to 1,024 stops with offsets
    /// from 0 to 1 in order.
    pub fn linear(
        start: Point,
        end: Point,
        stops: impl Into<Arc<[GradientStop]>>,
    ) -> Result<Self, &'static str> {
        let stops = stops.into();
        if ![start.x, start.y, end.x, end.y]
            .iter()
            .all(|v| v.is_finite())
            || start == end
            || !valid_stops(&stops)
        {
            return Err("invalid linear gradient");
        }
        Ok(Self::Linear { start, end, stops })
    }
    /// A radial gradient from offset 0 at the origin to 1 on the unit
    /// circle, both placed by `transform` (`Affine::scale(r, r)` then a
    /// translation to the centre for a circle of radius `r`), or an error
    /// unless `transform` is finite and invertible and the stops are as
    /// [`Brush::linear`] takes them.
    pub fn radial(
        transform: Affine,
        stops: impl Into<Arc<[GradientStop]>>,
    ) -> Result<Self, &'static str> {
        let stops = stops.into();
        if !placed(transform) || !valid_stops(&stops) {
            return Err("invalid radial gradient");
        }
        Ok(Self::Radial { transform, stops })
    }
    /// A conic gradient: offset 0 along +x from the origin, rising
    /// clockwise (as screen coordinates turn) to 1 a full turn round, all
    /// placed by `transform`; or an error as [`Brush::radial`]'s.
    pub fn conic(
        transform: Affine,
        stops: impl Into<Arc<[GradientStop]>>,
    ) -> Result<Self, &'static str> {
        let stops = stops.into();
        if !placed(transform) || !valid_stops(&stops) {
            return Err("invalid conic gradient");
        }
        Ok(Self::Conic { transform, stops })
    }
}
/// Whether a gradient's transform places it somewhere: finite, and
/// invertible (not flattened to a line).
fn placed(transform: Affine) -> bool {
    transform.is_finite() && transform.inverse().is_some()
}
/// 2 to 1,024 stops with offsets from 0 to 1 in order.
fn valid_stops(stops: &[GradientStop]) -> bool {
    (2..=1024).contains(&stops.len())
        && stops
            .iter()
            .all(|s| s.offset.is_finite() && (0. ..=1.).contains(&s.offset))
        && stops.windows(2).all(|s| s[0].offset <= s[1].offset)
}
/// Which parts of a self-overlapping path are inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FillRule {
    /// Inside where the path winds round a nonzero number of times (SVG's
    /// `nonzero`).
    #[default]
    Winding,
    /// Inside where a ray crosses the path an odd number of times.
    EvenOdd,
}
/// How a stroke's open ends look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineCap {
    /// Cut square at the end.
    #[default]
    Butt,
    /// Rounded past the end by half the width.
    Round,
    /// Squared off past the end by half the width.
    Square,
}
/// How a stroke turns its corners.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineJoin {
    /// A sharp point, beveled past `Stroke::miter_limit`.
    #[default]
    Miter,
    /// Rounded.
    Round,
    /// Cut off flat.
    Bevel,
}
/// How a path is stroked.
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    /// The line's width in logical points.
    pub width: f32,
    /// Its open ends.
    pub cap: LineCap,
    /// Its corners.
    pub join: LineJoin,
    /// How long a miter may be, in line widths, before it bevels.
    pub miter_limit: f32,
    /// Dash and gap lengths, alternating; empty draws a solid line. A dash
    /// may be 0 long: with round caps, a dot.
    pub dash: Vec<f32>,
    /// How far into the dash pattern the line starts.
    pub dash_offset: f32,
}
impl Default for Stroke {
    fn default() -> Self {
        Self {
            width: 1.,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.,
            dash: vec![],
            dash_offset: 0.,
        }
    }
}
/// One canvas drawing command.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    /// Clip later commands to `path` until the matching `Restore`.
    Clip {
        /// The clip's shape.
        path: Path,
        /// Which parts of it are inside.
        rule: FillRule,
    },
    /// End the most recent clip.
    Restore,
    /// Fill `path` with `brush`.
    Fill {
        /// The shape.
        path: Path,
        /// What fills it.
        brush: Brush,
        /// Which parts of it are inside.
        rule: FillRule,
    },
    /// Stroke `path` with `brush`.
    Stroke {
        /// The shape.
        path: Path,
        /// What the line is painted with.
        brush: Brush,
        /// The line's width, ends, corners and dashes.
        style: Stroke,
    },
    /// Draw `image` in `rect` (see [`Canvas::image`]).
    Image {
        /// The picture, drawn still.
        image: Arc<crate::image::ImageData>,
        /// The box it is drawn in.
        rect: Rect,
        /// How it fills the box, centred in it (CSS `object-fit`).
        fit: crate::style::ObjectFit,
        /// The box's corner radius: the picture is cut to its corners.
        radius: f32,
        /// How opaque it is, 0 to 1.
        opacity: f32,
    },
    /// The shadow `path` casts (see [`Canvas::shadow`]).
    Shadow {
        /// The shape casting it, filled by the nonzero rule.
        path: Path,
        /// Its colour.
        color: Color,
        /// How far it falls from the shape.
        offset: Point,
        /// Its blur's Gaussian sigma, in points; 0 draws it sharp.
        blur: f32,
        /// How far it grows past the shape, or below 0 shrinks into it.
        spread: f32,
    },
}
/// Where a text's point falls across it (`Canvas::text`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAnchor {
    /// The text starts there.
    #[default]
    Start,
    /// It is centred on it.
    Center,
    /// It ends there.
    End,
}
/// Where a text's point falls down it (`Canvas::text`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextBaseline {
    /// Its line's top.
    #[default]
    Top,
    /// Its line's middle.
    Middle,
    /// Its line's bottom.
    Bottom,
}
/// A line of text on a canvas, or a paragraph wrapped to `width`. It is
/// shaped and drawn as any text is, in the drawing's order (over the shapes
/// drawn before it, under those after that reach it: [`Canvas::layers`]),
/// cut by the box of the clips open where it is drawn
/// ([`Canvas::text_clips`]). Assistive technology doesn't
/// read it (a canvas's `label` speaks for the drawing).
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasText {
    /// The text.
    pub text: Arc<str>,
    /// Across, in the canvas's coordinates.
    pub x: f32,
    /// Down.
    pub y: f32,
    /// Its font size, in logical points.
    pub size: f32,
    /// Its colour.
    pub color: Color,
    /// Its font.
    pub font: crate::text_layout::FontStyle,
    /// Where `x` falls across it.
    pub anchor: TextAnchor,
    /// Where `y` falls down it.
    pub baseline: TextBaseline,
    /// Wrapped to lines at most this wide, in a box this wide that `anchor`
    /// places (`font.align` aligns the lines in it); `None`, one line, or
    /// one per `\n`.
    pub width: Option<f32>,
}
impl CanvasText {
    /// Its top left, given its size.
    pub fn origin(&self, (width, height): (f32, f32)) -> (f32, f32) {
        let x = match self.anchor {
            TextAnchor::Start => self.x,
            TextAnchor::Center => self.x - width / 2.,
            TextAnchor::End => self.x - width,
        };
        let y = match self.baseline {
            TextBaseline::Top => self.y,
            TextBaseline::Middle => self.y - height / 2.,
            TextBaseline::Bottom => self.y - height,
        };
        (x, y)
    }
}

thread_local! {
    /// The scene a canvas is drawing for: what `measure_text` measures in.
    static DRAWING_FOR: RefCell<Option<Rc<RefCell<crate::scene::Scene>>>> =
        const { RefCell::new(None) };
}
/// Runs `draw` with `scene` as what `measure_text` measures in: a canvas's
/// drawing, or one drawn with no window (`ui.render_canvas`).
pub fn drawing_for<R>(scene: &Rc<RefCell<crate::scene::Scene>>, draw: impl FnOnce() -> R) -> R {
    let previous = DRAWING_FOR.with(|slot| slot.replace(Some(scene.clone())));
    let drawn = draw();
    DRAWING_FOR.with(|slot| *slot.borrow_mut() = previous);
    drawn
}
/// The width and height `text` takes at `size` in `font`, while a canvas
/// draws: what placing a label around another needs. `None` outside a
/// canvas's drawing.
pub fn measure_text(
    text: &str,
    size: f32,
    font: &crate::text_layout::FontStyle,
) -> Option<(f32, f32)> {
    measure_text_in(text, size, None, font)
}
/// [`measure_text`] wrapped to lines at most `width` wide, as a
/// [`CanvasText`] with that `width` draws it: a paragraph's height.
pub fn measure_text_in(
    text: &str,
    size: f32,
    width: Option<f32>,
    font: &crate::text_layout::FontStyle,
) -> Option<(f32, f32)> {
    DRAWING_FOR.with(|slot| {
        let scene = slot.borrow();
        let scene = scene.as_ref()?;
        let scene = scene.try_borrow().ok()?;
        Some(scene.measure_text_with_font(text, size, width, font))
    })
}

/// The clip a canvas's text is cut by: the box its clips leave, its corners
/// rounded where that box is one rounded rectangle's or circle's
/// ([`Canvas::text_clips`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextClip {
    /// The box, in the canvas's coordinates; empty where the clips leave
    /// nothing.
    pub rect: Rect,
    /// Its corners' radius.
    pub radius: f32,
}
/// A run of a canvas's commands and the texts drawn over them, before the
/// next run's commands ([`Canvas::layers`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasLayer {
    /// Its commands' indices.
    pub commands: std::ops::Range<usize>,
    /// Its texts' indices.
    pub texts: std::ops::Range<usize>,
}
/// At most this many layers: past it, the texts left go over what follows,
/// as all once did.
const MAX_LAYERS: usize = 8;

/// Where a canvas's texts fall among its commands, whether it clips, and
/// the texts transformed: kept only once a command follows a text, a clip
/// opens or a text is transformed.
#[derive(Clone, Debug, Default, PartialEq)]
struct TextOrder {
    /// How many commands came before the first text, and each later text
    /// before which more had come (its index, and how many).
    first_text_after: usize,
    text_moves: Vec<(usize, usize)>,
    /// Whether a clip was opened.
    clipped: bool,
    /// The texts drawn transformed (`Canvas::text_transformed`): their
    /// indices, in order, and transforms.
    transformed: Vec<(usize, Affine)>,
}
impl TextOrder {
    /// How many commands came before the last text.
    fn last_text_after(&self) -> usize {
        self.text_moves
            .last()
            .map_or(self.first_text_after, |(_, after)| *after)
    }
}

/// Equal command lists preserve pixels, uploads and retained layout.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Canvas {
    commands: Vec<Draw>,
    /// Beside the commands: `Draw` is public and exhaustive. Empty, it
    /// allocates nothing.
    text: Vec<CanvasText>,
    /// None while every text comes after every command, nothing clips and
    /// no text is transformed, as a chart's labels and a terminal's rows:
    /// the canvas no larger than it was, and nothing allocated.
    order: Option<Box<TextOrder>>,
    /// Whether its text is drawn into its raster with its shapes
    /// (`Canvas::raster_text`), rather than as text nodes over it.
    raster_text: bool,
}
impl Canvas {
    /// An empty canvas.
    pub fn new() -> Self {
        Self::default()
    }
    /// The texts' order, kept from now on: while there was none, every
    /// text came after every command.
    fn order(&mut self) -> &mut TextOrder {
        let at = self.commands.len();
        self.order.get_or_insert_with(|| {
            Box::new(TextOrder {
                first_text_after: at,
                ..Default::default()
            })
        })
    }
    /// Add `command`, keeping the texts' order once it follows one.
    fn push(&mut self, command: Draw) {
        if matches!(command, Draw::Clip { .. }) {
            self.order().clipped = true;
        } else if !self.text.is_empty() && self.order.is_none() {
            self.order();
        }
        self.commands.push(command);
    }
    /// Fill `path` with `brush`, by the nonzero rule.
    pub fn fill(&mut self, path: Path, brush: impl Into<Brush>) -> &mut Self {
        self.fill_rule(path, brush, FillRule::Winding)
    }
    /// Fill `path` with `brush`, by `rule`.
    pub fn fill_rule(&mut self, path: Path, brush: impl Into<Brush>, rule: FillRule) -> &mut Self {
        self.push(Draw::Fill {
            path,
            brush: brush.into(),
            rule,
        });
        self
    }
    /// Stroke `path` with `brush` and `style`.
    pub fn stroke(&mut self, path: Path, brush: impl Into<Brush>, style: Stroke) -> &mut Self {
        self.push(Draw::Stroke {
            path,
            brush: brush.into(),
            style,
        });
        self
    }
    /// Draw `image` in `rect` as `fit` fills it, cut to the box rounded by
    /// `radius`, at `opacity`: a photo in a card.
    pub fn image(
        &mut self,
        image: Arc<crate::image::ImageData>,
        rect: Rect,
        fit: crate::style::ObjectFit,
        radius: f32,
        opacity: f32,
    ) -> &mut Self {
        self.push(Draw::Image {
            image,
            rect,
            fit,
            radius,
            opacity,
        });
        self
    }
    /// Draw the shadow `path` casts: the shape grown by `spread`, moved by
    /// `offset` and blurred by a Gaussian of `blur` sigma, in `color`, under
    /// what is drawn after it (the shape itself, drawn next, for a card's
    /// drop shadow). All of it is drawn, under the shape too.
    pub fn shadow(
        &mut self,
        path: Path,
        color: Color,
        offset: Point,
        blur: f32,
        spread: f32,
    ) -> &mut Self {
        self.push(Draw::Shadow {
            path,
            color,
            offset,
            blur,
            spread,
        });
        self
    }
    /// Clip the commands after this to `path` until `restore`.
    pub fn clip(&mut self, path: Path, rule: FillRule) -> &mut Self {
        self.push(Draw::Clip { path, rule });
        self
    }
    /// End the most recent clip.
    pub fn restore(&mut self) -> &mut Self {
        self.push(Draw::Restore);
        self
    }
    /// The commands, in order.
    pub fn commands(&self) -> &[Draw] {
        &self.commands
    }
    /// Add a line of text, over what was drawn before it and under what is
    /// drawn after, cut by the clips open now (see [`Canvas::layers`] and
    /// [`Canvas::text_clips`]).
    pub fn text(&mut self, text: CanvasText) -> &mut Self {
        let (at, index) = (self.commands.len(), self.text.len());
        if let Some(order) = &mut self.order {
            if index == 0 {
                order.first_text_after = at;
            } else if order.last_text_after() != at {
                order.text_moves.push((index, at));
            }
        }
        self.text.push(text);
        self
    }
    /// Add a line of text laid out as [`Canvas::text`] places it, then
    /// mapped by `transform` (in the canvas's coordinates): turned, scaled,
    /// mirrored or skewed with a shape drawn under the same transform.
    pub fn text_transformed(&mut self, text: CanvasText, transform: Affine) -> &mut Self {
        if transform != Affine::IDENTITY {
            let index = self.text.len();
            self.order().transformed.push((index, transform));
        }
        self.text(text)
    }
    /// Draw its text into its raster with its shapes, in their layers, as
    /// a picture of it draws them, rather than as text nodes over it: a
    /// canvas that pans and zooms (`View::canvas_view`) then moves and
    /// scales its words with its raster, as pixels, rather than laying out
    /// and drawing every one of them again at each new zoom.
    pub fn raster_text(&mut self) -> &mut Self {
        self.raster_text = true;
        self
    }
    /// Whether its text is drawn into its raster (`Canvas::raster_text`).
    pub fn rasters_text(&self) -> bool {
        self.raster_text
    }
    /// The canvas's lines of text.
    pub fn texts(&self) -> &[CanvasText] {
        &self.text
    }
    /// What the text at `index` is mapped by once placed: identity unless
    /// it was added with [`Canvas::text_transformed`].
    pub fn text_transform(&self, index: usize) -> Affine {
        let Some(order) = &self.order else {
            return Affine::IDENTITY;
        };
        match order
            .transformed
            .binary_search_by_key(&index, |(at, _)| *at)
        {
            Ok(found) => order.transformed[found].1,
            Err(_) => Affine::IDENTITY,
        }
    }
    /// Whether a text was added transformed.
    pub fn texts_transformed(&self) -> bool {
        self.order
            .as_ref()
            .is_some_and(|order| !order.transformed.is_empty())
    }
    /// How many commands came before the text at `index`: where it falls
    /// among them.
    pub fn text_after(&self, index: usize) -> usize {
        let Some(order) = &self.order else {
            return self.commands.len();
        };
        match order.text_moves.partition_point(|(at, _)| *at <= index) {
            0 => order.first_text_after,
            moved => order.text_moves[moved - 1].1,
        }
    }
    /// Whether every text comes after every command, nothing clips and no
    /// text is transformed: the drawing, then its words over it, as a
    /// chart's labels and a terminal's rows are.
    pub fn texts_last(&self) -> bool {
        self.order.is_none()
    }
    /// How many commands came before each text, in order.
    fn texts_after(&self) -> impl Iterator<Item = usize> + '_ {
        let (first, moves) = match &self.order {
            Some(order) => (order.first_text_after, &order.text_moves[..]),
            None => (self.commands.len(), &[][..]),
        };
        (0..self.text.len()).scan(0, move |moved, index| {
            if moves.get(*moved).is_some_and(|(at, _)| *at == index) {
                *moved += 1;
            }
            Some(match *moved {
                0 => first,
                moved => moves[moved - 1].1,
            })
        })
    }
    /// `other`'s commands after these, as drawn: a drawing made of parts
    /// kept apart (each item's, drawn again only when it changes).
    pub fn extend(&mut self, other: &Canvas) -> &mut Self {
        let (before, texts_before) = (self.commands.len(), self.text.len());
        // Kept from before `other`'s commands, which follow these texts.
        if texts_before > 0 && !other.commands.is_empty() {
            self.order();
        }
        if other.order.as_ref().is_some_and(|order| order.clipped) {
            self.order().clipped = true;
        }
        if other.texts_transformed() {
            self.order();
        }
        self.commands.extend_from_slice(&other.commands);
        if !other.text.is_empty() && (self.order.is_some() || other.order.is_some()) {
            let (first, moves) = match &other.order {
                Some(order) => (order.first_text_after, &order.text_moves[..]),
                None => (other.commands.len(), &[][..]),
            };
            let first = first + before;
            let order = self.order();
            if texts_before == 0 {
                order.first_text_after = first;
            } else if order.last_text_after() != first {
                order.text_moves.push((texts_before, first));
            }
            order.text_moves.extend(
                moves
                    .iter()
                    .map(|(at, after)| (at + texts_before, after + before)),
            );
        }
        if let (Some(order), Some(theirs)) = (&mut self.order, &other.order) {
            order.transformed.extend(
                theirs
                    .transformed
                    .iter()
                    .map(|(at, transform)| (at + texts_before, *transform)),
            );
        }
        self.text.extend_from_slice(&other.text);
        self
    }
    /// Each text's clip, in order: the box its open clips leave (their
    /// bounds', intersected), rounded where that is a rounded rectangle's
    /// or a circle's. `None` where none is open; empty when the canvas
    /// clips nothing.
    pub fn text_clips(&self) -> Vec<Option<TextClip>> {
        if self.text.is_empty() || !self.order.as_ref().is_some_and(|order| order.clipped) {
            return Vec::new();
        }
        let mut clips = Vec::with_capacity(self.text.len());
        let mut open: Vec<Option<TextClip>> = Vec::new();
        let mut texts = self.texts_after().peekable();
        for at in 0..=self.commands.len() {
            while texts.next_if(|after| *after == at).is_some() {
                clips.push(open.last().copied().flatten());
            }
            match self.commands.get(at) {
                Some(Draw::Clip { path, .. }) => {
                    let (rect, radius) = clip_box(path);
                    let clip = match open.last().copied().flatten() {
                        None => TextClip { rect, radius },
                        Some(outer) => {
                            let inner = outer
                                .rect
                                .intersection(rect)
                                .unwrap_or(Rect::new(rect.x, rect.y, 0., 0.));
                            let radius = if inner == rect {
                                radius
                            } else if inner == outer.rect {
                                outer.radius
                            } else {
                                0.
                            };
                            TextClip {
                                rect: inner,
                                radius,
                            }
                        }
                    };
                    open.push(Some(clip));
                }
                Some(Draw::Restore) => {
                    open.pop();
                }
                _ => {}
            }
        }
        clips
    }
    /// The canvas as runs of commands each with the texts over them, in
    /// order: a text goes under a later shape only where that shape can
    /// reach it, so a drawing that keeps its words clear of what follows
    /// is one layer (as is one with no text after its last command, which
    /// measures nothing). A stroke reaches only along its outline: a card's
    /// border drawn after its own words leaves them in its layer. `size`
    /// gives a text's width and height. At most 8 layers: past them, the
    /// two neighbours whose joining puts the least text over a later shape
    /// are joined, again and again, so a scrim or a dialog over many cards
    /// keeps every card's words under it.
    pub fn layers(&self, mut size: impl FnMut(&CanvasText) -> (f32, f32)) -> Vec<CanvasLayer> {
        let all = || CanvasLayer {
            commands: 0..self.commands.len(),
            texts: 0..self.text.len(),
        };
        if self.text.is_empty() || self.text_after(0) == self.commands.len() {
            return vec![all()];
        }
        let mut layers = Vec::new();
        let (mut first_command, mut first_text) = (0, 0);
        // Every measured text's box (a text's index is its box's), and the
        // union of this layer's.
        let mut boxes: Vec<Rect> = Vec::new();
        let mut covered: Option<Rect> = None;
        let mut next_text = 0;
        let mut afters = self.texts_after().peekable();
        for (at, command) in self.commands.iter().enumerate() {
            while afters.next_if(|after| *after == at).is_some() {
                let text = &self.text[next_text];
                let (width, height) = size(text);
                let (x, y) = text.origin((width, height));
                let area = self
                    .text_transform(next_text)
                    .bounds(Rect::new(x, y, width, height));
                covered = Some(covered.map_or(area, |c| c.union(area)));
                boxes.push(area);
                next_text += 1;
            }
            let Some(covered_now) = covered else { continue };
            let Some(reach) = command_bounds(command) else {
                continue;
            };
            if covered_now.intersects(reach)
                && boxes[first_text..next_text]
                    .iter()
                    .any(|b| covers(command, reach, *b) > 0.)
            {
                layers.push(CanvasLayer {
                    commands: first_command..at,
                    texts: first_text..next_text,
                });
                (first_command, first_text) = (at, next_text);
                covered = None;
            }
        }
        layers.push(CanvasLayer {
            commands: first_command..self.commands.len(),
            texts: first_text..self.text.len(),
        });
        if layers.len() > MAX_LAYERS {
            self.join_cheapest(&mut layers, &boxes);
        }
        layers
    }
    /// Join neighbouring layers until at most [`MAX_LAYERS`] are left, each
    /// time the two whose joining lays the least area of the first's text
    /// over the second's shapes (what goes wrong when they're one).
    fn join_cheapest(&self, layers: &mut Vec<CanvasLayer>, boxes: &[Rect]) {
        let cost = |a: &CanvasLayer, b: &CanvasLayer| -> f32 {
            let mut area = 0.;
            for command in &self.commands[b.commands.clone()] {
                let Some(reach) = command_bounds(command) else {
                    continue;
                };
                for text in a.texts.clone() {
                    if let Some(at) = boxes.get(text) {
                        area += covers(command, reach, *at);
                    }
                }
            }
            area
        };
        let mut costs: Vec<f32> = layers.windows(2).map(|w| cost(&w[0], &w[1])).collect();
        while layers.len() > MAX_LAYERS {
            let (at, _) = costs
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(b.1))
                .expect("more than one layer");
            let next = layers.remove(at + 1);
            layers[at].commands.end = next.commands.end;
            layers[at].texts.end = next.texts.end;
            costs.remove(at);
            if at > 0 {
                costs[at - 1] = cost(&layers[at - 1], &layers[at]);
            }
            if at < costs.len() {
                costs[at] = cost(&layers[at], &layers[at + 1]);
            }
        }
    }
    /// `layer`'s commands as a canvas of their own (no text): the clips
    /// open where it starts opened again first, and those open where it
    /// ends closed.
    pub fn layer(&self, layer: &CanvasLayer) -> Canvas {
        let mut open: Vec<usize> = Vec::new();
        for (at, command) in self.commands[..layer.commands.start].iter().enumerate() {
            match command {
                Draw::Clip { .. } => open.push(at),
                Draw::Restore => {
                    open.pop();
                }
                _ => {}
            }
        }
        let mut canvas = Canvas::new();
        canvas
            .commands
            .extend(open.iter().map(|at| self.commands[*at].clone()));
        let mut depth = open.len();
        for command in &self.commands[layer.commands.clone()] {
            match command {
                Draw::Clip { .. } => depth += 1,
                Draw::Restore => depth = depth.saturating_sub(1),
                _ => {}
            }
            canvas.commands.push(command.clone());
        }
        canvas
            .commands
            .extend(std::iter::repeat_n(Draw::Restore, depth));
        canvas
    }
}
/// Where a path lies: its points' bounds (a curve's control points
/// included, which may reach past it).
fn path_bounds(path: &Path) -> Rect {
    let positive = |r: Rect| {
        Rect::new(
            r.x.min(r.x + r.width),
            r.y.min(r.y + r.height),
            r.width.abs(),
            r.height.abs(),
        )
    };
    match path.shape() {
        Some(Shape::Rect { rect, .. } | Shape::Ellipse { rect }) => return positive(rect),
        Some(Shape::Line { from, to }) => {
            return positive(Rect::new(from.x, from.y, to.x - from.x, to.y - from.y));
        }
        None => {}
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for segment in path.segments() {
        let points: &[Point] = match segment {
            Segment::Move(p) | Segment::Line(p) => std::slice::from_ref(p),
            Segment::Quadratic(a, b) => &[*a, *b],
            Segment::Cubic(a, b, c) => &[*a, *b, *c],
            Segment::Close => &[],
        };
        for p in points {
            (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
        }
    }
    if x0 > x1 {
        return Rect::new(0., 0., 0., 0.);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}
/// The box a clip to `path` leaves text, and its corners' radius.
fn clip_box(path: &Path) -> (Rect, f32) {
    let rect = path_bounds(path);
    let radius = match path.shape() {
        Some(Shape::Rect { radius, .. }) => radius,
        Some(Shape::Ellipse { .. }) if (rect.width - rect.height).abs() < 1e-3 => rect.width / 2.,
        _ => 0.,
    };
    (rect, radius)
}
/// How much of `text` (a text's box) `command` can paint over, `reach`
/// being [`command_bounds`]'s: the boxes' overlap, less, for a stroke round a
/// rectangle or an ellipse, what lies inside the outline clear of the line
/// (a card's border over its own words paints none of them).
fn covers(command: &Draw, reach: Rect, text: Rect) -> f32 {
    let area = |r: Option<Rect>| r.map_or(0., |r| r.width * r.height);
    let over = area(reach.intersection(text));
    if over <= 0. {
        return 0.;
    }
    let Draw::Stroke { path, style, .. } = command else {
        return over;
    };
    // The line reaches half its width out of the outline (a miter more).
    let half = style.width.max(0.) / 2. * style.miter_limit.max(1.);
    let bounds = path_bounds(path);
    let inside = match path.shape() {
        // Clear of the corners: in by the radius too.
        Some(Shape::Rect { radius, .. }) => bounds.expand(-(half + radius.max(0.))),
        // The rectangle inscribed in it.
        Some(Shape::Ellipse { .. }) => {
            let k = std::f32::consts::FRAC_1_SQRT_2;
            Rect::new(
                bounds.x + bounds.width * (1. - k) / 2.,
                bounds.y + bounds.height * (1. - k) / 2.,
                bounds.width * k,
                bounds.height * k,
            )
            .expand(-half)
        }
        _ => return over,
    };
    if inside.width <= 0. || inside.height <= 0. {
        return over;
    }
    (over - area(inside.intersection(text))).max(0.)
}
/// What a command can paint, or `None` for a clip's.
fn command_bounds(command: &Draw) -> Option<Rect> {
    match command {
        Draw::Fill { path, .. } => Some(path_bounds(path)),
        // A miter reaches out its limit's widths; caps half one.
        Draw::Stroke { path, style, .. } => {
            Some(path_bounds(path).expand(style.width.max(0.) * style.miter_limit.max(1.)))
        }
        Draw::Image { rect, .. } => Some(Rect::new(
            rect.x.min(rect.x + rect.width),
            rect.y.min(rect.y + rect.height),
            rect.width.abs(),
            rect.height.abs(),
        )),
        // A blur reaches about three sigmas.
        Draw::Shadow {
            path,
            offset,
            blur,
            spread,
            ..
        } => {
            let shape = path_bounds(path);
            Some(
                Rect::new(
                    shape.x + offset.x,
                    shape.y + offset.y,
                    shape.width,
                    shape.height,
                )
                .expand(spread.max(0.) + 3. * blur.max(0.)),
            )
        }
        Draw::Clip { .. } | Draw::Restore => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The segments `f` gives a builder.
    fn built(f: impl FnOnce(&mut PathBuilder)) -> Vec<Segment> {
        let mut builder = PathBuilder::default();
        f(&mut builder);
        builder.segments
    }

    #[test]
    fn a_shape_paths_segments_are_made_when_asked_and_are_the_builders() {
        let rect = Rect::new(2., 3., 40., 20.);
        let rounded = Path::rounded_rect(rect, 30.).unwrap();
        assert!(rounded.segments.get().is_none(), "none made yet");
        // The radius clamps to half the shorter side, as the builder's.
        assert_eq!(rounded.shape(), Some(Shape::Rect { rect, radius: 10. }));
        assert_eq!(
            rounded.segments(),
            built(|b| {
                b.rounded_rect(rect, 30.);
            })
        );
        let ellipse = Path::ellipse(rect).unwrap();
        assert_eq!(
            ellipse.segments(),
            built(|b| {
                b.ellipse(rect);
            })
        );
        let (from, to) = (Point::new(1., 2.), Point::new(5., 9.));
        assert_eq!(
            Path::line(from, to).unwrap().segments(),
            [Segment::Move(from), Segment::Line(to)]
        );
        // A builder given the shape alone makes the same path.
        let mut builder = Path::builder();
        builder.rounded_rect(rect, 30.);
        assert_eq!(builder.build().unwrap(), rounded);
        assert_ne!(Path::rounded_rect(rect, 2.).unwrap(), rounded);
        assert!(Path::rounded_rect(Rect::new(f32::NAN, 0., 1., 1.), 0.).is_err());
        assert!(Path::ellipse(Rect::new(0., 0., f32::INFINITY, 1.)).is_err());
    }

    fn label(text: &str, x: f32, y: f32) -> CanvasText {
        CanvasText {
            text: text.into(),
            x,
            y,
            size: 10.,
            color: Color(0, 0, 0, 255),
            font: Default::default(),
            anchor: TextAnchor::Start,
            baseline: TextBaseline::Top,
            width: None,
        }
    }
    fn square(x: f32, y: f32) -> Path {
        Path::rectangle(Rect::new(x, y, 10., 10.))
    }

    #[test]
    fn a_text_goes_under_a_later_shape_only_where_it_reaches_it() {
        let red = Color(255, 0, 0, 255);
        // Every text 20 by 10.
        let size = |_: &CanvasText| (20., 10.);
        // Shapes then text: one layer, nothing measured.
        let mut canvas = Canvas::new();
        canvas.fill(square(0., 0.), red).text(label("a", 0., 0.));
        assert_eq!(
            canvas.layers(|_| unreachable!("measured")),
            [CanvasLayer {
                commands: 0..1,
                texts: 0..1
            }]
        );
        // A shape after a text, clear of it: still one.
        canvas.fill(square(50., 50.), red);
        assert_eq!(canvas.layers(size).len(), 1);
        assert_eq!(canvas.text_after(0), 1);
        assert!(
            canvas
                .order
                .as_ref()
                .is_some_and(|o| o.text_moves.is_empty()),
            "one place: no move kept"
        );
        // One over it: a second layer, from that shape on.
        canvas.fill(square(15., 5.), red).text(label("b", 0., 30.));
        assert_eq!(
            canvas.layers(size),
            [
                CanvasLayer {
                    commands: 0..2,
                    texts: 0..1
                },
                CanvasLayer {
                    commands: 2..3,
                    texts: 1..2
                }
            ]
        );
        // Kept apart and joined (an `each` item's), as drawn.
        let mut joined = Canvas::new();
        joined.fill(square(90., 90.), red).extend(&canvas);
        assert_eq!((joined.text_after(0), joined.text_after(1)), (2, 4));
        assert_eq!(joined.texts_after().collect::<Vec<_>>(), [2, 4]);
        assert_eq!(joined.layers(size)[0].commands, 0..3);
        // However many times a text is covered, at most 8 layers: the
        // cheapest neighbours joined, every text still in one of them.
        let mut many = Canvas::new();
        for _ in 0..20 {
            many.text(label("c", 0., 0.)).fill(square(0., 0.), red);
        }
        let layers = many.layers(size);
        assert_eq!(layers.len(), 8);
        assert_eq!((layers[0].texts.start, layers[7].texts.end), (0, 20));
    }

    /// Cards (a fill, its words, its border), then a scrim and a dialog
    /// over them all: the shape of P4, an overlay on a screen of cards.
    fn cards_under_a_dialog(cards: usize, badge: bool) -> Canvas {
        let (white, grey) = (Color(255, 255, 255, 255), Color(200, 200, 200, 255));
        let border = Stroke {
            width: 1.,
            ..Default::default()
        };
        let mut canvas = Canvas::new();
        for i in 0..cards {
            let (x, y) = ((i % 3) as f32 * 280., (i / 3) as f32 * 100.);
            let card = Rect::new(x, y, 260., 72.);
            canvas
                .fill(Path::rounded_rect(card, 12.).unwrap(), white)
                .text(label("Card title", x + 16., y + 16.));
            if badge {
                // a mark drawn over the corner of its title
                canvas.fill(Path::rectangle(Rect::new(x + 30., y + 18., 8., 8.)), grey);
            }
            canvas.stroke(Path::rounded_rect(card, 12.).unwrap(), grey, border.clone());
        }
        canvas
            .fill(
                Path::rectangle(Rect::new(0., 0., 900., 500.)),
                Color(9, 9, 11, 200),
            )
            .fill(
                Path::rounded_rect(Rect::new(200., 60., 480., 360.), 16.).unwrap(),
                white,
            )
            .text(label("Dialog title", 224., 84.));
        canvas
    }

    #[test]
    fn a_border_drawn_after_its_own_words_keeps_them_in_its_layer() {
        // Twelve stroked cards then a scrim and a dialog: two layers, every
        // card's words under the scrim (once, past 8 borders, the words left
        // went over the dialog).
        let canvas = cards_under_a_dialog(12, false);
        let layers = canvas.layers(|_| (60., 12.));
        assert_eq!(layers.len(), 2, "{layers:?}");
        assert_eq!(layers[0].texts, 0..12);
        assert_eq!(layers[1].texts, 12..13);
        // A stroke still reaches a text its line crosses.
        let mut crossed = Canvas::new();
        crossed.text(label("a", 0., 0.)).stroke(
            Path::rounded_rect(Rect::new(10., -10., 50., 40.), 4.).unwrap(),
            Color(0, 0, 0, 255),
            Stroke::default(),
        );
        assert_eq!(crossed.layers(|_| (20., 10.)).len(), 2);
    }

    #[test]
    fn past_8_layers_the_overlay_keeps_its_split() {
        // Each card's mark over its title starts a layer: 13 of them. Joined
        // down to 8, the cheapest first, never across the scrim: no card's
        // words go over it.
        let canvas = cards_under_a_dialog(12, true);
        let layers = canvas.layers(|_| (60., 12.));
        assert_eq!(layers.len(), 8, "{layers:?}");
        let scrim = canvas.commands().len() - 2;
        let over = layers
            .iter()
            .find(|l| l.commands.contains(&scrim))
            .expect("the scrim's layer");
        assert_eq!(over.texts.start, 12, "{layers:?}");
    }

    #[test]
    fn a_layer_opens_the_clips_open_where_it_starts_and_closes_its_own() {
        let red = Color(255, 0, 0, 255);
        let clip = square(0., 0.);
        let mut canvas = Canvas::new();
        canvas
            .clip(clip.clone(), FillRule::Winding)
            .text(label("a", 0., 0.))
            .fill(square(0., 0.), red)
            .clip(square(5., 5.), FillRule::EvenOdd)
            .fill(square(5., 5.), red)
            .restore()
            .restore();
        let layers = canvas.layers(|_| (20., 10.));
        assert_eq!(layers.len(), 2);
        assert_eq!(
            canvas.layer(&layers[0]).commands(),
            [
                Draw::Clip {
                    path: clip.clone(),
                    rule: FillRule::Winding
                },
                Draw::Restore
            ]
        );
        let second = canvas.layer(&layers[1]);
        // The card's clip again, then its own: a fill, a clip, a fill, and
        // both closed.
        assert_eq!(second.commands().len(), 6, "{:?}", second.commands());
        assert_eq!(second.commands()[4..], [Draw::Restore, Draw::Restore]);
        assert_eq!(
            second.commands()[0],
            Draw::Clip {
                path: clip,
                rule: FillRule::Winding
            }
        );
        assert!(second.texts().is_empty());
    }

    #[test]
    fn a_text_is_cut_by_the_box_its_open_clips_leave() {
        let mut canvas = Canvas::new();
        canvas.text(label("none", 0., 0.));
        assert!(canvas.text_clips().is_empty(), "nothing clips");
        let card = Rect::new(10., 10., 100., 50.);
        canvas
            .clip(Path::rounded_rect(card, 8.).unwrap(), FillRule::Winding)
            .text(label("card", 0., 0.))
            .clip(
                Path::rectangle(Rect::new(0., 20., 200., 20.)),
                FillRule::Winding,
            )
            .text(label("band", 0., 0.))
            .clip(
                Path::rounded_rect(Rect::new(20., 25., 10., 10.), 3.).unwrap(),
                FillRule::Winding,
            )
            .text(label("inner", 0., 0.))
            .restore()
            .restore()
            .clip(
                Path::ellipse(Rect::new(20., 20., 30., 30.)).unwrap(),
                FillRule::Winding,
            )
            .text(label("avatar", 0., 0.))
            .restore()
            .restore()
            .clip(
                Path::ellipse(Rect::new(0., 0., 30., 10.)).unwrap(),
                FillRule::Winding,
            )
            .text(label("oval", 0., 0.))
            .clip(square(100., 100.), FillRule::Winding)
            .text(label("gone", 0., 0.))
            .restore()
            .restore()
            .text(label("after", 0., 0.));
        let clip = |x, y, w, h, radius| {
            Some(TextClip {
                rect: Rect::new(x, y, w, h),
                radius,
            })
        };
        assert_eq!(
            canvas.text_clips(),
            [
                None,
                clip(10., 10., 100., 50., 8.),
                // The band across the card: square where it cuts it.
                clip(10., 20., 100., 20., 0.),
                clip(20., 25., 10., 10., 3.),
                // A circle in the card: round.
                clip(20., 20., 30., 30., 15.),
                // An oval: its box.
                clip(0., 0., 30., 10., 0.),
                // Nothing left.
                clip(100., 100., 0., 0., 0.),
                None,
            ]
        );
    }

    #[test]
    fn a_texts_transform_stays_with_it_when_drawings_join() {
        let text = |x| CanvasText {
            text: "a".into(),
            x,
            y: 0.,
            size: 10.,
            color: Color(0, 0, 0, 255),
            font: Default::default(),
            anchor: TextAnchor::Start,
            baseline: TextBaseline::Top,
            width: None,
        };
        let turned = Affine::rotation(1.);
        let mut item = Canvas::new();
        item.text(text(0.))
            .text_transformed(text(1.), turned)
            .text_transformed(text(2.), Affine::IDENTITY);
        assert_eq!(
            item.order.as_ref().map(|o| o.transformed.len()),
            Some(1),
            "identity is kept as none"
        );
        let mut board = Canvas::new();
        board
            .text_transformed(text(9.), Affine::scale(2., 2.))
            .extend(&item);
        let transforms: Vec<Affine> = (0..4).map(|i| board.text_transform(i)).collect();
        assert_eq!(
            transforms,
            [
                Affine::scale(2., 2.),
                Affine::IDENTITY,
                turned,
                Affine::IDENTITY
            ]
        );
        let mut plain = Canvas::new();
        plain.text_transformed(text(0.), Affine::IDENTITY);
        assert!(
            plain.texts_last(),
            "an identity transform keeps the plain path"
        );
    }
}
