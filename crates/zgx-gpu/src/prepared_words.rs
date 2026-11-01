//! Plain text prepared from its words, without a cosmic-text line.
//!
//! A new text (a label whose value changed) made a `BufferLine`, its
//! attribute list, a `ShapeLine` of spans and words, each word looked up in
//! cosmic-text's shape-run cache and its glyphs copied in, then laid out by
//! `layout_to_buffer`: shaping itself was a few percent of it, the rest the
//! line rebuilt around the cached words. Here a paragraph cosmic-text shapes
//! as one left-to-right span (`cosmic_text::plain_words` says which, and
//! where its words are, as `ShapeSpan::build` finds them) keeps its words as
//! shapes shared by every text holding them (`WordCache`), each shaped once,
//! alone, by `ShapeWord`. It is measured from them (`Compact::of_words`) and
//! laid out from them, wrapping as `layout_to_buffer` wraps (word, else
//! glyph) and placing glyphs as it places a left-to-right line's.
use cosmic_text::{Align, AttrsList, FontSystem, LayoutLine, ShapeGlyph, ShapeWord};
use rustc_hash::{FxHashMap, FxHasher};
use std::{
    hash::{Hash, Hasher},
    rc::Rc,
};
use zgx::text_layout::{FontFeatures, FontStyle};

/// Word shapes kept at most (estimated bytes): past it the cache starts
/// again (texts holding shapes keep theirs).
const BUDGET: usize = 4 * 1024 * 1024;

/// A word as cosmic-text shapes it: its glyphs, their byte ranges from its
/// start; its width, as `ShapeWord::width` adds it.
pub(super) struct Shape {
    pub glyphs: Box<[ShapeGlyph]>,
    pub blank: bool,
    pub width: f32,
}

/// A plain paragraph's words: where each starts, and its shape.
pub(super) struct Words {
    pub words: Vec<(usize, Rc<Shape>)>,
    pub font_size: f32,
    align: Option<Align>,
    /// Laid out at this wrap width (`lay_out`), into `lines`.
    laid: Option<Option<f32>>,
    pub lines: Vec<LayoutLine>,
}

/// A run of a paragraph in one font, as cosmic-text shapes it: to byte
/// `end`, in `font` at `size`. A rich text's span also sets its metrics
/// (line height) and metadata (its first run), as `rich_buffer_unshaped`
/// gives them; plain text's sets neither.
pub(super) struct Run<'a> {
    pub end: usize,
    pub font: &'a FontStyle,
    pub size: f32,
    pub span: Option<(f32, usize)>,
}

/// What a word is shaped in: a style, its size, and a span's line height
/// and metadata (`Run`).
type Font = (FontStyle, u32, Option<(u32, usize)>);

/// Every word shaped, by font, size and text.
#[derive(Default)]
pub(super) struct WordCache {
    /// Each style and size words are shaped in, with its attributes.
    fonts: Vec<(Font, AttrsList)>,
    shapes: FxHashMap<u64, (u32, Box<str>, Rc<Shape>)>,
    bytes: usize,
}

/// Whether text in `font` can be laid out from its words: its attributes
/// are one span (no fallback families), and nothing changes its glyphs
/// after shaping (masking, letter spacing, hyphenation).
pub(super) fn qualifies(font: &FontStyle) -> bool {
    font.fallbacks.is_empty()
        && !font.masked
        && font.letter_spacing.pixels() == 0.
        && !font
            .features
            .settings()
            .iter()
            .any(|(tag, _)| *tag == FontFeatures::HYPHENS)
}

