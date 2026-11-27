//! A long text's layout as its lines: an editor holding a file of 100,000
//! lines shapes the few in view, not the file. Lines are shaped when asked
//! for (a caret, a hit, a line shown) and kept by their text, so an edit
//! reshapes the line it changed and no other; where each line starts is an
//! index of offsets, a few bytes a line. Unwrapped, every line is a row
//! high; wrapped, a line's height is estimated from its length until it is
//! shaped, then corrected.
//!
//! Each hard line is laid out on its own, as a shaper lays out each of a
//! text's paragraphs, so carets, hits and selections are those of the
//! whole text's layout.
use crate::{
    scene::Rect,
    text::Text,
    text_layout::{TextAffinity, TextLayout, TextPosition},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ops::Range,
    rc::Rc,
    sync::Arc,
};

/// Shapes one line, wrapped at a width if given (`Scene::line_shaper`).
pub(crate) type LineShaper = Rc<dyn Fn(&str, Option<f32>) -> Box<dyn TextLayout>>;
/// Shapes a styled line (`Scene::rich_line_shaper`).
pub(crate) type RichLineShaper =
    Rc<dyn Fn(&crate::rich_text::RichText, Option<f32>) -> Box<dyn TextLayout>>;

/// A line's colours: byte ranges of the line, each with the colour its
/// text shows in (the rest, the editor's).
pub type LineColors = Vec<(Range<usize>, crate::scene::Color)>;
/// Colours a line of code (`EditorHandle::set_highlighter`), given what the
/// line before it left open (`0` before the first: a block comment or a
/// string running on are other numbers, the highlighter's own): its colours
/// and what it leaves the next line. It is asked for the lines that show
/// (and those above them, once, for what they leave), and again only for a
/// line that changes, or whose line before leaves another state; so it
/// must depend on nothing but its arguments.
pub type Highlighter = Rc<dyn Fn(&str, u32) -> (LineColors, u32)>;

/// How a range of a line shows (`SpanHighlighter`). Where spans overlap,
/// each of colour, weight and background is the first's (by start) that
/// sets it, and the flags are any's: a code span inside bold text keeps the
/// bold weight and shows its own colour.
///
/// Built from its default (or a colour) with its methods, so styles added
/// later leave the code that builds these as it is:
/// `SpanStyle::default().weight(600).italic()`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SpanStyle {
    /// Its text's colour (none: the editor's).
    pub color: Option<crate::scene::Color>,
    /// Its font weight (`600`): shaped so, so carets and hits follow it.
    pub weight: Option<u16>,
    /// Italic: shaped so.
    pub italic: bool,
    /// A line through it.
    pub strike: bool,
    /// A wash behind it (inline code).
    pub background: Option<crate::scene::Color>,
    /// A line under it (a link).
    pub underline: bool,
    /// Its look beyond these, which few spans have (a mention's, an
    /// equation's): its place in its editor's looks (`SpanLook`), from 1.
    /// Four bytes, so a plain span pays for no more.
    pub(crate) look: Option<std::num::NonZeroU32>,
    /// Its size, line pitch and font beyond the editor's, which few spans
    /// have (a note's heading, its inline code): their name among the
    /// thread's (`SpanText`), from 1. Two bytes, in room the span had.
    pub(crate) text: Option<std::num::NonZeroU16>,
}
impl From<crate::scene::Color> for SpanStyle {
    fn from(color: crate::scene::Color) -> Self {
        Self {
            color: Some(color),
            ..Self::default()
        }
    }
}
impl SpanStyle {
    /// Its text in `color`.
    pub const fn color(mut self, color: crate::scene::Color) -> Self {
        self.color = Some(color);
        self
    }
    /// Its text in font weight `weight` (`600`).
    pub const fn weight(mut self, weight: u16) -> Self {
        self.weight = Some(weight);
        self
    }
    /// Its text italic.
    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
    /// A line through its text.
    pub const fn strike(mut self) -> Self {
        self.strike = true;
        self
    }
    /// A wash of `color` behind its text (inline code).
    pub const fn background(mut self, color: crate::scene::Color) -> Self {
        self.background = Some(color);
        self
    }
    /// A line under its text.
    pub const fn underline(mut self) -> Self {
        self.underline = true;
        self
    }
    /// Its text `size` points large (a heading in a note): shaped so.
    #[doc(hidden)]
    pub fn size(self, size: f32) -> Self {
        self.with_text(|text| text.size = Some(size))
    }
    /// Its line `line_height` points high (a heading's): the line takes
    /// the tallest of its runs'.
    #[doc(hidden)]
    pub fn line_height(self, line_height: f32) -> Self {
        self.with_text(|text| text.line_height = Some(line_height))
    }
    /// Its text in the monospaced font (inline code): shaped so.
    #[doc(hidden)]
    pub fn mono(self) -> Self {
        self.with_text(|text| text.mono = true)
    }
    /// Its background as a chip: `padding` points across and down round
    /// its text (across, taking room in the line), corners of `radius`.
    #[doc(hidden)]
    pub fn chip(self, padding: [f32; 2], radius: f32) -> Self {
        self.with_text(|text| {
            text.padding = Some(padding);
            text.radius = Some(radius);
        })
    }
    /// Its size, line pitch and font beyond the editor's (`SpanStyle::size`,
    /// `line_height`, `mono`).
    #[doc(hidden)]
    pub fn text(&self) -> SpanText {
        self.text.map(SpanText::named).unwrap_or_default()
    }
    fn with_text(mut self, change: impl FnOnce(&mut SpanText)) -> Self {
        let mut text = self.text();
        change(&mut text);
        self.text = text.name();
        self
    }
    /// Its text shown as picture `id` (`EditorHandle::inline_svg`): each
    /// U+FFFC character in it that picture's box, or other text one box.
    #[doc(hidden)]
    pub const fn inline_svg(mut self, id: std::num::NonZeroU32) -> Self {
        self.look = Some(id);
        self
    }
    fn merge(&mut self, style: &SpanStyle) {
        self.color = self.color.or(style.color);
        self.weight = self.weight.or(style.weight);
        self.italic |= style.italic;
        self.strike |= style.strike;
        self.background = self.background.or(style.background);
        self.underline |= style.underline;
        self.look = self.look.or(style.look);
        self.text = match (self.text, style.text) {
            (Some(a), Some(b)) if a != b => {
                let (mut a, b) = (SpanText::named(a), SpanText::named(b));
                a.size = a.size.or(b.size);
                a.line_height = a.line_height.or(b.line_height);
                a.mono |= b.mono;
                a.padding = a.padding.or(b.padding);
                a.radius = a.radius.or(b.radius);
                a.name()
            }
            (a, b) => a.or(b),
        };
    }
    /// Whether it only colours its text.
    /// Its background, if it's a chip's (`chip`): the text area draws it
    /// as a mark, under the selection.
    fn chip_background(&self) -> Option<crate::scene::Color> {
        self.background
            .filter(|_| self.text.is_some() && self.text().padding.is_some())
    }
    fn colors_only(&self) -> bool {
        self.weight.is_none()
            && !self.italic
            && !self.strike
            && self.background.is_none()
            && !self.underline
            && self.look.is_none()
            && self.text.is_none()
    }
    /// Whether it changes how its text is shaped (and so its width): a
    /// look may (its lead, its picture).
    fn shapes(&self) -> bool {
        self.weight.is_some() || self.italic || self.look.is_some() || self.text.is_some()
    }
}
/// A span's size, line pitch and font beyond its editor's, and its
/// background's shape (`SpanStyle::size`, `line_height`, `mono`, `chip`).
/// Few spans have one, and few
/// differ: each is kept once on its thread, and a span names it in two
/// bytes (`SpanStyle::text`), so plain spans stay as small as they were.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SpanText {
    /// Its font size in points.
    pub size: Option<f32>,
    /// Its line pitch in points.
    pub line_height: Option<f32>,
    /// In the monospaced font.
    pub mono: bool,
    /// Room round its text inside its background, across and down, in
    /// points (a chip): across, it takes room in the line.
    pub padding: Option<[f32; 2]>,
    /// Its background's corner radius, in points.
    pub radius: Option<f32>,
}
thread_local! {
    /// The thread's span texts, a span's `SpanStyle::text` naming one by
    /// its place here, from 1. Only added to, so a name stays its text.
    static SPAN_TEXTS: RefCell<Vec<SpanText>> = const { RefCell::new(Vec::new()) };
}
impl SpanText {
    /// The text `name` names (`SpanStyle::text`).
    fn named(name: std::num::NonZeroU16) -> Self {
        SPAN_TEXTS.with_borrow(|texts| {
            texts
                .get(name.get() as usize - 1)
                .copied()
                .unwrap_or_default()
        })
    }
    /// Its name, the same for the same text (`None` for the editor's own,
    /// or past the 65,535 different texts a thread names).
    fn name(self) -> Option<std::num::NonZeroU16> {
        if self == Self::default() {
            return None;
        }
        let same = |a: Option<f32>, b: Option<f32>| a.map(f32::to_bits) == b.map(f32::to_bits);
        let pad = |p: Option<[f32; 2]>| p.map(|[x, y]| [x.to_bits(), y.to_bits()]);
        SPAN_TEXTS.with_borrow_mut(|texts| {
            let at = match texts.iter().position(|known| {
                same(known.size, self.size)
                    && same(known.line_height, self.line_height)
                    && known.mono == self.mono
                    && pad(known.padding) == pad(self.padding)
                    && same(known.radius, self.radius)
            }) {
                Some(at) => at,
                None if texts.len() < u16::MAX as usize => {
                    texts.push(self);
                    texts.len() - 1
                }
                None => return None,
            };
            std::num::NonZeroU16::new(at as u16 + 1)
        })
    }
}
/// A span's look beyond its `SpanStyle`, which few spans have: kept once in
/// its editor's looks (`LineCache::look`), which a span names
/// (`SpanStyle::look`), so the spans of plain text stay small.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpanLook {
    /// Its underline's colour, its thickness then the font's (a
    /// mention's): an underline with neither this nor an offset is one
    /// point thick in the text's colour.
    pub(crate) underline_color: Option<crate::scene::Color>,
    /// How far below the baseline its underline sits, in points.
    pub(crate) underline_offset: Option<f32>,
    /// Room before its text only, in points: where an atom's leading icon
    /// and left padding draw (`AtomStyle::icon`, `AtomStyle::padding`).
    pub(crate) lead: Option<f32>,
    /// Room after its text only, in points: an atom's right padding.
    pub(crate) trail: Option<f32>,
    /// Its text shown as a picture (an equation, an atom's): over U+FFFC
    /// characters, each one that picture's box; over other text, the span's
    /// text one box, its text kept for copy and search.
    pub(crate) picture: Option<Picture>,
}
impl SpanLook {
    /// Whether it adds nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.underline_color.is_none()
            && self.underline_offset.is_none()
            && self.lead.is_none()
            && self.trail.is_none()
            && self.picture.is_none()
    }
    /// Whether it is `other` (the same picture, not an equal one).
    fn same(&self, other: &SpanLook) -> bool {
        self.underline_color == other.underline_color
            && self.underline_offset == other.underline_offset
            && self.lead == other.lead
            && self.trail == other.trail
            && match (&self.picture, &other.picture) {
                (Some((a, at)), Some((b, bt))) => std::sync::Arc::ptr_eq(a, b) && at == bt,
                (a, b) => a.is_none() && b.is_none(),
            }
    }
}
/// Room around one line of a text area (`EditorHandle::set_line_insets`):
/// a list item's indent, a heading's space above. The line wraps in what
/// the indent leaves, and its carets, hits and selection move with it.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LineInset {
    /// Points before the line's text.
    pub left: f32,
    /// Points above the line.
    pub above: f32,
    /// Points below the line.
    pub below: f32,
    /// Not shown and no room taken (a closed toggle's lines): the app keeps
    /// the caret out of it.
    pub hidden: bool,
    /// Lines sharing an id scroll sideways together (a code block's): moved
    /// by the group's offset (`EditorHandle::set_group_scroll`) after their
    /// own layout, as their carets, hits and selection are, and shown only
    /// inside the group's frame (`GroupFrame`; by default from its first
    /// line's `left` to the area's right edge). Nothing moves down.
    pub scroll_group: Option<u32>,
    /// It never wraps (a code block's line, the web's `white-space: pre`):
    /// shaped once at its full width, it shows as far as its scroll group's
    /// frame, and scrolls with it.
    pub nowrap: bool,
}
impl LineInset {
    /// A line's height with its room: its text's `bare` height, or nothing
    /// when hidden.
    fn height(&self, bare: f32) -> f32 {
        if self.hidden {
            0.
        } else {
            bare + self.above + self.below
        }
    }
    /// A line not shown and taking no room.
    pub const fn hidden() -> Self {
        Self {
            left: 0.,
            above: 0.,
            below: 0.,
            hidden: true,
            scroll_group: None,
            nowrap: false,
        }
    }
    /// `left` points before the line's text, `above` over it and `below`
    /// under it.
    pub const fn new(left: f32, above: f32, below: f32) -> Self {
        Self {
            left,
            above,
            below,
            hidden: false,
            scroll_group: None,
            nowrap: false,
        }
    }
    /// In scroll group `group` (`scroll_group`).
    pub const fn in_group(mut self, group: u32) -> Self {
        self.scroll_group = Some(group);
        self
    }
    /// Never wrapped (`nowrap`).
    pub const fn unwrapped(mut self) -> Self {
        self.nowrap = true;
        self
    }
}
/// A scroll group's frame set by the app (`EditorHandle::set_group_frame`),
/// in points from the text area's left content edge: each part given
/// overrides what its lines say. All `None`: as its lines say.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GroupFrame {
    /// Where it starts; none: its first line's `left`.
    pub left: Option<f32>,
    /// How wide it shows; none: what the area's width leaves right of `left`.
    pub width: Option<f32>,
    /// How wide its lines reach from `left`, for how far it scrolls; none:
    /// as far as its lines reach.
    pub content: Option<f32>,
}
impl GroupFrame {
    /// A frame from `left`, `width` wide, its lines `content` wide (each
    /// `None`: as its lines say).
    pub const fn new(left: Option<f32>, width: Option<f32>, content: Option<f32>) -> Self {
        Self {
            left,
            width,
            content,
        }
    }
    // A builder for each field, named as the field is.
    /// With this `left` (see the field).
    #[must_use]
    pub fn left(mut self, left: impl Into<Option<f32>>) -> Self {
        self.left = left.into();
        self
    }
    /// With this `width` (see the field).
    #[must_use]
    pub fn width(mut self, width: impl Into<Option<f32>>) -> Self {
        self.width = width.into();
        self
    }
    /// With this `content` (see the field).
    #[must_use]
    pub fn content(mut self, content: impl Into<Option<f32>>) -> Self {
        self.content = content.into();
        self
    }
}
/// Each line's room (`LineInset`), given its index, its text and what the
/// line before it left its highlighter (`Highlighter`'s state; 0 for the
/// first line, or with no highlighter): a fenced code block's lines know
/// they are inside it as their colours do.
#[doc(hidden)]
pub type LineInsets = Rc<dyn Fn(usize, &str, u32) -> LineInset>;
/// A shape drawn beside a line of a text area (`EditorHandle::set_line_marks`):
/// a list item's bullet or number, a quote's rule, the wash behind a code
/// block's line, a horizontal rule. It scrolls with its line, under the text
/// (or over it, `over_text`).
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LineMark {
    /// Its left edge, in points from the text area's left content edge (from
    /// its right edge with `from_right`).
    pub x: f32,
    /// Its top, in points from the top of the line's text (below the line's
    /// room above).
    pub y: f32,
    /// Its width; infinite: to the area's right edge.
    pub width: f32,
    /// Its height; infinite: to the bottom of the line, its wrapped rows and
    /// its room below too, less `bottom`.
    pub height: f32,
    /// With an infinite height: how far above the line's bottom it ends.
    pub bottom: f32,
    /// The fill colour.
    pub fill: crate::scene::Color,
    /// The corner radius.
    pub radius: f32,
    /// The border's colour.
    pub border_color: crate::scene::Color,
    /// The border's width.
    pub border_width: f32,
    /// Its `x` is how far its right edge sits from the area's right edge (a
    /// code block's language).
    pub from_right: bool,
    /// Text drawn from its top left (an ordinal), its colour and size; a
    /// finite height is its line pitch.
    pub label: Option<(std::sync::Arc<str>, crate::scene::Color, f32)>,
    /// Its label's font (`mono`, `tabular`).
    pub(crate) label_font: u8,
    /// Drawn over the text, not under it.
    pub over_text: bool,
    /// It stays where it is as its line's scroll group scrolls sideways (a
    /// code block's wash and line numbers): only the text, and the marks not
    /// pinned, move.
    pub pinned: bool,
    /// What few marks have (`look`); none, for a plain one.
    pub(crate) look: Option<Arc<MarkLook>>,
}
/// What few line marks have (`LineMark::look`), set by `LineMark`'s builders
/// of the same names: apart, so a plain mark (a wash, a rule, a number) is no
/// bigger for it.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct MarkLook {
    /// A picture filling it (a note's image above its caption), its corners
    /// rounded by `radius`: a negative `y` puts it in its line's room above.
    pub image: Option<MarkImage>,
    /// Where it sits across the line's width (left, centre or right; `x`
    /// then offsets it); none: at `x` (or from the right edge with
    /// `from_right`).
    pub align: Option<crate::text_layout::TextAlign>,
    /// A picture drawn as given in its box, in place of its fill and border.
    pub svg: Option<Arc<crate::svg::SvgData>>,
    /// Per-side borders and per-corner radii (a table's outer cells), over
    /// `radius` and `border_width`.
    pub decoration: Option<Arc<crate::decoration::Decoration>>,
    /// Turned this far clockwise about its centre, in degrees (a toggle's
    /// chevron, 90 open): a change eases there over `motion.duration.fast`,
    /// easing out, unless the system reduces motion.
    pub rotate: f32,
}
/// `LineMark::label_font`'s bits.
pub(crate) const LABEL_MONO: u8 = 1;
pub(crate) const LABEL_TABULAR: u8 = 2;
/// A line mark's picture (`LineMark::image`): decoded pixels and how they
/// fit the mark's box. A file is decoded by the app, as an image view's is
/// (`zgx_gpu::assets::decode_image_kept`), and kept in its `ImageCache`.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct MarkImage {
    /// The pixels.
    pub image: Arc<crate::image::ImageData>,
    /// How they fit the mark's box
    /// ([`ObjectFit::Cover`](crate::style::ObjectFit::Cover): filling it,
    /// cropped).
    pub fit: crate::style::ObjectFit,
}
impl MarkImage {
    /// `image`, fitting the mark's box as `fit`.
    pub fn new(image: Arc<crate::image::ImageData>, fit: crate::style::ObjectFit) -> Self {
        Self { image, fit }
    }
    /// With this `image` (see `MarkLook`).
    #[must_use]
    pub fn image(mut self, image: Arc<crate::image::ImageData>) -> Self {
        self.image = image;
        self
    }
    /// With this `fit` (see the field).
    #[must_use]
    pub fn fit(mut self, fit: crate::style::ObjectFit) -> Self {
        self.fit = fit;
        self
    }
    /// `image` filling the mark, cropped to its box.
    pub fn cover(image: Arc<crate::image::ImageData>) -> Self {
        Self {
            image,
            fit: crate::style::ObjectFit::Cover,
        }
    }
}
impl Default for LineMark {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            width: 0.,
            height: 0.,
            bottom: 0.,
            fill: crate::scene::Color(0, 0, 0, 0),
            radius: 0.,
            border_color: crate::scene::Color(0, 0, 0, 0),
            border_width: 0.,
            from_right: false,
            label: None,
            label_font: 0,
            over_text: false,
            pinned: false,
            look: None,
        }
    }
}
impl LineMark {
    /// A box `width` by `height` at `x`, `y` (see the fields), filled with
    /// `fill`.
    pub fn new(x: f32, y: f32, width: f32, height: f32, fill: crate::scene::Color) -> Self {
        Self {
            x,
            y,
            width,
            height,
            fill,
            ..Self::default()
        }
    }
    // A builder for each field, named as the field is: the struct gains
    // fields, so code outside sets them so.
    /// With this `x` (see the field).
    #[must_use]
    pub fn x(mut self, x: f32) -> Self {
        self.x = x;
        self
    }
    /// With this `y` (see the field).
    #[must_use]
    pub fn y(mut self, y: f32) -> Self {
        self.y = y;
        self
    }
    /// With this `width` (see the field).
    #[must_use]
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }
    /// With this `height` (see the field).
    #[must_use]
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
    /// With this `bottom` (see the field).
    #[must_use]
    pub fn bottom(mut self, bottom: f32) -> Self {
        self.bottom = bottom;
        self
    }
    /// With this `fill` (see the field).
    #[must_use]
    pub fn fill(mut self, fill: crate::scene::Color) -> Self {
        self.fill = fill;
        self
    }
    /// With this `radius` (see the field).
    #[must_use]
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
    /// With this `border_color` (see the field).
    #[must_use]
    pub fn border_color(mut self, border_color: crate::scene::Color) -> Self {
        self.border_color = border_color;
        self
    }
    /// With this `border_width` (see the field).
    #[must_use]
    pub fn border_width(mut self, border_width: f32) -> Self {
        self.border_width = border_width;
        self
    }
    /// With this `from_right` (see the field).
    #[must_use]
    pub fn from_right(mut self, from_right: bool) -> Self {
        self.from_right = from_right;
        self
    }
    /// Its label in the monospaced font (a code block's line numbers).
    #[must_use]
    pub fn mono(mut self, mono: bool) -> Self {
        self.set_label_font(LABEL_MONO, mono);
        self
    }
    /// With this `label` (see the field).
    #[must_use]
    pub fn label(mut self, label: impl Into<Option<(Arc<str>, crate::scene::Color, f32)>>) -> Self {
        self.label = label.into();
        self
    }
    /// With this `image` (see `MarkLook`).
    #[must_use]
    pub fn image(self, image: impl Into<Option<MarkImage>>) -> Self {
        let image = image.into();
        self.with_look(|look| look.image = image)
    }
    /// With this `align` (see `MarkLook`).
    #[must_use]
    pub fn align(self, align: impl Into<Option<crate::text_layout::TextAlign>>) -> Self {
        let align = align.into();
        self.with_look(|look| look.align = align)
    }
    /// With this `svg` (see `MarkLook`).
    #[must_use]
    pub fn svg(self, svg: impl Into<Option<Arc<crate::svg::SvgData>>>) -> Self {
        let svg = svg.into();
        self.with_look(|look| look.svg = svg)
    }
    /// With this `decoration` (see `MarkLook`).
    #[must_use]
    pub fn decoration(
        self,
        decoration: impl Into<Option<Arc<crate::decoration::Decoration>>>,
    ) -> Self {
        let decoration = decoration.into();
        self.with_look(|look| look.decoration = decoration)
    }
    /// With this `pinned` (see the field).
    #[must_use]
    pub fn pinned(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }
    /// With this `over_text` (see the field).
    #[must_use]
    pub fn over_text(mut self, over_text: bool) -> Self {
        self.over_text = over_text;
        self
    }
    /// With this `rotate` (see `MarkLook`).
    #[must_use]
    pub fn rotate(mut self, rotate: f32) -> Self {
        // Kept at 0 too: a mark that turns back eases there.
        match &mut self.look {
            Some(look) => Arc::make_mut(look).rotate = rotate,
            None => self.set_look(MarkLook {
                rotate,
                ..MarkLook::default()
            }),
        }
        self
    }
    /// Its label in the monospaced font (`mono`).
    pub fn is_mono(&self) -> bool {
        self.label_font & LABEL_MONO != 0
    }
    /// Its label in tabular figures (`tabular`).
    pub fn is_tabular(&self) -> bool {
        self.label_font & LABEL_TABULAR != 0
    }
    fn set_label_font(&mut self, bit: u8, on: bool) {
        self.label_font = match on {
            true => self.label_font | bit,
            false => self.label_font & !bit,
        };
    }
    /// What few marks have; none, for a plain one.
    pub fn look(&self) -> Option<&MarkLook> {
        self.look.as_deref()
    }
    /// Its look changed so; a plain mark given nothing stays plain.
    fn with_look(mut self, change: impl FnOnce(&mut MarkLook)) -> Self {
        let mut look = MarkLook::default();
        match &mut self.look {
            Some(look) => change(Arc::make_mut(look)),
            None => {
                change(&mut look);
                if look != MarkLook::default() {
                    self.set_look(look);
                }
            }
        }
        self
    }
    fn set_look(&mut self, look: MarkLook) {
        self.look = Some(Arc::new(look));
    }
    /// Its label in tabular figures (OpenType's `tnum`), each digit a
    /// figure's width: an ordered list's numbers, lined up as the web's are.
    #[must_use]
    pub fn tabular(mut self, tabular: bool) -> Self {
        self.set_label_font(LABEL_TABULAR, tabular);
        self
    }
}
/// Each line's marks (`LineMark`), given its index, its text and what the
/// line before it left its highlighter, as `LineInsets` are.
#[doc(hidden)]
pub type LineMarks = Rc<dyn Fn(usize, &str, u32) -> Vec<LineMark>>;
/// `LineMarks` whose answers are kept by who gives them, and shared: a draw
/// copies none of them.
#[doc(hidden)]
pub type KeptLineMarks = Rc<dyn Fn(usize, &str, u32) -> Rc<[LineMark]>>;
/// What gives a text area's lines their marks.
#[derive(Clone)]
pub(crate) enum MarksGiver {
    /// Made at each call.
    Each(LineMarks),
    /// Kept, and shared.
    Kept(KeptLineMarks),
}
/// A line's marks, as they were given.
pub(crate) enum MarksOf {
    Each(Vec<LineMark>),
    Kept(Rc<[LineMark]>),
}
/// A line's styled ranges: byte ranges of the line, each with how it shows.
pub type LineSpans = Vec<(Range<usize>, SpanStyle)>;
/// A `Highlighter` that styles its ranges (`SpanStyle`) beyond colouring
/// them: weight and italics (which its lines are shaped with, so carets,
/// hits and wrapping follow them), strikes and washes behind the text.
pub type SpanHighlighter = Rc<dyn Fn(&str, u32) -> (LineSpans, u32)>;
/// A colouring highlighter as one of spans.
pub(crate) fn span_highlighter(highlighter: Highlighter) -> SpanHighlighter {
    Rc::new(move |line, before| {
        let (colors, leaves) = highlighter(line, before);
        let spans = colors
            .into_iter()
            .map(|(range, color)| (range, SpanStyle::from(color)))
            .collect();
        (spans, leaves)
    })
}

