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
    sync::Arc,
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
    // Shared by `Arc`: a `TextCache` is `Send` and `Sync`, as before words.
    pub words: Vec<(usize, Arc<Shape>)>,
    /// Its words' glyphs, all told (`bytes`).
    glyphs: usize,
    pub font_size: f32,
    align: Option<Align>,
    /// Laid out at this wrap width (`lay_out`), into `lines`.
    laid: Option<Option<f32>>,
    pub lines: Vec<LayoutLine>,
    /// Line breaking's state after each word a line ended at, laid out at
    /// `laid` (`visual_lines`): where it resumes for a text continuing this.
    breaks: Vec<Break>,
    /// Taken from the paragraph this one grew from (`WordCache::words`):
    /// its lines laid out at this width hold to this state, and the next
    /// `lay_out` at that width starts from it.
    resume: Option<(Option<f32>, Break)>,
    /// The length of the text it grew from, its lines taken with its words
    /// (`Grew::Owned`), and how many of them its last `lay_out` kept: those
    /// are that text's first lines at the same width, as they were.
    pub grew: Option<(usize, usize)>,
}

/// Line breaking's state once it has read the words before `next`, a line
/// having ended among the last it read: the same for any text whose words
/// begin with those, so breaking such a text resumes here.
#[derive(Clone, Copy)]
pub(super) struct Break {
    next: usize,
    /// Lines ended before it.
    lines: usize,
    line: Option<(At, At, f32)>,
    line_w: f32,
    fitting: At,
    range_width: f32,
    before_last_blank: f32,
}
impl Break {
    const START: Break = Break {
        next: 0,
        lines: 0,
        line: None,
        line_w: 0.,
        fitting: (0, 0),
        range_width: 0.,
        before_last_blank: 0.,
    };
}

/// The words of the paragraph a text grew from (`WordCache::words`):
/// borrowed and copied from, or its own, taken from a text that goes as
/// this is prepared (a stream's last state), its lines with it.
pub(super) enum Grew<'a> {
    Shared(&'a Words),
    Owned(Box<Words>, usize),
}

/// A run of a paragraph in one font, as cosmic-text shapes it: to byte
/// `end`, in `font` at `size`. A rich text's span also sets its metrics
/// (line height) and metadata, as `rich_buffer_unshaped` gives them (its
/// metadata here one for all: nothing reads a paragraph's but the
/// ellipsis's, which it has none of, and its words then hold whatever runs
/// its paint splits into); plain text's sets neither. `padding`: a padded
/// run's room before and after it, in ems (`Attrs::padding`).
pub(super) struct Run<'a> {
    pub end: usize,
    pub font: &'a FontStyle,
    pub size: f32,
    pub span: Option<(f32, usize)>,
    pub padding: f32,
}

/// What a word is shaped in: a style, its size, and a span's line height
/// and metadata (`Run`).
type Font = (FontStyle, u32, Option<(u32, usize)>);

/// Every word shaped, by font, size and text.
#[derive(Default)]
pub(super) struct WordCache {
    /// Each style and size words are shaped in, with its attributes.
    fonts: Vec<(Font, AttrsList)>,
    shapes: FxHashMap<u64, (u32, Box<str>, Arc<Shape>)>,
    bytes: usize,
}

