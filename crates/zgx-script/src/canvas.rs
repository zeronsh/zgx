//! Drawing for `ui.canvas`: a script's `draw(c, width, height)` records
//! paths and fills into zgx's retained canvas. It runs again only when the
//! canvas's size or the state it reads changes; equal drawings keep their
//! pixels. Colours may name theme tokens (`"$accent"`): the drawing reads
//! them as it reads state, so a theme change draws it again.
use crate::node::parse_color;
use mlua::{Lua, Table, UserData, UserDataMethods};
use std::{cell::RefCell, rc::Rc};
use zgx::canvas::{CanvasText, TextAnchor, TextBaseline};
use zgx::text_layout::{FontFamily, FontFeatures, FontStyle, LetterSpacing, LineHeight, TextAlign};
use zgx::theme::ThemeScope;
use zgx::{
    affine::Affine,
    canvas::{
        Brush, Canvas, FillRule, GradientStop, LineCap, LineJoin, Path, PathBuilder, Point, Stroke,
    },
    scene::Color,
    style::ObjectFit,
};

/// The canvas a draw function records into.
pub(crate) struct Recorder(pub RefCell<Canvas>);

/// `ui.path()`: a path built by chained calls.
pub(crate) struct PathUd(RefCell<Option<PathBuilder>>, RefCell<Option<Path>>);
impl PathUd {
    fn path(&self) -> mlua::Result<Path> {
        if let Some(builder) = self.0.borrow_mut().take() {
            let path = builder.build().map_err(mlua::Error::runtime)?;
            *self.1.borrow_mut() = Some(path);
        }
        self.1
            .borrow()
            .clone()
            .ok_or_else(|| mlua::Error::runtime("empty path"))
    }
    fn with(&self, f: impl FnOnce(&mut PathBuilder)) -> mlua::Result<()> {
        let mut builder = self.0.borrow_mut();
        let builder = builder
            .as_mut()
            .ok_or_else(|| mlua::Error::runtime("a drawn path cannot be extended"))?;
        f(builder);
        Ok(())
    }
}
impl UserData for PathUd {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_function("move", |_, (this, x, y): (mlua::AnyUserData, f32, f32)| {
            this.borrow::<PathUd>()?.with(|b| {
                b.move_to(x, y);
            })?;
            Ok(this)
        });
        methods.add_function("line", |_, (this, x, y): (mlua::AnyUserData, f32, f32)| {
            this.borrow::<PathUd>()?.with(|b| {
                b.line_to(x, y);
            })?;
            Ok(this)
        });
        methods.add_function(
            "quad",
            |_, (this, cx, cy, x, y): (mlua::AnyUserData, f32, f32, f32, f32)| {
                this.borrow::<PathUd>()?.with(|b| {
                    b.quadratic_to(cx, cy, x, y);
                })?;
                Ok(this)
            },
        );
        methods.add_function(
            "cubic",
            |_, (this, ax, ay, bx, by, x, y): (mlua::AnyUserData, f32, f32, f32, f32, f32, f32)| {
                this.borrow::<PathUd>()?.with(|b| {
                    b.cubic_to(Point::new(ax, ay), Point::new(bx, by), Point::new(x, y));
                })?;
                Ok(this)
            },
        );
        // Angles in degrees, clockwise from +x, as screen coordinates run.
        methods.add_function(
            "arc",
            |_, (this, cx, cy, r, start, sweep): (mlua::AnyUserData, f32, f32, f32, f32, f32)| {
                this.borrow::<PathUd>()?.with(|b| {
                    b.arc(
                        Point::new(cx, cy),
                        r,
                        start.to_radians(),
                        sweep.to_radians(),
                    );
                })?;
                Ok(this)
            },
        );
        // A rectangle (corners rounded by `r`) or an ellipse in a box, as a
        // closed subpath: a path that is just one is drawn as that shape.
        methods.add_function(
            "rect",
            |_, (this, x, y, w, h, r): (mlua::AnyUserData, f32, f32, f32, f32, Option<f32>)| {
                this.borrow::<PathUd>()?.with(|b| {
                    b.rounded_rect(zgx::scene::Rect::new(x, y, w, h), r.unwrap_or(0.));
                })?;
                Ok(this)
            },
        );
        methods.add_function(
            "ellipse",
            |_, (this, x, y, w, h): (mlua::AnyUserData, f32, f32, f32, f32)| {
                this.borrow::<PathUd>()?.with(|b| {
                    b.ellipse(zgx::scene::Rect::new(x, y, w, h));
                })?;
                Ok(this)
            },
        );
        methods.add_function("close", |_, this: mlua::AnyUserData| {
            this.borrow::<PathUd>()?.with(|b| {
                b.close();
            })?;
            Ok(this)
        });
    }
}

