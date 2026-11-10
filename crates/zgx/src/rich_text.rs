//! Immutable display text with continuous UTF-8 styled runs.
use crate::{scene::Color, text_layout::FontStyle};
use std::{ops::Range, sync::Arc};
pub(crate) type RichShaper =
    dyn Fn(&RichText, Option<f32>) -> Box<dyn crate::text_layout::TextLayout>;
pub(crate) type RichMeasurer = dyn Fn(&RichText, Option<f32>) -> (f32, f32);

/// Measures rich text exactly as layout does: with the host's installed
/// measurers (prepared text on desktop, where a size at a new width costs line
/// breaking arithmetic), or the headless fallback until one is installed.
/// Apps use it for the heights of content they have not mounted, such as the
/// estimates of a virtualized list; hosts can measure those without keeping
/// glyphs for drawing (`Scene::set_detached_rich_text_measurer`).
#[derive(Clone, Default)]
pub struct TextMeasure(std::rc::Rc<std::cell::RefCell<Measurers>>);
#[derive(Default)]
struct Measurers {
    layout: Option<Box<RichMeasurer>>,
    detached: Option<Box<RichMeasurer>>,
}
impl TextMeasure {
    /// `rich`'s (width, height) wrapped at `width`, or unwrapped for `None`.
    pub fn measure(&self, rich: &RichText, width: Option<f32>) -> (f32, f32) {
        let measurers = self.0.borrow();
        match measurers.detached.as_ref().or(measurers.layout.as_ref()) {
            Some(measure) => measure(rich, width),
            None => fallback_measure(rich, width),
        }
    }
    /// Whether a host measurer is installed (else the headless fallback).
    pub(crate) fn is_native(&self) -> bool {
        self.0.borrow().layout.is_some()
    }
    /// For mounted text, which is drawn next.
    pub(crate) fn measure_mounted(&self, rich: &RichText, width: Option<f32>) -> (f32, f32) {
        match &self.0.borrow().layout {
            Some(measure) => measure(rich, width),
            None => fallback_measure(rich, width),
        }
    }
    pub(crate) fn install(&self, measure: Box<RichMeasurer>) {
        self.0.borrow_mut().layout = Some(measure);
    }
    pub(crate) fn install_detached(&self, measure: Box<RichMeasurer>) {
        self.0.borrow_mut().detached = Some(measure);
    }
}

/// How a decoration's line is drawn (CSS's `text-decoration-style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineStyle {
    /// One unbroken line.
    #[default]
    Solid,
    /// Two lines.
    Double,
    /// A row of dots.
    Dotted,
    /// A row of dashes.
    Dashed,
    /// A wave, as a spelling mistake or a diagnostic is marked.
    Wavy,
}
/// A text decoration. An unspecified color uses the run foreground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoration {
    /// Its colour; `None`, the run's text colour.
    pub color: Option<Color>,
    thickness: f32,
    line: LineStyle,
    /// How far below the baseline it sits (CSS `text-underline-offset`);
    /// None, a tenth of the font size.
    offset: Option<f32>,
}
impl Decoration {
    /// A decoration `thickness` logical pixels thick (1 unless that is
    /// finite and positive), in the run's colour.
    pub fn new(thickness: f32) -> Self {
        Self {
            color: None,
            thickness: if thickness.is_finite() && thickness > 0. {
                thickness
            } else {
                1.
            },
            line: LineStyle::Solid,
            offset: None,
        }
    }
    /// Drawn as `line`.
    pub fn line(mut self, line: LineStyle) -> Self {
        self.line = line;
        self
    }
    /// How its line is drawn.
    pub fn line_style(self) -> LineStyle {
        self.line
    }
    /// Below the baseline by `px` (an underline's; CSS `text-underline-offset`).
    pub fn offset(mut self, px: f32) -> Self {
        self.offset = px.is_finite().then_some(px);
        self
    }
    /// How far below the baseline an underline sits (`offset`); `None`, a
    /// tenth of the font size.
    pub fn offset_px(self) -> Option<f32> {
        self.offset
    }
    /// Drawn in `color` rather than the run's text colour.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
    /// Its thickness, in logical pixels.
    pub fn thickness(self) -> f32 {
        self.thickness
    }
}
impl Default for Decoration {
    fn default() -> Self {
        Self::new(1.)
    }
}

