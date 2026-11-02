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
        let glyph = if matches!(grapheme, "\u{200B}" | "\u{2060}" | "\u{2063}") {
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

/// A box in a line (`RichText::with_boxes`): run `run` keeps its room (its
/// padding across, the run a character with no width of its own) and reaches
/// `above` the baseline and `below` it, the line growing to hold it, as a tall
/// glyph's would: an inline node's place.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineBox {
    /// The run that keeps its room.
    pub run: usize,
    /// How far it reaches above the baseline.
    pub above: f32,
    /// How far it reaches below the baseline.
    pub below: f32,
    /// The text it stands for: copied in its place, and read out.
    pub text: Arc<str>,
}

/// Validated, immutable styled display text. Runs cover the entire string exactly;
/// empty strings use no runs. Adjacent styles may cross grapheme boundaries: the
/// native shaper still shapes the paragraph continuously.
#[derive(Clone, Debug, PartialEq)]
pub struct RichText {
    options: crate::text_layout::TextOptions,
    text: Arc<str>,
    runs: Arc<[TextRun]>,
    /// Its runs that are boxes in the line, by run; most text has none.
    /// Behind one pointer: the text stays in its allocation's size class.
    boxes: Option<Arc<Vec<InlineBox>>>,
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
            text,
            runs,
            options: Default::default(),
            boxes: None,
        })
    }
    /// With `boxes`: runs that hold a box in the line (an inline node's
    /// room). Errs unless they are in run order, one a run, each a run of
    /// the text, reaching a finite distance no less than zero.
    pub fn with_boxes(mut self, boxes: impl Into<Vec<InlineBox>>) -> Result<Self, &'static str> {
        let boxes: Vec<InlineBox> = boxes.into();
        let mut next = 0;
        for inline in boxes.iter() {
            if inline.run < next || inline.run >= self.runs.len() {
                return Err("inline boxes must be on runs of the text, in order, one a run");
            }
            if !(inline.above.is_finite() && inline.below.is_finite())
                || inline.above < 0.
                || inline.below < 0.
            {
                return Err("an inline box must reach a finite distance no less than zero");
            }
            next = inline.run + 1;
        }
        self.boxes = (!boxes.is_empty()).then(|| Arc::new(boxes));
        Ok(self)
    }
    /// Its runs that are boxes in the line (`with_boxes`), in run order.
    pub fn boxes(&self) -> &[InlineBox] {
        self.boxes.as_deref().map_or(&[], Vec::as_slice)
    }
    /// Run `run`'s box, if it holds one.
    pub fn inline_box(&self, run: usize) -> Option<&InlineBox> {
        let boxes = self.boxes.as_deref()?;
        boxes
            .binary_search_by_key(&run, |inline| inline.run)
            .ok()
            .map(|at| &boxes[at])
    }
    /// `line`, run `run`'s line height, or its box's height if taller
    /// (`with_boxes`). Out of line: most text has no box.
    #[cold]
    #[inline(never)]
    pub(crate) fn box_line(&self, run: usize, line: f32) -> f32 {
        self.inline_box(run)
            .map_or(line, |inline| line.max(inline.above + inline.below))
    }
    /// Whether any run holds a box (`with_boxes`).
    #[inline]
    pub(crate) fn has_boxes(&self) -> bool {
        self.boxes.is_some()
    }
    /// With `options`: how text that doesn't fit ends, and its line clamp.
    pub fn with_options(mut self, options: crate::text_layout::TextOptions) -> Self {
        self.options = options;
        self
    }
    /// How text that doesn't fit ends, and its line clamp.
    pub fn options(&self) -> crate::text_layout::TextOptions {
        self.options
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
        self.options == other.options
            && self.text == other.text
            && self.boxes == other.boxes
            && self.runs.len() == other.runs.len()
            && self.runs.iter().zip(other.runs.iter()).all(|(a, b)| {
                a.range == b.range
                    && a.font == b.font
                    && a.font_size == b.font_size
                    && a.padding[0] == b.padding[0]
            })
    }
    /// `!same_metrics`, for `Scene::set_leaf_kind`: written apart so that
    /// `same_metrics` keeps the callers it is inlined into (a third one
    /// outlines it from `set_kind`).
    pub(crate) fn metrics_differ(&self, other: &Self) -> bool {
        self.options != other.options
            || self.text != other.text
            || self.boxes != other.boxes
            || self.runs.len() != other.runs.len()
            || self.runs.iter().zip(other.runs.iter()).any(|(a, b)| {
                a.range != b.range
                    || a.font != b.font
                    || a.font_size != b.font_size
                    || a.padding[0] != b.padding[0]
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
            .saturating_add(std::mem::size_of_val(self.boxes()))
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
}

/// Approximate, allocation-free headless metrics. Native hosts replace this with
/// full paragraph shaping; grapheme widths here follow the fallback text model.
pub fn fallback_measure(rich: &RichText, width: Option<f32>) -> (f32, f32) {
    use unicode_segmentation::UnicodeSegmentation;
    if rich.text.is_empty() {
        return (0., 0.);
    }
    let mut run_index = 0;
    let limit = rich
        .options
        .line_clamp
        .map(|n| n.get())
        .or((rich.options.overflow == crate::text_layout::TextOverflow::Ellipsis).then_some(1));
    let mut row = 1;
    let (mut x, mut y, mut max_x, mut pitch) = (0_f32, 0_f32, 0_f32, 0_f32);
    for (index, grapheme) in rich.text.grapheme_indices(true) {
        while rich.runs[run_index].range.end <= index {
            run_index += 1;
        }
        let run = &rich.runs[run_index];
        let mut line = run.font.line_height.resolve(run.font_size);
        if rich.has_boxes() {
            line = rich.box_line(run_index, line);
        }
        if grapheme == "\n" || grapheme == "\r\n" || grapheme == "\r" {
            if limit.is_some_and(|limit| row >= limit) {
                return (
                    width.map_or(max_x.max(x), |w| {
                        max_x
                            .max(
                                x + if rich.options.overflow
                                    == crate::text_layout::TextOverflow::Ellipsis
                                {
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
                        x + if rich.options.overflow == crate::text_layout::TextOverflow::Ellipsis {
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
        let mut boxes: Vec<InlineBox> = Vec::new();
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
            for (index, run) in self.runs().iter().enumerate() {
                let (a, b) = (
                    run.range.start.max(from),
                    run.range.end.min(from + line.len()),
                );
                if a < b {
                    let mut part = run.clone();
                    part.range = at + (a - from)..at + (b - from);
                    if let Some(inline) = self.inline_box(index) {
                        boxes.push(InlineBox {
                            run: runs.len(),
                            ..inline.clone()
                        });
                    }
                    runs.push(part);
                }
            }
            start = end;
        }
        RichText::new(lines, runs)
            .and_then(|lines| lines.with_boxes(boxes))
            .unwrap_or_else(|_| self.clone())
    }
}
#[cfg(test)]
mod tests {
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