/// `ui.linear`, `ui.radial` or `ui.conic`: a gradient brush for canvases.
#[derive(Clone)]
pub(crate) struct BrushUd(pub Brush);
impl UserData for BrushUd {}

thread_local! {
    /// The theme of the canvas drawing now (`draw_in`), whose tokens its
    /// colours name.
    static SCOPE: RefCell<Option<Rc<ThemeScope>>> = const { RefCell::new(None) };
    /// The tokens this drawing has resolved: a drawing names a few, many
    /// times over (a grid's thousand dots in `"$border"`).
    static RESOLVED: RefCell<Vec<(Box<str>, Color)>> = const { RefCell::new(Vec::new()) };
}
/// Run `f`, a canvas's draw, with `$token` colours resolved in `scope`.
pub(crate) fn draw_in<R>(scope: Rc<ThemeScope>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Rc<ThemeScope>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SCOPE.with(|s| *s.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SCOPE.with(|s| s.borrow_mut().replace(scope)));
    // Resolved in this scope only (a nested canvas's theme may differ).
    let resolved = RESOLVED.with(|r| r.take());
    struct Forget(Option<Vec<(Box<str>, Color)>>);
    impl Drop for Forget {
        fn drop(&mut self) {
            RESOLVED.with(|r| *r.borrow_mut() = self.0.take().unwrap_or_default());
        }
    }
    let _forget = Forget(Some(resolved));
    f()
}
fn token_color(name: &str) -> mlua::Result<Color> {
    if let Some(color) = RESOLVED.with(|r| {
        r.borrow()
            .iter()
            .find(|(token, _)| &**token == name)
            .map(|(_, color)| *color)
    }) {
        return Ok(color);
    }
    let color = resolve_token(name)?;
    RESOLVED.with(|r| r.borrow_mut().push((name.into(), color)));
    Ok(color)
}
fn resolve_token(name: &str) -> mlua::Result<Color> {
    let scope = SCOPE.with(|s| s.borrow().clone()).ok_or_else(|| {
        mlua::Error::runtime(format!(
            "`${name}`: theme colours resolve while a canvas draws (call this in `draw`)"
        ))
    })?;
    scope
        .get(zgx::theme::token(name))
        .color()
        .ok_or_else(|| mlua::Error::runtime(format!("`${name}` is not a colour token")))
}
fn color(v: &mlua::Value) -> mlua::Result<Color> {
    // A string read where it is, without copying it.
    if let mlua::Value::String(s) = v {
        let bytes = s.as_bytes();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return Err(mlua::Error::runtime("bad color: not text"));
        };
        if let Some(name) = text.strip_prefix('$') {
            return token_color(name);
        }
        return parse_color(text)
            .ok_or_else(|| mlua::Error::runtime(format!("bad color `{text}`")));
    }
    let text = match v {
        mlua::Value::Integer(n) => format!("#{n:06x}"),
        mlua::Value::Number(n) => format!("#{:06x}", *n as u32),
        other => {
            return Err(mlua::Error::runtime(format!(
                "a color is a string or number, not {}",
                other.type_name()
            )));
        }
    };
    if let Some(name) = text.strip_prefix('$') {
        return token_color(name);
    }
    parse_color(&text).ok_or_else(|| mlua::Error::runtime(format!("bad color `{text}`")))
}
fn brush(v: &mlua::Value) -> mlua::Result<Brush> {
    match v {
        mlua::Value::UserData(ud) if ud.is::<BrushUd>() => Ok(ud.borrow::<BrushUd>()?.0.clone()),
        other => Ok(Brush::Solid(color(other)?)),
    }
}
fn stroke(width: Option<f32>, options: Option<Table>) -> mlua::Result<Stroke> {
    let mut style = Stroke {
        width: width.unwrap_or(1.),
        ..Default::default()
    };
    if let Some(o) = options {
        style.cap = match o.get::<Option<String>>("cap")?.as_deref() {
            Some("round") => LineCap::Round,
            Some("square") => LineCap::Square,
            _ => LineCap::Butt,
        };
        style.join = match o.get::<Option<String>>("join")?.as_deref() {
            Some("round") => LineJoin::Round,
            Some("bevel") => LineJoin::Bevel,
            _ => LineJoin::Miter,
        };
        if let Some(dash) = o.get::<Option<Vec<f32>>>("dash")? {
            style.dash = dash;
        }
        style.dash_offset = o.get::<Option<f32>>("dash_offset")?.unwrap_or(0.);
    }
    Ok(style)
}
/// A rounded rectangle, a shape renderers draw without rasterizing it.
fn rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> mlua::Result<Path> {
    Path::rounded_rect(zgx::scene::Rect::new(x, y, w, h), r).map_err(mlua::Error::runtime)
}
fn path_arg(v: &mlua::AnyUserData) -> mlua::Result<Path> {
    v.borrow::<PathUd>()?.path()
}