/// One resolved style over a contiguous UTF-8 byte range.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// The UTF-8 byte range of the text it styles.
    pub range: Range<usize>,
    /// Its font: family, weight, italic, line height, spacing and features.
    pub font: FontStyle,
    /// The font size, in logical pixels (16 by default).
    pub font_size: f32,
    /// The text colour (white by default).
    pub color: Color,
    /// A colour behind its text.
    pub background: Option<Color>,
    /// A line under its text.
    pub underline: Option<Decoration>,
    /// A line through its text.
    pub strikethrough: Option<Decoration>,
    /// Room around the run, as CSS's inline padding: across, it is added
    /// before and after the run's text (it takes space in the line); both
    /// widen its background, which then hugs the text (its font's content
    /// area) instead of filling the line: an inline code chip.
    pub padding: [f32; 2],
    /// The background's corner radius.
    pub radius: f32,
}
impl TextRun {
    /// The approximate advance of `grapheme` at byte `at` (no shaper): 0.6
    /// em a cell, zero-width characters none, and the run's padding on its
    /// first and last.
    pub(crate) fn estimated_advance(&self, grapheme: &str, at: usize) -> f32 {
        let glyph = if matches!(grapheme, "\u{200B}" | "\u{2060}") {
            0.
        } else {
            self.font_size * 0.6 + self.font.letter_spacing.pixels()
        };
        let first = (at == self.range.start) as u8 as f32;
        let last = (at + grapheme.len() == self.range.end) as u8 as f32;
        (glyph + (first + last) * self.padding[0]).max(0.)
    }
}

impl Default for TextRun {
    fn default() -> Self {
        Self {
            range: 0..0,
            font: Default::default(),
            font_size: 16.,
            color: Color(255, 255, 255, 255),
            background: None,
            underline: None,
            strikethrough: None,
            padding: [0.; 2],
            radius: 0.,
        }
    }
}

/// What holds an inline box's place in a paragraph's text: the object
/// replacement character, a run of its own (`RichText::with_inline_boxes`).
pub const INLINE_BOX_CHAR: char = '\u{FFFC}';
/// [`INLINE_BOX_CHAR`] as text.
pub const INLINE_BOX_TEXT: &str = "\u{FFFC}";

/// How an inline box sits in its line, as CSS's `vertical-align` says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum InlineAlign {
    /// Its baseline on the line's (its bottom, if it has none).
    #[default]
    Baseline,
    /// Its middle half the text's x-height above the baseline.
    Middle,
    /// Its top at the text's ascent.
    TextTop,
    /// Its bottom at the text's descent.
    TextBottom,
    /// Its top at the line's top.
    Top,
    /// Its bottom at the line's bottom.
    Bottom,
    /// Its baseline this many points above the line's (below, if negative).
    Raise(f32),
}

/// An atomic box in a paragraph's line (CSS's inline-block): it holds the
/// place of the [`INLINE_BOX_CHAR`] at byte `at`, takes `width` in its line,
/// never breaks across lines, and its line grows to hold it as CSS's line
/// box does. What draws in it is the paragraph's business (a scene node laid
/// out in its place); the text draws nothing there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineBox {
    /// The byte offset of its character in the text.
    pub at: usize,
    /// Its width, in points.
    pub width: f32,
    /// Its height, in points.
    pub height: f32,
    /// Its baseline, in points below its top (its height, if it has none).
    pub baseline: f32,
    /// How it sits in its line.
    pub align: InlineAlign,
}
impl InlineBox {
    /// A box for the character at `at`, empty until sized, with no baseline
    /// of its own: it stands on the line's by its bottom, as CSS's
    /// inline-block with none does, whatever height it is given.
    pub fn new(at: usize) -> Self {
        Self {
            at,
            width: 0.,
            height: 0.,
            baseline: f32::INFINITY,
            align: InlineAlign::Baseline,
        }
    }
    /// How far it reaches above and below the baseline, for text reaching
    /// `ascent` above it and `descent` below with `x_height`; none for a box
    /// at its line's top or bottom, which is placed once the line is known.
    pub fn extents(&self, ascent: f32, descent: f32, x_height: f32) -> (f32, f32) {
        let h = self.height;
        match self.align {
            InlineAlign::Baseline => (self.baseline, h - self.baseline),
            InlineAlign::Middle => ((h + x_height) / 2., (h - x_height) / 2.),
            InlineAlign::TextTop => (ascent, h - ascent),
            InlineAlign::TextBottom => (h - descent, descent),
            InlineAlign::Raise(by) => (self.baseline + by, h - self.baseline - by),
            InlineAlign::Top | InlineAlign::Bottom => (0., 0.),
        }
    }
    /// Its sizes made finite and not negative, its baseline within it.
    fn sanitized(mut self) -> Self {
        let clean = |v: f32| if v.is_finite() { v.max(0.) } else { 0. };
        self.width = clean(self.width);
        self.height = clean(self.height);
        // None (`new`, or one that isn't a number): its bottom.
        self.baseline = if self.baseline.is_finite() {
            self.baseline.clamp(0., self.height)
        } else {
            self.height
        };
        if let InlineAlign::Raise(by) = &mut self.align
            && !by.is_finite()
        {
            *by = 0.;
        }
        self
    }
    /// `run` (its character's) holding this box: said in its font's features
    /// (`FontFeatures::with_inline_box`), which text without boxes has none
    /// of, and its padding across, which keeps it a run of its own wherever
    /// runs merge and, differing as its size does, a prepared text's hash
    /// apart; a box takes no padding of its own.
    fn on_run(&self, run: &mut TextRun) {
        run.font.features = run.font.features.with_inline_box(self);
        run.padding[0] = 1. + self.width + self.height / 64. + self.baseline / 4096.;
    }
}

