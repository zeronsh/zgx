//! Windows' caption buttons, as `Titlebar::Inset` draws them: where they are,
//! worked out once in the window's physical pixels, and what shows them.
//!
//! What is drawn, what Windows hit-tests (`WM_NCHITTEST`, so snap layouts) and
//! what `WindowInfo::controls` reports all read one [`Geometry`], so they
//! agree at any display scale and zoom. Like the buttons Windows would draw,
//! they don't zoom with the content: 46 by 32 points of the display whatever
//! the window's zoom, unless an app's `caption_height` has them span its own
//! title bar, which does zoom.
//!
//! The glyphs are boxes and bars, whole pixels each, not text: no icon font
//! to be missing (Windows 10 showed Segoe's as empty boxes). They are inked
//! from the app's theme as its own title bar controls are, quiet at rest, so
//! they sit with the app's icons rather than over them.
//!
//! Apart from the window procedure (`window_chrome::win`), so what they work
//! out is tested on every platform.
use zgx::{
    compose::prelude::*,
    reactive::Signal,
    scene::{Color, Rect},
    style::Styled,
    theme::{Themed, tok},
    widgets::Ui,
};

/// Windows 11's caption buttons, in points of the display.
const BUTTON_WIDTH: f64 = 46.;
const BUTTON_HEIGHT: f64 = 32.;
/// Resize bands inside a frameless window's edges, in points of the display:
/// it has no frame outside them to take the pointer.
const RESIZE_BAND: f64 = 5.;
/// WinUI's caption glyphs, in points of the display: a 10-point square (the
/// restored window's two 8-point ones in it).
const GLYPH: f64 = 10.;
const RESTORE_BOX: f64 = 8.;
/// Their line: an outline icon's (1.75 at 24 points) at 16, so they weigh
/// what the app's own icons do. 1 pixel at 100% and 125%, 2 at 150-200%.
const STROKE: f64 = 7. / 6.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptionButton {
    Minimize,
    Maximize,
    Close,
}