/// A canvas text's font and size, from its options.
fn text_style(options: Option<&Table>) -> mlua::Result<(f32, FontStyle)> {
    let mut font = FontStyle::default();
    let Some(o) = options else {
        return Ok((TEXT_SIZE, font));
    };
    // Points, as a text node's.
    if let Some(line_height) = o.get::<Option<f32>>("line_height")? {
        font.line_height = LineHeight::px(line_height);
    }
    if let Some(spacing) = o.get::<Option<f32>>("letter_spacing")? {
        font.letter_spacing = LetterSpacing::px(spacing);
    }
    if let Some(weight) = o.get::<Option<u16>>("weight")? {
        font.weight = weight;
    }
    font.italic = o.get::<Option<bool>>("italic")?.unwrap_or(false);
    if o.get::<Option<bool>>("tabular")?.unwrap_or(false) {
        font.features = FontFeatures::new([(*b"tnum", 1)]);
    }
    match o.get::<Option<String>>("font")?.as_deref() {
        None => {}
        Some("mono" | "monospace") => font.family = FontFamily::Monospace,
        Some("sans" | "sans-serif") => font.family = FontFamily::SansSerif,
        Some("serif") => font.family = FontFamily::Serif,
        Some(name) => font.family = FontFamily::Named(name.into()),
    }
    let size = o.get::<Option<f32>>("size")?.unwrap_or(TEXT_SIZE);
    Ok((size.max(1.), font))
}
/// A canvas text's `transform` option, `{ a, b, c, d, e, f }` as SVG's
/// `matrix` (x' is a x + c y + e, y' is b x + d y + f), mapping the text
/// once placed: identity without one.
fn text_transform(options: Option<&Table>) -> mlua::Result<zgx::affine::Affine> {
    let Some(matrix) = options
        .map(|o| o.get::<Option<Vec<f32>>>("transform"))
        .transpose()?
        .flatten()
    else {
        return Ok(zgx::affine::Affine::IDENTITY);
    };
    let [a, b, c, d, tx, ty] = matrix[..] else {
        return Err(mlua::Error::runtime(
            "a canvas text's `transform` is six numbers, { a, b, c, d, e, f }",
        ));
    };
    let transform = zgx::affine::Affine { a, b, c, d, tx, ty };
    if !transform.is_finite() {
        return Err(mlua::Error::runtime(
            "a canvas text's `transform` must be finite",
        ));
    }
    Ok(transform)
}
/// A canvas text's size where its options give none: a chart's labels.
const TEXT_SIZE: f32 = 12.;
/// The width a canvas text wraps to (`width`), if its options give one.
fn wrap_width(options: Option<&Table>) -> mlua::Result<Option<f32>> {
    let Some(width) = options.map_or(Ok(None), |o| o.get::<Option<f32>>("width"))? else {
        return Ok(None);
    };
    if !width.is_finite() || width <= 0. {
        return Err(mlua::Error::runtime(format!(
            "a canvas text's `width` is a number of points above 0, not {width}"
        )));
    }
    Ok(Some(width))
}
/// A canvas shadow's colour where its options give none: a `shadow`
/// property's.
const SHADOW: Color = Color(0, 0, 0, 90);