/// Validated, immutable styled display text. Runs cover the entire string exactly;
/// empty strings use no runs. Adjacent styles may cross grapheme boundaries: the
/// native shaper still shapes the paragraph continuously.
#[derive(Clone, Debug, PartialEq)]
pub struct RichText {
    /// Its options (`options`), apart, leaving room for `boxed`.
    overflow: crate::text_layout::TextOverflow,
    line_clamp: Option<std::num::NonZeroU32>,
    /// Whether a run holds an inline box: what reads them asks this, not
    /// each run.
    boxed: bool,
    text: Arc<str>,
    runs: Arc<[TextRun]>,
}
impl RichText {
    /// `text` styled by `runs`. Errs unless the runs cover the text in
    /// order, each a nonempty range on char boundaries following the last,
    /// with a finite, positive font size; empty text takes no runs.
    pub fn new(
        text: impl Into<Arc<str>>,
        runs: impl Into<Arc<[TextRun]>>,
    ) -> Result<Self, &'static str> {
        let text = text.into();
        let runs = runs.into();
        let mut end = 0;
        for run in runs.iter() {
            if run.range.start != end
                || run.range.end <= end
                || run.range.end > text.len()
                || !text.is_char_boundary(run.range.start)
                || !text.is_char_boundary(run.range.end)
            {
                return Err("rich text runs must cover contiguous nonempty UTF-8 ranges");
            }
            if !run.font_size.is_finite() || run.font_size <= 0. {
                return Err("rich text font size must be finite and positive");
            }
            end = run.range.end;
        }
        if end != text.len() {
            return Err("rich text runs must cover the entire string");
        }
        Ok(Self {
            overflow: Default::default(),
            line_clamp: None,
            boxed: false,
            text,
            runs,
        })
    }
    /// With inline `boxes` (see [`InlineBox`]), in order of their offsets.
    /// Errs unless each is at an [`INLINE_BOX_CHAR`] that is a run of its
    /// own. Sizes that aren't finite are zero; no boxes, none.
    pub fn with_inline_boxes(
        mut self,
        boxes: impl Into<Vec<InlineBox>>,
    ) -> Result<Self, &'static str> {
        let boxes: Vec<InlineBox> = boxes.into();
        if boxes.is_empty() {
            return Ok(self);
        }
        let mut runs = self.runs.to_vec();
        // Boxes and runs both in order: each found from where the last was,
        // so a long paragraph's take no longer than its runs.
        let mut rest = runs.iter_mut();
        let mut last = None;
        for b in &boxes {
            let end = b.at + INLINE_BOX_TEXT.len();
            let run = rest.find(|run| run.range.start >= b.at);
            let (false, Some(run), Some(INLINE_BOX_TEXT)) = (
                last.is_some_and(|last| b.at <= last),
                run.filter(|run| run.range == (b.at..end)),
                self.text.get(b.at..end),
            ) else {
                return Err("an inline box must be at a replacement character run of its own");
            };
            b.sanitized().on_run(run);
            last = Some(b.at);
        }
        self.runs = runs.into();
        self.boxed = true;
        Ok(self)
    }
    /// Its inline boxes, in order: none for most text. Read from its runs,
    /// for those that lay them out.
    pub fn inline_boxes(&self) -> Vec<InlineBox> {
        self.runs
            .iter()
            .filter_map(|run| run.font.features.inline_box(run.range.start))
            .collect()
    }
    /// `text` styled by `runs` made from this text's (a cut, its lines apart,
    /// its languages taken again), so keeping what its runs hold: the one way
    /// to remake a text from its runs, whatever else it keeps. Its options
    /// are the caller's to say.
    pub fn remade(
        &self,
        text: impl Into<Arc<str>>,
        runs: impl Into<Arc<[TextRun]>>,
    ) -> Result<Self, &'static str> {
        let mut remade = Self::new(text, runs)?;
        remade.boxed = self.boxed;
        Ok(remade)
    }
    /// Whether it has inline boxes.
    #[inline]
    pub fn has_inline_boxes(&self) -> bool {
        self.boxed
    }
    /// The same text with its boxes' sizes changed to `sized`'s (width,
    /// height, baseline), in order (where its layout measured what they
    /// hold), its text shared.
    pub fn with_box_sizes(&self, sized: impl IntoIterator<Item = (f32, f32, f32)>) -> Self {
        let mut out = self.clone();
        let mut sized = sized.into_iter();
        let mut runs = self.runs.to_vec();
        for run in &mut runs {
            let Some(b) = run.font.features.inline_box(run.range.start) else {
                continue;
            };
            let Some((width, height, baseline)) = sized.next() else {
                break;
            };
            InlineBox {
                width,
                height,
                baseline,
                ..b
            }
            .sanitized()
            .on_run(run);
        }
        out.runs = runs.into();
        out
    }
    /// With `options`: how text that doesn't fit ends, and its line clamp.
    pub fn with_options(mut self, options: crate::text_layout::TextOptions) -> Self {
        self.overflow = options.overflow;
        self.line_clamp = options.line_clamp;
        self
    }
    /// How text that doesn't fit ends, and its line clamp.
    pub fn options(&self) -> crate::text_layout::TextOptions {
        crate::text_layout::TextOptions {
            overflow: self.overflow,
            line_clamp: self.line_clamp,
        }
    }
    /// Whether `other` differs from `self` only in colors: same text, runs,
    /// fonts and decoration presence, so shaped glyphs and decoration geometry
    /// can be reused and recoloured.
    pub fn same_shape(&self, other: &Self) -> bool {
        self.same_metrics(other)
            && self.runs.iter().zip(other.runs.iter()).all(|(a, b)| {
                a.background.is_some() == b.background.is_some()
                    && a.underline.map(|d| (d.thickness(), d.offset, d.line))
                        == b.underline.map(|d| (d.thickness(), d.offset, d.line))
                    && a.strikethrough.map(|d| (d.thickness(), d.line))
                        == b.strikethrough.map(|d| (d.thickness(), d.line))
            })
    }
    pub(crate) fn same_metrics(&self, other: &Self) -> bool {
        self.overflow == other.overflow
            && self.line_clamp == other.line_clamp
            && self.text == other.text
            && self.runs.len() == other.runs.len()
            && self.runs.iter().zip(other.runs.iter()).all(|(a, b)| {
                // A run's padding across takes room in its line.
                a.range == b.range
                    && a.font_size == b.font_size
                    && a.padding[0] == b.padding[0]
                    && a.font == b.font
            })
    }
    /// `!same_metrics`, for `Scene::set_leaf_kind`: written apart so that
    /// `same_metrics` keeps the callers it is inlined into (a third one
    /// outlines it from `set_kind`).
    pub(crate) fn metrics_differ(&self, other: &Self) -> bool {
        self.overflow != other.overflow
            || self.line_clamp != other.line_clamp
            || self.text != other.text
            || self.runs.len() != other.runs.len()
            || self.runs.iter().zip(other.runs.iter()).any(|(a, b)| {
                a.range != b.range
                    || a.font_size != b.font_size
                    || a.padding[0] != b.padding[0]
                    || a.font != b.font
            })
    }
    /// The text.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// The text, sharing its allocation.
    pub fn text_arc(&self) -> Arc<str> {
        self.text.clone()
    }
    /// Conservative payload accounting for bounded caches; shared font names may
    /// be charged more than once. Excludes allocator and font-engine overhead.
    pub fn storage_bytes(&self) -> usize {
        self.text
            .len()
            .saturating_add(std::mem::size_of_val(self.runs.as_ref()))
            .saturating_add(
                self.runs
                    .iter()
                    .map(|run| run.font.storage_bytes())
                    .fold(0usize, usize::saturating_add),
            )
    }
    /// Its runs, covering the text in order.
    pub fn runs(&self) -> &[TextRun] {
        &self.runs
    }
    /// The same text with run `i`'s colour `color(i, colour)`: shaped
    /// alike (`same_shape`), so the renderer recolours its glyphs only.
    pub fn recoloured(&self, mut color: impl FnMut(usize, Color) -> Color) -> Self {
        let runs: Vec<TextRun> = self
            .runs
            .iter()
            .enumerate()
            .map(|(i, run)| TextRun {
                color: color(i, run.color),
                ..run.clone()
            })
            .collect();
        // All else as it is: its options, and whether a run holds a box.
        let mut recoloured = self.clone();
        recoloured.runs = runs.into();
        recoloured
    }
}