/// What lies at a point of a frameless window, if not its content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hit {
    Button(CaptionButton),
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// A frameless window's caption, in its physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    /// The display's scale (its DPI's), without the zoom.
    scale: f64,
    /// The window's zoom: the content's points are `scale * zoom` pixels.
    zoom: f64,
    /// One button's width and the strip's height.
    button: i32,
    height: i32,
    /// The person can resize it: it has resize bands and a Maximize button.
    resizable: bool,
    /// Windows run right to left: the buttons are at its top left.
    rtl: bool,
}
impl Geometry {
    /// For a window at display `scale` and `zoom`. `caption_height` is in the
    /// content's points, so the strip spans a title bar of that height at
    /// any zoom; without one it is Windows' 32 points of the display.
    pub(crate) fn new(
        scale: f64,
        zoom: f64,
        caption_height: Option<f64>,
        resizable: bool,
        rtl: bool,
    ) -> Self {
        let height = caption_height.map_or(BUTTON_HEIGHT * scale, |h| h * scale * zoom);
        Self {
            scale,
            zoom,
            button: ((BUTTON_WIDTH * scale).round() as i32).max(1),
            height: (height.round() as i32).max(1),
            resizable,
            rtl,
        }
    }
    /// How many buttons show: a fixed-size window has no Maximize.
    fn count(&self) -> i32 {
        if self.resizable { 3 } else { 2 }
    }
    /// `pixels` in the content's points.
    fn points(&self, pixels: f64) -> f32 {
        (pixels / (self.scale * self.zoom)) as f32
    }
    /// What client point (`x`, `y`) of a window `width` by `height` pixels
    /// is, if not its content: a resize band, or one of its caption buttons
    /// (`buttons`), at its top right, or top left where it runs right to
    /// left, as Windows lays out a mirrored (`WS_EX_LAYOUTRTL`) window's.
    pub(crate) fn hit(
        &self,
        (x, y): (i32, i32),
        (width, height): (i32, i32),
        maximized: bool,
        buttons: bool,
    ) -> Option<Hit> {
        let band = if maximized || !self.resizable {
            0
        } else {
            (RESIZE_BAND * self.scale).round() as i32
        };
        let (left, right) = (x < band, x >= width - band);
        let (top, bottom) = (y < band, y >= height - band);
        // The caption buttons keep all but the window's outermost pixels.
        if buttons && y >= band.min(1) && y < self.height {
            // Close nearest the edge they sit at.
            let from_edge = if self.rtl { x } else { width - 1 - x };
            if (band.min(1)..self.count() * self.button).contains(&from_edge) {
                return Some(Hit::Button(match (from_edge / self.button, self.count()) {
                    (0, _) => CaptionButton::Close,
                    (1, 3) => CaptionButton::Maximize,
                    _ => CaptionButton::Minimize,
                }));
            }
        }
        Some(match (top, bottom, left, right) {
            (true, _, true, _) => Hit::TopLeft,
            (true, _, _, true) => Hit::TopRight,
            (_, true, true, _) => Hit::BottomLeft,
            (_, true, _, true) => Hit::BottomRight,
            (true, ..) => Hit::Top,
            (_, true, ..) => Hit::Bottom,
            (.., true, _) => Hit::Left,
            (.., true) => Hit::Right,
            _ => return None,
        })
    }
    /// The caption strip of a window `width` pixels wide, less its buttons
    /// (`buttons`), and those buttons, in points of the display
    /// (`WindowInfo::refresh` puts them in the content's).
    pub(crate) fn strip(&self, width: i32, buttons: bool) -> (Rect, Option<Rect>) {
        let point = |pixels: i32| (f64::from(pixels) / self.scale) as f32;
        let (height, all) = (point(self.height), point(width));
        if !buttons {
            return (Rect::new(0., 0., all, height), None);
        }
        let taken = (self.count() * self.button).min(width);
        let (side, rest) = (point(taken), point(width - taken));
        if self.rtl {
            return (
                Rect::new(side, 0., rest, height),
                Some(Rect::new(0., 0., side, height)),
            );
        }
        (
            Rect::new(0., 0., rest, height),
            Some(Rect::new(rest, 0., side, height)),
        )
    }
    /// A stroke's pixels: whole, as the glyphs are drawn crisp.
    fn stroke(&self) -> f64 {
        (STROKE * self.scale).round().max(1.)
    }
    /// The glyph's square side, in pixels.
    fn side(&self) -> f64 {
        (GLYPH * self.scale).round().max(1.)
    }
    /// The glyph's square's top left in its button, on a whole pixel.
    fn glyph_at(&self) -> (f64, f64) {
        (
            ((f64::from(self.button) - self.side()) / 2.).floor(),
            ((f64::from(self.height) - self.side()) / 2.).floor(),
        )
    }
    /// The restored window's front box's side, and how far its back box
    /// stands up and to the right of it, in pixels.
    fn restore(&self) -> (f64, f64) {
        let front = (RESTORE_BOX * self.scale).round().min(self.side());
        (front, self.side() - front)
    }
}

/// Close's red under the pointer (WinUI's), and pressed.
const CLOSE_RED: Color = Color(196, 43, 28, 255);
const CLOSE_RED_PRESSED: Color = Color(196, 43, 28, 230);