impl UserData for Recorder {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "text",
            |_, c, (text, x, y, options): (String, f32, f32, Option<Table>)| {
                let (size, mut font) = text_style(options.as_ref())?;
                let width = wrap_width(options.as_ref())?;
                let color = match options
                    .as_ref()
                    .map(|o| o.get::<mlua::Value>("color"))
                    .transpose()?
                {
                    None | Some(mlua::Value::Nil) => token_color("text.primary")?,
                    Some(color) => self::color(&color)?,
                };
                let word = |key: &str| -> mlua::Result<Option<String>> {
                    options.as_ref().map_or(Ok(None), |o| o.get(key))
                };
                let anchor = match word("align")?.as_deref() {
                    None | Some("start") => TextAnchor::Start,
                    Some("center") => TextAnchor::Center,
                    Some("end") => TextAnchor::End,
                    Some(other) => {
                        return Err(mlua::Error::runtime(format!(
                            "a canvas text's `align` is \"start\", \"center\" or \"end\", not `{other}`"
                        )));
                    }
                };
                let baseline = match word("baseline")?.as_deref() {
                    None | Some("top") => TextBaseline::Top,
                    Some("middle") => TextBaseline::Middle,
                    Some("bottom") => TextBaseline::Bottom,
                    Some(other) => {
                        return Err(mlua::Error::runtime(format!(
                            "a canvas text's `baseline` is \"top\", \"middle\" or \"bottom\", not `{other}`"
                        )));
                    }
                };
                // Lines of a paragraph align as its box does on `x`.
                font.align = match anchor {
                    TextAnchor::Start => TextAlign::Start,
                    TextAnchor::Center => TextAlign::Center,
                    TextAnchor::End => TextAlign::End,
                };
                let transform = text_transform(options.as_ref())?;
                c.0.borrow_mut().text_transformed(
                    CanvasText {
                        text: text.into(),
                        x,
                        y,
                        size,
                        color,
                        font,
                        anchor,
                        baseline,
                        width,
                    },
                    transform,
                );
                Ok(())
            },
        );
        // What placing a label around another needs: its width and height.
        methods.add_method(
            "measure_text",
            |_, _, (text, options): (String, Option<Table>)| {
                let (size, font) = text_style(options.as_ref())?;
                let wrap = wrap_width(options.as_ref())?;
                let (width, height) =
                    zgx::canvas::measure_text_in(&text, size, wrap, &font).unwrap_or((0., 0.));
                Ok((width, height))
            },
        );
        methods.add_method(
            "rect",
            |_, c, (x, y, w, h, paint, radius): (f32, f32, f32, f32, mlua::Value, Option<f32>)| {
                c.0.borrow_mut()
                    .fill(rect_path(x, y, w, h, radius.unwrap_or(0.))?, brush(&paint)?);
                Ok(())
            },
        );
        methods.add_method(
            "circle",
            |_, c, (cx, cy, r, paint): (f32, f32, f32, mlua::Value)| {
                let r = r.max(0.);
                let circle = Path::ellipse(zgx::scene::Rect::new(cx - r, cy - r, 2. * r, 2. * r))
                    .map_err(mlua::Error::runtime)?;
                c.0.borrow_mut().fill(circle, brush(&paint)?);
                Ok(())
            },
        );
        methods.add_method(
            "line",
            |_,
             c,
             (x1, y1, x2, y2, paint, width, options): (
                f32,
                f32,
                f32,
                f32,
                mlua::Value,
                Option<f32>,
                Option<Table>,
            )| {
                c.0.borrow_mut().stroke(
                    Path::line(Point::new(x1, y1), Point::new(x2, y2))
                        .map_err(mlua::Error::runtime)?,
                    brush(&paint)?,
                    stroke(width, options)?,
                );
                Ok(())
            },
        );
        methods.add_method(
            "fill",
            |_, c, (path, paint, rule): (mlua::AnyUserData, mlua::Value, Option<String>)| {
                let rule = if rule.as_deref() == Some("evenodd") {
                    FillRule::EvenOdd
                } else {
                    FillRule::Winding
                };
                c.0.borrow_mut()
                    .fill_rule(path_arg(&path)?, brush(&paint)?, rule);
                Ok(())
            },
        );
        methods.add_method(
            "stroke",
            |_,
             c,
             (path, paint, width, options): (
                mlua::AnyUserData,
                mlua::Value,
                Option<f32>,
                Option<Table>,
            )| {
                let path = path_arg(&path)?;
                // `draw`: the fraction of the stroke drawn (or `{ start, end }`),
                // as anime.js draws SVG lines progressively.
                let draw = match &options {
                    Some(o) => match o.get::<mlua::Value>("draw")? {
                        mlua::Value::Integer(n) => Some((0., n as f64)),
                        mlua::Value::Number(n) => Some((0., n)),
                        mlua::Value::Table(t) => Some((t.get::<f64>(1)?, t.get::<f64>(2)?)),
                        _ => None,
                    },
                    None => None,
                };
                let mut style = stroke(width, options)?;
                if let Some((start, end)) = draw {
                    let length = crate::anime::svg::canvas_length(&path) as f32;
                    let (start, end) = (start.clamp(0., 1.) as f32, end.clamp(0., 1.) as f32);
                    if end <= start {
                        return Ok(());
                    }
                    style.dash = vec![length * (end - start), length * 2. + 1.];
                    style.dash_offset = -length * start;
                }
                c.0.borrow_mut().stroke(path, brush(&paint)?, style);
                Ok(())
            },
        );
        // A picture from a file, a URL, the host (`host:`) or a `Bitmap`,
        // fitted to its box as `ui.image` fits one. One not there yet (a
        // remote one loading) draws nothing, and the drawing follows it.
        methods.add_method(
            "image",
            |lua, c, (src, x, y, w, h, options): (mlua::Value, f32, f32, f32, f32, Option<Table>)| {
                let image = match &src {
                    mlua::Value::UserData(ud) if ud.is::<crate::node::BitmapUd>() => {
                        Some(ud.borrow::<crate::node::BitmapUd>()?.0.clone())
                    }
                    mlua::Value::String(src) => {
                        let env = crate::node::env(lua)?;
                        match env.vm.upgrade() {
                            Some(vm) => crate::lower::load_image_in(
                                &vm,
                                &env.assets,
                                &env.runtime,
                                &src.to_str()?,
                            ),
                            None => None,
                        }
                    }
                    other => {
                        return Err(mlua::Error::runtime(format!(
                            "c:image draws a path or a Bitmap, not {}",
                            other.type_name()
                        )));
                    }
                };
                let Some(image) = image else {
                    return Ok(());
                };
                let get = |key: &str| -> mlua::Result<Option<mlua::Value>> {
                    options.as_ref().map_or(Ok(None), |o| o.get(key))
                };
                let fit = match get("fit")? {
                    None | Some(mlua::Value::Nil) => ObjectFit::Fill,
                    Some(mlua::Value::String(fit)) => match &*fit.to_str()? {
                        "fill" => ObjectFit::Fill,
                        "contain" => ObjectFit::Contain,
                        "cover" => ObjectFit::Cover,
                        "none" => ObjectFit::None,
                        "scale_down" => ObjectFit::ScaleDown,
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "a canvas image's `fit` is \"fill\", \"contain\", \"cover\", \"none\" or \"scale_down\", not `{other}`"
                            )));
                        }
                    },
                    Some(other) => {
                        return Err(mlua::Error::runtime(format!(
                            "a canvas image's `fit` is a string, not {}",
                            other.type_name()
                        )));
                    }
                };
                let number = |key: &str, default: f32| -> mlua::Result<f32> {
                    Ok(options
                        .as_ref()
                        .map_or(Ok(None), |o| o.get::<Option<f32>>(key))?
                        .unwrap_or(default))
                };
                let (radius, opacity) = (number("radius", 0.)?, number("opacity", 1.)?);
                c.0.borrow_mut().image(
                    image,
                    zgx::scene::Rect::new(x, y, w, h),
                    fit,
                    radius.max(0.),
                    opacity.clamp(0., 1.),
                );
                Ok(())
            },
        );
        // The shadow a path casts, written as a `shadow` property is
        // (`{ x, y, blur, spread, color }`, CSS's blur): drawn first, under
        // the shape drawn after it.
        methods.add_method(
            "shadow",
            |_, c, (path, options): (mlua::AnyUserData, Option<Table>)| {
                let path = path_arg(&path)?;
                let number = |key: &str| -> mlua::Result<f32> {
                    Ok(options
                        .as_ref()
                        .map_or(Ok(None), |o| o.get::<Option<f32>>(key))?
                        .unwrap_or(0.))
                };
                let color = match options
                    .as_ref()
                    .map(|o| o.get::<mlua::Value>("color"))
                    .transpose()?
                {
                    None | Some(mlua::Value::Nil) => SHADOW,
                    Some(color) => self::color(&color)?,
                };
                // CSS's blur is twice the Gaussian's sigma zgx takes.
                c.0.borrow_mut().shadow(
                    path,
                    color,
                    Point::new(number("x")?, number("y")?),
                    number("blur")?.max(0.) / 2.,
                    number("spread")?,
                );
                Ok(())
            },
        );
        methods.add_method("clip", |_, c, path: mlua::AnyUserData| {
            c.0.borrow_mut().clip(path_arg(&path)?, FillRule::Winding);
            Ok(())
        });
        methods.add_method("restore", |_, c, ()| {
            c.0.borrow_mut().restore();
            Ok(())
        });
    }
}

pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    ui.set(
        "path",
        lua.create_function(|_, ()| {
            Ok(PathUd(
                RefCell::new(Some(Path::builder())),
                RefCell::new(None),
            ))
        })?,
    )?;
    ui.set(
        "linear",
        lua.create_function(|_, (x1, y1, x2, y2, stops): (f32, f32, f32, f32, Table)| {
            Brush::linear(Point::new(x1, y1), Point::new(x2, y2), even_stops(&stops)?)
                .map(BrushUd)
                .map_err(mlua::Error::runtime)
        })?,
    )?;
    ui.set(
        "radial",
        lua.create_function(
            |_, (cx, cy, r, stops, options): (f32, f32, f32, Table, Option<Table>)| {
                let ry = match &options {
                    Some(o) => o.get::<Option<f32>>("ry")?.unwrap_or(r),
                    None => r,
                };
                let placed = Affine::scale(r, ry)
                    .then(Affine::translation(cx, cy))
                    .then(gradient_transform(options.as_ref())?);
                Brush::radial(placed, even_stops(&stops)?)
                    .map(BrushUd)
                    .map_err(mlua::Error::runtime)
            },
        )?,
    )?;
    ui.set(
        "conic",
        lua.create_function(
            |_, (cx, cy, stops, options): (f32, f32, Table, Option<Table>)| {
                let angle = match &options {
                    Some(o) => o.get::<Option<f32>>("angle")?.unwrap_or(0.),
                    None => 0.,
                };
                let placed = Affine::rotation(angle.to_radians())
                    .then(Affine::translation(cx, cy))
                    .then(gradient_transform(options.as_ref())?);
                Brush::conic(placed, even_stops(&stops)?)
                    .map(BrushUd)
                    .map_err(mlua::Error::runtime)
            },
        )?,
    )?;
    ui.set("render_canvas", lua.create_function(render_canvas)?)?;
    Ok(())
}