impl WordCache {
    /// Keep only the words a prepared text holds: what was shown and went
    /// is shaped again if it comes back (cosmic-text's run cache, swept by
    /// age, may still hold it), and memory goes back to what is shown.
    pub fn sweep(&mut self) {
        self.shapes
            .retain(|_, (_, _, shape)| Rc::strong_count(shape) > 1);
        self.bytes = self
            .shapes
            .values()
            .map(|(_, word, shape)| shape.glyphs.len() * size_of::<ShapeGlyph>() + word.len() + 64)
            .sum();
        if self.shapes.capacity() > 2 * self.shapes.len() + 64 {
            self.shapes.shrink_to_fit();
        }
    }
    /// How many words it holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.shapes.len()
    }
    fn font(&mut self, run: &Run<'_>) -> u32 {
        let size = run.size.max(1.);
        let bits = size.to_bits();
        let span = run
            .span
            .map(|(height, metadata)| (height.to_bits(), metadata));
        if let Some(at) = self.fonts.iter().position(|((style, size, kept), _)| {
            *size == bits && *kept == span && style == run.font
        }) {
            return at as u32;
        }
        let mut attrs = crate::text::attrs(run.font, size);
        if let Some((height, metadata)) = run.span {
            attrs = attrs
                .metrics(cosmic_text::Metrics::new(size, height))
                .metadata(metadata);
        }
        let attrs = AttrsList::new(&attrs);
        self.fonts.push(((run.font.clone(), bits, span), attrs));
        self.fonts.len() as u32 - 1
    }
    /// `text`'s words in its `runs` (in order, the last ending at its end),
    /// shaped, where it is a paragraph laid out from them (`qualifies`,
    /// `plain_words`, each word inside one run), at `font_size` and aligned
    /// as `base` says (its buffer's): in one of `spare`'s, the lists of
    /// texts dropped, where it has one.
    // Boxed: an entry holds its words boxed (it stays small), and a spare
    // goes back into one without being boxed again.
    #[allow(clippy::vec_box)]
    pub fn words(
        &mut self,
        fonts: &mut FontSystem,
        text: &str,
        runs: &[Run<'_>],
        base: &FontStyle,
        font_size: f32,
        spare: &mut Vec<Box<Words>>,
    ) -> Option<Box<Words>> {
        if runs.iter().any(|run| !qualifies(run.font)) {
            return None;
        }
        let size = font_size.max(1.);
        let ids: smallvec::SmallVec<[u32; 4]> = runs.iter().map(|run| self.font(run)).collect();
        let mut ranges: smallvec::SmallVec<[(usize, usize, bool); 16]> = smallvec::SmallVec::new();
        // (`plain_line` was asked first: `TextCache::words_of`.)
        if !cosmic_text::line_words(text, |range, blank| {
            ranges.push((range.start, range.end, blank));
        }) {
            return None;
        }
        let mut kept = spare.pop().unwrap_or_else(|| {
            Box::new(Words {
                words: Vec::new(),
                font_size: size,
                align: None,
                laid: None,
                lines: Vec::new(),
            })
        });
        let words = &mut kept.words;
        words.clear();
        words.reserve(ranges.len());
        let mut run = 0;
        for (start, end, blank) in ranges {
            while runs[run].end <= start {
                run += 1;
            }
            // A word across runs is shaped in parts by cosmic-text.
            if end > runs[run].end {
                words.clear();
                spare.push(kept);
                return None;
            }
            let id = ids[run];
            let word = &text[start..end];
            let mut hasher = FxHasher::default();
            id.hash(&mut hasher);
            blank.hash(&mut hasher);
            word.hash(&mut hasher);
            let hash = hasher.finish();
            let shape = match self.shapes.get(&hash) {
                Some((font, text, shape)) if *font == id && **text == *word => shape.clone(),
                _ => {
                    let shaped = ShapeWord::plain(fonts, word, &self.fonts[id as usize].1, blank);
                    // A word whose glyphs set different line heights is
                    // measured from shaped lines (`Compact::of`).
                    let height = |glyph: &ShapeGlyph| glyph.metrics_opt.map(|m| m.line_height);
                    let first = shaped.glyphs.first().and_then(height);
                    if shaped.glyphs.iter().any(|glyph| height(glyph) != first) {
                        words.clear();
                        spare.push(kept);
                        return None;
                    }
                    let shape = Rc::new(Shape {
                        width: shaped.width(size),
                        blank: shaped.blank,
                        glyphs: shaped.glyphs.into_boxed_slice(),
                    });
                    let bytes = shape.glyphs.len() * size_of::<ShapeGlyph>() + word.len() + 64;
                    if self.bytes + bytes > BUDGET {
                        self.shapes.clear();
                        self.bytes = 0;
                    }
                    self.bytes += bytes;
                    self.shapes.insert(hash, (id, word.into(), shape.clone()));
                    shape
                }
            };
            words.push((start, shape));
        }
        kept.font_size = size;
        kept.align = crate::text::alignment(base);
        kept.laid = None;
        Some(kept)
    }
}