/// The wash under a button in `kind`'s state: the app's theme's `hover` and
/// `pressed`, as under its own controls; Close's red.
fn fill(kind: CaptionButton, hover: bool, down: bool) -> Themed<Color> {
    match (kind, hover, down) {
        (_, false, _) => Color(0, 0, 0, 0).into(),
        (CaptionButton::Close, _, false) => CLOSE_RED.into(),
        (CaptionButton::Close, _, true) => CLOSE_RED_PRESSED.into(),
        (_, _, false) => tok::HOVER.into(),
        (_, _, true) => tok::PRESSED.into(),
    }
}
/// The glyph's ink, from the app's theme as its title bar's other controls:
/// secondary at rest, primary under the pointer, tertiary in a window in the
/// back; white over Close's red (`red`). In a contrast theme `hover` is the
/// opaque highlight, which its window text doesn't read on (white on Night
/// sky's cyan): under the pointer then, the highlight's own text, `accent.text`.
fn ink(red: bool, hover: bool, active: bool, high_contrast: bool) -> Themed<Color> {
    match (red, hover, active) {
        (true, ..) => Color(255, 255, 255, 255).into(),
        (_, true, _) if high_contrast => tok::ACCENT_TEXT.into(),
        (_, true, _) => tok::TEXT_PRIMARY.into(),
        (_, _, true) => tok::TEXT_SECONDARY.into(),
        (_, _, false) => tok::TEXT_TERTIARY.into(),
    }
}