/// A line's reach above and below its baseline as the headless fallback
/// sees it, once it holds an inline box (CSS's line box): each run's
/// ascent (0.8 em) and descent (0.2 em) with half its leading, each box's
/// extents (an x-height of half an em), then the boxes at its top or bottom.
#[derive(Clone, Copy, Default)]
struct FallbackLine {
    above: f32,
    below: f32,
    top: f32,
    bottom: f32,
}
impl FallbackLine {
    fn metrics(run: &TextRun) -> (f32, f32, f32) {
        let size = run.font_size;
        (0.8 * size, 0.2 * size, run.font.line_height.resolve(size))
    }
    fn text(&mut self, run: &TextRun) {
        let (ascent, descent, line) = Self::metrics(run);
        let half = (line - ascent - descent) / 2.;
        self.above = self.above.max(ascent + half);
        self.below = self.below.max(descent + half);
    }
    fn inline(&mut self, b: &InlineBox, run: &TextRun) {
        self.text(run);
        match b.align {
            InlineAlign::Top => self.top = self.top.max(b.height),
            InlineAlign::Bottom => self.bottom = self.bottom.max(b.height),
            _ => {
                let (ascent, descent, _) = Self::metrics(run);
                let (above, below) = b.extents(ascent, descent, 0.5 * run.font_size);
                self.above = self.above.max(above);
                self.below = self.below.max(below);
            }
        }
    }
    /// How far its baseline is below its top, and its height.
    fn settled(&self) -> (f32, f32) {
        let below = self.below.max(self.top - self.above);
        let above = self.above.max(self.bottom - below);
        (above, above + below)
    }
    /// How far `b`'s top (in `run`) is below the line's top.
    fn top_of(&self, b: &InlineBox, run: &TextRun) -> f32 {
        let (baseline, height) = self.settled();
        match b.align {
            InlineAlign::Top => 0.,
            InlineAlign::Bottom => height - b.height,
            _ => {
                let (ascent, descent, _) = Self::metrics(run);
                baseline - b.extents(ascent, descent, 0.5 * run.font_size).0
            }
        }
    }
}
/// What the headless fallback lays out of text with inline boxes: each
/// grapheme's range and cell, each row's end, and each box's character and
/// top. Apart from text without boxes, whose lines it never reads.
pub(crate) struct FallbackBoxes {
    pub cells: Vec<(Range<usize>, crate::scene::Rect)>,
    /// Each row's first cell, its bottom and its widest.
    pub rows: Vec<(usize, f32, f32)>,
    pub places: Vec<(usize, f32)>,
    pub end: (f32, f32, f32),
}
#[cold]
#[inline(never)]
pub(crate) fn fallback_boxes(rich: &RichText, width: Option<f32>) -> FallbackBoxes {
    use unicode_segmentation::UnicodeSegmentation;
    let mut out = FallbackBoxes {
        cells: Vec::new(),
        rows: Vec::new(),
        places: Vec::new(),
        end: (0., 0., 0.),
    };
    let (mut x, mut y, mut line, mut row_start) = (0_f32, 0_f32, FallbackLine::default(), 0);
    let mut held: Vec<(usize, InlineBox, usize)> = Vec::new();
    let settle = |out: &mut FallbackBoxes,
                  x: f32,
                  y: f32,
                  row_start: usize,
                  line: &FallbackLine,
                  held: &mut Vec<(usize, InlineBox, usize)>| {
        let height = line.settled().1;
        for (_, rect) in &mut out.cells[row_start..] {
            rect.height = height;
        }
        for (cell, b, run) in held.drain(..) {
            out.places
                .push((cell, y + line.top_of(&b, &rich.runs[run])));
        }
        out.rows.push((row_start, y + height, x));
        height
    };
    let mut run_index = 0;
    for (index, grapheme) in rich.text.grapheme_indices(true) {
        while rich.runs[run_index].range.end <= index {
            run_index += 1;
        }
        let run = &rich.runs[run_index];
        let newline = grapheme.contains(['\r', '\n']);
        // Its grapheme may hold a combining mark past its character.
        let inline = grapheme
            .starts_with(INLINE_BOX_TEXT)
            .then(|| run.font.features.inline_box(index))
            .flatten();
        let advance = inline
            .as_ref()
            .map_or_else(|| run.estimated_advance(grapheme, index), |b| b.width);
        if newline || width.is_some_and(|w| x > 0. && x + advance > w.max(0.)) {
            if newline {
                line.text(run);
            }
            y += settle(&mut out, x, y, row_start, &line, &mut held);
            line = FallbackLine::default();
            x = 0.;
            row_start = out.cells.len();
            if newline {
                // The line it starts holds its run's strut until more comes,
                // as plain text's keeps a pitch.
                line.text(run);
                continue;
            }
        }
        match &inline {
            Some(b) => {
                line.inline(b, run);
                held.push((out.cells.len(), *b, run_index));
            }
            None => line.text(run),
        }
        out.cells.push((
            index..index + grapheme.len(),
            crate::scene::Rect::new(x, y, advance, 0.),
        ));
        x += advance;
    }
    let height = settle(&mut out, x, y, row_start, &line, &mut held);
    out.end = (x, y, height);
    out
}
/// `fallback_measure` for text with inline boxes: as far as the rows kept.
#[cold]
#[inline(never)]
fn fallback_measure_boxed(rich: &RichText, width: Option<f32>) -> (f32, f32) {
    let boxes = fallback_boxes(rich, width);
    let limit = rich
        .line_clamp
        .map(|n| n.get() as usize)
        .or((rich.overflow == crate::text_layout::TextOverflow::Ellipsis).then_some(1));
    let kept = limit.map_or(boxes.rows.len(), |limit| limit.min(boxes.rows.len()));
    let wide = boxes.rows[..kept].iter().fold(0_f32, |w, row| w.max(row.2));
    (
        wide,
        kept.checked_sub(1).map_or(0., |last| boxes.rows[last].1),
    )
}