/// The fonts a render shapes with beside the system's: the app's windows'
/// (`Platform::fonts`), in their order.
type Fonts = Vec<std::sync::Arc<[u8]>>;
/// Whether two font lists are the same files.
fn same_fonts(a: &Fonts, b: &Fonts) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
}
/// Each of `fonts` that is a font, installed by `install`.
fn each_font(fonts: &Fonts, mut install: impl FnMut(&zgx_gpu::text::FontData)) {
    for bytes in fonts {
        if let Ok(font) = zgx_gpu::text::FontData::new(bytes.clone()) {
            install(&font);
        }
    }
}

thread_local! {
    /// What `c:measure_text` measures in while `ui.render_canvas` draws:
    /// a scene with text services and nothing in it, made again when the
    /// app's fonts change.
    static MEASURING: RefCell<Option<(Fonts, Rc<RefCell<zgx::scene::Scene>>)>> =
        const { RefCell::new(None) };
}
/// The scene `ui.render_canvas` measures in with `fonts`.
fn measuring(fonts: &Fonts) -> Rc<RefCell<zgx::scene::Scene>> {
    MEASURING.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some((had, scene)) = &*slot
            && same_fonts(had, fonts)
        {
            return scene.clone();
        }
        let services = zgx_gpu::TextServices::new();
        each_font(fonts, |font| {
            font.install(&mut services.text_system().borrow_mut())
        });
        let mut scene = zgx::scene::Scene::new(1., 1.);
        services.install(&mut scene);
        let scene = Rc::new(RefCell::new(scene));
        *slot = Some((fonts.clone(), scene.clone()));
        scene
    })
}
/// The text rasterizer `ui.render_canvas`'s threads share: the system's
/// fonts and the app's, loaded again when the app's change.
static TEXT: std::sync::Mutex<Option<(Fonts, zgx_gpu::text::CpuTextRaster)>> =
    std::sync::Mutex::new(None);