/// What the buttons show, set by their host before a frame.
#[derive(Clone)]
pub(crate) struct CaptionLook {
    pub(crate) hovered: Signal<Option<CaptionButton>>,
    pub(crate) pressed: Signal<Option<CaptionButton>>,
    pub(crate) maximized: Signal<bool>,
    pub(crate) active: Signal<bool>,
    pub(crate) fullscreen: Signal<bool>,
    pub(crate) geometry: Signal<Geometry>,
    /// A contrast theme is on (`SystemPreferences::increased_contrast`).
    pub(crate) high_contrast: Signal<bool>,
}
impl CaptionLook {
    pub(crate) fn new(ui: &Ui, geometry: Geometry) -> Self {
        Self {
            hovered: ui.signal(None),
            pressed: ui.signal(None),
            maximized: ui.signal(false),
            active: ui.signal(true),
            fullscreen: ui.signal(false),
            geometry: ui.signal(geometry),
            high_contrast: ui.signal(false),
        }
    }
    /// The glyph's colour on `kind`'s button now.
    fn ink(&self, kind: CaptionButton) -> Themed<Color> {
        let hover = self.hovered.get() == Some(kind);
        ink(
            kind == CaptionButton::Close && hover,
            hover,
            self.active.get(),
            self.high_contrast.get(),
        )
    }
    /// The wash under `kind`'s button now.
    fn fill(&self, kind: CaptionButton) -> Themed<Color> {
        let hover = self.hovered.get() == Some(kind);
        fill(kind, hover, hover && self.pressed.get() == Some(kind))
    }
    /// The buttons, over the content's top right, or top left right to left:
    /// there the row runs from the right, Close outermost still.
    pub(crate) fn view(&self, rtl: bool) -> View {
        let fullscreen = self.fullscreen.clone();
        let strip = row().absolute().top(0.);
        let strip = if rtl { strip.left(0.) } else { strip.right(0.) };
        strip
            .pointer_events(false)
            .reactive_style(move || Styles::new().opacity(if fullscreen.get() { 0. } else { 1. }))
            .child(self.button(CaptionButton::Minimize, "zgx-caption-minimize"))
            .child(self.button(CaptionButton::Maximize, "zgx-caption-maximize"))
            .child(self.button(CaptionButton::Close, "zgx-caption-close"))
    }
    fn button(&self, kind: CaptionButton, id: &'static str) -> View {
        let look = self.clone();
        let glyph = match kind {
            CaptionButton::Minimize => vec![self.minimize_glyph()],
            CaptionButton::Maximize => self.maximize_glyph(),
            CaptionButton::Close => self.close_glyph(),
        };
        let at = self.clone();
        let inside = div()
            .absolute()
            .id(format!("{id}-glyph"))
            .reactive_style(move || {
                let geometry = at.geometry.get();
                let (x, y) = geometry.glyph_at();
                let side = geometry.points(geometry.side());
                Styles::new()
                    .left(geometry.points(x))
                    .top(geometry.points(y))
                    .size(side, side)
            });
        div()
            .id(id)
            .overflow_hidden()
            .reactive_style(move || {
                let geometry = look.geometry.get();
                // A fixed-size window's Maximize takes no room and shows nothing.
                let width = if kind == CaptionButton::Maximize && !geometry.resizable {
                    0.
                } else {
                    geometry.points(f64::from(geometry.button))
                };
                Styles::new()
                    .size(width, geometry.points(f64::from(geometry.height)))
                    .bg(look.fill(kind))
            })
            .child(inside.children(glyph))
    }
    /// A bar across the middle.
    fn minimize_glyph(&self) -> View {
        let look = self.clone();
        div().absolute().reactive_style(move || {
            let geometry = look.geometry.get();
            let (side, stroke) = (geometry.side(), geometry.stroke());
            Styles::new()
                .left(0.)
                .top(geometry.points(((side - stroke) / 2.).floor()))
                .size(geometry.points(side), geometry.points(stroke))
                .bg(look.ink(CaptionButton::Minimize))
        })
    }
    /// A box; restored, a smaller one with the top and right of another
    /// standing behind it.
    fn maximize_glyph(&self) -> Vec<View> {
        const KIND: CaptionButton = CaptionButton::Maximize;
        let front = {
            let look = self.clone();
            div().absolute().reactive_style(move || {
                let geometry = look.geometry.get();
                let (x, y, box_side) = if look.maximized.get() {
                    let (side, up) = geometry.restore();
                    (0., up, side)
                } else {
                    (0., 0., geometry.side())
                };
                let stroke = geometry.points(geometry.stroke());
                Styles::new()
                    .left(geometry.points(x))
                    .top(geometry.points(y))
                    .size(geometry.points(box_side), geometry.points(box_side))
                    .border(stroke)
                    .border_color(look.ink(KIND))
                    .rounded(stroke)
            })
        };
        let back_top = {
            let look = self.clone();
            div().absolute().reactive_style(move || {
                let geometry = look.geometry.get();
                let (_, up) = geometry.restore();
                Styles::new()
                    .left(geometry.points(up))
                    .top(0.)
                    .size(
                        geometry.points(geometry.side() - up),
                        geometry.points(geometry.stroke()),
                    )
                    .bg(look.ink(KIND))
                    .opacity(if look.maximized.get() { 1. } else { 0. })
            })
        };
        let back_right = {
            let look = self.clone();
            div().absolute().reactive_style(move || {
                let geometry = look.geometry.get();
                let (front, _) = geometry.restore();
                Styles::new()
                    .left(geometry.points(geometry.side() - geometry.stroke()))
                    .top(0.)
                    .size(geometry.points(geometry.stroke()), geometry.points(front))
                    .bg(look.ink(KIND))
                    .opacity(if look.maximized.get() { 1. } else { 0. })
            })
        };
        vec![front, back_top, back_right]
    }
    /// Two diagonals of the glyph's square.
    fn close_glyph(&self) -> Vec<View> {
        let line = |turn: f32| {
            let look = self.clone();
            div().absolute().rotate(turn).reactive_style(move || {
                let geometry = look.geometry.get();
                let (side, stroke) = (geometry.side(), geometry.stroke());
                let length = side * std::f64::consts::SQRT_2;
                Styles::new()
                    .left(geometry.points((side - length) / 2.))
                    .top(geometry.points((side - stroke) / 2.))
                    .size(geometry.points(length), geometry.points(stroke))
                    .bg(look.ink(CaptionButton::Close))
            })
        };
        vec![
            line(std::f32::consts::FRAC_PI_4),
            line(-std::f32::consts::FRAC_PI_4),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptionButton::*, *};

    /// Display scales and zooms the buttons must agree at.
    const SCALES: [f64; 4] = [1., 1.25, 1.5, 2.];
    const ZOOMS: [f64; 5] = [1., 0.9, 1.1, 1.25, 1.5];

    fn at(geometry: Geometry, x: i32, y: i32, width: i32) -> Option<Hit> {
        geometry.hit((x, y), (width, 2000), false, true)
    }
    fn flat(scale: f64) -> Geometry {
        Geometry::new(scale, 1., None, true, false)
    }

    /// Each drawn button's left, top, width and height in physical pixels,
    /// as a window of `width` pixels lays them out.
    fn drawn(geometry: Geometry, width: i32) -> Vec<(CaptionButton, (f32, f32, f32, f32))> {
        let pixel = geometry.scale * geometry.zoom;
        let mut ui = Ui::new(geometry.points(f64::from(width)), 600.);
        // Right to left, the window runs so (`system_colors::sync`).
        if geometry.rtl {
            let mut scene = ui.scene.borrow_mut();
            let root = scene.root();
            scene.set_direction(root, Some(zgx::scene::Direction::Rtl));
        }
        let look = CaptionLook::new(&ui, geometry);
        let view = ui.mount(look.view(geometry.rtl));
        ui.prepare_frame();
        let scene = ui.scene.borrow();
        [
            (Minimize, "zgx-caption-minimize"),
            (Maximize, "zgx-caption-maximize"),
            (Close, "zgx-caption-close"),
        ]
        .into_iter()
        .filter_map(|(kind, id)| {
            let rect = scene.bounds(view.find(id)?);
            let px = |v: f32| f64::from(v) * pixel;
            (rect.width > 0.).then_some((
                kind,
                (
                    px(rect.x) as f32,
                    px(rect.y) as f32,
                    px(rect.width) as f32,
                    px(rect.height) as f32,
                ),
            ))
        })
        .collect()
    }

    #[test]
    fn drawn_hit_and_reported_agree_at_every_scale_and_zoom() {
        let width = 1860;
        for rtl in [false, true] {
            for resizable in [true, false] {
                for scale in SCALES {
                    for zoom in ZOOMS {
                        let g = Geometry::new(scale, zoom, None, resizable, rtl);
                        let case = format!("{scale} × {zoom} rtl={rtl} resizable={resizable}");
                        let buttons = drawn(g, width);
                        assert_eq!(buttons.len(), if resizable { 3 } else { 2 }, "{case}");
                        for (kind, (x, y, w, h)) in &buttons {
                            let (x, w, h) = (*x, *w, *h);
                            assert!(y.abs() < 0.01, "{case}: {kind:?} at y {y}");
                            assert!((w - g.button as f32).abs() < 0.01, "{case}: {kind:?} {w}");
                            assert!((h - g.height as f32).abs() < 0.01, "{case}: {kind:?} {h}");
                            // Every pixel drawn as this button hit-tests as
                            // it, and those beside it don't (the strip's
                            // outermost pixel stays the resize band).
                            let (first, last) = (x.round() as i32, (x + w).round() as i32 - 1);
                            let row = g.height / 2;
                            let inner = |p: i32| {
                                if resizable {
                                    (1..width - 1).contains(&p)
                                } else {
                                    (0..width).contains(&p)
                                }
                            };
                            for p in [first, first + 1, last - 1, last] {
                                if inner(p) {
                                    assert_eq!(
                                        at(g, p, row, width),
                                        Some(Hit::Button(*kind)),
                                        "{case}: pixel {p} of {kind:?} drawn {first}..={last}"
                                    );
                                }
                            }
                            for p in [first - 1, last + 1] {
                                if inner(p) {
                                    assert_ne!(
                                        at(g, p, row, width),
                                        Some(Hit::Button(*kind)),
                                        "{case}: pixel {p} beside {kind:?}"
                                    );
                                }
                            }
                            assert_ne!(
                                at(g, first + 2, g.height, width),
                                Some(Hit::Button(*kind)),
                                "{case}: below {kind:?}"
                            );
                        }
                        // Reported, as WindowInfo has it, in the content's
                        // points: the drawn buttons' extent.
                        let (_, controls) = g.strip(width, true);
                        let controls = controls.unwrap();
                        let pixel = |v: f32| f64::from(v / zoom as f32) * scale * zoom;
                        let (left, right) = buttons.iter().fold((f32::MAX, 0f32), |(l, r), b| {
                            (l.min(b.1.0), r.max(b.1.0 + b.1.2))
                        });
                        let near = |a: f64, b: f32| (a - f64::from(b)).abs() < 0.05;
                        assert!(near(pixel(controls.x), left), "{case}: {controls:?} {left}");
                        assert!(
                            near(pixel(controls.x + controls.width), right),
                            "{case}: {controls:?} {right}"
                        );
                        assert!(near(pixel(controls.height), g.height as f32), "{case}");
                    }
                }
            }
        }
    }

    /// The ask's case: at 150% zoom the buttons were drawn 207 pixels wide,
    /// hit-tested 138, and reported 92 points.
    #[test]
    fn zoomed_150_percent_they_are_138_pixels_drawn_hit_and_reported() {
        let g = Geometry::new(1., 1.5, None, true, false);
        let width = 1860;
        let buttons = drawn(g, width);
        let span = buttons.last().unwrap().1.0 + buttons.last().unwrap().1.2 - buttons[0].1.0;
        assert!((span - 138.).abs() < 0.01, "{span}");
        assert_eq!(at(g, width - 138, 16, width), Some(Hit::Button(Minimize)));
        assert_eq!(at(g, width - 139, 16, width), None);
        let (_, controls) = g.strip(width, true);
        assert_eq!(
            controls.unwrap().width / 1.5,
            92.,
            "in the content's points"
        );
    }

    #[test]
    fn a_frameless_windows_edges_resize_and_its_corner_buttons_answer() {
        let size = (800, 600);
        let g = flat(1.);
        let at = |x, y| g.hit((x, y), size, false, true);
        assert_eq!(at(400, 300), None, "content");
        assert_eq!(at(0, 300), Some(Hit::Left));
        assert_eq!(at(799, 300), Some(Hit::Right));
        assert_eq!(at(400, 599), Some(Hit::Bottom));
        assert_eq!(at(0, 0), Some(Hit::TopLeft));
        assert_eq!(at(799, 599), Some(Hit::BottomRight));
        // The buttons, 46 wide from the right, 32 tall; the outermost pixel
        // stays the resize band.
        assert_eq!(at(790, 10), Some(Hit::Button(Close)));
        assert_eq!(at(799, 10), Some(Hit::Right));
        assert_eq!(at(799, 2), Some(Hit::TopRight));
        assert_eq!(at(740, 10), Some(Hit::Button(Maximize)));
        assert_eq!(at(700, 10), Some(Hit::Button(Minimize)));
        assert_eq!(at(640, 10), None, "left of the buttons");
        assert_eq!(at(790, 40), None, "below the buttons");
        // Without buttons (`Titlebar::Hidden`) the corner resizes.
        assert_eq!(g.hit((790, 2), size, false, false), Some(Hit::Top));
    }

    #[test]
    fn bands_and_buttons_scale_and_a_maximized_window_has_no_bands() {
        let size = (1600, 1200);
        // At 200% the band is 10 pixels and the buttons 92 by 64.
        let g = flat(2.);
        assert_eq!(g.hit((9, 600), size, false, false), Some(Hit::Left));
        assert_eq!(g.hit((10, 600), size, false, false), None);
        assert_eq!(
            g.hit((1599 - 91, 60), size, false, true),
            Some(Hit::Button(Close))
        );
        assert_eq!(
            g.hit((1599 - 92, 60), size, false, true),
            Some(Hit::Button(Maximize))
        );
        // Maximized, the edges are the screen's: nothing resizes, and the
        // buttons reach the very corner.
        let g = flat(1.);
        assert_eq!(g.hit((0, 600), size, true, true), None);
        assert_eq!(g.hit((1599, 0), size, true, true), Some(Hit::Button(Close)));
    }

    #[test]
    fn a_fixed_size_window_has_no_maximize_and_no_resize_bands() {
        let size = (800, 600);
        let g = Geometry::new(1., 1., None, false, false);
        let at = |x, y| g.hit((x, y), size, false, true);
        assert_eq!(at(790, 10), Some(Hit::Button(Close)));
        assert_eq!(at(799, 0), Some(Hit::Button(Close)), "to the very corner");
        // Minimize next to Close, where Maximize would be.
        assert_eq!(at(740, 10), Some(Hit::Button(Minimize)));
        assert_eq!(at(700, 10), None, "the Minimize of three buttons");
        for edge in [
            (0, 300),
            (799, 300),
            (400, 0),
            (400, 599),
            (0, 0),
            (799, 599),
        ] {
            assert_eq!(at(edge.0, edge.1), None, "{edge:?}");
        }
        let (area, controls) = g.strip(800, true);
        assert_eq!(area, Rect::new(0., 0., 708., 32.));
        assert_eq!(controls, Some(Rect::new(708., 0., 92., 32.)));
    }

    #[test]
    fn the_titlebar_area_is_the_caption_strip_less_the_buttons() {
        assert_eq!(
            flat(1.).strip(800, true),
            (
                Rect::new(0., 0., 662., 32.),
                Some(Rect::new(662., 0., 138., 32.))
            )
        );
        // `Hidden` has no buttons: the whole strip is the app's.
        assert_eq!(
            flat(1.).strip(800, false),
            (Rect::new(0., 0., 800., 32.), None)
        );
    }

    #[test]
    fn right_to_left_the_buttons_and_the_titlebar_area_mirror() {
        // As Windows lays out a mirrored window's: the buttons at the top
        // left, Close outermost.
        let g = Geometry::new(1., 1., None, true, true);
        let size = (800, 600);
        let at = |x, y| g.hit((x, y), size, false, true);
        assert_eq!(at(10, 10), Some(Hit::Button(Close)));
        assert_eq!(at(60, 10), Some(Hit::Button(Maximize)));
        assert_eq!(at(100, 10), Some(Hit::Button(Minimize)));
        assert_eq!(at(160, 10), None, "right of the buttons");
        assert_eq!(at(798, 10), Some(Hit::Right), "the right edge only resizes");
        assert_eq!(
            at(0, 10),
            Some(Hit::Left),
            "the outermost pixel still resizes"
        );
        assert_eq!(
            g.strip(800, true),
            (
                Rect::new(138., 0., 662., 32.),
                Some(Rect::new(0., 0., 138., 32.))
            )
        );
    }

    #[test]
    fn a_caption_height_spans_a_title_bar_of_that_height_at_any_zoom() {
        for (scale, zoom) in [(1., 1.), (1.25, 1.), (1., 1.5), (1.5, 0.9), (2., 1.25)] {
            let g = Geometry::new(scale, zoom, Some(38.), true, false);
            let tall = (38. * scale * zoom).round() as i32;
            assert_eq!(g.height, tall);
            let width = 2000;
            let (area, controls) = g.strip(width, true);
            for rect in [area, controls.unwrap()] {
                // In the content's points, as reported: within a pixel of 38.
                let reported = f64::from(rect.height) / zoom;
                assert!((reported - 38.).abs() * scale * zoom <= 0.51, "{reported}");
            }
            for (_, (_, _, _, h)) in drawn(g, width) {
                assert!((h - tall as f32).abs() < 0.01, "{h} of {tall}");
            }
            assert_eq!(at(g, width - 30, tall - 1, width), Some(Hit::Button(Close)));
            assert_eq!(at(g, width - 30, tall, width), None);
        }
        // Without one: 32 points of the display, whatever the zoom.
        assert_eq!(Geometry::new(1.5, 1.5, None, true, false).height, 48);
    }

    /// Whole pixels, centred, at the sizes WinUI's are at 100%.
    #[test]
    fn the_glyphs_are_whole_pixels_centred_in_their_button() {
        let g = flat(1.);
        assert_eq!((g.side(), g.stroke()), (10., 1.));
        assert_eq!(g.glyph_at(), (18., 11.));
        assert_eq!(g.restore(), (8., 2.));
        let g = flat(1.25);
        assert_eq!((g.side(), g.stroke()), (13., 1.));
        let g = flat(1.5);
        assert_eq!((g.side(), g.stroke()), (15., 2.));
        assert_eq!(g.glyph_at(), (27., 16.));
        let g = flat(2.);
        assert_eq!((g.side(), g.stroke()), (20., 2.));
        for scale in SCALES {
            let g = flat(scale);
            let (x, y) = g.glyph_at();
            assert_eq!((x.fract(), y.fract()), (0., 0.));
            // Not moved by the zoom: the same pixels at 150%.
            assert_eq!(
                Geometry::new(scale, 1.5, None, true, false).glyph_at(),
                (x, y)
            );
        }
    }

    /// The glyphs are nodes the engine paints, no text: nothing to be missing.
    #[test]
    fn the_glyphs_draw_no_text() {
        let mut ui = Ui::new(800., 600.);
        let look = CaptionLook::new(&ui, flat(1.));
        ui.mount(look.view(false));
        ui.prepare_frame();
        let scene = ui.scene.borrow();
        assert!(
            !scene
                .paint_items()
                .any(|item| matches!(item.kind, zgx::scene::NodeKind::Text { .. })),
            "a glyph is text"
        );
    }

    /// The glyphs take the app's text colours and washes, as its own title
    /// bar controls do, so a light app on a dark system has dark glyphs:
    /// secondary at rest, primary under the pointer, tertiary in the back;
    /// Close's red and white glyph kept.
    #[test]
    fn the_ink_is_the_apps_text_and_washes() {
        let ink_of = |red, hover, active| ink(red, hover, active, false);
        assert_eq!(
            ink_of(false, false, true),
            Themed::from(tok::TEXT_SECONDARY)
        );
        assert_eq!(ink_of(false, true, true), Themed::from(tok::TEXT_PRIMARY));
        assert_eq!(
            ink_of(false, false, false),
            Themed::from(tok::TEXT_TERTIARY)
        );
        assert_eq!(
            ink_of(true, true, true),
            Themed::from(Color(255, 255, 255, 255)),
            "on red"
        );
        // A contrast theme's hover is its highlight: its own text on it, and
        // Close keeps its red and white.
        assert_eq!(ink(false, true, true, true), Themed::from(tok::ACCENT_TEXT));
        assert_eq!(
            ink(false, false, true, true),
            Themed::from(tok::TEXT_SECONDARY)
        );
        assert_eq!(
            ink(true, true, true, true),
            Themed::from(Color(255, 255, 255, 255))
        );
        assert_eq!(fill(Minimize, true, false), Themed::from(tok::HOVER));
        assert_eq!(fill(Maximize, true, true), Themed::from(tok::PRESSED));
        assert_eq!(fill(Close, true, false), Themed::from(CLOSE_RED));
        assert_eq!(fill(Close, false, false), Themed::from(Color(0, 0, 0, 0)));
        let mut ui = Ui::new(800., 600.);
        let look = CaptionLook::new(&ui, flat(1.));
        ui.mount(look.view(false));
        ui.prepare_frame();
        assert_eq!(look.ink(Minimize), Themed::from(tok::TEXT_SECONDARY));
        look.hovered.set(Some(Minimize));
        ui.prepare_frame();
        assert_eq!(look.ink(Minimize), Themed::from(tok::TEXT_PRIMARY));
        assert_eq!(look.fill(Minimize), Themed::from(tok::HOVER));
        look.high_contrast.set(true);
        ui.prepare_frame();
        assert_eq!(look.ink(Minimize), Themed::from(tok::ACCENT_TEXT));
    }
}