/// The atoms' spans (`atoms`, over the whole text by start) a line over
/// `line` holds, in its own bytes, before the highlighter's `colors` (so an
/// atom's style wins where both set something), by start.
#[inline(never)]
fn atoms_on_line(
    atoms: &LineSpans,
    line: Range<usize>,
    colors: Option<Rc<LineSpans>>,
) -> Option<Rc<LineSpans>> {
    let first = atoms.partition_point(|(range, _)| range.end <= line.start);
    let mut spans: LineSpans = atoms[first..]
        .iter()
        .take_while(|(range, _)| range.start < line.end)
        .filter(|(range, _)| range.end > line.start)
        .map(|(range, style)| {
            let start = range.start.max(line.start) - line.start;
            let end = range.end.min(line.end) - line.start;
            (start..end, *style)
        })
        .filter(|(range, _)| !range.is_empty())
        .collect();
    if spans.is_empty() {
        return colors;
    }
    if let Some(colors) = colors {
        spans.extend(colors.iter().cloned());
        // Stable: at one start, the atom's first.
        spans.sort_by_key(|(range, _)| range.start);
    }
    Some(Rc::new(spans))
}
/// `line`'s runs as `spans` (in order by start, on its character
/// boundaries) style them, overlapping spans merged: each run's range, and
/// its style (`None` for the editor's own).
fn runs(line: &str, spans: &LineSpans) -> Vec<(Range<usize>, Option<SpanStyle>)> {
    // Apart (a code highlighter's tokens): each span a run, as they are.
    if spans.windows(2).all(|w| w[0].0.end <= w[1].0.start) {
        let mut runs = Vec::with_capacity(spans.len() * 2 + 1);
        let mut at = 0;
        for (range, style) in spans {
            if range.start > at {
                runs.push((at..range.start, None));
            }
            runs.push((range.clone(), Some(*style)));
            at = range.end;
        }
        if at < line.len() {
            runs.push((at..line.len(), None));
        }
        return runs;
    }
    // Where a run may change: each span's ends, and the line's.
    let mut cuts: Vec<usize> = Vec::with_capacity(spans.len() * 2 + 2);
    cuts.push(0);
    cuts.push(line.len());
    for (range, _) in spans {
        cuts.push(range.start);
        cuts.push(range.end);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut runs: Vec<(Range<usize>, Option<SpanStyle>)> = Vec::with_capacity(cuts.len());
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let mut style: Option<SpanStyle> = None;
        for (range, span) in spans {
            if range.start > start {
                break;
            }
            if range.end >= end {
                style.get_or_insert_default().merge(span);
            }
        }
        match runs.last_mut() {
            Some((last, last_style)) if *last_style == style => last.end = end,
            _ => runs.push((start..end, style)),
        }
    }
    runs
}
/// `line` as `spans` style it, in `font` at `size`, coloured `color` where
/// no span colours it (`None` if it is empty or has no spans).
pub(crate) fn styled_line(
    line: &str,
    spans: &LineSpans,
    color: crate::scene::Color,
    size: f32,
    font: &crate::text_layout::FontStyle,
    looks: &[SpanLook],
) -> Option<crate::rich_text::RichText> {
    if line.is_empty() || spans.is_empty() {
        return None;
    }
    // Spans that only colour (a code highlighter's): runs straight from
    // them, as before styles, an earlier span keeping what it overlaps.
    if spans.iter().all(|(_, style)| style.colors_only()) {
        let run = |range: Range<usize>, color| crate::rich_text::TextRun {
            range,
            font: font.clone(),
            font_size: size,
            color,
            ..Default::default()
        };
        let mut runs = Vec::with_capacity(spans.len() * 2 + 1);
        let mut at = 0;
        for (range, style) in spans {
            let start = range.start.max(at);
            if start >= range.end {
                continue;
            }
            if start > at {
                runs.push(run(at..start, color));
            }
            runs.push(run(start..range.end, style.color.unwrap_or(color)));
            at = range.end;
        }
        if at < line.len() {
            runs.push(run(at..line.len(), color));
        }
        return crate::rich_text::RichText::new(line, runs).ok();
    }
    let mut styled = runs(line, spans);
    let boxes = if styled
        .iter()
        .any(|(_, s)| s.is_some_and(|s| s.look.is_some()))
    {
        picture_runs(line, &mut styled, looks, size)
    } else {
        Vec::new()
    };
    let runs: Vec<_> = styled
        .into_iter()
        .map(|(range, style)| {
            let style = style.unwrap_or_default();
            let mut font = font.clone();
            if let Some(weight) = style.weight {
                font.weight = weight.clamp(1, 1000);
            }
            font.italic |= style.italic;
            let text = style.text();
            if text.mono {
                font.family = crate::text_layout::FontFamily::Monospace;
            }
            if let Some(line_height) = text.line_height {
                font.line_height = crate::text_layout::LineHeight::px(line_height);
            }
            let look = style.look.and_then(|id| look(looks, id));
            // Edge padding is room before and/or after the atom's text:
            // `padding[1] < 0` encodes `before = 2 * padding[0]`,
            // `after = -(padding[1] + 1)` (`TextRun::across_padding`). A
            // chip span's room across is round its text (`SpanStyle::chip`),
            // added to an atom's.
            let lead = look.and_then(|l| l.lead).filter(|v| *v > 0.).unwrap_or(0.);
            let trail = look.and_then(|l| l.trail).filter(|v| *v > 0.).unwrap_or(0.);
            let across = text.padding.map_or(0., |p| p[0].max(0.));
            let padding = if lead > 0. || trail > 0. {
                [(lead + across) / 2., -1. - (trail + across)]
            } else {
                text.padding.unwrap_or_default()
            };
            // A chip's background is the text area's to draw, as an atom's
            // wash is: a mark behind the text, under the selection
            // (`LineCache::chips_between`). The run keeps its room.
            let chip = text.padding.is_some();
            crate::rich_text::TextRun {
                range,
                font,
                font_size: text.size.unwrap_or(size),
                color: style.color.unwrap_or(color),
                background: style.background.filter(|_| !chip),
                padding,
                radius: if chip {
                    0.
                } else {
                    text.radius.unwrap_or_default()
                },
                underline: style.underline.then(|| underline(look)),
                strikethrough: style.strike.then(|| crate::rich_text::Decoration::new(1.)),
            }
        })
        .collect();
    crate::rich_text::RichText::new(line, runs)
        .ok()?
        .with_inline_boxes(boxes)
        .ok()
}
/// A picture a span's text shows as (`SpanLook::picture`): its SVG and its
/// `[width, height, descent]` in EM.
pub(crate) type Picture = (std::sync::Arc<crate::svg::SvgData>, [f32; 3]);
/// The look `id` names among `looks`.
fn look(looks: &[SpanLook], id: std::num::NonZeroU32) -> Option<&SpanLook> {
    looks.get(id.get() as usize - 1)
}
/// `styled`'s pictures (`SpanLook::picture`) as runs of their own and
/// their boxes: each U+FFFC character a box of its own, other text the
/// span's whole text one box. Rare, out of line.
#[cold]
#[inline(never)]
fn picture_runs(
    line: &str,
    styled: &mut Vec<(Range<usize>, Option<SpanStyle>)>,
    looks: &[SpanLook],
    size: f32,
) -> Vec<crate::rich_text::InlineBox> {
    use crate::rich_text::{INLINE_BOX_CHAR, INLINE_BOX_TEXT, InlineBox};
    let mut out: Vec<(Range<usize>, Option<SpanStyle>)> = Vec::with_capacity(styled.len());
    let mut boxes = Vec::new();
    // The last run out is a box over text, of this look.
    let mut over_text = None;
    for (range, style) in styled.drain(..) {
        let pictured = style.and_then(|s| s.look).and_then(|id| {
            let (_, [w, h, d]) = look(looks, id)?.picture.as_ref()?;
            Some((id, [*w, *h, *d]))
        });
        let starts_boxed = line[range.clone()].starts_with(INLINE_BOX_CHAR);
        let a_box = |at: usize, [w, h, d]: [f32; 3]| InlineBox {
            width: w * size,
            height: h * size,
            baseline: (h - d) * size,
            ..InlineBox::new(at)
        };
        match (pictured, out.last_mut()) {
            // A box over text: one run, whatever else styles parts of it.
            (Some((id, _)), Some((last, _)))
                if !starts_boxed && over_text == Some(id) && last.end == range.start =>
            {
                last.end = range.end;
            }
            // Over replacement characters: each one a run and a box of its
            // own, the text between them the span's but no box.
            (Some((_, extents)), _) if starts_boxed => {
                over_text = None;
                let mut at = range.start;
                for (offset, c) in line[range.clone()].char_indices() {
                    let here = range.start + offset;
                    if c == INLINE_BOX_CHAR {
                        if at < here {
                            out.push((at..here, style));
                        }
                        boxes.push(a_box(here, extents));
                        out.push((here..here + INLINE_BOX_TEXT.len(), style));
                        at = here + INLINE_BOX_TEXT.len();
                    }
                }
                if at < range.end {
                    out.push((at..range.end, style));
                }
            }
            // Over text: one box, at its start.
            (Some((id, extents)), _) => {
                over_text = Some(id);
                boxes.push(a_box(range.start, extents));
                out.push((range, style));
            }
            _ => {
                over_text = None;
                out.push((range, style));
            }
        }
    }
    *styled = out;
    boxes
}
/// An underline in `look`'s colour and place: one point thick in its
/// text's colour, or the font's thickness when it says either.
fn underline(look: Option<&SpanLook>) -> crate::rich_text::Decoration {
    let (color, offset) = look.map_or((None, None), |l| (l.underline_color, l.underline_offset));
    if color.is_none() && offset.is_none() {
        return crate::rich_text::Decoration::new(1.);
    }
    let mut line = crate::rich_text::Decoration::default();
    if let Some(color) = color {
        line = line.color(color);
    }
    if let Some(offset) = offset {
        line = line.offset(offset);
    }
    line
}
/// What shapes a styled line beyond its text: its runs' weights and
/// slants (`None`: it shapes as its plain text does).
fn shape_key(spans: &LineSpans) -> Option<ShapeKey> {
    if !spans.iter().any(|(_, s)| s.shapes()) {
        return None;
    }
    Some(
        spans
            .iter()
            .filter(|(_, s)| s.shapes())
            .map(|(r, s)| {
                (
                    r.start as u32,
                    r.end as u32,
                    s.weight.unwrap_or(0),
                    s.italic,
                    s.look.map_or(0, std::num::NonZeroU32::get),
                    s.text.map_or(0, std::num::NonZeroU16::get),
                )
            })
            .collect(),
    )
}

