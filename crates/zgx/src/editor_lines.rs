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
    }
    /// Whether it changes how its text is shaped (and so its width or height).
    fn shapes(&self) -> bool {
        self.weight.is_some()
            || self.italic
            || self.size.is_some()
            || self.line_height.is_some()
            || self.mono
            || self.padding.is_some()
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
}
impl LineInset {
    /// The width the line wraps in when the text wraps at `wrap`: its own, or what its indent leaves.
    fn wrap(&self, wrap: Option<f32>) -> Option<f32> {
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
    let runs: Vec<_> = runs(line, spans)
        .into_iter()
        .map(|(range, style)| {
            let style = style.unwrap_or_default();
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
            crate::rich_text::TextRun {
                range,
                font,
                font_size: style.size.unwrap_or(size),
                color: style.color.unwrap_or(color),
                background: style.background,
                padding: style.padding.unwrap_or_default(),
                radius: style.radius.unwrap_or_default(),
                underline: style
                    .underline
                    .then(|| crate::rich_text::Decoration::new(1.)),
                strikethrough: style.strike.then(|| crate::rich_text::Decoration::new(1.)),
                ..Default::default()
            }
        })
        .collect();
    crate::rich_text::RichText::new(line, runs).ok()
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
type ShapeKey = Box<[(u32, u32, u16, bool, u32, u32, bool)]>;
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
        self.highlighter.borrow().is_some()
    }
    /// Whether a line's spans shape it now that did not before: what was
    /// laid out without them is laid out again.
    pub(crate) fn take_restyled(&self) -> bool {
        self.restyled.replace(false)
    }
    /// Line `i`'s spans, if it has a highlighter: what the lines above
    /// it leave found first (each once, until one changes).
    pub(crate) fn spans(&self, layout: &LinesLayout, i: usize) -> Option<Rc<LineSpans>> {
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
    /// Of lines `own` high, those at `joins` (in order) sharing a row with the next.
    fn with_rows(own: Vec<f32>, joins: &[usize]) -> Self {
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
            let tallest = own[start..=last].iter().copied().fold(0., f32::max);
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
        };
        // every line's own height, unwrapped too: a heading's taller pitch and a line's room push the lines below
        {
            let width = wrap.unwrap_or(f32::INFINITY);
            let known = layout.cache.heights.borrow();
            let mut joins = Vec::new();
            let each = (0..layout.line_count())
                .map(|i| {
                    let inset = layout.inset(i);
                    if inset.joins_next {
                        joins.push(i);
                    }
                    layout.measured(i, &inset, &known, width)
                })
                .collect();
            drop(known);
            layout.heights = Some(RefCell::new(Heights::with_rows(each, &joins)));
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
                joined |= inset.joins_next;
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
                let joins = |i: usize| i + 1 < count && layout.inset(i).joins_next;
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
                    let tallest = own[row..=last].iter().copied().fold(0., f32::max);
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
        // a cell wraps in its own width: its text's height elsewhere is not its
        let bare = match inset.width {
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
            Some(heights) => heights.borrow().above(i) as f32,
            None => i as f32 * self.row,
        }
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
            while start > 0 && self.inset(start - 1).joins_next {
                start -= 1;
            }
            while end < count && self.inset(end - 1).joins_next {
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
        let mut found = None;
        for j in row {
            let inset = self.inset(j);
            if inset.hidden {
                continue;
            }
            if found.is_none() || inset.left <= x {
                found = Some(j);
            }
        }
        (found.unwrap_or(i), true)
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
    pub(crate) fn draw_width(&self, i: usize, wrap: Option<f32>) -> Option<f32> {
        let inset = self.inset(i);
        inset
            .width
            .map(|w| w.max(1.))
            .or(wrap.map(|w| (w - inset.left).max(1.)))
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
        let key = if let Some(width) = inset.width {
            std::borrow::Cow::Owned(format!("{}\u{1}{}", width.to_bits(), &*text))
        } else if inset.left > 0. {
            std::borrow::Cow::Owned(format!("{}\u{0}{}", inset.left.to_bits(), &*text))
        } else {
            std::borrow::Cow::Borrowed(&*text)
        };
        let layout = match self.cache.styled(self, i, &text, wrap, &key) {
            Some(layout) => layout,
            None => self.cache.line(&text, &self.shape, wrap, &key),
        };
        let (width, height) = layout.size();
        if width > self.widest.get() {
            self.widest.set(width);
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
                let tallest = heights.own[row.clone()].iter().copied().fold(0., f32::max);
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
            rect.x + inset.left,
            rect.y + self.top(i) + inset.above,
            rect.width,
            rect.height,
        )
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
        let (i, cell) = self.line_at_point(x, y);
        let layout = self.line(i);
        let inset = self.inset(i);
        let mut down = y.max(0.) - self.top(i) - inset.above;
        if cell {
            // under a cell shorter than its row: its last row at x, as beside it
            down = down.min(layout.size().1 - 0.5);
        }
        let local = layout.hit_position(x - inset.left, down);
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
        let top = self.top(i);
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