/// `ui.render_canvas(draw, { width, height, scale, x, y, background, format, quality })`: a
/// promise of `draw(c, width, height)`'s drawing as a PNG (or a JPEG or
/// WebP, `format`), in base64. The
/// drawing is what a `ui.canvas` given that `draw` shows: its shapes and
/// text in the same layers, clips and transforms, `x`, `y` at the
/// picture's top left. No window shows
/// it, so it works with none showing (a plugin's tool, an export). It draws
/// now; the pixels and the PNG are made on a thread of their own.
fn render_canvas(
    lua: &Lua,
    (draw, options): (mlua::Function, Table),
) -> mlua::Result<crate::calls::PromiseUd> {
    let size = |key: &str| -> mlua::Result<f32> {
        match options.get::<Option<f32>>(key)? {
            Some(v) if v.is_finite() && v > 0. => Ok(v),
            _ => Err(mlua::Error::runtime(format!(
                "ui.render_canvas: `{key}` is a size in points, over 0"
            ))),
        }
    };
    let (width, height) = (size("width")?, size("height")?);
    let scale = match options.get::<Option<f32>>("scale")? {
        None => 1.,
        Some(_) => size("scale")?,
    };
    // The file: "png" (the default), "jpeg" (`quality` 1 to 100, 90 unless
    // said; no transparency, laid over white) or "webp" (lossless).
    let format = match options.get::<Option<String>>("format")?.as_deref() {
        None | Some("png") => Format::Png,
        Some("jpeg" | "jpg") => {
            let quality = options.get::<Option<f64>>("quality")?.unwrap_or(90.);
            Format::Jpeg(quality.round().clamp(1., 100.) as u8)
        }
        Some("webp") => Format::Webp,
        Some(other) => {
            return Err(mlua::Error::runtime(format!(
                "ui.render_canvas: `format` is \"png\", \"jpeg\" or \"webp\", not \"{other}\""
            )));
        }
    };
    let x = options.get::<Option<f32>>("x")?.unwrap_or(0.);
    let y = options.get::<Option<f32>>("y")?.unwrap_or(0.);
    let env = crate::node::env(lua)?;
    let theme = zgx::theme::Themes::of(&env.runtime).root().clone();
    // The app's own fonts (bundled, loaded), as its windows shape with.
    let fonts: Fonts = (env.platform.borrow().as_ref()).map_or_else(Vec::new, |p| p.0.fonts());
    let recorder = lua.create_userdata(Recorder(Default::default()))?;
    let background = draw_in(theme, || {
        let background = match options.get::<mlua::Value>("background")? {
            mlua::Value::Nil => None,
            v => Some(self::color(&v)?),
        };
        zgx::canvas::drawing_for(&measuring(&fonts), || {
            draw.call::<()>((&recorder, width, height))
        })?;
        mlua::Result::Ok(background)
    })?;
    let canvas = recorder.take::<Recorder>()?.0.into_inner();
    let view = zgx::scene::CanvasView {
        scale: 1.,
        x: -x,
        y: -y,
    };
    let (tell, mut heard) = zgx::task::latest::<Result<String, String>>();
    let promise = crate::calls::pending(lua, "ui.render_canvas".into())?;
    std::thread::spawn(move || {
        let mut held = TEXT.lock().unwrap_or_else(|e| e.into_inner());
        if !held
            .as_ref()
            .is_some_and(|(had, _)| same_fonts(had, &fonts))
        {
            let mut text = zgx_gpu::text::CpuTextRaster::default();
            each_font(&fonts, |font| text.register_font(font));
            *held = Some((fonts, text));
        }
        let text = &mut held.as_mut().expect("made just now").1;
        tell.send(
            zgx_gpu::canvas::picture(&canvas, view, width, height, scale, background, text)
                .and_then(|image| encode(&image, format))
                .map(|file| base64(&file))
                .map_err(|e| e.0),
        );
    });
    crate::calls::settle_with(lua, &promise, crate::errors::Kind::Io, async move {
        match heard.recv().await {
            Some(Ok(png)) => Ok(crate::Value::Str(png.into())),
            Some(Err(error)) => Err(format!("ui.render_canvas: {error}")),
            None => Err("ui.render_canvas: the drawing thread stopped".into()),
        }
    })?;
    Ok(crate::calls::PromiseUd(promise))
}