/// A text this long (in bytes, or lines) is laid out by its lines; a
/// shorter one whole, as before.
pub(crate) const LONG_TEXT_BYTES: usize = 64 * 1024;
pub(crate) const LONG_TEXT_LINES: usize = 2_000;
/// Whether `text` is long enough to lay out by its lines.
pub(crate) fn is_long(text: &str) -> bool {
    // Its line breaks counted only where there could be enough, a word of
    // bytes at a time (an iterator of matches costs a call each).
    text.len() > LONG_TEXT_BYTES
        || (text.len() > LONG_TEXT_LINES
            && text.bytes().filter(|byte| *byte == b'\n').count() > LONG_TEXT_LINES)
}

/// Lines kept shaped: about a few screens' worth.
const SHAPED_LINES: usize = 1_024;

/// What the shaped lines are for: a change of any shapes them again.
#[derive(Clone, PartialEq)]
struct Shaping {
    size: u32,
    wrap: Option<u32>,
    font: crate::text_layout::FontStyle,
    revision: u64,
}

/// What an editor keeps across its long text's layouts (one an edit): the
/// lines it shaped, by their text, and its last layout.
#[derive(Default)]
pub(crate) struct LineCache {
    shaping: RefCell<Option<Shaping>>,
    shaped: RefCell<HashMap<Box<str>, Shaped>>,
    /// Lines whose spans shape them (weights, italics), by their text and
    /// those spans' shaping (`shape_key`).
    styled: RefCell<HashMap<StyledKey, Shaped>>,
    /// A wrapped line's height once shaped, by its text: kept when its
    /// layout is evicted, so the lines above what shows keep their places.
    heights: RefCell<HashMap<Box<str>, f32>>,
    clock: Cell<u64>,
    last: RefCell<Option<Rc<LinesLayout>>>,
    /// The editor's text revision `last` is of, if it is its committed text.
    last_revision: Cell<Option<u64>>,
    highlighter: RefCell<Option<SpanHighlighter>>,
    /// Its atoms' spans and its spans' looks (`RareLines`): one pointer
    /// until it has any.
    rare: RefCell<crate::text_edit::RareBox<RareLines>>,
    /// Whether `rare` holds anything: asked first, so a line of an editor
    /// without atoms or looks reads no `RefCell` for them.
    rare_set: Cell<bool>,
    /// The highlighter has given a span that shapes its line (a weight,
    /// italics): only then is a line asked for its spans as it is shaped,
    /// so one that only colours costs what it did. `restyled`: it just
    /// became so (`take_restyled`).
    shapes: Cell<bool>,
    restyled: Cell<bool>,
    /// The highlighter has given a span with a size, pitch or font of its
    /// own (`SpanStyle::text`: a note's heading): its lines' heights
    /// differ, so each is kept, unwrapped too.
    tall: Cell<bool>,
    /// The highlighter has given a chip span with a background
    /// (`SpanStyle::chip`): only then does the text area look for chips to
    /// draw (`chips_between`). One has a text style, so `tall` comes first.
    chips: Cell<bool>,
    /// What each line leaves the next, from the first; as far as known.
    states: RefCell<Vec<u32>>,
    /// What the lines below the last edit left before it: once the lines
    /// it changed leave what they did, these hold again (the edit's
    /// colours end there).
    tail: RefCell<Option<Tail>>,
    /// Highlighted lines, by their text and what the line before left.
    colored: RefCell<HashMap<(Box<str>, u32), Colored>>,
}
/// What few editors' lines have (`LineCache::rare`): their atoms' spans
/// and their spans' looks.
#[derive(Default)]
struct RareLines {
    /// Spans its atoms' styles give (`AtomStyle`), over the whole text by
    /// start: each line takes those it holds before the highlighter's.
    atom_spans: Option<Rc<LineSpans>>,
    /// Its spans' looks (`SpanStyle::look` names one by its place here,
    /// from 1): an atom's, a highlighter's picture. Only added to, so a
    /// name stays the look's; `pictured`: one holds a picture.
    looks: Vec<SpanLook>,
    pictured: bool,
    /// Each line's room (`set_insets`): a note's list items and headings.
    insets: Option<LineInsets>,
    /// Each line's marks (`set_marks`): a quote's rule, a code block's wash.
    marks: Option<MarksGiver>,
    /// Its scroll groups' offsets and frames (`LineInset::scroll_group`):
    /// made as the first is moved, framed or listened to.
    groups: Option<Box<GroupState>>,
}
/// What a text area keeps of its scroll groups (`RareLines::groups`): read
/// as carets, hits and drawing are, so moving one lays nothing out again.
#[derive(Default)]
struct GroupState {
    /// Each group's sideways offset, by its id (0 until set).
    offsets: HashMap<u32, f32>,
    /// Frames the app set (`EditorHandle::set_group_frame`), by id.
    frames: HashMap<u32, GroupFrame>,
    /// The group that moved last: the one its group scrollbar shows.
    last: Option<u32>,
    /// Told of each group's new offset (`EditorHandle::on_group_scroll`).
    listener: Option<GroupListener>,
    /// Groups brought back to their lines as they were drawn, told once the
    /// draw is done (`LineCache::tell_settled`).
    settled: Vec<(u32, f32)>,
}
/// Told of a scroll group's id and new offset (`EditorHandle::on_group_scroll`).
pub(crate) type GroupListener = Rc<dyn Fn(u32, f32)>;
/// A scroll group's frame and extent (`LinesLayout::group`), in points from
/// the area's left content edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Group {
    /// Where it shows from.
    pub(crate) left: f32,
    /// How wide it shows (infinite in a text area that doesn't wrap).
    pub(crate) view: f32,
    /// How wide its lines are from `left`.
    pub(crate) content: f32,
    /// Its first and last lines (hidden ones too).
    pub(crate) first: usize,
    pub(crate) last: usize,
}
impl Group {
    /// The furthest it scrolls: its content past its view.
    pub(crate) fn limit(&self) -> f32 {
        (self.content - self.view).max(0.)
    }
    /// Its right edge.
    pub(crate) fn right(&self) -> f32 {
        self.left + self.view
    }
}
/// What a scroll group's lines say of it (`LinesLayout::groups`), before
/// the app's frame.
#[derive(Clone, Copy, Debug)]
struct GroupLines {
    /// Its first line's `left`.
    left: f32,
    /// How far right its lines reach, estimated before they're shaped.
    right: f32,
    first: usize,
    last: usize,
}
/// What a line's room says of its scroll group (`LayoutGroups::each`).
#[derive(Clone, Copy, Debug, Default)]
struct LineGroup {
    group: Option<u32>,
    left: f32,
    /// How far right it reaches, estimated (none hidden).
    right: Option<f32>,
}
/// A layout's scroll groups (`LinesLayout::groups`): their lines, found once
/// asked for, and how far their unwrapped lines reach as shaped.
#[derive(Default)]
struct LayoutGroups {
    /// Each line's, in an area with a line in a group: as it was measured,
    /// so finding the groups asks no line's room again.
    each: Vec<LineGroup>,
    lines: Option<HashMap<u32, GroupLines>>,
    reach: HashMap<u32, f32>,
}
/// What the lines below an edit left before it (`LineCache::edited`).
struct Tail {
    /// The line the first of them is now.
    from: usize,
    /// What entered the first of them.
    entering: u32,
    states: Vec<u32>,
}
/// A shaped line, and when it was last used.
type Shaped = (Rc<dyn TextLayout>, u64);
/// What shapes a styled line's runs: each one's range, weight and slant.
type ShapeKey = Box<[(u32, u32, u16, bool, u32, u16)]>;
/// A styled line's text and what shapes its runs (`shape_key`).
type StyledKey = (Box<str>, ShapeKey);
/// A highlighted line: its colours, what it leaves the next, when used.
struct Colored {
    colors: Rc<LineSpans>,
    leaves: u32,
    used: u64,
}
impl LineCache {
    /// The last layout made: the text it shows, by its lines.
    pub(crate) fn last_layout(&self) -> Option<Rc<LinesLayout>> {
        self.last.borrow().clone()
    }
    /// Whether the last layout's lines are in scroll groups (`LineInset::scroll_group`).
    pub(crate) fn last_grouped(&self) -> bool {
        (self.last.borrow().as_ref()).is_some_and(|last| last.grouped)
    }
    /// `text` as a `Text`: the last layout's, edited to it (`Text::edited_to`),
    /// so a short text's keystroke copies a chunk of it, not all of it.
    pub(crate) fn text_of(&self, text: &str) -> Text {
        match self.last.borrow().as_ref() {
            Some(last) => last.text.edited_to(text),
            None => Text::new(text),
        }
    }
    /// Let go of the last layout, which holds this cache in turn: called as
    /// its editor goes (`widgets`), or the two would keep each other, their
    /// lines and the shaper alive.
    pub(crate) fn forget_last(&self) {
        self.last.borrow_mut().take();
        self.last_revision.set(None);
    }
    /// The layout of the text `text` makes, its last one if the text and
    /// shaping are the same: known by the text's `revision`
    /// (`TextEditor::revision`) if it is an editor's committed text, before
    /// `text` is asked for, else by comparing them (a pointer, for a long
    /// editor's shared text).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn layout(
        self: &Rc<Self>,
        text: impl FnOnce() -> Text,
        text_revision: Option<u64>,
        size: f32,
        wrap: Option<f32>,
        font: &crate::text_layout::FontStyle,
        revision: u64,
        shape: impl FnOnce() -> (LineShaper, RichLineShaper),
    ) -> Rc<LinesLayout> {
        let shaping = Shaping {
            size: size.to_bits(),
            wrap: wrap.map(f32::to_bits),
            font: font.clone(),
            revision,
        };
        if self.shaping.borrow().as_ref() != Some(&shaping) {
            self.shaped.borrow_mut().clear();
            self.styled.borrow_mut().clear();
            self.heights.borrow_mut().clear();
            *self.last.borrow_mut() = None;
            self.last_revision.set(None);
            *self.shaping.borrow_mut() = Some(shaping);
        }
        let previous = self.last.borrow().clone();
        if let (Some(last), Some(now)) = (&previous, text_revision)
            && self.last_revision.get() == Some(now)
        {
            return last.clone();
        }
        let text = text();
        if let Some(last) = &previous
            && last.text == text
        {
            self.last_revision.set(text_revision);
            return last.clone();
        }
        let layout = Rc::new(match &previous {
            // An edit: the old layout's lines, those it changed found again.
            Some(old) => {
                let prefix = old.text.common_prefix(&text);
                let suffix = old.text.common_suffix(&text, prefix);
                // Before its lines are measured: their room may depend on
                // what the lines above leave now (`LineInsets`).
                self.edited(Some((old, prefix, suffix)), &text);
                LinesLayout::edited(old, text, prefix, suffix)
            }
            None => {
                self.edited(None, &text);
                LinesLayout::new(text, wrap, shape(), self.clone())
            }
        });
        *self.last.borrow_mut() = Some(layout.clone());
        self.last_revision.set(text_revision);
        layout
    }
    /// The text went from `old` to `new`'s: what the lines above the edit
    /// leave holds, and what those below it left is kept for when the
    /// edited lines leave what they did.
    fn edited(&self, old: Option<(&LinesLayout, usize, usize)>, new: &Text) {
        let mut states = self.states.borrow_mut();
        let mut tail = self.tail.borrow_mut();
        *tail = None;
        let Some((old, prefix, suffix)) = old else {
            states.clear();
            return;
        };
        let first = new.line_of(prefix);
        let (old_last, new_last) = (
            old.line_of(old.text.len() - suffix),
            new.line_of(new.len() - suffix),
        );
        // The lines below the edit, as far as their states were known, with
        // what entered the first of them.
        if states.len() > old_last + 1 {
            *tail = Some(Tail {
                from: new_last + 1,
                entering: states[old_last],
                states: states[old_last + 1..].to_vec(),
            });
        }
        states.truncate(first);
    }
    /// The next line leaves `leaves`. Below an edit, once what a line
    /// leaves is what it left before, the lines below it leave what they
    /// did: those are known again, and asked for no more.
    fn push_state(&self, states: &mut Vec<u32>, leaves: u32) {
        states.push(leaves);
        let line = states.len() - 1;
        let mut tail = self.tail.borrow_mut();
        let Some(below) = tail.as_ref() else {
            return;
        };
        let holds = if line + 1 == below.from {
            (leaves == below.entering).then_some(0)
        } else if line >= below.from {
            match below.states.get(line - below.from) {
                Some(old) => (leaves == *old).then_some(line - below.from + 1),
                None => {
                    *tail = None;
                    return;
                }
            }
        } else {
            None
        };
        if let Some(from) = holds {
            states.extend_from_slice(&below.states[from..]);
            *tail = None;
        }
    }
    /// Give each line the room `insets` gives it (none: none), shaping
    /// lines anew.
    pub(crate) fn set_insets(&self, insets: Option<LineInsets>) {
        let mut rare = self.rare.borrow_mut();
        if insets.is_some() || rare.is_some() {
            rare.get_or_insert_default().insets = insets;
            self.rare_set.set(true);
        }
        drop(rare);
        // Its lines are asked for in `styled`, where their room goes in.
        if self.has_insets() {
            self.shapes.set(true);
        }
        self.shaped.borrow_mut().clear();
        self.styled.borrow_mut().clear();
        self.heights.borrow_mut().clear();
        self.forget_last();
    }
    /// Whether its lines' heights differ beyond wrapping: kept for each
    /// line unwrapped too.
    pub(crate) fn keeps_heights(&self) -> bool {
        self.tall.get() || self.has_insets()
    }
    /// Whether its lines have room of their own (`set_insets`).
    pub(crate) fn has_insets(&self) -> bool {
        self.rare_set.get()
            && (self.rare.borrow().as_ref()).is_some_and(|rare| rare.insets.is_some())
    }
    /// Give each line the marks `marks` gives it (none: none).
    pub(crate) fn set_marks(&self, marks: Option<MarksGiver>) {
        let mut rare = self.rare.borrow_mut();
        if marks.is_some() || rare.is_some() {
            rare.get_or_insert_default().marks = marks;
            self.rare_set.set(true);
        }
    }
    /// Whether its lines have marks (`set_marks`).
    pub(crate) fn has_marks(&self) -> bool {
        self.rare_set.get()
            && (self.rare.borrow().as_ref()).is_some_and(|rare| rare.marks.is_some())
    }
    /// Line `i` of `layout`'s marks (`set_marks`).
    #[cold]
    #[inline(never)]
    pub(crate) fn marks(&self, layout: &LinesLayout, i: usize) -> MarksOf {
        let Some(marks) = (self.rare.borrow().as_ref()).and_then(|rare| rare.marks.clone()) else {
            return MarksOf::Each(Vec::new());
        };
        let before = self.state_before(layout, i);
        let text = layout.line_text(i);
        match marks {
            MarksGiver::Each(marks) => MarksOf::Each(marks(i, &text, before)),
            MarksGiver::Kept(marks) => MarksOf::Kept(marks(i, &text, before)),
        }
    }
    /// Scroll group `group`'s offset (0 until set).
    pub(crate) fn group_scroll(&self, group: u32) -> f32 {
        if !self.rare_set.get() {
            return 0.;
        }
        (self.rare.borrow().as_ref())
            .and_then(|rare| rare.groups.as_ref())
            .and_then(|groups| groups.offsets.get(&group).copied())
            .unwrap_or(0.)
    }
    /// Its scroll groups' state, made if it has none.
    fn groups_mut<R>(&self, change: impl FnOnce(&mut GroupState) -> R) -> R {
        let mut rare = self.rare.borrow_mut();
        self.rare_set.set(true);
        change(rare.get_or_insert_default().groups.get_or_insert_default())
    }
    /// Move scroll group `group` to `x` (clamped by the caller); whether it
    /// moved. Its listener hears of it. Nothing is laid out again.
    pub(crate) fn set_group_scroll(&self, group: u32, x: f32) -> bool {
        let x = if x.is_finite() { x.max(0.) } else { 0. };
        let moved = self.groups_mut(|groups| {
            let previous = groups.offsets.insert(group, x).unwrap_or(0.);
            (previous != x).then(|| {
                groups.last = Some(group);
                groups.listener.clone()
            })
        });
        let Some(listener) = moved else {
            return false;
        };
        if let Some(listener) = listener {
            listener(group, x);
        }
        true
    }
    /// Bring each group scrolled past its lines in `layout` (an edit cut them
    /// short) back to as far as they reach; whether any was. Its lines are
    /// shaped first, so one not shaped since the edit counts whole. The
    /// listener hears of it once the draw is done (`tell_settled`).
    #[cold]
    #[inline(never)]
    pub(crate) fn settle_groups(&self, layout: &LinesLayout) -> bool {
        let scrolled: Vec<(u32, f32)> = (self.rare.borrow().as_ref())
            .and_then(|rare| rare.groups.as_ref())
            .map(|groups| {
                (groups.offsets.iter())
                    .filter(|(_, x)| **x > 0.)
                    .map(|(id, x)| (*id, *x))
                    .collect()
            })
            .unwrap_or_default();
        let mut settled = false;
        for (id, x) in scrolled {
            let past = |group: &Group| x > group.limit();
            let Some(group) = layout.group(id).filter(past) else {
                continue;
            };
            for i in group.first..=group.last {
                if layout.line_group(i) == Some(id) {
                    layout.line(i);
                }
            }
            let Some(group) = layout.group(id).filter(past) else {
                continue;
            };
            let to = group.limit();
            self.groups_mut(|groups| {
                groups.offsets.insert(id, to);
                groups.settled.push((id, to));
            });
            settled = true;
        }
        settled
    }
    /// Tell the listener of the groups `settle_groups` moved, the draw done.
    pub(crate) fn tell_settled(&self) {
        let (settled, listener) = {
            let mut rare = self.rare.borrow_mut();
            let Some(groups) = rare.as_mut().and_then(|rare| rare.groups.as_mut()) else {
                return;
            };
            if groups.settled.is_empty() {
                return;
            }
            (std::mem::take(&mut groups.settled), groups.listener.clone())
        };
        if let Some(listener) = listener {
            for (id, x) in settled {
                listener(id, x);
            }
        }
    }
    /// The scroll group that moved last, if one has.
    pub(crate) fn last_scrolled_group(&self) -> Option<u32> {
        if !self.rare_set.get() {
            return None;
        }
        (self.rare.borrow().as_ref())
            .and_then(|rare| rare.groups.as_ref())
            .and_then(|groups| groups.last)
    }
    /// Be told of each scroll group's new offset (none: no more).
    pub(crate) fn on_group_scroll(&self, listener: Option<GroupListener>) {
        self.groups_mut(|groups| groups.listener = listener);
    }
    /// Set scroll group `group`'s frame (an all-`None` one: as its lines say).
    pub(crate) fn set_group_frame(&self, group: u32, frame: GroupFrame) {
        self.groups_mut(|groups| {
            if frame == GroupFrame::default() {
                groups.frames.remove(&group);
            } else {
                groups.frames.insert(group, frame);
            }
        });
    }
    /// The frame the app set for scroll group `group`, if any.
    fn group_frame(&self, group: u32) -> GroupFrame {
        if !self.rare_set.get() {
            return GroupFrame::default();
        }
        (self.rare.borrow().as_ref())
            .and_then(|rare| rare.groups.as_ref())
            .and_then(|groups| groups.frames.get(&group).copied())
            .unwrap_or_default()
    }
    /// Line `i` of `layout`'s room (`set_insets`): none for an editor
    /// without.
    #[inline]
    pub(crate) fn inset(&self, layout: &LinesLayout, i: usize) -> LineInset {
        if !self.rare_set.get() {
            return LineInset::default();
        }
        self.inset_of(layout, i)
    }
    #[cold]
    #[inline(never)]
    fn inset_of(&self, layout: &LinesLayout, i: usize) -> LineInset {
        let Some(insets) = (self.rare.borrow().as_ref()).and_then(|rare| rare.insets.clone())
        else {
            return LineInset::default();
        };
        let before = self.state_before(layout, i);
        insets(i, &layout.line_text(i), before)
    }
    /// What the line before line `i` left its highlighter (0 for the first,
    /// or with none): the states above it found first, each once.
    fn state_before(&self, layout: &LinesLayout, i: usize) -> u32 {
        if i == 0 {
            return 0;
        }
        let Some(highlight) = self.highlighter.borrow().clone() else {
            return 0;
        };
        self.states_through(&highlight, layout, i);
        self.states.borrow()[i - 1]
    }
    /// The line after the last whose highlighter state before it the edit
    /// `layout` was just made by changed, of those from line `from` on (the
    /// first below the edit): a fence typed above lines, which are inside a
    /// block now. What lines leave is found down to where it is what it was.
    #[cold]
    #[inline(never)]
    fn restated(&self, layout: &LinesLayout, from: usize) -> usize {
        let Some(highlight) = self.highlighter.borrow().clone() else {
            return from;
        };
        let known = self.tail.borrow().is_some();
        for i in from..layout.line_count() {
            self.states_through(&highlight, layout, i);
            // The lines below leave what they did again (`push_state`).
            if known && self.tail.borrow().is_none() && self.states.borrow().len() > i {
                return i;
            }
        }
        layout.line_count()
    }
    /// What each line above line `i` leaves the next, found as far as it.
    #[inline]
    fn states_through(&self, highlight: &SpanHighlighter, layout: &LinesLayout, i: usize) {
        loop {
            let known = self.states.borrow().len();
            if known >= i {
                break;
            }
            let before = if known == 0 {
                0
            } else {
                self.states.borrow()[known - 1]
            };
            let (_, leaves) = self.highlight(highlight, &layout.line_text(known), before);
            self.push_state(&mut self.states.borrow_mut(), leaves);
        }
    }
    /// Style lines with `highlighter` (none: the editor's style).
    pub(crate) fn set_highlighter(&self, highlighter: Option<SpanHighlighter>) {
        *self.highlighter.borrow_mut() = highlighter;
        // Lines with room are asked for in `styled` whatever their spans, and
        // are measured again: their room may depend on its states.
        let room = self.has_insets();
        self.shapes.set(room);
        if room {
            self.forget_last();
        }
        self.tall.set(false);
        self.chips.set(false);
        self.states.borrow_mut().clear();
        *self.tail.borrow_mut() = None;
        self.colored.borrow_mut().clear();
    }
    /// Ask the highlighter for the colours of lines shown anew (a theme's):
    /// what lines leave the next, and so the room of those with some, holds.
    pub(crate) fn recolor(&self) {
        let room = self.has_insets();
        self.shapes.set(room);
        self.tall.set(false);
        self.chips.set(false);
        if !room {
            self.states.borrow_mut().clear();
            *self.tail.borrow_mut() = None;
        }
        self.colored.borrow_mut().clear();
    }
    /// Ask the highlighter for every line anew.
    pub(crate) fn rehighlight(&self) {
        let highlighter = self.highlighter.borrow().clone();
        self.set_highlighter(highlighter);
    }
    pub(crate) fn highlighted(&self) -> bool {
        self.highlighter.borrow().is_some()
            || (self.rare_set.get()
                && (self.rare.borrow().as_ref())
                    .is_some_and(|rare| rare.atom_spans.is_some() || rare.insets.is_some()))
    }
    /// The name a span gives `look` (`SpanStyle::look`): the same for the
    /// same look.
    pub(crate) fn look(&self, look: SpanLook) -> std::num::NonZeroU32 {
        let mut rare = self.rare.borrow_mut();
        let rare = rare.get_or_insert_default();
        self.rare_set.set(true);
        let at = match rare.looks.iter().position(|known| known.same(&look)) {
            Some(at) => at,
            None => {
                rare.pictured |= look.picture.is_some();
                rare.looks.push(look);
                rare.looks.len() - 1
            }
        };
        std::num::NonZeroU32::new(at as u32 + 1).expect("a look's name")
    }
    /// The name a span gives `svg` at `extents` (`[width, height, descent]`
    /// in EM) to show as a picture (`SpanStyle::inline_svg`): the same for
    /// the same picture at the same size.
    pub(crate) fn picture(
        &self,
        svg: std::sync::Arc<crate::svg::SvgData>,
        extents: [f32; 3],
    ) -> std::num::NonZeroU32 {
        self.look(SpanLook {
            picture: Some((svg, extents)),
            ..SpanLook::default()
        })
    }
    /// Its spans' looks, as `SpanStyle::look` names them.
    pub(crate) fn looks(&self) -> std::cell::Ref<'_, [SpanLook]> {
        std::cell::Ref::map(self.rare.borrow(), |rare| {
            rare.as_ref().map_or(&[][..], |rare| &rare.looks[..])
        })
    }
    /// Whether one of its looks is a picture.
    pub(crate) fn pictured(&self) -> bool {
        self.rare_set.get() && (self.rare.borrow().as_ref()).is_some_and(|rare| rare.pictured)
    }
    /// Whether it has looks or atom spans at all (`looks` is empty without).
    pub(crate) fn has_rare(&self) -> bool {
        self.rare_set.get()
    }
    /// Where the pictures on the lines between `top` and `bottom` show,
    /// in the area's points, and what each is: every box a span's picture
    /// holds (an atom's, a highlighter's).
    pub(crate) fn pictures_between(
        &self,
        layout: &LinesLayout,
        top: f32,
        bottom: f32,
        size: f32,
    ) -> Vec<(Rect, std::sync::Arc<crate::svg::SvgData>)> {
        if !self.pictured() {
            return Vec::new();
        }
        let mut shown = Vec::new();
        for i in layout.lines_between(top, bottom) {
            let Some(spans) = self.spans(layout, i) else {
                continue;
            };
            for (at, x, y) in layout.line(i).inline_boxes() {
                let looks = self.looks();
                let Some((svg, [w, h, _])) = spans
                    .iter()
                    .filter(|(range, _)| range.contains(&at))
                    .find_map(|(_, style)| look(&looks, style.look?)?.picture.as_ref())
                else {
                    continue;
                };
                let rect = layout.shift(i, Rect::new(x, y, w * size, h * size));
                shown.push((rect, svg.clone()));
            }
        }
        shown
    }
    /// The spans styled as chips with a background (`SpanStyle::chip`) on
    /// the lines between `top` and `bottom`: each one's range of the whole
    /// text, its background, its padding down and its corners. The text
    /// area draws them as it does its atoms' washes, behind the text and
    /// under the selection.
    pub(crate) fn chips_between(
        &self,
        layout: &LinesLayout,
        top: f32,
        bottom: f32,
    ) -> Vec<(Range<usize>, crate::scene::Color, f32, f32)> {
        let mut shown = Vec::new();
        if !self.chips.get() {
            return shown;
        }
        for i in layout.lines_between(top, bottom) {
            let Some(spans) = self.spans(layout, i) else {
                continue;
            };
            let start = layout.line_range(i).start;
            for (range, style) in spans.iter() {
                let text = style.text();
                if let (Some(padding), Some(background)) = (text.padding, style.chip_background()) {
                    shown.push((
                        start + range.start..start + range.end,
                        background,
                        padding[1].max(0.),
                        text.radius.unwrap_or(0.).max(0.),
                    ));
                }
            }
        }
        shown
    }
    /// Its atoms' spans over the whole text, by start (none: no atom styled).
    pub(crate) fn set_atom_spans(&self, spans: Option<LineSpans>) {
        if let Some(spans) = &spans
            && !self.shapes.get()
            && spans.iter().any(|(_, style)| style.shapes())
        {
            self.shapes.set(true);
            self.restyled.set(true);
        }
        let mut rare = self.rare.borrow_mut();
        if spans.is_some() || rare.is_some() {
            rare.get_or_insert_default().atom_spans = spans.map(Rc::new);
            self.rare_set.set(true);
        }
    }
    /// Whether a line's spans shape it now that did not before: what was
    /// laid out without them is laid out again.
    pub(crate) fn take_restyled(&self) -> bool {
        self.restyled.replace(false)
    }
    /// Line `i`'s spans, if it has a highlighter: what the lines above
    /// it leave found first (each once, until one changes).
    pub(crate) fn spans(&self, layout: &LinesLayout, i: usize) -> Option<Rc<LineSpans>> {
        if self.rare_set.get() {
            return self.spans_with_atoms(layout, i);
        }
        self.highlighted_spans(layout, i)
    }
    /// `spans` of an editor whose atoms have spans: its atoms' before its
    /// highlighter's. Rare, out of line.
    #[cold]
    #[inline(never)]
    fn spans_with_atoms(&self, layout: &LinesLayout, i: usize) -> Option<Rc<LineSpans>> {
        let atoms = (self.rare.borrow().as_ref()).and_then(|rare| rare.atom_spans.clone());
        let colors = self.highlighted_spans(layout, i);
        let Some(atoms) = atoms else {
            return colors;
        };
        atoms_on_line(&atoms, layout.line_range(i), colors)
    }
    /// Line `i`'s highlighter spans, if it has one. In `spans` itself, as
    /// before atoms: a call of its own cost every line drawn.
    #[inline(always)]
    fn highlighted_spans(&self, layout: &LinesLayout, i: usize) -> Option<Rc<LineSpans>> {
        let highlight = self.highlighter.borrow().clone()?;
        self.states_through(&highlight, layout, i);
        let before = if i == 0 {
            0
        } else {
            self.states.borrow()[i - 1]
        };
        let (colors, leaves) = self.highlight(&highlight, &layout.line_text(i), before);
        let mut states = self.states.borrow_mut();
        if states.len() == i {
            self.push_state(&mut states, leaves);
        }
        Some(colors)
    }
    /// `text` highlighted after a line that left `before` (from the cache
    /// if it is there; no borrow is held while the highlighter runs).
    fn highlight(
        &self,
        highlight: &SpanHighlighter,
        text: &str,
        before: u32,
    ) -> (Rc<LineSpans>, u32) {
        let tick = self.clock.get() + 1;
        self.clock.set(tick);
        let key = (Box::<str>::from(text), before);
        if let Some(hit) = self.colored.borrow_mut().get_mut(&key) {
            hit.used = tick;
            return (hit.colors.clone(), hit.leaves);
        }
        let (mut colors, leaves) = highlight(text, before);
        // Within the line, in order, on character boundaries.
        colors.retain_mut(|(range, _)| {
            range.end = range.end.min(text.len());
            range.start < range.end
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
        });
        colors.sort_by_key(|(range, _)| range.start);
        if !self.shapes.get() && colors.iter().any(|(_, style)| style.shapes()) {
            self.shapes.set(true);
            self.restyled.set(true);
        }
        if !self.tall.get() && colors.iter().any(|(_, style)| style.text.is_some()) {
            self.tall.set(true);
            self.restyled.set(true);
        }
        if self.tall.get()
            && !self.chips.get()
            && colors
                .iter()
                .any(|(_, style)| style.chip_background().is_some())
        {
            self.chips.set(true);
        }
        let colors = Rc::new(colors);
        let mut colored = self.colored.borrow_mut();
        if colored.len() >= SHAPED_LINES * 4 {
            let mut uses: Vec<u64> = colored.values().map(|c| c.used).collect();
            let half = uses.len() / 2;
            let (_, median, _) = uses.select_nth_unstable(half);
            let median = *median;
            colored.retain(|_, c| c.used > median);
        }
        colored.insert(
            key,
            Colored {
                colors: colors.clone(),
                leaves,
                used: tick,
            },
        );
        (colors, leaves)
    }
    /// Line `i` shaped with the runs its spans give it, if they shape it
    /// (a weight, italics): `None` for a line shaped as its plain text.
    fn styled(&self, layout: &LinesLayout, i: usize, text: &str) -> Option<Rc<dyn TextLayout>> {
        if !self.shapes.get() {
            return None;
        }
        // Lines with room of their own (`set_insets`, which makes lines be
        // asked here): shaped in it, out of line.
        if layout.room {
            return self.styled_in_room(layout, i, text);
        }
        self.styled_at(layout, i, text, (layout.wrap, text))
    }
    /// `styled` for an editor whose lines have room of their own: line `i`
    /// shaped in what its indent leaves (kept apart from the same text
    /// unindented), its room carried by its layout (`InsetLayout`).
    #[cold]
    #[inline(never)]
    fn styled_in_room(
        &self,
        layout: &LinesLayout,
        i: usize,
        text: &str,
    ) -> Option<Rc<dyn TextLayout>> {
        let inset = layout.inset(i);
        if inset == LineInset::default() {
            return self.styled_at(layout, i, text, (layout.wrap, text));
        }
        let indented;
        let (wrap, key) = match layout.wrap {
            // Unwrapped (a code block's line): kept apart from the same text
            // wrapped.
            Some(_) if inset.nowrap => {
                indented = format!("{}\u{2}{}", inset.left.to_bits(), text);
                (None, &*indented)
            }
            Some(_) if inset.left > 0. => {
                indented = format!("{}\u{0}{}", inset.left.to_bits(), text);
                (layout.draw_width(i, layout.wrap), &*indented)
            }
            wrap => (wrap, text),
        };
        let inner = match self.styled_at(layout, i, text, (wrap, key)) {
            Some(inner) => inner,
            None => self.line_at(text, &layout.shape, (wrap, key)),
        };
        // In a scroll group: moved by its offset after its own layout, and
        // cut to its frame.
        let group = match inset.scroll_group {
            Some(id) => {
                if inset.nowrap {
                    layout.reached(id, inset.left + inner.size().0);
                }
                layout.group(id).map(|group| GroupShift {
                    offset: self.group_scroll(id),
                    left: group.left,
                    right: group.right(),
                })
            }
            None => None,
        };
        Some(Rc::new(InsetLayout {
            inner,
            inset,
            group,
        }))
    }
    /// `styled`, shaped at `wrap` and kept by `key` (its text, or its text
    /// and indent: `LinesLayout::line_in_room`).
    #[inline(always)]
    fn styled_at(
        &self,
        layout: &LinesLayout,
        i: usize,
        text: &str,
        (wrap, key): (Option<f32>, &str),
    ) -> Option<Rc<dyn TextLayout>> {
        if !self.shapes.get() {
            return None;
        }
        let spans = self.spans(layout, i)?;
        let key = (Box::<str>::from(key), shape_key(&spans)?);
        let tick = self.clock.get() + 1;
        self.clock.set(tick);
        if let Some((layout, used)) = self.styled.borrow_mut().get_mut(&key) {
            *used = tick;
            return Some(layout.clone());
        }
        let (size, font) = {
            let shaping = self.shaping.borrow();
            let shaping = shaping.as_ref()?;
            (f32::from_bits(shaping.size), shaping.font.clone())
        };
        let rich = styled_line(
            text,
            &spans,
            crate::scene::Color(0, 0, 0, 255),
            size,
            &font,
            &self.looks(),
        )?;
        let shaped: Rc<dyn TextLayout> = (layout.rich)(&rich, wrap).into();
        let mut styled = self.styled.borrow_mut();
        evict(&mut styled);
        styled.insert(key, (shaped.clone(), tick));
        Some(shaped)
    }
    /// Line `text`, shaped (from the cache if it is there).
    fn line(&self, text: &str, shape: &LineShaper, wrap: Option<f32>) -> Rc<dyn TextLayout> {
        self.line_at(text, shape, (wrap, text))
    }
    /// `line`, kept by `key` (its text, or its text and indent).
    #[inline(always)]
    fn line_at(
        &self,
        text: &str,
        shape: &LineShaper,
        (wrap, key): (Option<f32>, &str),
    ) -> Rc<dyn TextLayout> {
        let tick = self.clock.get() + 1;
        self.clock.set(tick);
        if let Some((layout, used)) = self.shaped.borrow_mut().get_mut(key) {
            *used = tick;
            return layout.clone();
        }
        let layout: Rc<dyn TextLayout> = shape(text, wrap).into();
        let mut shaped = self.shaped.borrow_mut();
        evict(&mut shaped);
        shaped.insert(key.into(), (layout.clone(), tick));
        layout
    }
}
/// Shaped lines past the most kept: the least recently used half goes
/// (amortized, a line a shape).
fn evict<K: Eq + std::hash::Hash>(shaped: &mut HashMap<K, Shaped>) {
    if shaped.len() >= SHAPED_LINES {
        let mut uses: Vec<u64> = shaped.values().map(|(_, used)| *used).collect();
        let (_, median, _) = uses.select_nth_unstable(SHAPED_LINES / 2);
        let median = *median;
        shaped.retain(|_, (_, used)| *used > median);
    }
}