/// Where a visual line starts or ends: a word, and a glyph in it.
type At = (usize, usize);

impl Words {
    /// Its glyphs' widths, as `Compact` keeps them.
    fn advances(&self, word: usize) -> impl Iterator<Item = f32> + '_ {
        let size = self.font_size;
        self.words[word].1.glyphs.iter().map(move |g| g.width(size))
    }
    /// Its visual lines at wrap width `width`, as `layout_to_buffer` breaks
    /// a line of one span (word, else glyph): each one's start and end and
    /// width. The steps `Compact::size` takes.
    fn visual_lines(&self, width: Option<f32>) -> smallvec::SmallVec<[(At, At, f32); 2]> {
        let limit = width.unwrap_or(f32::INFINITY);
        let mut lines = smallvec::SmallVec::new();
        // The line being filled: its width, and the one range it takes.
        let mut line: Option<(At, At, f32)> = None;
        let mut line_w = 0_f32;
        let add =
            |line: &mut Option<(At, At, f32)>, line_w: &mut f32, start: At, end: At, w: f32| {
                if start != end {
                    *line_w += w;
                    *line = Some((start, end, *line_w));
                }
            };
        let push = |line: &mut Option<(At, At, f32)>,
                    line_w: &mut f32,
                    lines: &mut smallvec::SmallVec<[(At, At, f32); 2]>| {
            if let Some(done) = line.take() {
                lines.push(done);
            }
            *line_w = 0.;
        };
        let words = &self.words;
        let mut fitting = (0, 0);
        let mut range_width = 0_f32;
        let mut before_last_blank = 0_f32;
        for (i, (_, word)) in words.iter().enumerate() {
            if line_w + (range_width + word.width) <= limit
                || (word.blank && line_w + range_width <= limit)
            {
                if word.blank {
                    before_last_blank = range_width;
                }
                range_width += word.width;
            } else if word.width > limit {
                if range_width > 0. {
                    add(&mut line, &mut line_w, fitting, (i, 0), range_width);
                    push(&mut line, &mut line_w, &mut lines);
                    range_width = 0.;
                    fitting = (i, 0);
                }
                for (glyph, advance) in self.advances(i).enumerate() {
                    if line_w + (range_width + advance) <= limit {
                        range_width += advance;
                    } else {
                        add(&mut line, &mut line_w, fitting, (i, glyph), range_width);
                        push(&mut line, &mut line_w, &mut lines);
                        range_width = advance;
                        fitting = (i, glyph);
                    }
                }
            } else {
                if range_width > 0. {
                    // A blank before the break is left out.
                    if i > 0 && words[i - 1].1.blank {
                        add(
                            &mut line,
                            &mut line_w,
                            fitting,
                            (i - 1, 0),
                            before_last_blank,
                        );
                    } else {
                        add(&mut line, &mut line_w, fitting, (i, 0), range_width);
                    }
                }
                push(&mut line, &mut line_w, &mut lines);
                if word.blank {
                    range_width = 0.;
                    fitting = (i + 1, 0);
                } else {
                    range_width = word.width;
                    fitting = (i, 0);
                }
            }
        }
        add(
            &mut line,
            &mut line_w,
            fitting,
            (words.len(), 0),
            range_width,
        );
        push(&mut line, &mut line_w, &mut lines);
        lines
    }
    /// Laid out at `width` (the wrap width given cosmic-text): its glyphs
    /// placed as `layout_to_buffer` places a left-to-right line's.
    pub fn lay_out(&mut self, width: Option<f32>) {
        if self.laid == Some(width) {
            return;
        }
        self.laid = Some(width);
        let visual = self.visual_lines(width);
        let font_size = self.font_size;
        let line_width =
            width.unwrap_or_else(|| visual.iter().fold(0_f32, |w, line| w.max(line.2)));
        // Its lines from before are filled again, their lists kept.
        let mut lines = std::mem::take(&mut self.lines);
        let mut made = 0;
        for (start, end, w) in visual {
            let mut x = match self.align.unwrap_or(Align::Left) {
                Align::Left | Align::Justified => 0.,
                Align::Right | Align::End => (line_width - w).max(0.),
                Align::Center => (line_width - w).max(0.) / 2.0,
            };
            let mut y = 0.;
            let (mut max_ascent, mut max_descent) = (0_f32, 0_f32);
            if made == lines.len() {
                lines.push(LayoutLine {
                    w: 0.,
                    max_ascent: 0.,
                    max_descent: 0.,
                    line_height_opt: None,
                    glyphs: Vec::new(),
                    decorations: Vec::new(),
                });
            }
            let line = &mut lines[made];
            made += 1;
            let glyphs = &mut line.glyphs;
            glyphs.clear();
            let words = start.0..end.0 + usize::from(end.1 != 0);
            let count: usize = self.words[words.clone()]
                .iter()
                .map(|(_, word)| word.glyphs.len())
                .sum();
            glyphs.reserve_exact(count);
            for i in words {
                let (offset, word) = &self.words[i];
                let all = &word.glyphs[..];
                let included = match (i == start.0, i == end.0) {
                    (false, false) => all,
                    (true, false) => &all[start.1..],
                    (false, true) => &all[..end.1],
                    (true, true) => &all[start.1..end.1],
                };
                for glyph in included {
                    let glyph_font_size = glyph.metrics_opt.map_or(font_size, |m| m.font_size);
                    let x_advance = glyph_font_size.mul_add(glyph.x_advance, 0.0);
                    let mut laid = glyph.laid(
                        glyph_font_size,
                        glyph.metrics_opt.map(|m| m.line_height),
                        x,
                        y,
                        x_advance,
                    );
                    laid.start += offset;
                    laid.end += offset;
                    glyphs.push(laid);
                    x += x_advance;
                    y += glyph_font_size * glyph.y_advance;
                    max_ascent = max_ascent.max(glyph_font_size * glyph.ascent);
                    max_descent = max_descent.max(glyph_font_size * glyph.descent);
                }
            }
            line.line_height_opt = glyphs
                .iter()
                .filter_map(|glyph| glyph.line_height_opt)
                .reduce(f32::max);
            line.w = w;
            line.max_ascent = max_ascent;
            line.max_descent = max_descent;
        }
        if made == 0 {
            // No visual line: one empty, as `layout_to_buffer` gives.
            if lines.is_empty() {
                lines.push(LayoutLine {
                    w: 0.,
                    max_ascent: 0.,
                    max_descent: 0.,
                    line_height_opt: None,
                    glyphs: Vec::new(),
                    decorations: Vec::new(),
                });
            }
            let line = &mut lines[0];
            line.glyphs.clear();
            (
                line.w,
                line.max_ascent,
                line.max_descent,
                line.line_height_opt,
            ) = (0., 0., 0., None);
            made = 1;
        }
        lines.truncate(made);
        self.lines = lines;
    }
    /// Keep its lists for another text (`WordCache::words`): its words let
    /// go, its lines' glyph lists only if short.
    pub fn recycle(mut self: Box<Self>) -> Option<Box<Self>> {
        self.words.clear();
        self.lines.truncate(4);
        for line in &mut self.lines {
            if line.glyphs.capacity() > 256 {
                line.glyphs = Vec::new();
            }
        }
        (self.words.capacity() <= 256).then_some(self)
    }
    /// Estimated bytes it holds laid out: the same all its life, so the
    /// cache's count of them holds (a layout glyph for each of its glyphs).
    pub fn bytes(&self) -> usize {
        let glyphs: usize = self.words.iter().map(|(_, word)| word.glyphs.len()).sum();
        256 + self.words.len() * 16 + glyphs * size_of::<cosmic_text::LayoutGlyph>()
    }
}