/// The file `ui.render_canvas` makes.
#[derive(Clone, Copy)]
enum Format {
    Png,
    Jpeg(u8),
    Webp,
}
fn encode(image: &zgx::image::ImageData, format: Format) -> Result<Vec<u8>, zgx_gpu::GpuError> {
    #[allow(unused_variables)]
    let without = |what: &str| -> Result<Vec<u8>, zgx_gpu::GpuError> {
        Err(zgx_gpu::GpuError(format!(
            "zgx-script is built without `{what}`"
        )))
    };
    match format {
        Format::Png => png(image),
        #[cfg(feature = "jpeg")]
        Format::Jpeg(quality) => zgx_gpu::canvas::jpeg(image, quality),
        #[cfg(not(feature = "jpeg"))]
        Format::Jpeg(_) => without("jpeg"),
        #[cfg(feature = "webp")]
        Format::Webp => zgx_gpu::canvas::webp(image),
        #[cfg(not(feature = "webp"))]
        Format::Webp => without("webp"),
    }
}
#[cfg(feature = "png")]
fn png(image: &zgx::image::ImageData) -> Result<Vec<u8>, zgx_gpu::GpuError> {
    zgx_gpu::canvas::png(image)
}
#[cfg(not(feature = "png"))]
fn png(_: &zgx::image::ImageData) -> Result<Vec<u8>, zgx_gpu::GpuError> {
    Err(zgx_gpu::GpuError(
        "zgx-script is built without `png`".into(),
    ))
}
/// `bytes` in base64 (RFC 4648, padded): what an MCP image's data is.
fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(DIGITS[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
/// A gradient's colours, spread evenly from offset 0 to 1.
fn even_stops(stops: &Table) -> mlua::Result<Vec<GradientStop>> {
    let colors: Vec<mlua::Value> = stops.sequence_values().collect::<mlua::Result<_>>()?;
    let n = colors.len().max(2) - 1;
    colors
        .iter()
        .enumerate()
        .map(|(i, c)| {
            Ok(GradientStop {
                offset: i as f32 / n as f32,
                color: color(c)?,
            })
        })
        .collect()
}
/// A gradient's `transform` option, `{ a, b, c, d, e, f }` as SVG's
/// `matrix` (x' is a x + c y + e, y' is b x + d y + f): what it maps the
/// gradient by once placed, as a drawing's own transform. Identity without
/// one.
fn gradient_transform(options: Option<&Table>) -> mlua::Result<Affine> {
    let Some(matrix) = options
        .map(|o| o.get::<Option<Vec<f32>>>("transform"))
        .transpose()?
        .flatten()
    else {
        return Ok(Affine::IDENTITY);
    };
    let [a, b, c, d, tx, ty] = matrix[..] else {
        return Err(mlua::Error::runtime(
            "a gradient's `transform` is six numbers, { a, b, c, d, e, f }",
        ));
    };
    Ok(Affine { a, b, c, d, tx, ty })
}