/// Heights a line at a time, and the sum of those above any line, both in
/// `O(log n)` (a Fenwick tree).
struct Heights {
    tree: Vec<f64>,
    each: Vec<f32>,
}
impl Heights {
    fn new(each: Vec<f32>) -> Self {
        let mut tree: Vec<f64> = each.iter().map(|h| *h as f64).collect();
        for i in 0..tree.len() {
            let parent = i | (i + 1);
            if parent < tree.len() {
                tree[parent] += tree[i];
            }
        }
        Self { tree, each }
    }
    /// The heights of the lines above line `i`.
    fn above(&self, i: usize) -> f64 {
        let mut sum = 0.;
        let mut at = i;
        while at > 0 {
            sum += self.tree[at - 1];
            at &= at - 1;
        }
        sum
    }
    fn set(&mut self, i: usize, height: f32) {
        let delta = height as f64 - self.each[i] as f64;
        self.each[i] = height;
        let mut at = i;
        while at < self.tree.len() {
            self.tree[at] += delta;
            at |= at + 1;
        }
    }
    /// The line at `y` (the last if below them all).
    fn line_at(&self, y: f64) -> usize {
        let n = self.tree.len();
        let (mut at, mut rest) = (0, y);
        let mut step = n.next_power_of_two();
        while step > 0 {
            let next = at + step;
            if next <= n && self.tree[next - 1] <= rest {
                at = next;
                rest -= self.tree[next - 1];
            }
            step >>= 1;
        }
        at.min(n.saturating_sub(1))
    }
    fn total(&self) -> f64 {
        self.above(self.tree.len())
    }
}