/// Whether text in `font` can be laid out from its words: its attributes
/// are one span (no fallback families), and nothing changes its glyphs
/// after shaping (masking, letter spacing, hyphenation).
pub(super) fn qualifies(font: &FontStyle) -> bool {
    font.fallbacks.is_empty()
        && !font.masked
        && font.letter_spacing == zgx::text_layout::LetterSpacing::ZERO
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
            .retain(|_, (_, _, shape)| Arc::strong_count(shape) > 1);
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
    ///
    /// `grew`: the words of a paragraph in the same runs that `text` starts
    /// with (a stream's last state), and where they hold to
    /// (`Words::resume_at`): those before are taken, and only the rest of
    /// `text` is read, its words looked up and shaped. Taken whole where
    /// its own (`Grew::Owned`, with that text's length): only its words
    /// from there go, and its lines are laid out again only from where they
    /// may change (`lay_out`). With the words, how many were taken so.
    // Boxed: an entry holds its words boxed (it stays small), and a spare
    // goes back into one without being boxed again.
    #[allow(clippy::vec_box, clippy::too_many_arguments)]
    #[inline(always)]
    pub fn words(
        &mut self,
        fonts: &mut FontSystem,
        text: &str,
        runs: &[Run<'_>],
        base: &FontStyle,
        font_size: f32,
        spare: &mut Vec<Box<Words>>,
        grew: Option<(Grew<'_>, usize)>,
    ) -> Option<(Box<Words>, usize)> {
        if runs.iter().any(|run| !qualifies(run.font)) {
            if let Some((Grew::Owned(words, _), _)) = grew {
                spare.extend(words.recycle());
            }
            return None;
        }
        let size = font_size.max(1.);
        let ids: smallvec::SmallVec<[u32; 4]> = runs.iter().map(|run| self.font(run)).collect();
        let mut ranges: smallvec::SmallVec<[(usize, usize, bool); 16]> = smallvec::SmallVec::new();
        // (`plain_line` was asked first: `TextCache::words_of`.) Read from
        // where the words it grew from stop holding.
        let from = match &grew {
            None => 0,
            Some((Grew::Shared(grew), at)) => grew.words[*at].0,
            Some((Grew::Owned(grew, _), at)) => grew.words[*at].0,
        };
        if !cosmic_text::line_words(&text[from..], |range, blank| {
            ranges.push((from + range.start, from + range.end, blank));
        }) {
            if let Some((Grew::Owned(words, _), _)) = grew {
                spare.extend(words.recycle());
            }
            return None;
        }
        let new = || {
            Box::new(Words {
                words: Vec::new(),
                glyphs: 0,
                font_size: size,
                align: None,
                laid: None,
                lines: Vec::new(),
                breaks: Vec::new(),
                resume: None,
                grew: None,
            })
        };
        let (mut kept, taken) = match grew {
            None => {
                let mut kept = spare.pop().unwrap_or_else(new);
                kept.words.clear();
                kept.glyphs = 0;
                kept.words.reserve(ranges.len());
                (kept, 0)
            }
            Some((Grew::Shared(grew), at)) => {
                let mut kept = spare.pop().unwrap_or_else(new);
                let words = &mut kept.words;
                words.clear();
                // Room for all of it at once: reserved after the words taken,
                // it would grow by doubling, and keep what it does not use.
                words.reserve(at + ranges.len());
                words.extend_from_slice(&grew.words[..at]);
                kept.glyphs = kept.words.iter().map(|(_, word)| word.glyphs.len()).sum();
                (kept, at)
            }
            // Its own: the words from `at` go, the rest stay where they are.
            Some((Grew::Owned(mut kept, length), at)) => {
                kept.keep(at, length);
                kept.words.reserve(ranges.len());
                (kept, at)
            }
        };
        let mut run = 0;
        for (start, end, blank) in ranges {
            while runs[run].end <= start {
                run += 1;
            }
            let shape = match end <= runs[run].end && runs[run].padding == 0. {
                true => self.shape(fonts, ids[run], &text[start..end], blank, size),
                // Across runs, or padded: in parts, as cosmic-text shapes it.
                false => self.composed(fonts, text, (start, end), blank, size, (runs, &ids, run)),
            };
            let Some(shape) = shape else {
                spare.extend(kept.recycle());
                return None;
            };
            kept.glyphs += shape.glyphs.len();
            kept.words.push((start, shape));
        }
        // (Lines taken in another size or alignment are laid out again.)
        let align = crate::text::alignment(base);
        if (kept.font_size, kept.align) != (size, align) {
            (kept.font_size, kept.align) = (size, align);
            kept.resume = None;
        }
        kept.laid = None;
        Some((kept, taken))
    }
}

impl WordCache {
    /// `word` shaped in font `id`, found or shaped once; None where its
    /// glyphs set different line heights (it is measured from shaped lines:
    /// `Compact::of`).
    #[inline(always)]
    fn shape(
        &mut self,
        fonts: &mut FontSystem,
        id: u32,
        word: &str,
        blank: bool,
        size: f32,
    ) -> Option<Arc<Shape>> {
        let mut hasher = FxHasher::default();
        id.hash(&mut hasher);
        blank.hash(&mut hasher);
        word.hash(&mut hasher);
        let hash = hasher.finish();
        if let Some((font, text, shape)) = self.shapes.get(&hash)
            && *font == id
            && **text == *word
        {
            return Some(shape.clone());
        }
        self.shaped(fonts, (id, hash), word, blank, size)
    }
    /// `shape` of a word not shaped before: shaped and kept.
    #[inline(never)]
    fn shaped(
        &mut self,
        fonts: &mut FontSystem,
        (id, hash): (u32, u64),
        word: &str,
        blank: bool,
        size: f32,
    ) -> Option<Arc<Shape>> {
        let shaped = ShapeWord::plain(fonts, word, &self.fonts[id as usize].1, blank);
        if !one_line_height(&shaped.glyphs) {
            return None;
        }
        let shape = Arc::new(Shape {
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
        Some(shape)
    }
    /// The word of `text` at `start..end` beginning in run `first` of
    /// `runs` (fonts `ids`), as cosmic-text shapes a word that crosses runs
    /// or is padded (`ShapeWord::build`, `pad`): each part in runs of one
    /// font alone (parts in other fonts, `Attrs::compatible`, meeting where
    /// a grapheme starts; runs in compatible fonts of another size, which
    /// cosmic-text shapes as one, are not taken), then a padded run's first
    /// glyph widened before and its last after (a chip split in two by a
    /// fade, each half's). Its own shape, not kept: only a stream's last
    /// words are made so again.
    #[allow(clippy::type_complexity)]
    fn composed(
        &mut self,
        fonts: &mut FontSystem,
        text: &str,
        (start, end): (usize, usize),
        blank: bool,
        size: f32,
        (runs, ids, first): (&[Run<'_>], &[u32], usize),
    ) -> Option<Arc<Shape>> {
        let word = &text[start..end];
        let mut glyphs: Vec<ShapeGlyph> = Vec::new();
        let (mut at, mut run) = (start, first);
        while at < end {
            // This part: to the first run in another font.
            let mut last = run;
            while runs[last].end < end && ids[last + 1] == ids[run] {
                last += 1;
            }
            let part_end = end.min(runs[last].end);
            if part_end < end {
                let (before, now) = (runs[last].font, runs[last + 1].font);
                let compatible = before.family == now.family
                    && before.weight == now.weight
                    && before.italic == now.italic;
                let grapheme =
                    unicode_segmentation::UnicodeSegmentation::grapheme_indices(word, true)
                        .any(|(i, _)| start + i == part_end);
                if compatible || !grapheme {
                    return None;
                }
            }
            let part = self.shape(fonts, ids[run], &text[at..part_end], blank, size)?;
            glyphs.extend(part.glyphs.iter().map(|glyph| {
                let mut glyph = glyph.clone();
                glyph.start += at - start;
                glyph.end += at - start;
                glyph
            }));
            at = part_end;
            run = last + 1;
        }
        // As `pad`: a padded run's glyph at its start widened before it,
        // one ending where it does after.
        let mut run = first;
        for glyph in &mut glyphs {
            let byte = start + glyph.start;
            while runs[run].end <= byte {
                run += 1;
            }
            let padding = runs[run].padding;
            if padding <= 0. {
                continue;
            }
            let run_start = match run {
                0 => 0,
                _ => runs[run - 1].end,
            };
            if byte == run_start {
                glyph.x_advance += padding;
                glyph.x_offset += padding;
            }
            if start + glyph.end >= text.len() || start + glyph.end == runs[run].end {
                glyph.x_advance += padding;
            }
        }
        if !one_line_height(&glyphs) {
            return None;
        }
        let width = glyphs
            .iter()
            .fold(0., |width, glyph| width + glyph.width(size));
        Some(Arc::new(Shape {
            width,
            blank,
            glyphs: glyphs.into_boxed_slice(),
        }))
    }
}

/// Whether `glyphs` set one line height (or none): a word whose glyphs set
/// different ones is measured from shaped lines (`Compact::of`).
fn one_line_height(glyphs: &[ShapeGlyph]) -> bool {
    let height = |glyph: &ShapeGlyph| glyph.metrics_opt.map(|m| m.line_height);
    let first = glyphs.first().and_then(height);
    glyphs.iter().all(|glyph| height(glyph) == first)
}

/// Where a visual line starts or ends: a word, and a glyph in it.
type At = (usize, usize);

impl Words {
    /// Its first `at` words kept, as the start of a text continuing the one
    /// of `length` bytes it holds the words of: the rest go, and its lines
    /// laid out hold to the last line ended among the words kept (`resume`).
    fn keep(&mut self, at: usize, length: usize) {
        let gone: usize = self.words[at..]
            .iter()
            .map(|(_, word)| word.glyphs.len())
            .sum();
        self.glyphs -= gone;
        self.words.truncate(at);
        // A state once word `next - 1` is read read only words kept.
        let held = self.breaks.iter().rposition(|done| done.next <= at);
        self.breaks.truncate(held.map_or(0, |held| held + 1));
        self.resume = match (self.laid, self.breaks.last()) {
            (Some(width), Some(done)) => Some((width, *done)),
            _ => None,
        };
        self.grew = Some((length, 0));
    }
    /// Its lines, if laid out at `width` (a wrap width, as `lay_out` takes).
    pub fn lines_at(&self, width: Option<f32>) -> Option<&[LayoutLine]> {
        (self.laid == Some(width)).then_some(&self.lines[..])
    }
    /// Its first lines that are those of the text of `length` bytes it
    /// grew from (`grew`), as that text laid them out at `width` (a wrap
    /// width, as `lay_out` takes): taken with its words, to be kept as they
    /// are (`resume`), or kept by its last `lay_out`, at that width.
    pub fn kept_lines(&self, length: usize, width: Option<f32>) -> Option<&[LayoutLine]> {
        let (from, kept) = self.grew?;
        if from != length {
            return None;
        }
        let kept = match self.resume {
            Some((at, done)) if at == width => done.lines,
            Some(_) => return None,
            None if self.laid == Some(width) => kept,
            None => return None,
        };
        self.lines.get(..kept)
    }
    /// The word of these, `text`'s (or a text `text` starts with), from
    /// which a text continuing `text` is read again: the last after spaces
    /// that follow a letter or digit. Before it, its words, and where its
    /// lines may break, are the same whatever follows (a line may break
    /// after spaces; the rules that look back across them start from
    /// punctuation, which this leaves out). Its index, after the first.
    pub fn resume_at(&self, text: &str) -> Option<usize> {
        let bytes = text.as_bytes();
        // Only the last few: one an append extends, and what follows it.
        let first = self.words.len().saturating_sub(8).max(1);
        (first..self.words.len()).rev().find(|&at| {
            let (start, shape) = &self.words[at];
            let mut space = *start;
            while space > 0 && bytes[space - 1] == b' ' {
                space -= 1;
            }
            !shape.blank && space < *start && space > 0 && bytes[space - 1].is_ascii_alphanumeric()
        })
    }
    /// Its glyphs' widths, as `Compact` keeps them.
    fn advances(&self, word: usize) -> impl Iterator<Item = f32> + '_ {
        let size = self.font_size;
        self.words[word].1.glyphs.iter().map(move |g| g.width(size))
    }
    /// Its visual lines at wrap width `width`, as `layout_to_buffer` breaks
    /// a line of one span (word, else glyph): each one's start and end and
    /// width. The steps `Compact::size` takes. From `from` (a state it left
    /// earlier at this width: the lines from its on), noting in `breaks`
    /// (a stream's, which grew from another: `grew`) each state after a word
    /// a line ended at.
    /// (`STREAM`: a stream's, which grew from another (`grew`); else from
    /// the start, noting nothing, as before streams were.)
    #[inline(always)]
    fn visual_lines<const STREAM: bool>(
        &self,
        width: Option<f32>,
        from: &Break,
        breaks: &mut Vec<Break>,
    ) -> smallvec::SmallVec<[(At, At, f32); 2]> {
        let record = STREAM;
        let from = match STREAM {
            true => from,
            false => &Break::START,
        };
        let limit = width.unwrap_or(f32::INFINITY);
        let mut lines = smallvec::SmallVec::new();
        // The line being filled: its width, and the one range it takes.
        let mut line: Option<(At, At, f32)> = from.line;
        let mut line_w = from.line_w;
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
        let mut fitting = from.fitting;
        let mut range_width = from.range_width;
        let mut before_last_blank = from.before_last_blank;
        for (i, (_, word)) in words.iter().enumerate().skip(from.next) {
            let ended = lines.len();
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
            // (Words apart: a narrow width ends a line at each word.)
            if record
                && lines.len() != ended
                && breaks
                    .last()
                    .is_none_or(|last| i + 1 >= last.next + super::BREAKS_APART)
            {
                breaks.push(Break {
                    next: i + 1,
                    lines: from.lines + lines.len(),
                    line,
                    line_w,
                    fitting,
                    range_width,
                    before_last_blank,
                });
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
        // A text that never grew from another lays out whole, noting no
        // states: what a stream keeps costs it nothing.
        match self.grew.is_some() {
            true => self.lay_out_from::<true>(width),
            false => self.lay_out_from::<false>(width),
        }
    }
    /// `lay_out`, `STREAM` for a stream's text (`visual_lines`).
    #[inline(always)]
    fn lay_out_from<const STREAM: bool>(&mut self, width: Option<f32>) {
        // Its lines from the text it grew from, at this width, to where they
        // may change: unless a line's place in it depends on the widest
        // line (aligned in an unwrapped text).
        let from = match STREAM {
            true => {
                let left = matches!(self.align, None | Some(Align::Left | Align::Justified));
                let from = match self.resume.take() {
                    Some((at, from)) if at == width && (left || width.is_some()) => from,
                    _ => Break::START,
                };
                if let Some((_, kept)) = &mut self.grew {
                    *kept = from.lines;
                }
                from
            }
            false => Break::START,
        };
        let visual = match STREAM {
            true => {
                let mut breaks = std::mem::take(&mut self.breaks);
                breaks.truncate(
                    breaks
                        .iter()
                        .take_while(|done| done.next <= from.next)
                        .count(),
                );
                if from.next == 0 {
                    breaks.clear();
                }
                let visual = self.visual_lines::<true>(width, &from, &mut breaks);
                self.breaks = breaks;
                visual
            }
            false => self.visual_lines::<false>(width, &from, &mut Vec::new()),
        };
        let font_size = self.font_size;
        let line_width =
            width.unwrap_or_else(|| visual.iter().fold(0_f32, |w, line| w.max(line.2)));
        // Its lines from before are filled again, their lists kept.
        let mut lines = std::mem::take(&mut self.lines);
        let mut made = from.lines;
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
        self.glyphs = 0;
        self.breaks.clear();
        (self.resume, self.grew, self.laid) = (None, None, None);
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
        256 + self.words.len() * 16 + self.glyphs * size_of::<cosmic_text::LayoutGlyph>()
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;
    use zgx::text_layout::LetterSpacing;

    /// A font on a grid of cells is shaped whole, as tracking is, not word
    /// by word from the cache (its words' advances are the grid's).
    #[test]
    fn a_grid_is_shaped_whole() {
        let grid = FontStyle {
            letter_spacing: LetterSpacing::cells(7.),
            fallbacks: std::sync::Arc::from([]),
            ..FontStyle::default()
        };
        assert!(!qualifies(&grid));
        assert!(qualifies(&FontStyle {
            letter_spacing: LetterSpacing::ZERO,
            ..grid
        }));
    }
}
