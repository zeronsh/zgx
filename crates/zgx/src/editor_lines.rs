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
#[derive(Clone, Debug, Default, PartialEq)]
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
    /// Its font size (a heading in a note): shaped so.
    pub size: Option<f32>,
    /// Its line pitch in points (a heading's): the line takes the tallest.
    pub line_height: Option<f32>,
    /// A line under it (a link).
    pub underline: bool,
    /// In the monospaced font (inline code): shaped so.
    pub mono: bool,
    /// Room round its text inside its background, across and down (a chip): across, it takes room in the line.
    pub padding: Option<[f32; 2]>,
    /// Its background's corner radius.
    pub radius: Option<f32>,
    /// Its underline's colour, its thickness then the font's (a
    /// mention's): an underline with neither this nor an offset is one
    /// point thick in the text's colour.
    pub underline_color: Option<crate::scene::Color>,
    /// How far below the baseline its underline sits, in points.
    pub underline_offset: Option<f32>,
    /// Room before its text only, in points: where an atom's leading icon
    /// draws (`AtomStyle::icon`); it moves with the text's first word.
    pub lead: Option<f32>,
    /// Its text shown as one box `[width, height, descent]` in EM (an
    /// equation's picture, `AtomStyle::svg`): as wide as that, the line as
    /// tall as it needs, its text kept for copy and search.
    pub boxed: Option<[f32; 3]>,
    /// A picture standing for each [`INLINE_BOX_CHAR`](crate::rich_text::INLINE_BOX_CHAR) in its range, sized in ems
    /// of its text, its baseline on the line's (a formula): one atom to carets and hits, its line grown to hold it.
    /// Drawn as given, not tinted.
    pub inline_svg: Option<crate::compose::InlineSvg>,
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
    /// Its text `size` points large.
    pub const fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }
    /// Its line `line_height` points high.
    pub const fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = Some(line_height);
        self
    }
    /// A line under its text.
    pub const fn underline(mut self) -> Self {
        self.underline = true;
        self
    }
    /// Its background as a chip: `padding` across and down round its text, corners of `radius`.
    pub const fn chip(mut self, padding: [f32; 2], radius: f32) -> Self {
        self.padding = Some(padding);
        self.radius = Some(radius);
        self
    }
    /// `svg` in place of each inline box character in its range (`SpanStyle::inline_svg`).
    pub fn inline_svg(mut self, svg: crate::compose::InlineSvg) -> Self {
        self.inline_svg = Some(svg);
        self
    }
    /// Its text in the monospaced font.
    pub const fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
    fn merge(&mut self, style: &SpanStyle) {
        self.color = self.color.or(style.color);
        self.weight = self.weight.or(style.weight);
        self.italic |= style.italic;
        self.strike |= style.strike;
        self.background = self.background.or(style.background);
        self.size = self.size.or(style.size);
        self.line_height = self.line_height.or(style.line_height);
        self.underline |= style.underline;
        self.mono |= style.mono;
        self.padding = self.padding.or(style.padding);
        self.radius = self.radius.or(style.radius);
        self.underline_color = self.underline_color.or(style.underline_color);
        self.underline_offset = self.underline_offset.or(style.underline_offset);
        self.lead = self.lead.or(style.lead);
        self.boxed = self.boxed.or(style.boxed);
        if self.inline_svg.is_none() {
            self.inline_svg.clone_from(&style.inline_svg);
        }
    }
    /// Whether it only colours its text.
    fn colors_only(&self) -> bool {
        self.weight.is_none()
            && !self.italic
            && !self.strike
            && self.background.is_none()
            && self.size.is_none()
            && self.line_height.is_none()
            && !self.underline
            && !self.mono
            && self.padding.is_none()
            && self.radius.is_none()
            && self.lead.is_none()
            && self.boxed.is_none()
            && self.inline_svg.is_none()
    }
    /// Whether it changes how its text is shaped (and so its width or height).
    fn shapes(&self) -> bool {
        self.weight.is_some()
            || self.italic
            || self.size.is_some()
            || self.line_height.is_some()
            || self.mono
            || self.padding.is_some()
            || self.lead.is_some()
            || self.boxed.is_some()
            || self.inline_svg.is_some()
    }
}
/// Room around one line of a text area (`EditorHandle::set_line_insets`): a list item's indent, a heading's space
/// above. The line wraps in what the indent leaves, and its carets, hits and selection move with it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LineInset {
    /// Points before the line's text.
    pub left: f32,
    /// Points above the line.
    pub above: f32,
    /// Points below the line.
    pub below: f32,
    /// Not shown and no room taken (a closed toggle's lines): the app keeps the caret out of it.
    pub hidden: bool,
    /// The width the line wraps in, from its `left` (a table's cell); none: what the editor's width leaves.
    pub width: Option<f32>,
    /// This line and the next share a row (a table's cells, side by side): both start at the row's top, each at its
    /// own `left`, and the lines after the row start below its tallest line.
    pub joins_next: bool,
    /// This line and the next are one cell of a row (a table cell's paragraphs): the next starts under this one, at
    /// its own `left`, and the cell is as tall as its lines together.
    pub stacks_next: bool,
    /// How its rows align in the width it wraps in (a centred caption); none: as the editor's text. Carets, hits and
    /// selection follow where its text shows. An unwrapped line without its own `width` has no width to align in.
    pub align: Option<crate::text_layout::TextAlign>,
    /// Lines sharing an id scroll sideways together (a table wider than the note): their text and marks move by the
    /// group's offset (`EditorHandle::set_group_scroll`) and show only inside the group's frame, from its first
    /// line's `left` across `group_width`. Carets, hits and selection follow; nothing moves vertically.
    pub scroll_group: Option<u32>,
    /// The width a scroll group shows, read from its first line; none: what the editor's width leaves right of
    /// that line's `left`.
    pub group_width: Option<f32>,
    /// It never wraps (a code block's line, the web's `white-space: pre`): shaped once at its full width, it shows
    /// as far as its scroll group's frame and scrolls sideways with the group (`scroll_group`), which it belongs in.
    #[doc(hidden)]
    pub nowrap: bool,
}
/// A scroll group's frame set by the app (`EditorHandle::set_group_frame`): each part given overrides what its
/// lines say (a table's cells, whose borders and fills sit outside their text). All `None`: as its lines say.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroupFrame {
    /// Where the frame starts, in points from the text area's left content edge; none: its first line's `left`.
    pub left: Option<f32>,
    /// How wide it shows; none: its first line's `group_width`, else what the editor's width leaves right of `left`.
    pub width: Option<f32>,
    /// How wide its lines reach from `left`, for how far it scrolls; none: as far as its lines reach.
    pub content: Option<f32>,
}
impl LineInset {
    /// Whether the next line is in its row.
    fn links(&self) -> bool {
        self.joins_next || self.stacks_next
    }
    /// The width the line wraps in when the text wraps at `wrap`: its own, or what its indent leaves.
    fn wrap(&self, wrap: Option<f32>) -> Option<f32> {
        if self.nowrap {
            return None;
        }
        match self.width {
            Some(width) => Some(width.max(1.)),
            None if self.left > 0. => wrap.map(|w| (w - self.left).max(1.)),
            None => wrap,
        }
    }
    /// A line's height with its room: its text's `bare` height, or nothing when hidden.
    fn height(&self, bare: f32) -> f32 {
        if self.hidden {
            0.
        } else {
            bare + self.above + self.below
        }
    }
}
/// Each line's room, by its index.
pub type LineInsets = Rc<dyn Fn(usize) -> LineInset>;
/// A shape drawn beside a line of a text area (`EditorHandle::set_line_marks`): a list item's bullet or number, a
/// to-do's box, a quote's rule, the wash behind a code line. It scrolls with its line, under the text.
#[derive(Clone, Debug, PartialEq)]
pub struct LineMark {
    /// Its left edge, in points from the text area's left content edge.
    pub x: f32,
    /// Its top, in points from the top of the line's text (below the line's room above).
    pub y: f32,
    /// Its width; infinite: to the area's right edge (to its line's, where the line has a width: a cell's).
    pub width: f32,
    /// Its height; infinite: to the bottom of the line, its wrapped rows and its room below too (of its row, where
    /// the line shares one: a cell's border is as tall as the row).
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
    /// Its `x` is how far its right edge sits from the area's right edge (a code block's language); from its line's,
    /// where the line has a width.
    pub from_right: bool,
    /// Its label in the monospaced font (a code block's line numbers).
    pub mono: bool,
    /// Text drawn from its top left (an ordinal), its colour and size; a finite height is its line pitch.
    pub label: Option<(std::sync::Arc<str>, crate::scene::Color, f32)>,
    /// A picture filling it (a note's image above its caption), its corners rounded by `radius`.
    pub image: Option<MarkImage>,
    /// Where it sits across the line's width (left, centre or right; `x` then offsets it); none: at `x` (or from the
    /// right edge with `from_right`).
    pub align: Option<crate::text_layout::TextAlign>,
    /// A picture drawn as given in its box, in place of its fill and border.
    pub svg: Option<std::sync::Arc<crate::svg::SvgData>>,
    /// Per-side borders and per-corner radii (a table's outer cells), over `radius` and `border_width`.
    pub decoration: Option<std::sync::Arc<crate::decoration::Decoration>>,
    /// It stays where it is as its line's scroll group scrolls sideways (a code block's surface and line numbers):
    /// only the text, and the marks not pinned, move.
    #[doc(hidden)]
    pub pinned: bool,
    /// Drawn over the text, not under it (a code block's actions, which cover the code beneath): its fill's alpha
    /// is how much it covers.
    #[doc(hidden)]
    pub over_text: bool,
    /// Turned this far clockwise about its centre, in degrees (a toggle's
    /// chevron, 90 open): a change eases there over `motion.duration.fast`,
    /// easing out, unless the system reduces motion.
    #[doc(hidden)]
    pub rotate: f32,
}
/// A line mark's picture (`LineMark::image`): decoded pixels and how they fit the mark's box. A file is decoded by
/// the app, as an image view's is (`zgx_gpu::assets::decode_image_kept`), and kept in its `ImageCache`.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkImage {
    /// The pixels.
    pub image: std::sync::Arc<crate::image::ImageData>,
    /// How they fit the mark's box ([`ObjectFit::Cover`](crate::style::ObjectFit::Cover): filling it, cropped).
    pub fit: crate::style::ObjectFit,
}
impl MarkImage {
    /// `image` filling the mark, cropped to its box.
    pub fn cover(image: std::sync::Arc<crate::image::ImageData>) -> Self {
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
            from_right: false,
            mono: false,
            fill: crate::scene::Color(0, 0, 0, 0),
            radius: 0.,
            border_color: crate::scene::Color(0, 0, 0, 0),
            border_width: 0.,
            label: None,
            image: None,
            align: None,
            svg: None,
            decoration: None,
            pinned: false,
            over_text: false,
            rotate: 0.,
        }
    }
}
/// Each line's marks, by its index.
pub type LineMarks = Rc<dyn Fn(usize) -> Vec<LineMark>>;
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
            (start..end, style.clone())
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
            runs.push((range.clone(), Some(style.clone())));
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
    // A box is one run, whatever else styles parts of it.
    styled.dedup_by(|next, run| match (&run.1, &next.1) {
        (Some(a), Some(b))
            if a.boxed.is_some() && a.boxed == b.boxed && run.0.end == next.0.start =>
        {
            run.0.end = next.0.end;
            true
        }
        _ => false,
    });
    let mut boxes: Vec<crate::rich_text::InlineBox> = styled
        .iter()
        .filter_map(|(range, style)| {
            let [w, h, d] = style.as_ref()?.boxed?;
            Some(crate::rich_text::InlineBox {
                width: w * size,
                height: h * size,
                baseline: (h - d) * size,
                ..crate::rich_text::InlineBox::new(range.start)
            })
        })
        .collect();
    let runs: Vec<_> = styled
        .into_iter()
        .flat_map(|(range, style)| boxed_runs(line, range, style))
        .map(|(range, style)| {
            let style = style.unwrap_or_default();
            if let Some(svg) = &style.inline_svg
                && &line[range.clone()] == crate::rich_text::INLINE_BOX_TEXT
            {
                let em = style.size.unwrap_or(size);
                boxes.push(crate::rich_text::InlineBox {
                    width: svg.width * em,
                    height: (svg.height + svg.depth) * em,
                    baseline: svg.height * em,
                    ..crate::rich_text::InlineBox::new(range.start)
                });
            }
            let mut font = font.clone();
            if let Some(weight) = style.weight {
                font.weight = weight.clamp(1, 1000);
            }
            font.italic |= style.italic;
            if style.mono {
                font.family = crate::text_layout::FontFamily::Monospace;
            }
            if let Some(line_height) = style.line_height {
                font.line_height = crate::text_layout::LineHeight::px(line_height);
            }
            // A lead is room before only: half of it across (its run is one
            // grapheme, so as much in all), negative down (`TextRun::padding`).
            let padding = match (style.padding, style.lead) {
                (Some(padding), _) => padding,
                (None, Some(lead)) if lead > 0. => [lead / 2., -1.],
                _ => [0.; 2],
            };
            crate::rich_text::TextRun {
                range,
                font,
                font_size: style.size.unwrap_or(size),
                color: style.color.unwrap_or(color),
                background: style.background,
                padding,
                radius: style.radius.unwrap_or_default(),
                underline: style.underline.then(|| underline(&style)),
                strikethrough: style.strike.then(|| crate::rich_text::Decoration::new(1.)),
                ..Default::default()
            }
        })
        .collect();
    // An atom's box (`boxed`) and a span's pictures (`inline_svg`), in the
    // text's order, as `with_inline_boxes` takes them.
    boxes.sort_by_key(|b| b.at);
    boxes.dedup_by_key(|b| b.at);
    let rich = crate::rich_text::RichText::new(line, runs).ok()?;
    if boxes.is_empty() {
        Some(rich)
    } else {
        rich.with_inline_boxes(boxes).ok()
    }
}
/// `style`'s underline: one point thick in its text's colour, or the
/// font's thickness in its own colour and place when it says them.
fn underline(style: &SpanStyle) -> crate::rich_text::Decoration {
    if style.underline_color.is_none() && style.underline_offset.is_none() {
        return crate::rich_text::Decoration::new(1.);
    }
    let mut line = crate::rich_text::Decoration::default();
    if let Some(color) = style.underline_color {
        line = line.color(color);
    }
    if let Some(offset) = style.underline_offset {
        line = line.offset(offset);
    }
    line
}
/// A run whose style has an inline SVG, cut so each inline box character in it is a run of its own.
fn boxed_runs(
    line: &str,
    range: Range<usize>,
    style: Option<SpanStyle>,
) -> Vec<(Range<usize>, Option<SpanStyle>)> {
    if style.as_ref().is_none_or(|s| s.inline_svg.is_none())
        || !line[range.clone()].contains(crate::rich_text::INLINE_BOX_CHAR)
    {
        return vec![(range, style)];
    }
    let mut out = Vec::new();
    let mut at = range.start;
    for (i, _) in line[range.clone()].match_indices(crate::rich_text::INLINE_BOX_CHAR) {
        let start = range.start + i;
        let end = start + crate::rich_text::INLINE_BOX_TEXT.len();
        if start > at {
            out.push((at..start, style.clone()));
        }
        out.push((start..end, style.clone()));
        at = end;
    }
    if at < range.end {
        out.push((at..range.end, style));
    }
    out
}
/// Each inline SVG of a line (`SpanStyle::inline_svg`): its box character's byte offset, picture and text size.
pub(crate) fn inline_svgs(
    line: &str,
    spans: &LineSpans,
) -> Vec<(usize, crate::compose::InlineSvg, Option<f32>)> {
    let mut out: Vec<(usize, crate::compose::InlineSvg, Option<f32>)> = Vec::new();
    for (range, style) in spans {
        let Some(svg) = &style.inline_svg else {
            continue;
        };
        let Some(text) = line.get(range.clone()) else {
            continue;
        };
        for (i, _) in text.match_indices(crate::rich_text::INLINE_BOX_CHAR) {
            let at = range.start + i;
            if !out.iter().any(|(seen, ..)| *seen == at) {
                out.push((at, svg.clone(), style.size));
            }
        }
    }
    out
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
                    s.size.map_or(0, f32::to_bits),
                    s.line_height.map_or(0, f32::to_bits),
                    s.mono,
                    s.lead.map_or(0, f32::to_bits)
                        ^ s.boxed.map_or(0, |[w, h, d]| {
                            w.to_bits().rotate_left(7) ^ h.to_bits().rotate_left(17) ^ d.to_bits()
                        }),
                    s.inline_svg.as_ref().map_or([0; 3], |svg| {
                        [
                            svg.width.to_bits(),
                            svg.height.to_bits(),
                            svg.depth.to_bits(),
                        ]
                    }),
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
    /// Spans its atoms' styles give (`AtomStyle`), over the whole text by
    /// start: each line takes those it holds before the highlighter's.
    atom_spans: RefCell<Option<Rc<LineSpans>>>,
    /// Whether `atom_spans` holds any: asked first, so a line of an editor
    /// without atoms reads no `RefCell` for them.
    has_atom_spans: Cell<bool>,
    /// The highlighter has given a span that shapes its line (a weight,
    /// italics): only then is a line asked for its spans as it is shaped,
    /// so one that only colours costs what it did. `restyled`: it just
    /// became so (`take_restyled`).
    shapes: Cell<bool>,
    restyled: Cell<bool>,
    /// What each line leaves the next, from the first; as far as known.
    states: RefCell<Vec<u32>>,
    /// What the lines below the last edit left before it: once the lines
    /// it changed leave what they did, these hold again (the edit's
    /// colours end there).
    tail: RefCell<Option<Tail>>,
    /// Highlighted lines, by their text and what the line before left.
    colored: RefCell<HashMap<(Box<str>, u32), Colored>>,
    insets: RefCell<Option<LineInsets>>,
    /// Each scroll group's sideways offset (`LineInset::scroll_group`), by its id: read as carets, hits and
    /// drawing are, so moving one lays nothing out again.
    group_scroll: RefCell<HashMap<u32, f32>>,
    /// The scroll group that moved last: the one a text area's group scrollbar shows (`View::group_scrollbar`).
    last_scrolled: Cell<Option<u32>>,
    /// Told of each group's new offset (`EditorHandle::on_group_scroll`).
    on_group_scroll: RefCell<Option<GroupListener>>,
    /// Frames the app set for groups (`EditorHandle::set_group_frame`), by id: read as each frame is asked for,
    /// so setting one lays nothing out again.
    group_frames: RefCell<HashMap<u32, GroupFrame>>,
}
/// What a scroll group's lines say of it (`LinesLayout::groups`), before the app's frame.
#[derive(Clone, Copy, Debug)]
struct GroupLines {
    /// Its first line's `left` and `group_width`.
    left: f32,
    width: Option<f32>,
    /// How far right its lines reach, from the area's left content edge.
    right: f32,
    /// Its first and last lines shown (for its scrollbar, at its bottom edge).
    first: usize,
    last: usize,
}
/// Told of a scroll group's id and new offset (`EditorHandle::on_group_scroll`).
pub(crate) type GroupListener = Rc<dyn Fn(u32, f32)>;
/// A scroll group's frame and extent (`LinesLayout::group`), in points from the area's left content edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Group {
    /// Where it shows from: its first line's `left`.
    pub(crate) left: f32,
    /// How wide it shows.
    pub(crate) view: f32,
    /// How wide its lines are from `left`.
    pub(crate) content: f32,
    /// Its first and last lines shown.
    pub(crate) first: usize,
    pub(crate) last: usize,
}
impl Group {
    /// The furthest it scrolls: its content past its view.
    pub(crate) fn limit(&self) -> f32 {
        (self.content - self.view).max(0.)
    }
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
type ShapeKey = Box<[(u32, u32, u16, bool, u32, u32, bool, u32, [u32; 3])]>;
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
                let layout = LinesLayout::edited(old, text, prefix, suffix);
                self.edited(Some((old, prefix, suffix)), &layout);
                layout
            }
            None => {
                let layout = LinesLayout::new(text, wrap, shape(), self.clone());
                self.edited(None, &layout);
                layout
            }
        });
        *self.last.borrow_mut() = Some(layout.clone());
        self.last_revision.set(text_revision);
        layout
    }
    /// The text went from `old` to `new`'s: what the lines above the edit
    /// leave holds, and what those below it left is kept for when the
    /// edited lines leave what they did.
    fn edited(&self, old: Option<(&LinesLayout, usize, usize)>, new: &LinesLayout) {
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
            new.line_of(new.text.len() - suffix),
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
    /// Style lines with `highlighter` (none: the editor's style).
    pub(crate) fn set_insets(&self, insets: Option<LineInsets>) {
        *self.insets.borrow_mut() = insets;
        self.shaped.borrow_mut().clear();
        self.styled.borrow_mut().clear();
        self.heights.borrow_mut().clear();
        self.forget_last();
    }
    /// Scroll group `group`'s offset (0 until set).
    pub(crate) fn group_scroll(&self, group: u32) -> f32 {
        self.group_scroll
            .borrow()
            .get(&group)
            .copied()
            .unwrap_or(0.)
    }
    /// The scroll group that moved last, if one has.
    pub(crate) fn last_scrolled_group(&self) -> Option<u32> {
        self.last_scrolled.get()
    }
    /// Move scroll group `group` to `x` (clamped by the caller); whether it moved. Its listener hears of it.
    pub(crate) fn set_group_scroll(&self, group: u32, x: f32) -> bool {
        let x = if x.is_finite() { x.max(0.) } else { 0. };
        let previous = self.group_scroll.borrow_mut().insert(group, x);
        if previous.unwrap_or(0.) == x {
            return false;
        }
        self.last_scrolled.set(Some(group));
        let listener = self.on_group_scroll.borrow().clone();
        if let Some(listener) = listener {
            listener(group, x);
        }
        true
    }
    /// Set scroll group `group`'s frame (an all-`None` one: as its lines say).
    pub(crate) fn set_group_frame(&self, group: u32, frame: GroupFrame) {
        let mut frames = self.group_frames.borrow_mut();
        if frame == GroupFrame::default() {
            frames.remove(&group);
        } else {
            frames.insert(group, frame);
        }
    }
    pub(crate) fn on_group_scroll(&self, listener: Option<GroupListener>) {
        *self.on_group_scroll.borrow_mut() = listener;
    }
    pub(crate) fn set_highlighter(&self, highlighter: Option<SpanHighlighter>) {
        *self.highlighter.borrow_mut() = highlighter;
        self.shapes.set(false);
        self.states.borrow_mut().clear();
        *self.tail.borrow_mut() = None;
        self.colored.borrow_mut().clear();
    }
    /// Ask the highlighter for every line anew.
    pub(crate) fn rehighlight(&self) {
        let highlighter = self.highlighter.borrow().clone();
        self.set_highlighter(highlighter);
    }
    pub(crate) fn highlighted(&self) -> bool {
        self.highlighter.borrow().is_some() || self.has_atom_spans.get()
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
        self.has_atom_spans.set(spans.is_some());
        *self.atom_spans.borrow_mut() = spans.map(Rc::new);
    }
    /// Whether a line's spans shape it now that did not before: what was
    /// laid out without them is laid out again.
    pub(crate) fn take_restyled(&self) -> bool {
        self.restyled.replace(false)
    }
    /// Line `i`'s spans, if it has a highlighter: what the lines above
    /// it leave found first (each once, until one changes).
    pub(crate) fn spans(&self, layout: &LinesLayout, i: usize) -> Option<Rc<LineSpans>> {
        if self.has_atom_spans.get() {
            return self.spans_with_atoms(layout, i);
        }
        self.highlighted_spans(layout, i)
    }
    /// `spans` of an editor whose atoms have spans: its atoms' before its
    /// highlighter's. Rare, out of line.
    #[cold]
    #[inline(never)]
    fn spans_with_atoms(&self, layout: &LinesLayout, i: usize) -> Option<Rc<LineSpans>> {
        let atoms = self.atom_spans.borrow().clone();
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
            let (_, leaves) = self.highlight(&highlight, &layout.line_text(known), before);
            self.push_state(&mut self.states.borrow_mut(), leaves);
        }
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
    fn styled(
        &self,
        layout: &LinesLayout,
        i: usize,
        text: &str,
        wrap: Option<f32>,
        key_text: &str,
    ) -> Option<Rc<dyn TextLayout>> {
        if !self.shapes.get() {
            return None;
        }
        let spans = self.spans(layout, i)?;
        let key = (Box::<str>::from(key_text), shape_key(&spans)?);
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
        let rich = styled_line(text, &spans, crate::scene::Color(0, 0, 0, 255), size, &font)?;
        let shaped: Rc<dyn TextLayout> = (layout.rich)(&rich, wrap).into();
        let mut styled = self.styled.borrow_mut();
        evict(&mut styled);
        styled.insert(key, (shaped.clone(), tick));
        Some(shaped)
    }
    /// Line `i` shaped with its rows aligned `align` in `wrap` (`LineInset::align`): its spans' runs if they shape it,
    /// else its text as one run, in the editor's font aligned so (kept apart from the same text unaligned).
    fn aligned(
        &self,
        layout: &LinesLayout,
        i: usize,
        text: &str,
        wrap: Option<f32>,
        key_text: &str,
        align: crate::text_layout::TextAlign,
    ) -> Rc<dyn TextLayout> {
        let spans = if self.shapes.get() {
            self.spans(layout, i)
        } else {
            None
        };
        let key: Box<str> = format!(
            "{align:?}\u{2}{:?}\u{2}{key_text}",
            spans.as_deref().and_then(shape_key)
        )
        .into();
        let tick = self.clock.get() + 1;
        self.clock.set(tick);
        if let Some((shaped, used)) = self.shaped.borrow_mut().get_mut(&key) {
            *used = tick;
            return shaped.clone();
        }
        let (size, mut font) = {
            let shaping = self.shaping.borrow();
            let shaping = shaping.as_ref().expect("a layout's shaping");
            (f32::from_bits(shaping.size), shaping.font.clone())
        };
        font.align = align;
        let color = crate::scene::Color(0, 0, 0, 255);
        let rich = spans
            .as_deref()
            .filter(|spans| spans.iter().any(|(_, style)| style.shapes()))
            .and_then(|spans| styled_line(text, spans, color, size, &font))
            .or_else(|| {
                crate::rich_text::RichText::new(
                    text,
                    vec![crate::rich_text::TextRun {
                        range: 0..text.len(),
                        font: font.clone(),
                        font_size: size,
                        color,
                        ..Default::default()
                    }],
                )
                .ok()
            })
            .expect("a line's text as one run");
        let shaped: Rc<dyn TextLayout> = (layout.rich)(&rich, wrap).into();
        let mut cache = self.shaped.borrow_mut();
        evict(&mut cache);
        cache.insert(key, (shaped.clone(), tick));
        shaped
    }
    /// Line `text`, shaped (from the cache if it is there).
    fn line(
        &self,
        text: &str,
        shape: &LineShaper,
        wrap: Option<f32>,
        key: &str,
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
    /// What each line adds to the tops of those below it: its height, but in a row (`LineInset::joins_next`)
    /// nothing for every line but the last, which adds the row's.
    each: Vec<f32>,
    /// Each line's own height, where any share a row (empty where none does: `each` is it).
    own: Vec<f32>,
}
impl Heights {
    /// Of lines `own` high, those at `joins` (in order) sharing a row with the next, those at `stacks` (in order,
    /// among `joins`) its cell.
    fn with_rows(own: Vec<f32>, joins: &[usize], stacks: &[usize]) -> Self {
        if joins.is_empty() {
            return Self::new(own);
        }
        let mut each = own.clone();
        let mut k = 0;
        while k < joins.len() {
            let start = joins[k];
            while k + 1 < joins.len() && joins[k + 1] == joins[k] + 1 {
                k += 1;
            }
            let last = (joins[k] + 1).min(own.len() - 1);
            k += 1;
            let tallest = tallest_cell(&own, start..last + 1, |j| stacks.binary_search(&j).is_ok());
            each[start..last].fill(0.);
            each[last] = tallest;
        }
        let mut heights = Self::new(each);
        heights.own = own;
        heights
    }
    fn new(each: Vec<f32>) -> Self {
        let mut tree: Vec<f64> = each.iter().map(|h| *h as f64).collect();
        for i in 0..tree.len() {
            let parent = i | (i + 1);
            if parent < tree.len() {
                tree[parent] += tree[i];
            }
        }
        Self {
            tree,
            each,
            own: Vec::new(),
        }
    }
    /// Line `i`'s own height.
    fn own(&self, i: usize) -> f32 {
        if self.own.is_empty() {
            self.each[i]
        } else {
            self.own[i]
        }
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

/// The height of the row of lines `row`, `own` high: its tallest cell's, a cell's lines (`stacks`: the line and the
/// next are one cell) together.
fn tallest_cell(own: &[f32], row: Range<usize>, stacks: impl Fn(usize) -> bool) -> f32 {
    let (mut tallest, mut cell) = (0f32, 0f32);
    let last = row.end - 1;
    for j in row {
        cell += own[j];
        if j == last || !stacks(j) {
            tallest = tallest.max(cell);
            cell = 0.;
        }
    }
    tallest
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
    /// Lines across chunks, joined once (a long line's every draw would
    /// copy it): a few, by number.
    joined: RefCell<HashMap<usize, Arc<str>>>,
    /// Whether any line is in a scroll group (`LineInset::scroll_group`): only then are the groups found.
    grouped: bool,
    /// Each scroll group's frame and extent, found once asked for.
    groups: RefCell<Option<Rc<HashMap<u32, GroupLines>>>>,
    /// How far each scroll group's unwrapped lines reach (`LineInset::nowrap`), as far as they are shaped: past
    /// its estimate, a group scrolls that far.
    group_reach: RefCell<HashMap<u32, f32>>,
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
            cache,
            joined: RefCell::default(),
            grouped: false,
            groups: RefCell::default(),
            group_reach: RefCell::default(),
        };
        // every line's own height, unwrapped too: a heading's taller pitch and a line's room push the lines below
        {
            let width = wrap.unwrap_or(f32::INFINITY);
            let known = layout.cache.heights.borrow();
            let mut joins = Vec::new();
            let mut stacks = Vec::new();
            let mut grouped = false;
            // Asked once, not each line's: without any, each line's is the default.
            let insets = layout.cache.insets.borrow().clone();
            let each = (0..layout.line_count())
                .map(|i| {
                    let inset = match &insets {
                        Some(insets) => {
                            let inset = insets(i);
                            grouped |= inset.scroll_group.is_some();
                            inset
                        }
                        None => LineInset::default(),
                    };
                    if inset.joins_next || inset.stacks_next {
                        joins.push(i);
                    }
                    if inset.stacks_next {
                        stacks.push(i);
                    }
                    layout.measured(i, &inset, &known, width)
                })
                .collect();
            drop(known);
            layout.grouped = grouped;
            layout.heights = Some(RefCell::new(Heights::with_rows(each, &joins, &stacks)));
        }
        layout
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
            joined: RefCell::default(),
            grouped: old.grouped,
            groups: RefCell::default(),
            group_reach: RefCell::default(),
        };
        // The edited lines' lengths count for the longest.
        for i in kept - 1..first_moved.min(layout.line_count()) {
            layout.longest = layout.longest.max(layout.line_range(i).len());
        }
        if let Some(heights) = &old.heights {
            let width = old.wrap.unwrap_or(f32::INFINITY);
            let old = heights.borrow();
            let known = layout.cache.heights.borrow();
            let count = layout.line_count();
            let mut joined = !old.own.is_empty();
            let mut measured = Vec::with_capacity(first_moved + 1 - kept);
            for i in kept - 1..first_moved {
                let inset = layout.inset(i);
                joined |= inset.joins_next || inset.stacks_next;
                layout.grouped |= inset.scroll_group.is_some();
                measured.push(layout.measured(i, &inset, &known, width));
            }
            drop(known);
            let spliced = |from: &[f32]| {
                let mut all = Vec::with_capacity(count);
                all.extend_from_slice(&from[..kept - 1]);
                all.extend_from_slice(&measured);
                all.extend_from_slice(&from[moved..]);
                all
            };
            let mut each = spliced(&old.each);
            let new = if joined {
                // The rows the edit touches take their heights again; the rest keep theirs.
                let own = if old.own.is_empty() {
                    each.clone()
                } else {
                    spliced(&old.own)
                };
                let joins = |i: usize| i + 1 < count && layout.inset(i).links();
                let mut start = kept - 1;
                while start > 0 && joins(start - 1) {
                    start -= 1;
                }
                let mut end = first_moved.max(kept).min(count);
                while end < count && joins(end - 1) {
                    end += 1;
                }
                let mut row = start;
                while row < end {
                    let mut last = row;
                    while joins(last) {
                        last += 1;
                    }
                    let tallest =
                        tallest_cell(&own, row..last + 1, |j| layout.inset(j).stacks_next);
                    each[row..last].fill(0.);
                    each[last] = tallest;
                    row = last + 1;
                }
                let mut new = Heights::new(each);
                new.own = own;
                new
            } else {
                Heights::new(each)
            };
            drop(old);
            layout.heights = Some(RefCell::new(new));
        }
        layout
    }
    /// Line `i`'s height with its room before it is shaped: the height its text had (`known`), else an estimate.
    fn measured(
        &self,
        i: usize,
        inset: &LineInset,
        known: &HashMap<Box<str>, f32>,
        width: f32,
    ) -> f32 {
        if inset.hidden {
            return 0.;
        }
        let line = self.line_text(i);
        // a cell wraps in its own width: its text's height elsewhere is not its; a line that never wraps is a row
        let bare = match inset.width {
            _ if inset.nowrap => self.row,
            Some(own) => self.estimate(&line, own),
            None => known
                .get(&*line)
                .copied()
                .unwrap_or_else(|| self.estimate(&line, width)),
        };
        inset.height(bare)
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
            Some(heights) => {
                let row_top = heights.borrow().above(i) as f32;
                row_top + self.cell_offset(i)
            }
            None => i as f32 * self.row,
        }
    }
    /// How far line `i` starts below its row's top: the heights of its cell's lines above it.
    fn cell_offset(&self, i: usize) -> f32 {
        if !self.has_rows() {
            return 0.;
        }
        let mut below = 0.;
        let mut j = i;
        while j > 0 && self.inset(j - 1).stacks_next {
            j -= 1;
            below += self.height(j);
        }
        below
    }
    /// Line `i`'s height (as shaped, or estimated if it is not yet): its own, in a row too.
    pub(crate) fn height(&self, i: usize) -> f32 {
        match &self.heights {
            Some(heights) => heights.borrow().own(i),
            None => self.row,
        }
    }
    /// Whether any lines share a row (`LineInset::joins_next`): only then is a line asked which row it is in.
    fn has_rows(&self) -> bool {
        self.heights
            .as_ref()
            .is_some_and(|heights| !heights.borrow().own.is_empty())
    }
    /// The lines of line `i`'s row: itself, or the run of lines joined to it.
    pub(crate) fn row_of(&self, i: usize) -> Range<usize> {
        let (mut start, mut end) = (i, i + 1);
        if self.has_rows() {
            let count = self.line_count();
            while start > 0 && self.inset(start - 1).links() {
                start -= 1;
            }
            while end < count && self.inset(end - 1).links() {
                end += 1;
            }
        }
        start..end
    }
    /// The height of line `i`'s row: its tallest line's (the line's own, alone).
    pub(crate) fn row_height(&self, i: usize) -> f32 {
        if !self.has_rows() {
            return self.height(i);
        }
        let last = self.row_of(i).end - 1;
        self.heights
            .as_ref()
            .map_or(self.row, |h| h.borrow().each[last])
    }
    /// The line at `y` down the layout: of a row, the last.
    fn last_line_at(&self, y: f32) -> usize {
        let y = y.max(0.);
        match &self.heights {
            Some(heights) => heights.borrow().line_at(y as f64),
            None => ((y / self.row) as usize).min(self.line_count() - 1),
        }
    }
    /// The line at `y` down the layout (of a row, the first).
    pub(crate) fn line_at(&self, y: f32) -> usize {
        let i = self.last_line_at(y);
        if self.has_rows() {
            self.row_of(i).start
        } else {
            i
        }
    }
    /// The line at `x`, `y`: in a row, the one whose room holds `x` (the first before them all, the last after).
    /// Whether it is a cell of a row.
    fn line_at_point(&self, x: f32, y: f32) -> (usize, bool) {
        let i = self.last_line_at(y);
        if !self.has_rows() {
            return (i, false);
        }
        let row = self.row_of(i);
        if row.len() == 1 {
            return (i, false);
        }
        let x = self.within_group(row.start, x);
        let mut found = None;
        for j in row {
            let inset = self.inset(j);
            if inset.hidden {
                continue;
            }
            if found.is_none() || self.left(&inset) <= x {
                found = Some(j);
            }
        }
        let Some(mut found) = found else {
            return (i, true);
        };
        // of a cell's stacked lines, the one at `y` (the last below them)
        while found > 0 && self.inset(found - 1).stacks_next && self.top(found) > y {
            found -= 1;
        }
        (found, true)
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
    /// Line `i`, shaped.
    /// The width line `i` is drawn in when the text wraps at `wrap`: what its indent leaves, as it was wrapped.
    #[cfg(test)]
    pub(crate) fn draw_width(&self, i: usize, wrap: Option<f32>) -> Option<f32> {
        Self::draw_width_in(&self.inset(i), wrap)
    }
    /// `draw_width`, for a line whose room is already asked for.
    pub(crate) fn draw_width_in(inset: &LineInset, wrap: Option<f32>) -> Option<f32> {
        if inset.nowrap {
            return None;
        }
        inset
            .width
            .map(|w| w.max(1.))
            .or(wrap.map(|w| (w - inset.left).max(1.)))
    }
    /// How line `i`'s rows align, where it has a width to align in (`LineInset::align`), given its room `inset`:
    /// shaped so for its carets and hits (`line`), and drawn so.
    pub(crate) fn align_in(
        &self,
        i: usize,
        inset: &LineInset,
    ) -> Option<crate::text_layout::TextAlign> {
        Self::aligned(inset, inset.wrap(self.wrap), &self.line_text(i))
    }
    /// `align`, for a line whose room is already asked for: each asks it once.
    fn aligned(
        inset: &LineInset,
        wrap: Option<f32>,
        text: &str,
    ) -> Option<crate::text_layout::TextAlign> {
        let align = inset.align?;
        (wrap.is_some() && !text.is_empty()).then_some(align)
    }
    /// Line `i`'s room (`LineCache::set_insets`).
    pub(crate) fn inset(&self, i: usize) -> LineInset {
        self.cache
            .insets
            .borrow()
            .as_ref()
            .map_or_else(LineInset::default, |insets| insets(i))
    }
    pub(crate) fn line(&self, i: usize) -> Rc<dyn TextLayout> {
        let text = self.line_text(i);
        let inset = self.inset(i);
        // an indented line wraps narrower: kept apart from the same text unindented
        let wrap = inset.wrap(self.wrap);
        let key = if inset.nowrap {
            // unwrapped: kept apart from the same text wrapped
            std::borrow::Cow::Owned(format!("{}\u{2}{}", inset.left.to_bits(), &*text))
        } else if let Some(width) = inset.width {
            std::borrow::Cow::Owned(format!("{}\u{1}{}", width.to_bits(), &*text))
        } else if inset.left > 0. {
            std::borrow::Cow::Owned(format!("{}\u{0}{}", inset.left.to_bits(), &*text))
        } else {
            std::borrow::Cow::Borrowed(&*text)
        };
        // aligned in a width: its rows placed there as the text layout aligns them, carets and hits with them
        let aligned = Self::aligned(&inset, wrap, &text);
        let layout = match aligned {
            Some(align) => self.cache.aligned(self, i, &text, wrap, &key, align),
            None => match self.cache.styled(self, i, &text, wrap, &key) {
                Some(layout) => layout,
                None => self.cache.line(&text, &self.shape, wrap, &key),
            },
        };
        let (width, height) = layout.size();
        if width > self.widest.get() {
            self.widest.set(width);
        }
        if inset.nowrap
            && let Some(group) = inset.scroll_group
        {
            let mut reach = self.group_reach.borrow_mut();
            let far = reach.entry(group).or_insert(0.);
            *far = far.max(inset.left + width);
        }
        if let Some(heights) = &self.heights {
            // cached without its room: the same text elsewhere (or once its block changes) has its own
            let bare = height.max(self.row);
            let height = inset.height(bare);
            if heights.borrow().own.is_empty() {
                if heights.borrow().each[i] != height {
                    heights.borrow_mut().set(i, height);
                    self.cache.heights.borrow_mut().insert((*text).into(), bare);
                }
            } else if heights.borrow().own[i] != height {
                // its row is as tall as its tallest line now
                let row = self.row_of(i);
                let mut heights = heights.borrow_mut();
                heights.own[i] = height;
                let tallest =
                    tallest_cell(&heights.own, row.clone(), |j| self.inset(j).stacks_next);
                heights.set(row.end - 1, tallest);
                if inset.width.is_none() {
                    self.cache.heights.borrow_mut().insert((*text).into(), bare);
                }
            }
        }
        layout
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
        let inset = self.inset(i);
        Rect::new(
            rect.x + self.left(&inset),
            rect.y + self.top(i) + inset.above,
            rect.width,
            rect.height,
        )
    }
    /// Where a line with room `inset` starts across: its `left`, less its scroll group's offset.
    #[inline]
    fn left(&self, inset: &LineInset) -> f32 {
        match inset.scroll_group {
            Some(group) => inset.left - self.cache.group_scroll(group),
            None => inset.left,
        }
    }
    /// Whether any line is in a scroll group.
    #[inline]
    pub(crate) fn grouped(&self) -> bool {
        self.grouped
    }
    /// The clip that keeps a box from `x` across `width` (where it shows, as line `i`'s) inside line `i`'s scroll
    /// group frame shown from `shown_left`: how much to cut from its left and right. None outside a group.
    pub(crate) fn group_clip(
        &self,
        i: usize,
        shown_left: f32,
        x: f32,
        width: f32,
    ) -> Option<(f32, f32)> {
        let (_, group) = self.group_of(i)?;
        let width = width.max(0.);
        let left = (shown_left - x).clamp(0., width);
        let right = (x + width - (shown_left + group.view)).clamp(0., width - left);
        Some((left, right))
    }
    /// Line `i`'s scroll group offset (0 outside one).
    pub(crate) fn group_offset(&self, inset: &LineInset) -> f32 {
        inset
            .scroll_group
            .map_or(0., |group| self.cache.group_scroll(group))
    }
    /// Each scroll group's frame and extent; none without groups.
    fn groups(&self) -> Option<Rc<HashMap<u32, GroupLines>>> {
        if !self.grouped {
            return None;
        }
        if let Some(groups) = &*self.groups.borrow() {
            return Some(groups.clone());
        }
        let mut groups: HashMap<u32, GroupLines> = HashMap::new();
        for i in 0..self.line_count() {
            let inset = self.inset(i);
            let Some(id) = inset.scroll_group else {
                continue;
            };
            let group = groups.entry(id).or_insert(GroupLines {
                left: inset.left,
                width: inset.group_width,
                right: inset.left,
                first: i,
                last: i,
            });
            if inset.hidden {
                continue;
            }
            group.last = i;
            let width = match inset.width.or(self.wrap.map(|w| w - inset.left)) {
                Some(width) if !inset.nowrap => width.max(1.),
                _ => self.line_range(i).len() as f32 * self.advance,
            };
            group.right = group.right.max(inset.left + width);
        }
        let groups = Rc::new(groups);
        *self.groups.borrow_mut() = Some(groups.clone());
        Some(groups)
    }
    /// Scroll group `id`'s frame and extent, if a line is in it.
    pub(crate) fn group(&self, id: u32) -> Option<Group> {
        let lines = *self.groups()?.get(&id)?;
        let frame = self
            .cache
            .group_frames
            .borrow()
            .get(&id)
            .copied()
            .unwrap_or_default();
        let left = frame.left.unwrap_or(lines.left);
        let view = frame
            .width
            .or(lines.width)
            .or(self.wrap.map(|w| w - left))
            .unwrap_or(f32::INFINITY)
            .max(1.);
        let reach = self.group_reach.borrow().get(&id).copied().unwrap_or(0.);
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
    /// `x`, inside line `i`'s scroll group frame when it has one: what is clipped is not hit.
    fn within_group(&self, i: usize, x: f32) -> f32 {
        match self.group_of(i) {
            Some((_, group)) => {
                x.clamp(group.left, (group.left + group.view - 0.5).max(group.left))
            }
            None => x,
        }
    }
    /// The offset scroll group `id` needs for `rect` (where it shows now) to show inside its frame, if it moves.
    pub(crate) fn reveal_in_group(&self, i: usize, rect: Rect) -> Option<(u32, f32)> {
        let (id, group) = self.group_of(i)?;
        let now = self.cache.group_scroll(id);
        let right = group.left + group.view;
        let wanted = if rect.x < group.left {
            now - (group.left - rect.x)
        } else if rect.x + rect.width > right {
            now + (rect.x + rect.width - right)
        } else {
            return None;
        };
        let wanted = wanted.clamp(0., group.limit());
        (wanted != now).then_some((id, wanted))
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
    /// Where the inline boxes on the lines between `top` and `bottom` are laid
    /// out (an atom shown as a picture): each one's byte offset in the text,
    /// and its left edge and top in the area's points.
    pub(crate) fn inline_boxes_between(&self, top: f32, bottom: f32) -> Vec<(usize, f32, f32)> {
        let mut boxes = Vec::new();
        for i in self.lines_between(top, bottom) {
            let start = self.line_range(i).start;
            for (at, x, y) in self.line(i).inline_boxes() {
                let shown = self.shift(i, Rect::new(x, y, 0., 0.));
                boxes.push((start + at, shown.x, shown.y));
            }
        }
        boxes
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
                    let at = rects.len();
                    rects.push(self.shift(i, sliver));
                    self.clip_to_group(i, &mut rects, at);
                }
                continue;
            }
            let layout = self.line(i);
            let at = rects.len();
            rects.extend(
                layout
                    .selection(a - line.start..b - line.start)
                    .into_iter()
                    .map(|rect| self.shift(i, rect)),
            );
            self.clip_to_group(i, &mut rects, at);
        }
        rects
    }
    /// `rects[from..]`, of line `i`, cut to its scroll group's frame: what is scrolled out of it shows nothing.
    fn clip_to_group(&self, i: usize, rects: &mut Vec<Rect>, from: usize) {
        let Some((_, group)) = self.group_of(i) else {
            return;
        };
        let right = group.left + group.view;
        let mut kept = from;
        for k in from..rects.len() {
            let r = rects[k];
            let (l, r_edge) = (r.x.max(group.left), (r.x + r.width).min(right));
            if r_edge > l {
                rects[kept] = Rect::new(l, r.y, r_edge - l, r.height);
                kept += 1;
            }
        }
        rects.truncate(kept);
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
        let (i, cell) = self.line_at_point(x, y);
        let x = self.within_group(i, x);
        let layout = self.line(i);
        let inset = self.inset(i);
        let mut down = y.max(0.) - self.top(i) - inset.above;
        if cell {
            // under a cell shorter than its row: its last row at x, as beside it
            down = down.min(layout.size().1 - 0.5);
        }
        let local = layout.hit_position(x - self.left(&inset), down);
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
    fn vertical_step(&self, from: TextPosition, x: f32, down: bool) -> Option<TextPosition> {
        if !self.has_rows() {
            return None;
        }
        let i = self.line_of(from.byte_offset.min(self.text.len()));
        let row = self.row_of(i);
        if row.len() == 1 {
            return None;
        }
        let layout = self.line(i);
        let caret = layout.caret_position(self.local(i, from));
        let to = caret.y
            + if down {
                caret.height * 1.5
            } else {
                -caret.height * 0.5
            };
        // Another of its own wrapped rows: found as in any line.
        if to >= 0. && to < layout.size().1 {
            return None;
        }
        // the next or the last of its cell's stacked lines
        if down && self.inset(i).stacks_next {
            return Some(self.hit_position(x, self.top(i + 1) + self.inset(i + 1).above + 0.5));
        }
        if !down && i > 0 && self.inset(i - 1).stacks_next {
            return Some(self.hit_position(x, self.top(i) - 0.5));
        }
        let top = self.top(row.start);
        if down {
            let below = top + self.row_height(i);
            (row.end < self.line_count()).then(|| self.hit_position(x, below + 0.5))
        } else {
            (row.start > 0 && top > 0.).then(|| self.hit_position(x, top - 0.5))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let rich = styled_line("abcdefgh", &spans, ink, 14., &font).unwrap();
        let runs: Vec<_> = rich
            .runs()
            .iter()
            .map(|r| (r.range.clone(), r.color))
            .collect();
        assert_eq!(runs, [(0..1, ink), (1..4, red), (4..6, blue), (6..8, ink)]);
    }
    /// A note's heading and inline code: size, line pitch, underline and the monospaced font reach the runs.
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
        let rich = styled_line("Title code link", &spans, ink, 16., &font).unwrap();
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

    /// An inset line keeps its carets, hits and selection inside its room, and wraps in what the indent leaves.
    #[test]
    fn insets_move_carets_hits_and_wrapping() {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i| {
            if i == 1 {
                LineInset {
                    left: 36.,
                    above: 8.,
                    below: 4.,
                    ..LineInset::default()
                }
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

    /// An indented line is drawn in the width it was wrapped to, not the whole text's: its rows end at the same edge.
    #[test]
    fn an_indented_line_draws_in_the_width_it_wraps_to() {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i| {
            if i == 1 {
                LineInset {
                    left: 36.,
                    ..LineInset::default()
                }
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
            layout.line(1).size().0 <= 164.,
            "its rows fit the width it is drawn in"
        );
        assert_eq!(layout.draw_width(1, None), None);
    }

    /// A note's heading, then a paragraph and a list item: each line starts where the one above ends (its taller
    /// pitch and its room included), wrapped or not, and a heading's line is as tall as its pitch (nothing clipped).
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
            cache.set_insets(Some(Rc::new(|i| match i {
                1 => LineInset {
                    above: 24.,
                    ..LineInset::default()
                },
                2 => LineInset {
                    above: 12.,
                    ..LineInset::default()
                },
                3 => LineInset {
                    left: 32.,
                    above: 8.,
                    ..LineInset::default()
                },
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
            // as the editor does: painting asks for the spans, a span that shapes lays the text out again
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

    /// A table of two rows of two cells between ordinary lines: `first` is its first cell's line. Each cell is 100
    /// wide, the second column 110 in; the second column's cells have room above and below.
    const TABLE: &str = "before\nA1 with text long enough to wrap in its cell\nB1\nA2\nB2\nafter";
    fn table_insets(first: Rc<Cell<usize>>) -> LineInsets {
        Rc::new(move |i| match i.checked_sub(first.get()) {
            Some(0 | 2) => LineInset {
                width: Some(100.),
                joins_next: true,
                ..LineInset::default()
            },
            Some(1 | 3) => LineInset {
                left: 110.,
                above: 2.,
                below: 3.,
                width: Some(100.),
                ..LineInset::default()
            },
            _ => LineInset::default(),
        })
    }
    fn table(text: &str, first: usize) -> (Rc<LineCache>, Rc<LinesLayout>, Rc<Cell<usize>>) {
        let first = Rc::new(Cell::new(first));
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(table_insets(first.clone())));
        let layout = relaid(&cache, text);
        (cache, layout, first)
    }
    fn relaid(cache: &Rc<LineCache>, text: &str) -> Rc<LinesLayout> {
        cache.layout(
            || Text::new(text),
            None,
            14.,
            Some(400.),
            &FontStyle::default(),
            0,
            shaper,
        )
    }
    /// A table four 300-point columns wide in a 680-point note, one scroll group (7) shown 680 wide: lines 1 to 4
    /// are its cells, between ordinary lines.
    const WIDE: &str = "before\nc1 one\nc2 two\nc3 three\nc4 four\nafter";
    fn wide() -> (Rc<LineCache>, Rc<LinesLayout>) {
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i| match i {
            1..=4 => LineInset {
                left: 300. * (i - 1) as f32,
                width: Some(300.),
                joins_next: i < 4,
                scroll_group: Some(7),
                group_width: Some(680.),
                ..LineInset::default()
            },
            _ => LineInset::default(),
        })));
        let layout = cache.layout(
            || Text::new(WIDE),
            None,
            14.,
            Some(680.),
            &FontStyle::default(),
            0,
            shaper,
        );
        layout.lines_between(0., 10_000.);
        (cache, layout)
    }

    /// A scroll group's frame is its first line's left across its width, its extent its cells'; its lines move
    /// together by its offset, and the lines outside it don't.
    #[test]
    fn a_scroll_group_frames_its_lines_and_scrolls_them_together() {
        let (cache, layout) = wide();
        let frame = layout.group(7).unwrap();
        assert_eq!((frame.left, frame.view, frame.content), (0., 680., 1200.));
        assert_eq!(frame.limit(), 520.);
        let at = |needle: &str| WIDE.find(needle).unwrap();
        assert_eq!(layout.caret(at("c3")).x, 600.);
        assert_eq!(layout.caret(at("c4")).x, 900.);
        cache.set_group_scroll(7, 520.);
        assert_eq!(layout.caret(at("c3")).x, 80.);
        assert_eq!(layout.caret(at("c4")).x, 380.);
        assert_eq!(
            layout.caret(at("before")).x,
            0.,
            "a line outside the group stays"
        );
        assert_eq!(layout.caret(at("after")).x, 0.);
    }

    /// A click in the third column's text places the caret there however far the group is scrolled; the last
    /// columns are reachable, and what the frame clips is not hit.
    #[test]
    fn a_click_in_a_scrolled_cell_places_the_caret_there() {
        let (cache, layout) = wide();
        let at = |needle: &str| WIDE.find(needle).unwrap();
        let (c3, c4) = (layout.line_range(3), layout.line_range(4));
        let y = layout.top(1) + 2.;
        for offset in (0..=520).step_by(40) {
            cache.set_group_scroll(7, offset as f32);
            let x = 600. - offset as f32 + 1.;
            let hit = layout.hit_test(x, y);
            assert!(
                c3.contains(&hit) || hit == c3.end,
                "at {offset}: {hit} in column 3 ({c3:?})"
            );
            assert_eq!(
                hit,
                at("c3"),
                "at {offset}: the caret at the column's start"
            );
        }
        cache.set_group_scroll(7, 520.);
        let hit = layout.hit_test(381., y);
        assert_eq!(hit, at("c4"), "the last column, scrolled to");
        cache.set_group_scroll(7, 0.);
        let hit = layout.hit_test(700., y);
        assert!(
            c3.contains(&hit) || hit == c3.end,
            "past the frame: its last column shown, not the clipped one"
        );
        assert!(!c4.contains(&hit));
    }

    /// A selection across the frame's edge shows only what the frame shows: none of a cell scrolled out of it.
    #[test]
    fn a_selection_across_the_clip_shows_only_what_the_frame_shows() {
        let (cache, layout) = wide();
        let range = WIDE.find("c3").unwrap()..WIDE.find("after").unwrap() - 1;
        let shown = layout.selection(range.clone());
        assert!(!shown.is_empty(), "the third column shows in part");
        for rect in &shown {
            assert!(
                rect.x >= 0. && rect.x + rect.width <= 680. + 0.01,
                "inside the frame: {rect:?}"
            );
            assert!(rect.x < 680., "{rect:?}");
        }
        let top_c4 = layout.top(4);
        cache.set_group_scroll(7, 520.);
        let scrolled = layout.selection(range);
        assert!(
            scrolled
                .iter()
                .any(|r| r.x >= 380. - 0.01 && r.y >= top_c4 - 0.01),
            "scrolled, the last column's selection shows: {scrolled:?}"
        );
        for rect in &scrolled {
            assert!(
                rect.x >= 0. && rect.x + rect.width <= 680. + 0.01,
                "{rect:?}"
            );
        }
    }

    /// A caret in a cell the frame clips asks the group to move just enough to show it, either way, within its
    /// limit; one it shows asks nothing.
    #[test]
    fn a_caret_in_a_clipped_cell_is_revealed() {
        let (cache, layout) = wide();
        let at = |needle: &str| WIDE.find(needle).unwrap();
        let caret = layout.caret(at("c4") + 2);
        let (group, to) = layout
            .reveal_in_group(4, caret)
            .expect("the fourth column is clipped");
        assert_eq!(group, 7);
        assert!(
            (to - (caret.x + caret.width - 680.)).abs() < 0.01,
            "{to} for {caret:?}"
        );
        cache.set_group_scroll(7, to);
        let shown = layout.caret(at("c4") + 2);
        assert!(shown.x + shown.width <= 680. + 0.01, "{shown:?}");
        assert_eq!(
            layout.reveal_in_group(4, shown),
            None,
            "shown: nothing to move"
        );
        // Back to the first column: to 0.
        cache.set_group_scroll(7, 520.);
        let first = layout.caret(at("c1"));
        assert_eq!(layout.reveal_in_group(1, first), Some((7, 0.)));
        // A line outside the group asks nothing.
        assert_eq!(layout.reveal_in_group(0, layout.caret(0)), None);
    }

    /// Lines in no scroll group lay out, hit and select as before whatever offsets are set, and finding groups
    /// asks nothing of their insets.
    #[test]
    fn lines_without_a_scroll_group_ignore_group_offsets() {
        let (cache, layout, _) = table(TABLE, 1);
        layout.lines_between(0., 10_000.);
        assert!(!layout.grouped());
        assert_eq!(layout.group_of(2), None);
        let carets: Vec<Rect> = (0..TABLE.len()).map(|i| layout.caret(i)).collect();
        let hits: Vec<usize> = (0..40)
            .map(|k| layout.hit_test(k as f32 * 6., layout.top(1) + 3.))
            .collect();
        cache.set_group_scroll(0, 50.);
        cache.set_group_scroll(7, 120.);
        assert_eq!(
            carets,
            (0..TABLE.len())
                .map(|i| layout.caret(i))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            hits,
            (0..40)
                .map(|k| layout.hit_test(k as f32 * 6., layout.top(1) + 3.))
                .collect::<Vec<_>>()
        );
    }

    /// Every line's top, own height and row height, and the whole height, all shaped.
    fn geometry(layout: &LinesLayout) -> Vec<(f32, f32, f32)> {
        layout.lines_between(0., 10_000.);
        let mut all: Vec<_> = (0..layout.line_count())
            .map(|i| (layout.top(i), layout.height(i), layout.row_height(i)))
            .collect();
        all.push((layout.size().1, 0., 0.));
        all
    }

    /// The cells of a row start at the row's top, each at its own left; the row is as tall as its tallest cell, and
    /// the lines after it start below it.
    #[test]
    fn a_row_of_lines_shares_a_top_and_is_as_tall_as_its_tallest() {
        let (_, layout, _) = table(TABLE, 1);
        assert_eq!(layout.lines_between(0., 10_000.), 0..6);
        let row = layout.row;
        assert_eq!(layout.top(1), layout.height(0));
        assert_eq!(layout.top(2), layout.top(1));
        assert!(
            layout.height(1) >= 2. * row,
            "the first cell wraps: {}",
            layout.height(1)
        );
        assert_eq!(
            layout.height(2),
            row + 5.,
            "a cell's own height, its room included"
        );
        assert_eq!(layout.row_height(1), layout.height(1));
        assert_eq!(layout.row_height(2), layout.height(1));
        assert_eq!(layout.top(3), layout.top(1) + layout.row_height(1));
        assert_eq!(layout.top(4), layout.top(3));
        assert_eq!(
            layout.row_height(3),
            row + 5.,
            "the second row: its cell with room is the tallest"
        );
        assert_eq!(layout.top(5), layout.top(3) + row + 5.);
        assert_eq!(layout.size().1, layout.top(5) + layout.height(5));
        assert_eq!(layout.row_of(1), 1..3);
        assert_eq!(layout.row_of(4), 3..5);
        assert_eq!(layout.row_of(5), 5..6);
        assert_eq!(layout.draw_width(1, Some(400.)), Some(100.));
        assert_eq!(layout.draw_width(2, Some(400.)), Some(100.));
        assert!(layout.line(1).size().0 <= 100.);
    }

    /// A hit in a row finds the cell whose room holds its x (the first before them all, the last after), and carets
    /// sit in their cells.
    #[test]
    fn hits_and_carets_in_a_row_follow_its_cells() {
        let (_, layout, _) = table(TABLE, 1);
        layout.lines_between(0., 10_000.);
        let at = |needle: &str| TABLE.find(needle).unwrap();
        let (a1, b1, a2, b2) = (at("A1"), at("B1"), at("A2"), at("B2"));
        let (top, row) = (layout.top(1), layout.row);
        assert_eq!(
            layout.caret(a1),
            Rect::new(0., top, layout.caret(a1).width, row)
        );
        assert_eq!((layout.caret(b1).x, layout.caret(b1).y), (110., top + 2.));
        assert_eq!(
            (layout.caret(b2).x, layout.caret(b2).y),
            (110., layout.top(3) + 2.)
        );
        assert_eq!(layout.caret(a2).y, layout.top(3));
        assert_eq!(layout.hit_test(1., top + 1.), a1);
        assert_eq!(layout.hit_test(-20., top + 1.), a1);
        assert_eq!(layout.hit_test(111., top + 3.), b1);
        assert_eq!(
            layout.hit_test(900., top + 3.),
            b1 + 2,
            "past the last cell: its end"
        );
        // between the cells: the cell before the gap
        let gap = layout.hit_test(105., top + 1.);
        assert!((a1..b1).contains(&gap), "{gap}");
        // lower in the row than the short cell's text: still that cell
        let low = layout.hit_position(111., top + layout.row_height(1) - 1.);
        assert_eq!(low.byte_offset, b1, "its row at that x, not its end");
        let wrapped = layout.hit_test(1., top + row * 1.5);
        assert!(
            wrapped > a1 && wrapped < b1,
            "the first cell's second row: {wrapped}"
        );
        assert_eq!(layout.hit_test(1., layout.top(3) + 1.), a2);
        assert_eq!(layout.hit_test(111., layout.top(3) + 3.), b2);
        assert_eq!(layout.hit_test(1., layout.top(5) + 1.), at("after"));
        assert_eq!(layout.hit_test(1., 1.), 0);
        for offset in 0..=TABLE.len() {
            let caret = layout.caret(offset);
            assert_eq!(
                layout.hit_test(caret.x + 0.5, caret.y + 1.),
                offset,
                "a hit at caret {offset} finds it"
            );
        }
    }

    /// A selection over two cells paints each cell's lines inside the cell, never a band across the row.
    #[test]
    fn a_selection_across_cells_stays_inside_each_cell() {
        let (_, layout, _) = table(TABLE, 1);
        let (a1, b1) = (TABLE.find("A1").unwrap(), TABLE.find("B1").unwrap());
        let rects = layout.selection(a1 + 3..b1 + 2);
        assert!(rects.len() >= 3, "{rects:?}");
        let (top, bottom) = (layout.top(1), layout.top(1) + layout.row_height(1));
        let (mut left, mut right) = (0, 0);
        for r in &rects {
            assert!(r.y >= top && r.y + r.height <= bottom, "in the row: {r:?}");
            if r.x < 110. {
                assert!(r.x + r.width <= 100.5, "in the first cell: {r:?}");
                left += 1;
            } else {
                assert!(r.x + r.width <= 210.5, "in the second cell: {r:?}");
                assert_eq!(r.y, top + 2.);
                right += 1;
            }
        }
        assert!(left >= 2 && right == 1, "{rects:?}");
        assert_eq!(
            layout.selection_between(a1 + 3..b1 + 2, top + 1., top + 2.),
            rects
        );
        // into the next row: its first cell only
        let a2 = TABLE.find("A2").unwrap();
        let more = layout.selection(b1..a2 + 1);
        assert!(
            more.iter().any(|r| r.y == layout.top(3) && r.x == 0.),
            "{more:?}"
        );
        assert!(more.iter().all(|r| r.x + r.width <= 210.5), "{more:?}");
    }

    /// A row in view shows all its lines, and the line at a y in it is its first.
    #[test]
    fn lines_between_gives_the_whole_row() {
        let (_, layout, _) = table(TABLE, 1);
        layout.lines_between(0., 10_000.);
        let top = layout.top(1);
        assert_eq!(layout.lines_between(top + 1., top + 2.), 1..3);
        assert_eq!(layout.line_at(top + 1.), 1);
        let second = layout.top(3);
        assert_eq!(layout.lines_between(second + 1., second + 2.), 3..5);
        assert_eq!(layout.lines_between(top + 1., second + 1.), 1..5);
        assert_eq!(layout.line_at(layout.top(5) + 1.), 5);
        assert_eq!(layout.line_at(0.), 0);
    }

    /// An edit that grows a cell makes its row taller and moves the lines below; edits around the table (which
    /// move its lines) keep its rows: all as a layout of the text made has them.
    #[test]
    fn an_edit_in_a_row_lays_it_out_again() {
        let (cache, layout, first) = table(TABLE, 1);
        let before = geometry(&layout);
        let mut text = String::from(TABLE);
        let b1 = text.find("B1").unwrap();
        text.insert_str(
            b1 + 2,
            " grows until it is the tallest cell of its row by far, three rows or more",
        );
        let grown = relaid(&cache, &text);
        let after = geometry(&grown);
        assert_eq!(after, geometry(&table(&text, 1).1));
        assert!(
            after[2].1 > before[1].1,
            "the second cell is the tallest now"
        );
        assert_eq!(after[1].2, after[2].1);
        assert_eq!(
            after[3].0,
            after[1].0 + after[2].1,
            "the next row starts below it"
        );
        assert!(after[5].0 > before[5].0);
        assert_eq!(
            after[1].1, before[1].1,
            "the first cell's own height is its own"
        );
        // a line break above the table: its lines move down one, their rows with them
        text.insert_str(0, "new\n");
        first.set(2);
        let moved = relaid(&cache, &text);
        assert_eq!(moved.row_of(2), 2..4);
        assert_eq!(geometry(&moved), geometry(&table(&text, 2).1));
        // the last cell of the table edited, then the line after it
        let b2 = text.find("B2").unwrap();
        text.insert_str(
            b2,
            "taller than the first cell beside it, wrapping onto a few rows ",
        );
        assert_eq!(
            geometry(&relaid(&cache, &text)),
            geometry(&table(&text, 2).1)
        );
        let end = text.len();
        text.insert_str(end, "\nlast");
        assert_eq!(
            geometry(&relaid(&cache, &text)),
            geometry(&table(&text, 2).1)
        );
        // and before any of it is shaped again, the heights the edit keeps are a fresh layout's
        text.replace_range(0..4, "");
        first.set(1);
        let kept = relaid(&cache, &text);
        assert_eq!(kept.row_of(1), 1..3);
        assert_eq!(geometry(&kept), geometry(&table(&text, 1).1));
    }

    /// A hidden line takes no room in a text with rows, alone or as a cell.
    #[test]
    fn a_hidden_line_among_rows_takes_no_room() {
        let text = "before\nA1\nB1 hidden cell with text long enough to wrap\nC1\nhidden\nafter";
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(|i| match i {
            1 => LineInset {
                width: Some(100.),
                joins_next: true,
                ..LineInset::default()
            },
            2 => LineInset {
                left: 110.,
                width: Some(60.),
                joins_next: true,
                hidden: true,
                ..LineInset::default()
            },
            3 => LineInset {
                left: 110.,
                width: Some(100.),
                ..LineInset::default()
            },
            4 => LineInset {
                hidden: true,
                ..LineInset::default()
            },
            _ => LineInset::default(),
        })));
        let layout = relaid(&cache, text);
        assert_eq!(layout.lines_between(0., 10_000.), 0..6);
        let row = layout.row;
        assert_eq!(layout.row_of(2), 1..4);
        assert_eq!(
            (layout.top(1), layout.top(2), layout.top(3)),
            (row, row, row)
        );
        assert_eq!(layout.height(2), 0.);
        assert_eq!(
            layout.row_height(1),
            row,
            "the hidden cell's wrapped text adds nothing"
        );
        assert_eq!(layout.top(4), 2. * row);
        assert_eq!(layout.top(5), 2. * row);
        assert_eq!(layout.size().1, 3. * row);
        assert_eq!(layout.hit_test(111., row + 1.), text.find("C1").unwrap());
        assert_eq!(
            layout.hit_test(1., 2. * row + 1.),
            text.find("after").unwrap()
        );
    }

    /// Up and down leave a row for the line above or below it at the caret's x (the cell under it, if that is a
    /// row), once the cell's own wrapped rows are done; a text with no rows steps as it did.
    #[test]
    fn vertical_steps_leave_a_row() {
        let (_, layout, _) = table(TABLE, 1);
        layout.lines_between(0., 10_000.);
        let at = |needle: &str| TABLE.find(needle).unwrap();
        let step = |from: usize, x: f32, down: bool| {
            let from = TextPosition {
                byte_offset: from,
                affinity: TextAffinity::After,
            };
            layout.vertical_step(from, x, down).map(|to| to.byte_offset)
        };
        assert_eq!(
            step(at("B1"), 110., true),
            Some(at("B2")),
            "the cell below, not B1's end"
        );
        assert_eq!(
            step(at("B2"), 110., true),
            Some(TABLE.len()),
            "the line below, at its x"
        );
        assert_eq!(step(at("B2"), 110., false), Some(at("B1")));
        assert_eq!(step(at("A1"), 0., false), Some(0));
        assert_eq!(step(at("A1"), 0., true), None, "its own second row first");
        assert_eq!(
            step(at("B1") - 1, 0., true),
            Some(at("A2")),
            "from the cell's last row"
        );
        assert_eq!(
            step(at("A2"), 0., false).map(|to| layout.line_of(to)),
            Some(1)
        );
        assert_eq!(
            step(at("after"), 0., false),
            None,
            "an ordinary line steps by a hit"
        );
        let above = layout.hit_test(110., layout.top(5) - layout.row * 0.5);
        assert_eq!(
            layout.line_of(above),
            4,
            "which finds the cell above at its x"
        );
        let plain = lines(TABLE, Some(400.));
        assert!(!plain.has_rows());
        let from = TextPosition {
            byte_offset: at("B1"),
            affinity: TextAffinity::After,
        };
        assert_eq!(plain.vertical_step(from, 0., true), None);
    }

    /// A text with no rows asks each line's room no more than it did: once a line as it is laid out, and for a
    /// hit or a caret only its own line's.
    #[test]
    fn a_text_without_rows_asks_no_more_of_its_insets() {
        let asked = Rc::new(Cell::new(0usize));
        let count = asked.clone();
        let cache = Rc::new(LineCache::default());
        cache.set_insets(Some(Rc::new(move |_| {
            count.set(count.get() + 1);
            LineInset::default()
        })));
        let text: String = (0..1_000).map(|i| format!("line {i}\n")).collect();
        let layout = relaid(&cache, &text);
        assert_eq!(asked.get(), layout.line_count(), "once a line");
        asked.set(0);
        layout.hit_test(5., 500. * layout.row);
        assert_eq!(asked.get(), 2, "the line shaped, and its room for the hit");
        asked.set(0);
        layout.line_at(300. * layout.row);
        layout.top(700);
        layout.height(700);
        layout.row_height(700);
        assert_eq!(asked.get(), 0);
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
}