/// A long text laid out by its lines (see the module).
pub(crate) struct LinesLayout {
    /// The text, shared with its editor (its lines, and where each starts).
    text: Text,
    wrap: Option<f32>,
    /// A row's height, and an estimate of a byte's advance (for unwrapped
    /// width and wrapped heights before a line is shaped).
    row: f32,
    advance: f32,
    /// The longest line's bytes, for the unwrapped width.
    longest: usize,
    widest: Cell<f32>,
    heights: Option<RefCell<Heights>>,
    shape: LineShaper,
    rich: RichLineShaper,
    cache: Rc<LineCache>,
    /// Its lines have room of their own (`set_line_insets`), as when it was
    /// made: a text area's insets set lay it out anew.
    room: bool,
    /// A line is in a scroll group (`LineInset::scroll_group`): only then
    /// are the groups found.
    grouped: bool,
    /// Its scroll groups, made as they are first asked for.
    groups: RefCell<Option<Box<LayoutGroups>>>,
    /// Lines across chunks, joined once (a long line's every draw would
    /// copy it): a few, by number.
    joined: RefCell<HashMap<usize, Arc<str>>>,
}
/// A line shaped with its room (`LineInset`) in it: its carets, hits,
/// selection and size moved past its indent and the room above, its height
/// the room above and below too.
struct InsetLayout {
    inner: Rc<dyn TextLayout>,
    inset: LineInset,
    /// Its scroll group's offset and frame, as it was asked for.
    group: Option<GroupShift>,
}
/// A grouped line's offset (`LineInset::scroll_group`), applied after its
/// own layout, and its group's frame across.
#[derive(Clone, Copy)]
struct GroupShift {
    offset: f32,
    left: f32,
    right: f32,
}
impl InsetLayout {
    /// How far across its text shows from where it was shaped: its indent,
    /// less its group's offset.
    fn across(&self) -> f32 {
        match &self.group {
            Some(group) => self.inset.left - group.offset,
            None => self.inset.left,
        }
    }
    fn out(&self, r: Rect) -> Rect {
        Rect::new(
            r.x + self.across(),
            r.y + self.inset.above,
            r.width,
            r.height,
        )
    }
    /// `rects`, placed, and cut to its group's frame: what is scrolled out
    /// of it shows nothing.
    fn out_all(&self, rects: Vec<Rect>) -> Vec<Rect> {
        let placed = rects.into_iter().map(|r| self.out(r));
        let Some(group) = self.group else {
            return placed.collect();
        };
        placed
            .filter_map(|r| {
                let (left, right) = (r.x.max(group.left), (r.x + r.width).min(group.right));
                (right > left).then(|| Rect::new(left, r.y, right - left, r.height))
            })
            .collect()
    }
}
impl TextLayout for InsetLayout {
    fn cache_weight(&self) -> Option<usize> {
        self.inner.cache_weight()
    }
    fn size(&self) -> (f32, f32) {
        let (w, h) = self.inner.size();
        (w + self.inset.left, h + self.inset.above + self.inset.below)
    }
    fn hit_test(&self, x: f32, y: f32) -> usize {
        self.hit_position(x, y).byte_offset
    }
    fn hit_position(&self, x: f32, y: f32) -> TextPosition {
        // What its group's frame cuts is not hit: what shows at its edge is.
        let x = match &self.group {
            Some(group) => x.clamp(group.left, (group.right - 0.5).max(group.left)),
            None => x,
        };
        self.inner
            .hit_position(x - self.across(), y - self.inset.above)
    }
    fn caret(&self, byte_offset: usize) -> Rect {
        self.out(self.inner.caret(byte_offset))
    }
    fn caret_position(&self, position: TextPosition) -> Rect {
        self.out(self.inner.caret_position(position))
    }
    fn selection(&self, range: Range<usize>) -> Vec<Rect> {
        self.out_all(self.inner.selection(range))
    }
    fn selection_within(&self, range: Range<usize>, top: f32, bottom: f32) -> Vec<Rect> {
        let above = self.inset.above;
        self.out_all(
            self.inner
                .selection_within(range, top - above, bottom - above),
        )
    }
    fn line_carets(&self, text: &str) -> Option<Vec<crate::text_layout::LineCarets>> {
        let mut lines = self.inner.line_carets(text)?;
        let across = self.across();
        for line in &mut lines {
            line.top += self.inset.above;
            for x in &mut line.carets {
                *x += across;
            }
        }
        Some(lines)
    }
    fn carets_x(&self, offsets: &[usize]) -> Vec<f32> {
        let mut xs = self.inner.carets_x(offsets);
        let across = self.across();
        for x in &mut xs {
            *x += across;
        }
        xs
    }
    fn visual_line_range(&self, position: TextPosition) -> (TextPosition, TextPosition) {
        self.inner.visual_line_range(position)
    }
    fn visual_line_edge(&self, position: TextPosition, end: bool) -> TextPosition {
        self.inner.visual_line_edge(position, end)
    }
    fn inline_boxes(&self) -> Vec<(usize, f32, f32)> {
        let across = self.across();
        (self.inner.inline_boxes().into_iter())
            .map(|(at, x, y)| (at, x + across, y + self.inset.above))
            .collect()
    }
}
/// A line's text: in its chunk, or joined across chunks and kept.
pub(crate) enum LineText<'a> {
    In(&'a str),
    Joined(Arc<str>),
}
impl std::ops::Deref for LineText<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            LineText::In(text) => text,
            LineText::Joined(text) => text,
        }
    }
}
impl LineText<'_> {
    /// As a node's text: the joined line shared, not copied.
    pub(crate) fn to_arc(&self) -> Arc<str> {
        match self {
            LineText::In(text) => Arc::from(*text),
            LineText::Joined(text) => text.clone(),
        }
    }
}
/// Joined lines a layout keeps, at most.
const JOINED_LINES: usize = 64;
impl LinesLayout {
    fn new(
        text: Text,
        wrap: Option<f32>,
        (shape, rich): (LineShaper, RichLineShaper),
        cache: Rc<LineCache>,
    ) -> Self {
        let longest = text.longest_line();
        let sample = shape("0", None);
        let (advance, row) = sample.size();
        let mut layout = Self {
            text,
            wrap,
            // Any height a line can have (a fraction of a point too), but
            // none: a line is found by dividing by it.
            row: if row > 0. { row } else { 1. },
            advance: advance.max(0.),
            longest,
            widest: Cell::new(0.),
            heights: None,
            shape,
            rich,
            room: cache.has_insets(),
            grouped: false,
            groups: RefCell::default(),
            cache,
            joined: RefCell::default(),
        };
        // Each line's height is kept where lines differ: wrapped, or with
        // room of their own. Otherwise every line is a row high.
        if wrap.is_some() || layout.cache.keeps_heights() {
            let width = wrap.unwrap_or(f32::INFINITY);
            if layout.room {
                layout.measure_in_room(width);
                return layout;
            }
            let known = layout.cache.heights.borrow();
            let each = (0..layout.line_count())
                .map(|i| layout.bare_height(&layout.line_text(i), &known, width))
                .collect();
            drop(known);
            layout.heights = Some(RefCell::new(Heights::new(each)));
        }
        layout
    }
    /// Line `line`'s height before it is shaped: as last shaped, or estimated
    /// at `width`.
    fn bare_height(&self, line: &str, known: &HashMap<Box<str>, f32>, width: f32) -> f32 {
        known
            .get(line)
            .copied()
            .unwrap_or_else(|| self.estimate(line, width))
    }
    /// Line `i`'s height with its room before it is shaped (an unwrapped
    /// line's text, a row), and what its room says of its scroll group: its
    /// room asked for once.
    fn measure(&self, i: usize, width: f32) -> (f32, LineGroup) {
        let inset = self.inset(i);
        let bare = if inset.nowrap {
            self.row
        } else if inset.hidden {
            0.
        } else {
            self.estimate(&self.line_text(i), width)
        };
        let group = LineGroup {
            group: inset.scroll_group,
            left: inset.left,
            right: (inset.scroll_group.is_some() && !inset.hidden).then(|| {
                let width = match self.wrap {
                    Some(wrap) if !inset.nowrap => (wrap - inset.left).max(1.),
                    _ => self.line_range(i).len() as f32 * self.advance,
                };
                inset.left + width
            }),
        };
        (inset.height(bare), group)
    }
    /// Each line's height, its lines with room of their own.
    #[cold]
    #[inline(never)]
    fn measure_in_room(&mut self, width: f32) {
        let count = self.line_count();
        let (mut each, mut groups) = (Vec::with_capacity(count), Vec::new());
        for i in 0..count {
            let (height, group) = self.measure(i, width);
            each.push(height);
            if group.group.is_some() {
                groups.resize(i, LineGroup::default());
                groups.push(group);
            }
        }
        self.measured(each, groups);
    }
    /// The heights of `old`'s text edited to this one's (`edited`), its lines
    /// with room of their own: lines `lines` measured again, the rest kept,
    /// from `moved` of `old`'s on after them.
    #[cold]
    #[inline(never)]
    fn measure_edited(&mut self, old: &LinesLayout, lines: Range<usize>, moved: usize) {
        let Some(heights) = &old.heights else {
            return;
        };
        let width = self.wrap.unwrap_or(f32::INFINITY);
        let count = self.line_count();
        let mut each = Vec::with_capacity(count);
        let mut groups: Vec<LineGroup> = Vec::new();
        for i in lines.clone() {
            let (height, group) = self.measure(i, width);
            each.push(height);
            if group.group.is_some() {
                groups.resize(i, LineGroup::default());
                groups.push(group);
            }
        }
        let (measured, old_heights) = (each, heights.borrow());
        let mut each = Vec::with_capacity(count);
        each.extend_from_slice(&old_heights.each[..lines.start]);
        each.extend_from_slice(&measured);
        each.extend_from_slice(&old_heights.each[moved..]);
        drop(old_heights);
        let old_groups = old.groups.borrow();
        if let Some(old) = old_groups.as_ref().filter(|old| !old.each.is_empty()) {
            groups.resize(lines.end, LineGroup::default());
            groups[..lines.start].copy_from_slice(&old.each[..lines.start]);
            groups.extend_from_slice(&old.each[moved..]);
        }
        drop(old_groups);
        self.measured(each, groups);
    }
    /// Its lines' heights `each`, and groups `groups` (as far as any is in
    /// one; none: no line is).
    fn measured(&mut self, each: Vec<f32>, mut groups: Vec<LineGroup>) {
        self.grouped = groups.iter().any(|line| line.group.is_some());
        if self.grouped {
            groups.resize(each.len(), LineGroup::default());
            self.groups.get_mut().get_or_insert_default().each = groups;
        }
        self.heights = Some(RefCell::new(Heights::new(each)));
    }
    /// `old`'s layout of the text it became, `text`: the same as `text`'s
    /// own. The text starts with `prefix` bytes of `old`'s and ends with
    /// `suffix`: the lines between are measured again, the rest kept.
    fn edited(old: &LinesLayout, text: Text, prefix: usize, suffix: usize) -> Self {
        let (old_len, new_len) = (old.text.len(), text.len());
        // Lines starting before the edit; those after it, moved.
        let kept = old.text.line_of(prefix) + 1;
        let moved = old.text.line_of(old_len - suffix) + 1;
        let first_moved = text.line_of(new_len - suffix) + 1;
        let mut layout = Self {
            text,
            wrap: old.wrap,
            row: old.row,
            advance: old.advance,
            longest: old.longest,
            widest: Cell::new(old.widest.get()),
            heights: None,
            shape: old.shape.clone(),
            rich: old.rich.clone(),
            cache: old.cache.clone(),
            room: old.cache.has_insets(),
            grouped: false,
            groups: RefCell::default(),
            joined: RefCell::default(),
        };
        // The edited lines' lengths count for the longest.
        for i in kept - 1..first_moved.min(layout.line_count()) {
            layout.longest = layout.longest.max(layout.line_range(i).len());
        }
        if layout.room {
            // Those below whose state before them the edit changed too, whose
            // room may depend on it.
            let measured = layout.cache.restated(&layout, first_moved);
            let moved = moved + (measured - first_moved);
            layout.measure_edited(old, kept - 1..measured, moved);
        } else if let Some(heights) = &old.heights {
            let width = old.wrap.unwrap_or(f32::INFINITY);
            let old_each = &heights.borrow().each;
            let known = layout.cache.heights.borrow();
            let mut each = Vec::with_capacity(layout.line_count());
            each.extend_from_slice(&old_each[..kept - 1]);
            for i in kept - 1..first_moved {
                each.push(layout.bare_height(&layout.line_text(i), &known, width));
            }
            each.extend_from_slice(&old_each[moved..]);
            drop(known);
            layout.heights = Some(RefCell::new(Heights::new(each)));
        }
        layout
    }
    /// A wrapped line's height before it is shaped: its rows at the
    /// estimated advance.
    fn estimate(&self, line: &str, width: f32) -> f32 {
        let rows = ((line.len() as f32 * self.advance) / width.max(1.))
            .ceil()
            .max(1.);
        rows * self.row
    }
    /// How many lines it has.
    pub(crate) fn line_count(&self) -> usize {
        self.text.line_count()
    }
    /// Line `i`'s bytes in the text, its line break excluded.
    pub(crate) fn line_range(&self, i: usize) -> Range<usize> {
        self.text.line_range(i)
    }
    pub(crate) fn line_text(&self, i: usize) -> LineText<'_> {
        if let Some(text) = self.joined.borrow().get(&i) {
            return LineText::Joined(text.clone());
        }
        match self.text.line(i) {
            std::borrow::Cow::Borrowed(text) => LineText::In(text),
            std::borrow::Cow::Owned(text) => {
                let text: Arc<str> = text.into();
                let mut joined = self.joined.borrow_mut();
                if joined.len() >= JOINED_LINES {
                    joined.clear();
                }
                joined.insert(i, text.clone());
                LineText::Joined(text)
            }
        }
    }
    /// The line holding byte `offset` (a line's end, before its break, is
    /// its own; the offset after the break, the next line's).
    pub(crate) fn line_of(&self, offset: usize) -> usize {
        self.text.line_of(offset)
    }
    /// Where line `i` starts down the layout.
    pub(crate) fn top(&self, i: usize) -> f32 {
        match &self.heights {
            Some(heights) => heights.borrow().above(i) as f32,
            None => i as f32 * self.row,
        }
    }
    /// Line `i`'s height (as shaped, or estimated if it is not yet).
    pub(crate) fn height(&self, i: usize) -> f32 {
        match &self.heights {
            Some(heights) => heights.borrow().each[i],
            None => self.row,
        }
    }
    /// The line at `y` down the layout.
    pub(crate) fn line_at(&self, y: f32) -> usize {
        let y = y.max(0.);
        match &self.heights {
            Some(heights) => heights.borrow().line_at(y as f64),
            None => ((y / self.row) as usize).min(self.line_count() - 1),
        }
    }
    /// The lines that show between `top` and `bottom`, shaped: their
    /// numbers, the first and the last but one.
    pub(crate) fn lines_between(&self, top: f32, bottom: f32) -> Range<usize> {
        let first = self.line_at(top);
        let mut end = first;
        // Shaping a wrapped line corrects its height, which moves those
        // below: each is placed after the one above it is shaped.
        while end < self.line_count() && self.top(end) < bottom {
            self.line(end);
            end += 1;
        }
        first..end.max(first + 1).min(self.line_count())
    }
    /// The width line `i` is drawn in when the text wraps at `wrap`: what
    /// its indent leaves, as it was wrapped.
    pub(crate) fn draw_width(&self, i: usize, wrap: Option<f32>) -> Option<f32> {
        let inset = self.inset(i);
        if inset.nowrap {
            return None;
        }
        let left = inset.left;
        wrap.map(|w| if left > 0. { (w - left).max(1.) } else { w })
    }
    /// Whether a line is in a scroll group (`LineInset::scroll_group`).
    #[inline]
    pub(crate) fn grouped(&self) -> bool {
        self.grouped
    }
    /// Scroll group `id`'s offset.
    pub(crate) fn group_scroll(&self, id: u32) -> f32 {
        self.cache.group_scroll(id)
    }
    /// The scroll group that moved last, if one has.
    pub(crate) fn last_scrolled_group(&self) -> Option<u32> {
        self.cache.last_scrolled_group()
    }
    /// Line `i`'s scroll group, as it was measured.
    fn line_group(&self, i: usize) -> Option<u32> {
        (self.groups.borrow().as_ref()).and_then(|groups| groups.each.get(i)?.group)
    }
    /// Bring groups scrolled past their lines back (`LineCache::settle_groups`).
    pub(crate) fn settle_groups(&self) -> bool {
        self.cache.settle_groups(self)
    }
    /// Move scroll group `id` to `x` (`LineCache::set_group_scroll`).
    pub(crate) fn set_group_scroll(&self, id: u32, x: f32) -> bool {
        self.cache.set_group_scroll(id, x)
    }
    /// Scroll group `group`'s unwrapped line reaches `right`, as shaped: past
    /// its estimate, the group scrolls that far.
    fn reached(&self, group: u32, right: f32) {
        let mut groups = self.groups.borrow_mut();
        let far = groups
            .get_or_insert_default()
            .reach
            .entry(group)
            .or_insert(0.);
        *far = far.max(right);
    }
    /// Each scroll group's lines, found once, as each was measured.
    fn group_lines(&self, id: u32) -> Option<GroupLines> {
        if let Some(lines) = (self.groups.borrow().as_ref()).and_then(|g| g.lines.as_ref()) {
            return lines.get(&id).copied();
        }
        let mut found: HashMap<u32, GroupLines> = HashMap::new();
        for (i, line) in (self.groups.borrow().iter())
            .flat_map(|groups| &groups.each)
            .enumerate()
        {
            let Some(group) = line.group else {
                continue;
            };
            let lines = found.entry(group).or_insert(GroupLines {
                left: line.left,
                right: line.left,
                first: i,
                last: i,
            });
            lines.last = i;
            lines.right = lines.right.max(line.right.unwrap_or(line.left));
        }
        let mine = found.get(&id).copied();
        self.groups.borrow_mut().get_or_insert_default().lines = Some(found);
        mine
    }
    /// Scroll group `id`'s frame and extent, if a line is in it: as its lines
    /// say, past what the app set (`GroupFrame`).
    pub(crate) fn group(&self, id: u32) -> Option<Group> {
        if !self.grouped {
            return None;
        }
        let lines = self.group_lines(id)?;
        let frame = self.cache.group_frame(id);
        let left = frame.left.unwrap_or(lines.left);
        let view = frame
            .width
            .or(self.wrap.map(|w| w - left))
            .unwrap_or(f32::INFINITY)
            .max(1.);
        let reach = (self.groups.borrow().as_ref())
            .and_then(|groups| groups.reach.get(&id).copied())
            .unwrap_or(0.);
        let content = frame
            .content
            .unwrap_or(lines.right.max(reach) - left)
            .max(0.);
        Some(Group {
            left,
            view,
            content,
            first: lines.first,
            last: lines.last,
        })
    }
    /// Line `i`'s scroll group: its id and frame.
    pub(crate) fn group_of(&self, i: usize) -> Option<(u32, Group)> {
        if !self.grouped {
            return None;
        }
        let id = self.inset(i).scroll_group?;
        Some((id, self.group(id)?))
    }
    /// The offset line `i`'s scroll group needs for `rect` (where it shows
    /// now: a caret) to show inside its frame, if it moves.
    pub(crate) fn reveal_in_group(&self, i: usize, rect: Rect) -> Option<(u32, f32)> {
        let (id, group) = self.group_of(i)?;
        let now = self.cache.group_scroll(id);
        let wanted = if rect.x < group.left {
            now - (group.left - rect.x)
        } else if rect.x + rect.width > group.right() {
            now + (rect.x + rect.width - group.right())
        } else {
            return None;
        };
        let wanted = wanted.clamp(0., group.limit());
        (wanted != now).then_some((id, wanted))
    }
    /// Whether its lines have room of their own (`set_line_insets`).
    pub(crate) fn has_insets(&self) -> bool {
        self.room
    }
    /// Whether its lines have marks (`EditorHandle::set_line_marks`).
    pub(crate) fn has_marks(&self) -> bool {
        self.cache.has_marks()
    }
    /// Line `i`'s marks (`EditorHandle::set_line_marks`).
    pub(crate) fn marks(&self, i: usize) -> MarksOf {
        self.cache.marks(self, i)
    }
    /// Line `i`'s room (`EditorHandle::set_line_insets`).
    #[inline]
    pub(crate) fn inset(&self, i: usize) -> LineInset {
        if !self.room {
            return LineInset::default();
        }
        self.cache.inset(self, i)
    }
    /// Line `i`, shaped. A line with room of its own (`set_line_insets`)
    /// is shaped with its room in it (`InsetLayout`, from `LineCache::styled`):
    /// its carets, hits and size carry it, so nothing here asks for it.
    pub(crate) fn line(&self, i: usize) -> Rc<dyn TextLayout> {
        let text = self.line_text(i);
        let layout = match self.cache.styled(self, i, &text) {
            Some(layout) => layout,
            None => self.cache.line(&text, &self.shape, self.wrap),
        };
        let (width, height) = layout.size();
        if width > self.widest.get() {
            self.widest.set(width);
        }
        if let Some(heights) = &self.heights {
            let height = if self.room {
                self.height_in_room(i, height)
            } else {
                height.max(self.row)
            };
            if heights.borrow().each[i] != height {
                heights.borrow_mut().set(i, height);
                // A line's room isn't its text's: kept only without.
                if !self.room {
                    self.cache
                        .heights
                        .borrow_mut()
                        .insert((*text).into(), height);
                }
            }
        }
        layout
    }
    /// Line `i`'s height from its layout's (its room in it): none hidden.
    #[cold]
    #[inline(never)]
    fn height_in_room(&self, i: usize, height: f32) -> f32 {
        let inset = self.inset(i);
        if inset.hidden {
            0.
        } else {
            height.max(self.row + inset.above + inset.below)
        }
    }
    /// Line `i`'s local position for text position `position`.
    fn local(&self, i: usize, position: TextPosition) -> TextPosition {
        let range = self.line_range(i);
        TextPosition {
            byte_offset: position.byte_offset.clamp(range.start, range.end) - range.start,
            affinity: position.affinity,
        }
    }
    fn shift(&self, i: usize, rect: Rect) -> Rect {
        Rect::new(rect.x, rect.y + self.top(i), rect.width, rect.height)
    }
    /// The selection of `range`, only on the lines between `top` and
    /// `bottom`: what an editor draws of a selection of the whole file.
    pub(crate) fn selection_between(
        &self,
        range: Range<usize>,
        top: f32,
        bottom: f32,
    ) -> Vec<Rect> {
        let (start, end) = (range.start.min(range.end), range.start.max(range.end));
        if start == end {
            return Vec::new();
        }
        let shown = self.lines_between(top, bottom);
        let lines = shown.start.max(self.line_of(start))..shown.end.min(self.line_of(end) + 1);
        self.selection_on(start..end, lines)
    }
    /// The selection of `range` on `lines`, each shaped.
    fn selection_on(&self, range: Range<usize>, lines: Range<usize>) -> Vec<Rect> {
        let (start, end) = (range.start, range.end);
        let mut rects = Vec::new();
        for i in lines {
            let line = self.line_range(i);
            let (a, b) = (start.max(line.start), end.min(line.end));
            if a >= b {
                // An empty line in the selection: its line break, a little wide.
                if line.is_empty() && start <= line.start && end > line.end {
                    let row = self.line(i).caret_position(Default::default());
                    let width = crate::text_layout::empty_line_width(row.height);
                    let sliver = Rect::new(row.x, row.y, width, row.height);
                    rects.push(self.shift(i, sliver));
                }
                continue;
            }
            let layout = self.line(i);
            rects.extend(
                layout
                    .selection(a - line.start..b - line.start)
                    .into_iter()
                    .map(|rect| self.shift(i, rect)),
            );
        }
        rects
    }
}
impl TextLayout for LinesLayout {
    fn size(&self) -> (f32, f32) {
        let height = match &self.heights {
            Some(heights) => heights.borrow().total() as f32,
            None => self.line_count() as f32 * self.row,
        };
        let width = match self.wrap {
            Some(width) => width,
            None => self.widest.get().max(self.longest as f32 * self.advance),
        };
        (width, height)
    }
    fn hit_test(&self, x: f32, y: f32) -> usize {
        self.hit_position(x, y).byte_offset
    }
    fn hit_position(&self, x: f32, y: f32) -> TextPosition {
        let i = self.line_at(y);
        let layout = self.line(i);
        let local = layout.hit_position(x, y.max(0.) - self.top(i));
        let range = self.line_range(i);
        TextPosition {
            byte_offset: range.start + local.byte_offset.min(range.len()),
            // Its line's end is before its break, which a caret sits after.
            affinity: if local.byte_offset >= range.len() && i + 1 < self.line_count() {
                TextAffinity::After
            } else {
                local.affinity
            },
        }
    }
    fn caret(&self, byte_offset: usize) -> Rect {
        self.caret_position(TextPosition {
            byte_offset,
            affinity: TextAffinity::After,
        })
    }
    fn caret_position(&self, position: TextPosition) -> Rect {
        let i = self.line_of(position.byte_offset.min(self.text.len()));
        let layout = self.line(i);
        self.shift(i, layout.caret_position(self.local(i, position)))
    }
    /// The selection of `range`: its own lines shaped, not those above it
    /// (a composition's underline, a few characters of a long file).
    fn selection(&self, range: Range<usize>) -> Vec<Rect> {
        let (start, end) = (range.start.min(range.end), range.start.max(range.end));
        if start == end {
            return Vec::new();
        }
        self.selection_on(start..end, self.line_of(start)..self.line_of(end) + 1)
    }
    fn selection_within(&self, range: Range<usize>, top: f32, bottom: f32) -> Vec<Rect> {
        self.selection_between(range, top, bottom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain mark is as small as it was before marks had looks: a note's
    /// marks are kept by their thousands.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn a_plain_mark_is_no_bigger_for_the_looks_few_have() {
        assert_eq!(std::mem::size_of::<LineMark>(), 72);
    }

    /// A highlighted line's spans are this big and no bigger: main's
    /// sixteen bytes, an underline, and a four-byte name of the rare rest
    /// (`SpanLook`: an underline's colour and place, a lead, a picture). A
    /// span and its range stay forty bytes.
    #[test]
    fn span_style_stays_its_size() {
        assert_eq!(std::mem::size_of::<Option<std::num::NonZeroU32>>(), 4);
        assert_eq!(std::mem::size_of::<SpanStyle>(), 24);
        assert_eq!(std::mem::size_of::<(Range<usize>, SpanStyle)>(), 40);
    }

    /// A note's heading and inline code: size, line pitch, underline and
    /// the monospaced font reach the runs; a span with none of them names
    /// no text, and the same text is the same name.
    #[test]
    fn note_spans_set_size_pitch_underline_and_font() {
        use crate::scene::Color;
        let ink = Color(0, 0, 0, 255);
        let spans: LineSpans = vec![
            (
                0..5,
                SpanStyle::default().size(24.).line_height(32.).weight(600),
            ),
            (6..10, SpanStyle::default().mono()),
            (11..15, SpanStyle::default().underline()),
        ];
        let font = crate::text_layout::FontStyle::default();
        let rich = styled_line("Title code link", &spans, ink, 16., &font, &[]).unwrap();
        let runs = rich.runs();
        let title = runs.iter().find(|r| r.range == (0..5)).unwrap();
        assert_eq!(title.font_size, 24.);
        assert_eq!(title.font.line_height.pixels(), Some(32.));
        assert_eq!(title.font.weight, 600);
        let code = runs.iter().find(|r| r.range == (6..10)).unwrap();
        assert_eq!(code.font.family, crate::text_layout::FontFamily::Monospace);
        assert_eq!(code.font_size, 16.);
        assert!(
            runs.iter()
                .find(|r| r.range == (11..15))
                .unwrap()
                .underline
                .is_some()
        );
        assert!(
            shape_key(&spans).is_some(),
            "size and font reshape the line"
        );
        assert!(shape_key(&vec![(11..15, SpanStyle::default().underline())]).is_none());
        assert_eq!(SpanStyle::default().text, None);
        assert_eq!(
            SpanStyle::default().size(24.).text,
            SpanStyle::default().size(24.).text
        );
        assert_ne!(
            SpanStyle::default().size(24.).text,
            SpanStyle::default().size(20.).text
        );
    }

    /// A chip span takes room across round its text and shapes its line, so
    /// carets follow the room; its background is drawn as a mark.
    #[test]
    fn a_chip_span_pads_and_rounds_its_background() {
        use crate::scene::Color;
        let wash = Color(200, 200, 255, 255);
        let spans: LineSpans = vec![(
            4..8,
            SpanStyle::default().background(wash).chip([4., 2.], 6.),
        )];
        let font = crate::text_layout::FontStyle::default();
        let rich = styled_line(
            "see chip here",
            &spans,
            Color(0, 0, 0, 255),
            14.,
            &font,
            &[],
        )
        .unwrap();
        let chip = rich
            .runs()
            .iter()
            .find(|r| r.range == (4..8))
            .unwrap()
            .clone();
        assert_eq!(chip.padding, [4., 2.]);
        // Its background and corners are the text area's mark, under the
        // selection (`LineCache::chips_between`): not the run's.
        assert_eq!(chip.background, None);
        assert_eq!(chip.radius, 0.);
        assert!(
            shape_key(&spans).is_some(),
            "its room across reshapes the line"
        );
    }

    /// Overlapping spans merge their texts as their other styles: the
    /// first's size, and the monospaced font if either has it (code in a
    /// heading).
    #[test]
    fn overlapping_span_texts_merge() {
        use crate::scene::Color;
        let spans: LineSpans = vec![
            (0..10, SpanStyle::default().size(24.)),
            (6..10, SpanStyle::default().mono()),
        ];
        let font = crate::text_layout::FontStyle::default();
        let rich = styled_line("Title code", &spans, Color(0, 0, 0, 255), 16., &font, &[]).unwrap();
        let code = rich
            .runs()
            .iter()
            .find(|r| r.range == (6..10))
            .unwrap()
            .clone();
        assert_eq!(code.font_size, 24.);
        assert_eq!(code.font.family, crate::text_layout::FontFamily::Monospace);
    }

    /// Spans that only colour give their runs as before styles: in order,
    /// an earlier span keeping what a later one overlaps, the rest the
    /// editor's colour.
    #[test]
    fn colour_spans_give_runs_as_before_styles() {
        use crate::scene::Color;
        let (red, blue, ink) = (
            Color(255, 0, 0, 255),
            Color(0, 0, 255, 255),
            Color(0, 0, 0, 255),
        );
        let spans: LineSpans = vec![(1..4, red.into()), (2..6, blue.into())];
        let font = crate::text_layout::FontStyle::default();
        let rich = styled_line("abcdefgh", &spans, ink, 14., &font, &[]).unwrap();
        let runs: Vec<_> = rich
            .runs()
            .iter()
            .map(|r| (r.range.clone(), r.color))
            .collect();
        assert_eq!(runs, [(0..1, ink), (1..4, red), (4..6, blue), (6..8, ink)]);
    }
    use crate::text_layout::{FallbackTextLayout, FontStyle};

    fn rich() -> RichLineShaper {
        Rc::new(|rich, width| Box::new(FallbackTextLayout::with_rich(rich, width)))
    }
    fn shaper() -> (LineShaper, RichLineShaper) {
        (
            Rc::new(|text, width| Box::new(FallbackTextLayout::new(text, 14., width))),
            rich(),
        )
    }
    fn lines(text: &str, wrap: Option<f32>) -> Rc<LinesLayout> {
        Rc::new(LineCache::default()).layout(
            || Text::new(text),
            None,
            14.,
            wrap,
            &FontStyle::default(),
            0,
            shaper,
        )
    }

    /// An inset line keeps its carets, hits and selection inside its room,
    /// and wraps in what the indent leaves; the lines around it are as
    /// they were.
    #[test]
    fn insets_move_carets_hits_and_wrapping() {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i, _: &str, _| {
            if i == 1 {
                LineInset::new(36., 8., 4.)
            } else {
                LineInset::default()
            }
        })));
        let text = "first\nsecond line that is long enough to wrap in a narrow box\nthird";
        let layout = |cache: &Rc<LineCache>| {
            cache.layout(
                || Text::new(text),
                None,
                14.,
                Some(200.),
                &FontStyle::default(),
                0,
                shaper,
            )
        };
        let inset = layout(&cache);
        let plain = layout(&Rc::new(LineCache::default()));
        let second = text.find("second").unwrap();
        let (a, b) = (inset.caret(second), plain.caret(second));
        assert_eq!(a.x, b.x + 36.);
        assert_eq!(a.y, b.y + 8.);
        assert_eq!(inset.hit_test(a.x + 1., a.y + 1.), second);
        assert!(
            inset.height(1) > plain.height(1) + 11.,
            "above, below and a narrower wrap add height"
        );
        let third = text.find("third").unwrap();
        assert!(inset.caret(third).y > plain.caret(third).y + 11.);
        assert_eq!(inset.caret(0), plain.caret(0));
    }

    /// Unwrapped, a line's room still pushes the lines below it; an editor
    /// without insets keeps every line a row high, as before.
    #[test]
    fn insets_hold_unwrapped_too() {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i, _: &str, _| {
            if i == 0 {
                LineInset::new(0., 10., 6.)
            } else {
                LineInset::default()
            }
        })));
        let text = "one\ntwo";
        let lay = |cache: &Rc<LineCache>| {
            cache.layout(
                || Text::new(text),
                None,
                14.,
                None,
                &FontStyle::default(),
                0,
                shaper,
            )
        };
        let inset = lay(&cache);
        let plain = lay(&Rc::new(LineCache::default()));
        inset.lines_between(0., 1000.);
        assert_eq!(inset.top(1), plain.top(1) + 16.);
        assert_eq!(inset.caret(0).y, 10.);
        assert!(
            plain.heights.is_none(),
            "plain unwrapped lines keep no heights"
        );
    }

    /// A note's heading, then a paragraph and a list item: each line starts
    /// where the one above ends (its taller pitch and its room included),
    /// wrapped or not, and a heading's line is as tall as its pitch.
    #[test]
    fn a_heading_pushes_the_lines_below_it() {
        for wrap in [None, Some(400.)] {
            let cache = Rc::new(LineCache::default());
            cache.set_highlighter(Some(Rc::new(|line: &str, state: u32| {
                let spans: LineSpans = if state == 1 {
                    vec![(
                        0..line.len(),
                        SpanStyle::default().size(24.).line_height(32.).weight(600),
                    )]
                } else {
                    Vec::new()
                };
                (spans, state + 1)
            })));
            cache.set_insets(Some(Rc::new(|i, _: &str, _| match i {
                1 => LineInset::new(0., 24., 0.),
                2 => LineInset::new(0., 12., 0.),
                3 => LineInset::new(32., 8., 0.),
                _ => LineInset::default(),
            })));
            let text = "Title\nA heading\nbody text\nList item";
            let lay = || {
                cache.layout(
                    || Text::new(text),
                    None,
                    14.,
                    wrap,
                    &FontStyle::default(),
                    0,
                    shaper,
                )
            };
            // As the editor does: painting asks for the spans, and a span
            // that shapes lays the text out again.
            let first = lay();
            for i in 0..4 {
                cache.spans(&first, i);
            }
            assert!(cache.take_restyled());
            cache.forget_last();
            let layout = lay();
            layout.lines_between(0., 1000.);
            assert!(
                layout.height(1) >= 32. + 24.,
                "{wrap:?}: the heading's pitch and room: {}",
                layout.height(1)
            );
            for i in 1..4 {
                assert_eq!(
                    layout.top(i),
                    layout.top(i - 1) + layout.height(i - 1),
                    "{wrap:?}: line {i} starts below line {}",
                    i - 1
                );
            }
            let heading = layout.caret(text.find("A heading").unwrap());
            let body = layout.caret(text.find("body").unwrap());
            let item = layout.caret(text.find("List item").unwrap());
            assert_eq!(heading.y, layout.top(1) + 24.);
            assert!(
                body.y >= heading.y + 32. + 12.,
                "{wrap:?}: the paragraph sits below the heading: {} vs {}",
                body.y,
                heading.y
            );
            assert!(
                item.y >= body.y + 8. && item.x >= 32.,
                "{wrap:?}: the list item, indented, below the paragraph"
            );
        }
    }

    /// Unwrapped, a highlighter that only colours, weighs or slants keeps
    /// no heights: every line a row high, as before.
    #[test]
    fn plain_spans_keep_no_heights_unwrapped() {
        let cache = Rc::new(LineCache::default());
        cache.set_highlighter(Some(Rc::new(|line: &str, state: u32| {
            (
                vec![(0..line.len(), SpanStyle::default().weight(600))],
                state,
            )
        })));
        let lay = || {
            cache.layout(
                || Text::new("a\nb"),
                None,
                14.,
                None,
                &FontStyle::default(),
                0,
                shaper,
            )
        };
        let first = lay();
        cache.spans(&first, 0);
        cache.forget_last();
        assert!(lay().heights.is_none());
    }

    /// A hidden line takes no room: the line after it starts where it
    /// would have.
    #[test]
    fn a_hidden_line_takes_no_room() {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i, _: &str, _| {
            if i == 1 {
                LineInset::hidden()
            } else {
                LineInset::default()
            }
        })));
        for wrap in [None, Some(300.)] {
            cache.forget_last();
            let layout = cache.layout(
                || Text::new("one\ntwo\nthree"),
                None,
                14.,
                wrap,
                &FontStyle::default(),
                0,
                shaper,
            );
            layout.lines_between(0., 1000.);
            assert_eq!(layout.height(1), 0., "{wrap:?}");
            assert_eq!(layout.top(2), layout.top(1), "{wrap:?}");
        }
    }

    /// An indented line is drawn in the width it was wrapped to, not the
    /// whole text's: its rows end at the same edge.
    #[test]
    fn an_indented_line_draws_in_the_width_it_wraps_to() {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i, _: &str, _| {
            if i == 1 {
                LineInset::new(36., 0., 0.)
            } else {
                LineInset::default()
            }
        })));
        let text = "first\nsecond line that is long enough to wrap in a narrow box";
        let layout = cache.layout(
            || Text::new(text),
            None,
            14.,
            Some(200.),
            &FontStyle::default(),
            0,
            shaper,
        );
        assert_eq!(layout.draw_width(1, Some(200.)), Some(164.));
        assert_eq!(layout.draw_width(0, Some(200.)), Some(200.));
        assert!(
            layout.line(1).size().0 <= 164. + 36.,
            "its rows fit the width it is drawn in, past its indent"
        );
        assert_eq!(layout.draw_width(1, None), None);
    }

    /// Laid out by its lines, a text has the carets, hits and selections it
    /// has laid out whole, wrapped or not.
    #[test]
    fn lines_lay_out_as_the_whole_text_does() {
        let text =
            "fn main() {\n\n    let wide = \"a line long enough to wrap twice over\";\n}\nend";
        for wrap in [None, Some(120.)] {
            let whole = FallbackTextLayout::new(text, 14., wrap);
            let lines = lines(text, wrap);
            assert_eq!(whole.size().1, lines.size().1, "{wrap:?}");
            for offset in 0..=text.len() {
                if !text.is_char_boundary(offset) {
                    continue;
                }
                for affinity in [TextAffinity::Before, TextAffinity::After] {
                    let position = TextPosition {
                        byte_offset: offset,
                        affinity,
                    };
                    assert_eq!(
                        whole.caret_position(position),
                        lines.caret_position(position),
                        "caret {offset} {affinity:?} {wrap:?}"
                    );
                }
                for end in offset..=text.len() {
                    assert_eq!(
                        whole.selection(offset..end),
                        lines.selection(offset..end),
                        "selection {offset}..{end} {wrap:?}"
                    );
                }
            }
            let (_, height) = whole.size();
            let mut y = -5.;
            while y < height + 10. {
                let mut x = -5.;
                while x < 400. {
                    assert_eq!(
                        whole.hit_position(x, y).byte_offset,
                        lines.hit_position(x, y).byte_offset,
                        "hit {x} {y} {wrap:?}"
                    );
                    x += 7.;
                }
                y += 5.;
            }
        }
    }

    /// An edit finds the lines it changed and moves those after it: the
    /// same lines a layout of the new text has, wrapped or not.
    #[test]
    fn an_edit_lays_out_as_the_text_it_makes_does() {
        let mut text = String::from("one\ntwo\n\nthree four five six seven\neight");
        let edits: [(usize, usize, &str); 8] = [
            (0, 0, "zero\n"),
            (4, 9, ""),
            (7, 7, "a\nb\nc"),
            (3, 12, "x"),
            (0, 5, ""),
            (2, 2, "\n\n"),
            (1, 3, "long line that wraps around and around"),
            (0, 0, ""),
        ];
        for wrap in [None, Some(60.)] {
            let cache = Rc::new(LineCache::default());
            let mut current = text.clone();
            cache.layout(
                || Text::new(&current),
                None,
                14.,
                wrap,
                &FontStyle::default(),
                0,
                shaper,
            );
            for (start, end, insert) in edits {
                let (start, end) = (start.min(current.len()), end.min(current.len()));
                current.replace_range(start.min(end)..end, insert);
                let layout = cache.layout(
                    || Text::new(&current),
                    None,
                    14.,
                    wrap,
                    &FontStyle::default(),
                    0,
                    shaper,
                );
                let fresh = lines(&current, wrap);
                assert_eq!(
                    (0..layout.line_count())
                        .map(|i| layout.line_range(i))
                        .collect::<Vec<_>>(),
                    (0..fresh.line_count())
                        .map(|i| fresh.line_range(i))
                        .collect::<Vec<_>>(),
                    "{current:?} {wrap:?}"
                );
                assert_eq!(layout.size().1, fresh.size().1, "{current:?} {wrap:?}");
                assert!(layout.longest >= fresh.longest);
            }
            text = current;
        }
    }

    /// A file of 100,000 lines shapes the lines asked for, not the file,
    /// and keeps a few bytes a line.
    #[test]
    fn a_long_file_shapes_only_the_lines_it_shows() {
        let shaped = Rc::new(Cell::new(0));
        let count = shaped.clone();
        let shape: LineShaper = Rc::new(move |text, width| {
            count.set(count.get() + 1);
            Box::new(FallbackTextLayout::new(text, 14., width))
        });
        let text: String = (0..100_000).map(|i| format!("line {i}\n")).collect();
        let cache = Rc::new(LineCache::default());
        let layout = cache.layout(
            || Text::new(&text),
            None,
            14.,
            None,
            &FontStyle::default(),
            0,
            || (shape, rich()),
        );
        let shown = layout.lines_between(50_000. * layout.row, 50_000. * layout.row + 600.);
        assert_eq!(shown.start, 50_000);
        assert!(shown.len() < 40, "{shown:?}");
        let caret = layout.caret(text.find("line 99999").unwrap());
        assert_eq!(caret.y, 99_999. * layout.row);
        // The sample row, the lines shown and the caret's.
        assert!(shaped.get() < 45, "{}", shaped.get());
        // A selection (a composition's underline) shapes its own lines, not
        // every line above it.
        let at = text.find("line 70000").unwrap();
        let before = shaped.get();
        assert_eq!(layout.selection(at..at + 4).len(), 1);
        assert!(shaped.get() - before <= 1, "{}", shaped.get() - before);
    }

    /// A note 200 points wide: a prose line, a code block (lines 1 and 2)
    /// unwrapped in scroll group 3 and 30 points in, its first line far
    /// wider than the note, and a prose line after it.
    const BLOCK: &str =
        "intro\nlet wide = \"0123456789012345678901234567890123456789\";\nshort\nafter";
    /// `BLOCK` laid out, its lines shaped, and how many times it shaped.
    fn block() -> (Rc<LineCache>, Rc<LinesLayout>, Rc<Cell<usize>>) {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i, _: &str, _| match i {
            1 | 2 => LineInset::new(30., 0., 0.).in_group(3).unwrapped(),
            _ => LineInset::default(),
        })));
        let shaped = Rc::new(Cell::new(0));
        let counted = shaped.clone();
        let layout = cache.layout(
            || Text::new(BLOCK),
            None,
            14.,
            Some(200.),
            &FontStyle::default(),
            0,
            move || {
                let plain: LineShaper = Rc::new(move |text, width| {
                    counted.set(counted.get() + 1);
                    Box::new(FallbackTextLayout::new(text, 14., width))
                });
                (plain, rich())
            },
        );
        layout.lines_between(0., 10_000.);
        (cache, layout, shaped)
    }

    /// A code block's line never wraps: one row at its full width, its group
    /// framed from its left to the note's edge and scrolling as far as it
    /// reaches. The prose around it wraps as before.
    #[test]
    fn an_unwrapped_line_is_one_row_and_its_group_scrolls_its_width() {
        let (_, layout, _) = block();
        assert!(layout.grouped());
        let wide = layout.line(1);
        assert!(wide.size().0 > 200., "{:?}", wide.size());
        assert_eq!(layout.height(1), layout.row, "one row");
        let group = layout.group(3).unwrap();
        assert_eq!((group.left, group.view), (30., 170.));
        assert_eq!((group.first, group.last), (1, 2));
        assert!(
            (group.content - (wide.size().0 - 30.)).abs() < 0.5,
            "{group:?}"
        );
        assert_eq!(layout.group_of(0), None, "prose is in no group");
    }

    /// Scrolled, a group's carets move by its offset and the prose's don't;
    /// nothing is shaped again.
    #[test]
    fn a_groups_offset_moves_its_carets_after_their_layout() {
        let (cache, layout, shaped) = block();
        let at = BLOCK.find("wide").unwrap();
        let (before, prose) = (layout.caret(at), layout.caret(BLOCK.find("after").unwrap()));
        let shapes = shaped.get();
        assert!(cache.set_group_scroll(3, 50.));
        assert!(!cache.set_group_scroll(3, 50.), "the same offset: no move");
        let after = layout.caret(at);
        assert_eq!(after.x, before.x - 50.);
        assert_eq!(after.y, before.y);
        assert_eq!(layout.caret(BLOCK.find("short").unwrap()).x, 30. - 50.);
        assert_eq!(layout.caret(BLOCK.find("after").unwrap()), prose);
        assert_eq!(shaped.get(), shapes, "a scroll shapes nothing");
    }

    /// A click in a scrolled group lands on what shows under it; past its
    /// frame, on what shows at the frame's edge, never on text cut off.
    #[test]
    fn hits_in_a_scrolled_group_land_on_what_shows() {
        let (cache, layout, shaped) = block();
        let shapes = shaped.get();
        let y = layout.top(1) + 2.;
        for offset in [0., 40., 120.] {
            cache.set_group_scroll(3, offset);
            let at = BLOCK.find("2345").unwrap() + 20;
            let caret = layout.caret(at);
            if caret.x > 30. && caret.x < 200. {
                assert_eq!(layout.hit_test(caret.x + 0.5, y), at, "at {offset}");
            }
            // Left of the frame (the gutter): the first character shown.
            let left = layout.hit_test(5., y);
            assert!(
                (layout.caret(left).x - 30.).abs() < 9.,
                "at {offset}: {left} shows at {:?}",
                layout.caret(left)
            );
            // Past it: the last character shown, not one cut off.
            let right = layout.hit_test(400., y);
            assert!(layout.caret(right).x <= 200., "at {offset}: {right}");
        }
        assert_eq!(shaped.get(), shapes);
    }

    /// A selection across a scrolled group's line shows only inside its
    /// frame, moved with its text.
    #[test]
    fn a_selection_in_a_group_is_cut_to_its_frame() {
        let (cache, layout, shaped) = block();
        let line = layout.line_range(1);
        let shapes = shaped.get();
        for offset in [0., 60.] {
            cache.set_group_scroll(3, offset);
            let rects = layout.selection(line.clone());
            assert!(!rects.is_empty());
            for rect in &rects {
                assert!(
                    rect.x >= 30. - 0.01 && rect.x + rect.width <= 200. + 0.01,
                    "at {offset}: {rect:?}"
                );
            }
        }
        // Its first characters scrolled out: none of them shows.
        assert!(layout.selection(line.start..line.start + 3).is_empty());
        assert_eq!(shaped.get(), shapes);
    }

    /// A caret the frame cuts asks the group to move just enough to show it,
    /// within how far it scrolls; one it shows asks nothing.
    #[test]
    fn a_caret_cut_by_a_groups_frame_is_revealed() {
        let (cache, layout, _) = block();
        let end = layout.line_range(1).end;
        let caret = layout.caret(end);
        let (group, to) = layout.reveal_in_group(1, caret).expect("cut off");
        assert_eq!(group, 3);
        let limit = layout.group(3).unwrap().limit();
        assert!(limit > 0.);
        // Just enough, or as far as it goes (the caret's own width past its
        // line's end).
        assert_eq!(to, (caret.x + caret.width - 200.).min(limit), "{caret:?}");
        cache.set_group_scroll(3, to);
        assert_eq!(layout.reveal_in_group(1, layout.caret(end)), None);
        let start = layout.line_range(1).start;
        assert_eq!(
            layout.reveal_in_group(1, layout.caret(start)),
            Some((3, 0.))
        );
        assert_eq!(layout.reveal_in_group(0, layout.caret(0)), None);
    }

    /// The app's frame overrides what the lines say: where it starts, how
    /// wide, how far its lines reach.
    #[test]
    fn an_apps_group_frame_overrides_its_lines() {
        let (cache, layout, _) = block();
        cache.set_group_frame(3, GroupFrame::new(Some(20.), Some(100.), Some(500.)));
        let group = layout.group(3).unwrap();
        assert_eq!((group.left, group.view, group.content), (20., 100., 500.));
        assert_eq!(group.limit(), 400.);
        cache.set_group_frame(3, GroupFrame::default());
        assert_eq!(layout.group(3).unwrap().left, 30.);
    }

    /// A text without groups lays out, hits and selects as before whatever
    /// offsets are set.
    #[test]
    fn lines_in_no_group_ignore_group_offsets() {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i, _: &str, _| {
            if i == 1 {
                LineInset::new(30., 0., 0.)
            } else {
                LineInset::default()
            }
        })));
        let layout = cache.layout(
            || Text::new(BLOCK),
            None,
            14.,
            Some(200.),
            &FontStyle::default(),
            0,
            shaper,
        );
        layout.lines_between(0., 10_000.);
        assert!(!layout.grouped());
        let carets: Vec<Rect> = (0..BLOCK.len()).map(|i| layout.caret(i)).collect();
        cache.set_group_scroll(0, 50.);
        cache.set_group_scroll(3, 120.);
        assert_eq!(
            carets,
            (0..BLOCK.len())
                .map(|i| layout.caret(i))
                .collect::<Vec<_>>()
        );
        assert_eq!(layout.group(3), None);
    }

    /// A note whose fenced lines (its highlighter's state 1) are 30 in, in
    /// scroll group 3 and unwrapped, and a closed toggle's (state 2, from a
    /// line `>` to one `<`) hidden; how many lines were asked for their room.
    fn fenced() -> (Rc<LineCache>, Rc<Cell<usize>>) {
        let cache = Rc::new(LineCache::default());
        cache.set_highlighter(Some(fences()));
        let asked = Rc::new(Cell::new(0));
        let count = asked.clone();
        cache.set_insets(Some(Rc::new(move |_, line: &str, state| {
            count.set(count.get() + 1);
            let mut inset = LineInset::default();
            match state {
                1 if line != "```" => {
                    inset.left = 30.;
                    inset.scroll_group = Some(3);
                    inset.nowrap = true;
                }
                2 => inset.hidden = true,
                _ => {}
            }
            inset
        })));
        (cache, asked)
    }
    fn fences() -> SpanHighlighter {
        Rc::new(|line: &str, state| {
            let leaves = match (line, state) {
                ("```", 0) => 1,
                ("```", 1) => 0,
                (">", 0) => 2,
                ("<", 2) => 0,
                _ => state,
            };
            (Vec::new(), leaves)
        })
    }
    fn laid(cache: &Rc<LineCache>, text: &str) -> Rc<LinesLayout> {
        cache.layout(
            || Text::new(text),
            None,
            14.,
            Some(200.),
            &FontStyle::default(),
            0,
            shaper,
        )
    }
    /// What `layout` says of each line: whether it's hidden and its group
    /// before any is shaped, and its top, height and group once all are.
    #[allow(clippy::type_complexity)]
    fn said(layout: &LinesLayout) -> (Vec<(bool, Option<u32>)>, Vec<(f32, f32, Option<u32>)>) {
        let group = |i: usize| layout.group_of(i).map(|(id, _)| id);
        let lines = 0..layout.line_count();
        let before = (lines.clone())
            .map(|i| (layout.height(i) == 0., group(i)))
            .collect();
        layout.lines_between(0., 10_000.);
        let after = lines
            .map(|i| (layout.top(i), layout.height(i), group(i)))
            .collect();
        (before, after)
    }

    /// A fence typed above lines puts them in its block: their room, height
    /// and group follow, unshown ones' too, as a layout made new has them;
    /// the fence closed, or gone, puts those below it back.
    #[test]
    fn an_edit_above_lines_remeasures_those_its_state_reaches() {
        let long = "a line wide enough to wrap more than once in two hundred points";
        let (cache, _) = fenced();
        let first = laid(&cache, &format!("a\n{long}\nb\n{long}"));
        assert!(!first.grouped());
        for text in [
            format!("a\n```\n{long}\nb\n{long}"),
            format!("a\n```\n{long}\n```\nb\n{long}"),
            format!("a\n{long}\n```\nb\n{long}"),
            format!("a\n{long}\nb\n{long}"),
            // A toggle closed above them: they take no room.
            format!("a\n>\n{long}\nb\n{long}"),
            format!("a\n>\n{long}\nb\n<\n{long}"),
            // A whole block in one edit: its own lines are in it.
            format!("a\n```\n{long}\n```\nb"),
        ] {
            let edited = laid(&cache, &text);
            let fresh = laid(&fenced().0, &text);
            if fresh.grouped() {
                assert!(edited.grouped(), "{text:?}");
            }
            assert_eq!(said(&edited), said(&fresh), "{text:?}");
        }
    }

    /// A typed letter asks for its own line's room alone, and a fence for
    /// its block's, not for the text below it.
    #[test]
    fn an_edit_remeasures_no_line_its_state_does_not_reach() {
        let below: String = (0..1_000).map(|i| format!("\nline {i}")).collect();
        let (cache, asked) = fenced();
        laid(&cache, &format!("a\nb\nc\n```\nd\n```{below}"));
        asked.set(0);
        laid(&cache, &format!("ax\nb\nc\n```\nd\n```{below}"));
        assert_eq!(asked.get(), 1);
        asked.set(0);
        // Closed two lines on.
        laid(&cache, &format!("ax\n```\nb\n```\nc\n```\nd\n```{below}"));
        assert!(asked.get() < 10, "{}", asked.get());
    }

    /// A highlighter set after the lines were measured measures them again:
    /// their room may depend on its states.
    #[test]
    fn a_highlighter_set_later_remeasures_lines_with_room() {
        let (cache, _) = fenced();
        cache.set_highlighter(None);
        let text = "a\n```\ncode\n```\nb";
        assert!(!laid(&cache, text).grouped());
        cache.set_highlighter(Some(fences()));
        let layout = laid(&cache, text);
        assert!(layout.grouped());
        assert_eq!(layout.group_of(2).map(|(id, _)| id), Some(3));
    }

    /// A group an edit cut short is brought back to its lines, the listener
    /// told after; one whose widest line, wider than estimated, isn't shaped
    /// yet in the new layout is shaped first, and left as scrolled.
    #[test]
    fn a_group_past_its_lines_is_settled_and_one_not_shaped_is_not() {
        let wide: LineShaper = Rc::new(|text: &str, width| {
            // `W` lines are three times as wide as estimated.
            let size = if text.starts_with('W') { 42. } else { 14. };
            Box::new(FallbackTextLayout::new(text, size, width))
        });
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i, _: &str, _| {
            if i > 0 {
                LineInset::new(0., 0., 0.).in_group(3).unwrapped()
            } else {
                LineInset::default()
            }
        })));
        let lay = |text: &str| {
            let wide = wide.clone();
            cache.layout(
                || Text::new(text),
                None,
                14.,
                Some(200.),
                &FontStyle::default(),
                0,
                move || (wide, rich()),
            )
        };
        let told = Rc::new(RefCell::new(Vec::new()));
        let heard = told.clone();
        cache.on_group_scroll(Some(Rc::new(move |id, x| heard.borrow_mut().push((id, x)))));
        let long = format!("a\nW{}\nb", "w".repeat(29));
        let layout = lay(&long);
        layout.lines_between(0., 1_000.);
        let limit = layout.group(3).unwrap().limit();
        assert!(limit > 500., "{limit}");
        cache.set_group_scroll(3, limit);
        // An edit elsewhere: the wide line is estimated again, narrower.
        let edited = lay(&format!("ax\nW{}\nb", "w".repeat(29)));
        assert!(edited.group(3).unwrap().limit() < limit);
        assert!(!edited.settle_groups(), "shaped, it reaches as far");
        assert_eq!(edited.group_scroll(3), limit);
        let cut = lay("ax\nW\nb");
        assert!(cut.settle_groups());
        assert_eq!(cut.group_scroll(3), 0.);
        assert_eq!(*told.borrow(), [(3, limit)], "not while it is drawn");
        cache.tell_settled();
        cache.tell_settled();
        assert_eq!(*told.borrow(), [(3, limit), (3, 0.)], "once after");
    }

    /// New colours (a theme's) ask for the lines shown alone, and for no
    /// line's room, as before insets; the letter typed after asks for its
    /// own line. What lines leave is kept.
    #[test]
    fn new_colours_remeasure_nothing() {
        let cache = Rc::new(LineCache::default());
        let highlights = Rc::new(Cell::new(0));
        let (count, fences) = (highlights.clone(), fences());
        cache.set_highlighter(Some(Rc::new(move |line: &str, state| {
            count.set(count.get() + 1);
            fences(line, state)
        })));
        let asked = Rc::new(Cell::new(0));
        let count = asked.clone();
        cache.set_insets(Some(Rc::new(move |_, line: &str, state| {
            count.set(count.get() + 1);
            let mut inset = LineInset::default();
            if state == 1 && line != "```" {
                inset.scroll_group = Some(3);
            }
            inset
        })));
        let below: String = (0..1_000).map(|i| format!("\nline {i}")).collect();
        let layout = laid(&cache, &format!("a\n```{below}"));
        assert!(layout.grouped());
        highlights.set(0);
        asked.set(0);
        cache.recolor();
        let same = laid(&cache, &format!("a\n```{below}"));
        assert!(Rc::ptr_eq(&layout, &same), "the layout holds");
        for i in 0..10 {
            cache.spans(&same, i);
        }
        assert_eq!((highlights.get(), asked.get()), (10, 0));
        let typed = laid(&cache, &format!("ax\n```{below}"));
        assert_eq!(asked.get(), 1, "its own line");
        assert!(typed.grouped());
    }

    /// A fence typed above a block's worth of lines, and the group it makes,
    /// ask each line's room once.
    #[test]
    fn a_fence_asks_each_lines_room_once() {
        let below: String = (0..1_000).map(|i| format!("\nline {i}")).collect();
        let (cache, asked) = fenced();
        laid(&cache, &format!("a\n``{below}"));
        asked.set(0);
        let typed = laid(&cache, &format!("a\n```{below}"));
        assert!(typed.group(3).is_some());
        assert_eq!(asked.get(), 1_001, "the fence's line and those below");
        asked.set(0);
        let letter = laid(&cache, &format!("ax\n```{below}"));
        assert!(letter.group(3).is_some());
        assert_eq!(asked.get(), 1, "its own line, the group kept");
        let (fresh, count) = fenced();
        assert_eq!(
            said(&laid(&fresh, &format!("ax\n```{below}"))),
            said(&letter)
        );
        assert!(count.get() > 0);
    }
}