/// Approximate, allocation-free headless metrics. Native hosts replace this with
/// full paragraph shaping; grapheme widths here follow the fallback text model.
pub fn fallback_measure(rich: &RichText, width: Option<f32>) -> (f32, f32) {
    use unicode_segmentation::UnicodeSegmentation;
    if rich.text.is_empty() {
        return (0., 0.);
    }
    if rich.has_inline_boxes() {
        // Through a pointer. Called directly, rustc's MIR inliner stops inlining
        // this crate's functions into what can reach here, a frame's walk among
        // them (`Scene::text_ink`), and the walk cost 2.8% more.
        let boxed: fn(&RichText, Option<f32>) -> (f32, f32) = fallback_measure_boxed;
        return boxed(rich, width);
    }
    let mut run_index = 0;
    let limit = rich
        .line_clamp
        .map(|n| n.get())
        .or((rich.overflow == crate::text_layout::TextOverflow::Ellipsis).then_some(1));
    let mut row = 1;
    let (mut x, mut y, mut max_x, mut pitch) = (0_f32, 0_f32, 0_f32, 0_f32);
    for (index, grapheme) in rich.text.grapheme_indices(true) {
        while rich.runs[run_index].range.end <= index {
            run_index += 1;
        }
        let run = &rich.runs[run_index];
        let line = run.font.line_height.resolve(run.font_size);
        if grapheme == "\n" || grapheme == "\r\n" || grapheme == "\r" {
            if limit.is_some_and(|limit| row >= limit) {
                return (
                    width.map_or(max_x.max(x), |w| {
                        max_x
                            .max(
                                x + if rich.overflow == crate::text_layout::TextOverflow::Ellipsis {
                                    run.font_size * 0.6
                                } else {
                                    0.
                                },
                            )
                            .min(w.max(0.))
                    }),
                    y + pitch.max(line),
                );
            }
            row += 1;
            max_x = max_x.max(x);
            y += pitch.max(line);
            x = 0.;
            pitch = line;
            continue;
        }
        let advance = run.estimated_advance(grapheme, index);
        if width.is_some_and(|w| x > 0. && x + advance > w.max(0.)) {
            if limit.is_some_and(|limit| row >= limit) {
                return (
                    width.unwrap().max(0.).min(max_x.max(
                        x + if rich.overflow == crate::text_layout::TextOverflow::Ellipsis {
                            advance
                        } else {
                            0.
                        },
                    )),
                    y + pitch,
                );
            }
            row += 1;
            max_x = max_x.max(x);
            y += pitch;
            x = 0.;
            pitch = 0.;
        }
        pitch = pitch.max(line);
        x += advance;
    }
    (max_x.max(x), y + pitch)
}

impl RichText {
    /// As `text_layout::min_content_lines` breaks plain text: a line at
    /// every opportunity to break one, each run over its part of every line
    /// and a line's break in its last run. Unwrapped, its widest line is
    /// its min-content width.
    pub fn min_content_lines(&self) -> RichText {
        let text = self.text();
        let mut lines = String::with_capacity(text.len() + 16);
        let mut runs: Vec<TextRun> = Vec::with_capacity(self.runs().len() + 4);
        let mut start = 0;
        for end in crate::text_layout::break_opportunities(text) {
            if start > 0 {
                lines.push('\n');
                match runs.last_mut() {
                    Some(run) => run.range.end = lines.len(),
                    None => runs.extend(self.runs().first().cloned().map(|mut run| {
                        run.range = 0..lines.len();
                        run
                    })),
                }
            }
            let line = text[start..end].trim_end();
            let (from, at) = (start, lines.len());
            lines.push_str(line);
            for run in self.runs() {
                let (a, b) = (
                    run.range.start.max(from),
                    run.range.end.min(from + line.len()),
                );
                if a < b {
                    let mut part = run.clone();
                    part.range = at + (a - from)..at + (b - from);
                    runs.push(part);
                }
            }
            start = end;
        }
        self.remade(lines, runs).unwrap_or_else(|_| self.clone())
    }
}
#[cfg(test)]
mod tests {
    /// A grid in a rich run reads as no tracking: the estimate's 0.6 em.
    #[test]
    fn an_estimate_on_a_grid_is_a_plain_cell() {
        let run = TextRun {
            font: crate::text_layout::FontStyle {
                letter_spacing: crate::text_layout::LetterSpacing::cells(7.),
                ..Default::default()
            },
            ..TextRun::default()
        };
        assert_eq!(run.estimated_advance("a", 0), run.font_size * 0.6);
    }

    use super::*;
    fn run(range: Range<usize>) -> TextRun {
        TextRun {
            range,
            font: Default::default(),
            font_size: 16.,
            color: Color(255, 255, 255, 255),
            ..Default::default()
        }
    }
    /// `parts` as text with inline boxes: a `Some` part a box that big
    /// (width, height, baseline), its character a run of its own.
    /// A part: its text, or a box's (width, height, baseline).
    type Part<'a> = (&'a str, Option<(f32, f32, f32)>);
    fn boxed(parts: &[Part<'_>]) -> RichText {
        let (mut text, mut runs, mut boxes) = (String::new(), Vec::new(), Vec::new());
        for (part, inline) in parts {
            let at = text.len();
            match inline {
                Some((width, height, baseline)) => {
                    text.push(INLINE_BOX_CHAR);
                    boxes.push(InlineBox {
                        width: *width,
                        height: *height,
                        baseline: *baseline,
                        ..InlineBox::new(at)
                    });
                }
                None => text.push_str(part),
            }
            runs.push(run(at..text.len()));
        }
        RichText::new(text, runs)
            .unwrap()
            .with_inline_boxes(boxes)
            .unwrap()
    }

    /// A box must be at a replacement character that is a run of its own,
    /// boxes in order; sizes that aren't finite are zero.
    /// A box sized with no baseline stands on the line by its bottom, as
    /// CSS's inline-block with none does: not hung below the line.
    #[test]
    fn a_box_with_no_baseline_stands_on_its_bottom() {
        let text = format!("a{INLINE_BOX_CHAR}");
        let rich = RichText::new(text.as_str(), vec![run(0..1), run(1..4)])
            .unwrap()
            .with_inline_boxes(vec![InlineBox {
                width: 8.,
                height: 12.,
                ..InlineBox::new(1)
            }])
            .unwrap();
        assert_eq!(rich.inline_boxes()[0].baseline, 12.);
        let [(_, _, top)] = crate::text_layout::FallbackTextLayout::inline_places(&rich, None)[..]
        else {
            panic!()
        };
        // 16 points: the line's baseline, less the box above it.
        let line = crate::text_layout::LineHeight::NORMAL.resolve(16.);
        let baseline = (line - 16.) / 2. + 0.8 * 16.;
        assert!((top - (baseline - 12.)).abs() < 0.01, "{top} {baseline}");
    }
    /// Boxes found from where the last was: thousands in a paragraph take
    /// a pass over its runs, whatever order is wrong still refused.
    #[test]
    fn many_inline_boxes_are_found_in_one_pass() {
        let count = 4000;
        let text = INLINE_BOX_TEXT.repeat(count);
        let at = |i: usize| i * INLINE_BOX_TEXT.len();
        let runs: Vec<_> = (0..count).map(|i| run(at(i)..at(i + 1))).collect();
        let boxes: Vec<_> = (0..count).map(|i| InlineBox::new(at(i))).collect();
        let rich = RichText::new(text.as_str(), runs).unwrap();
        let laid = rich.clone().with_inline_boxes(boxes.clone()).unwrap();
        assert_eq!(laid.inline_boxes().len(), count);
        let mut wrong = boxes;
        wrong.swap(10, 11);
        assert!(rich.with_inline_boxes(wrong).is_err());
    }
    /// Every paragraph has one, behind an `Arc`: past 40 bytes it moves up
    /// a malloc size class, which x86's gate counts in text-heavy cells.
    #[test]
    fn rich_text_stays_forty_bytes() {
        assert_eq!(std::mem::size_of::<RichText>(), 40);
    }
    #[test]
    fn inline_boxes_are_validated() {
        let text = format!("a{INLINE_BOX_CHAR}b");
        let rich = || RichText::new(text.as_str(), vec![run(0..1), run(1..4), run(4..5)]).unwrap();
        assert!(rich().with_inline_boxes(vec![InlineBox::new(1)]).is_ok());
        assert!(rich().with_inline_boxes(vec![InlineBox::new(0)]).is_err());
        let merged = RichText::new(text.as_str(), vec![run(0..4), run(4..5)]).unwrap();
        assert!(merged.with_inline_boxes(vec![InlineBox::new(1)]).is_err());
        let two = format!("{INLINE_BOX_CHAR}{INLINE_BOX_CHAR}");
        let two = || RichText::new(two.as_str(), vec![run(0..3), run(3..6)]).unwrap();
        assert!(
            two()
                .with_inline_boxes(vec![InlineBox::new(3), InlineBox::new(0)])
                .is_err()
        );
        let wild = rich()
            .with_inline_boxes(vec![InlineBox {
                width: f32::NAN,
                height: -3.,
                baseline: f32::INFINITY,
                ..InlineBox::new(1)
            }])
            .unwrap();
        let b = wild.inline_boxes()[0];
        assert_eq!((b.width, b.height, b.baseline), (0., 0., 0.));
        assert!(rich().with_inline_boxes(Vec::new()).unwrap() == rich());
        // A box's size is part of the text's metrics.
        let sized = rich().with_inline_boxes(vec![InlineBox::new(1)]).unwrap();
        assert!(sized.metrics_differ(&sized.with_box_sizes([(4., 4., 4.)])));
        assert!(!sized.metrics_differ(&sized.with_box_sizes([(0., 0., 0.)])));
    }

    /// The headless fallback gives a box its width in the line, and a box
    /// taller than the text grows its line as CSS's line box does: as far
    /// above the baseline as the box, as far below as the text. Laid out,
    /// each box is where its room is; a line clamp leaves out those cut off.
    #[test]
    fn the_fallback_lays_inline_boxes_out() {
        use crate::text_layout::{FallbackTextLayout, TextLayout};
        let plain = RichText::new("ab", vec![run(0..2)]).unwrap();
        let rich = boxed(&[("a", None), ("", Some((40., 10., 10.))), ("b", None)]);
        let (w, h) = fallback_measure(&plain, None);
        let (bw, bh) = fallback_measure(&rich, None);
        assert!((bw - w - 40.).abs() < 0.01 && bh == h, "{bw} {bh}");
        let tall = boxed(&[("a", None), ("", Some((40., 50., 48.))), ("b", None)]);
        // 16 points in its normal line: 0.8 em and half the leading above
        // the baseline, 0.2 em and the other half below.
        let line = crate::text_layout::LineHeight::NORMAL.resolve(16.);
        let below = 0.2 * 16. + (line - 16.) / 2.;
        let (_, height) = fallback_measure(&tall, None);
        assert!((height - (48. + below)).abs() < 0.01, "{height}");
        let layout = FallbackTextLayout::with_rich(&tall, None);
        assert_eq!(layout.size().1, height);
        let [(at, x, top)] = FallbackTextLayout::inline_places(&tall, None)[..] else {
            panic!()
        };
        assert!(at == 1 && (x - w / 2.).abs() < 0.01 && top.abs() < 0.01);
        // Wrapped, it starts the next line, its top the first line's height
        // below the top.
        let [(at, x, top)] = FallbackTextLayout::inline_places(&rich, Some(w / 2. + 20.))[..]
        else {
            panic!()
        };
        assert_eq!((at, x), (1, 0.));
        assert!(top > h * 0.5, "{top}");
        let clamped = rich.clone().with_options(crate::text_layout::TextOptions {
            line_clamp: std::num::NonZeroU32::new(1),
            ..Default::default()
        });
        assert!(
            FallbackTextLayout::with_rich(&clamped, Some(w / 2. + 20.))
                .inline_boxes()
                .is_empty()
        );
        // Its min-content lines keep each box at its character.
        let lines = rich.min_content_lines();
        assert!(lines.has_inline_boxes(), "runs remade keep their boxes");
        assert_eq!(lines.inline_boxes().len(), 1);
        let at = lines.inline_boxes()[0].at;
        assert_eq!(&lines.text()[at..at + 3], INLINE_BOX_TEXT);
    }

    /// A line ending after a box leaves an empty line under it, as
    /// plain text's does.
    #[test]
    fn review_the_fallback_keeps_the_empty_line_after_a_box() {
        let one = boxed(&[("", Some((10., 10., 10.)))]);
        let two = boxed(&[("", Some((10., 10., 10.))), ("\n", None)]);
        let (_, h1) = fallback_measure(&one, None);
        let (_, h2) = fallback_measure(&two, None);
        let plain =
            |t: &str| fallback_measure(&RichText::new(t, vec![run(0..t.len())]).unwrap(), None).1;
        let pitch = plain("x\n") - plain("x");
        assert!(pitch > 0.);
        assert!((h2 - h1 - pitch).abs() < 0.01, "{h1} {h2} pitch {pitch}");
    }
    /// The headless fallback finds a box whose grapheme a combining
    /// mark joined.
    #[test]
    fn review_the_fallback_keeps_a_box_a_mark_follows() {
        use crate::text_layout::FallbackTextLayout;
        let rich = boxed(&[("", Some((40., 10., 10.))), ("\u{301}x", None)]);
        assert_eq!(rich.inline_boxes().len(), 1);
        assert_eq!(FallbackTextLayout::inline_places(&rich, None).len(), 1);
    }

    #[test]
    fn validates_unicode_coverage_sizes_and_empty_text() {
        assert!(RichText::new("é字", vec![run(0..2), run(2..5)]).is_ok());
        for ranges in [
            vec![0..1, 1..5],
            vec![0..2, 3..5],
            std::iter::once(0..2).collect(),
            vec![0..0, 0..5],
        ] {
            assert!(RichText::new("é字", ranges.into_iter().map(run).collect::<Vec<_>>()).is_err());
        }
        assert!(RichText::new("", Vec::new()).is_ok());
        for size in [0., -1., f32::NAN, f32::INFINITY] {
            let mut r = run(0..1);
            r.font_size = size;
            assert!(RichText::new("x", vec![r]).is_err());
        }
    }
}
