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

/// What preparing and laying out a paragraph did, for tests that a stream's
/// chunk costs the same however long its paragraph: words read to break
/// lines, glyphs placed, words copied from what it grew from, packed into
/// its compact, and read to measure it.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Work {
    pub lines: usize,
    pub placed: usize,
    pub copied: usize,
    pub compact: usize,
    pub measured: usize,
    /// Layouts that kept lines from what they grew from.
    pub resumed: usize,
}
#[cfg(test)]
thread_local! {
    pub(super) static WORK: std::cell::Cell<Work> = const {
        std::cell::Cell::new(Work {
            lines: 0,
            placed: 0,
            copied: 0,
            compact: 0,
            measured: 0,
            resumed: 0,
        })
    };
}
/// Count `f`'s share of `Work` (tests only).
#[cfg(test)]
pub(super) fn worked(f: impl FnOnce(&mut Work)) {
    WORK.with(|work| {
        let mut done = work.get();
        f(&mut done);
        work.set(done);
    });
}

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
    pub font_size: f32,
    align: Option<Align>,
    /// Its words' glyphs, all told (`bytes`).
    glyphs: usize,
    /// Laid out at this wrap width (`lay_out`), into `lines`.
    laid: Option<Option<f32>>,
    pub lines: Vec<LayoutLine>,
    /// What a paragraph long enough to stream into keeps, to be laid out
    /// and measured from where a chunk changes it; none for a short one,
    /// most texts, which carry no more for it.
    stream: Option<Box<Stream>>,
}

/// Words a paragraph has at least for its lines to be kept (`Stream`).
const STREAM_WORDS: usize = 16;

/// A long paragraph's lines (`Words::stream`).
#[derive(Default)]
struct Stream {
    /// Where each of its `lines` starts and ends ((word, glyph)).
    bounds: Vec<(At, At)>,
    /// Grown from the paragraph these were (a stream's last state): its
    /// lines laid out at this width, its words before this one the same
    /// (`WordCache::words`). `lay_out` keeps the lines those words decide.
    grown: Option<(Option<f32>, usize)>,
    /// Its last measures (`measure`), one a width (a layout asks a text at
    /// more than one: its own, and its least or most): `MEASURED` at most.
    measured: Vec<Measured>,
}

/// Widths a long paragraph keeps its measure at (`Stream::measured`).
const MEASURED: usize = 4;

/// A paragraph's size and reach at a width (`Words::measure`), with each
/// visual line's start and what the lines before it came to.
struct Measured {
    width: Option<f32>,
    /// Where it stood before every `CHECKPOINT`th word (`Measuring`).
    checkpoints: Vec<(usize, Measuring)>,
    size: ((f32, f32), f32),
    /// Grown since (a stream's chunks): the words before this one the
    /// same, the lines they decide kept (`Words::measure`).
    grown: Option<usize>,
}

/// Words between a measure's checkpoints (`Measured`): a chunk measures
/// from the last before its words, at most this many again. A few in
/// tests, so that one falls among the words a chunk changes.
const CHECKPOINT: usize = if cfg!(test) { 3 } else { 32 };

/// Where measuring a paragraph's words stands before one of them (all
/// `Words::measure` carries from word to word, which only the words before
/// decide): the line being filled, what the lines done came to, and where
/// the words not yet added to it start.
#[derive(Clone, Copy, Default)]
struct Measuring {
    line: Line,
    sums: Sums,
    any: bool,
    fitting: At,
    range_width: f32,
    before_last_blank: f32,
    /// The height the words not yet added set (`fitting` to the word
    /// being read), and those before the last blank among them: kept as
    /// they are read, not read again when added (one line of a paragraph
    /// with no wrap width holds all of it).
    range_height: Option<f32>,
    before_blank_height: Option<f32>,
}

/// What lines measured so far came to (`Compact::size_reach`'s running
/// values): the widest, the height, and the most that fit and the least
/// that did not.
#[derive(Clone, Copy)]
struct Sums {
    widest: f32,
    height: f32,
    fit: f32,
    beyond: f32,
}
impl Default for Sums {
    fn default() -> Self {
        Self {
            widest: 0.,
            height: 0.,
            fit: 0.,
            beyond: f32::INFINITY,
        }
    }
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
    /// with (a stream's last state, superseded: taken, not copied), and
    /// where they hold to (`Words::resume_at`): those before are kept, and
    /// only the rest of `text` is read, its words looked up and shaped, and
    /// its lines laid out from the first they can change (`lay_out`). With
    /// the words, how many were kept so.
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
        grew: Option<(Box<Words>, usize)>,
    ) -> Option<(Box<Words>, usize)> {
        if runs.iter().any(|run| !qualifies(run.font)) {
            return None;
        }
        let size = font_size.max(1.);
        let ids: smallvec::SmallVec<[u32; 4]> = runs.iter().map(|run| self.font(run)).collect();
        let mut ranges: smallvec::SmallVec<[(usize, usize, bool); 16]> = smallvec::SmallVec::new();
        // (`plain_line` was asked first: `TextCache::words_of`.) Read from
        // where the words it grew from stop holding.
        let (grew, taken) = match grew {
            None => {
                if !cosmic_text::line_words(text, |range, blank| {
                    ranges.push((range.start, range.end, blank));
                }) {
                    return None;
                }
                (None, 0)
            }
            Some((grew, at)) => {
                let from = grew.words[at].0;
                if !cosmic_text::line_words(&text[from..], |range, blank| {
                    ranges.push((from + range.start, from + range.end, blank));
                }) {
                    give_back(spare, grew);
                    return None;
                }
                (Some(grew), at)
            }
        };
        let resumed = grew.is_some();
        let mut kept = grew.unwrap_or_else(|| {
            spare.pop().unwrap_or_else(|| {
                Box::new(Words {
                    words: Vec::new(),
                    font_size: size,
                    align: None,
                    glyphs: 0,
                    laid: None,
                    lines: Vec::new(),
                    stream: None,
                })
            })
        });
        // Its words before `taken` kept, where it grew from them.
        if resumed {
            let gone: usize = kept.words[taken..]
                .iter()
                .map(|(_, word)| word.glyphs.len())
                .sum();
            kept.glyphs -= gone;
        } else {
            kept.glyphs = 0;
        }
        let words = &mut kept.words;
        words.truncate(taken);
        words.reserve(ranges.len());
        let mut run = 0;
        for (start, end, blank) in ranges {
            while runs[run].end <= start {
                run += 1;
            }
            // A word across runs is shaped in parts by cosmic-text.
            if end > runs[run].end {
                give_back(spare, kept);
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
                        give_back(spare, kept);
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
                    shape
                }
            };
            kept.glyphs += shape.glyphs.len();
            kept.words.push((start, shape));
        }
        let align = crate::text::alignment(base);
        if let Some(stream) = &mut kept.stream {
            let same = resumed && kept.font_size == size;
            // Its lines kept only where laid out as it would lay them out,
            // and its measures where they were in this size: from the
            // fewer of the words they still hold, a chunk since not laid out
            // or measured at a width holding fewer.
            let fewer = |grown: Option<usize>| Some(grown.map_or(taken, |at| at.min(taken)));
            stream.grown = match (kept.laid, stream.grown) {
                (Some(width), _) if same && kept.align == align => Some((width, taken)),
                (None, Some((width, at))) if same && kept.align == align => {
                    Some((width, at.min(taken)))
                }
                _ => None,
            };
            match same {
                true => {
                    for measured in &mut stream.measured {
                        measured.grown = fewer(measured.grown);
                    }
                }
                false => stream.measured.clear(),
            }
        }
        kept.font_size = size;
        kept.align = align;
        kept.laid = None;
        Some((kept, taken))
    }
}

/// A word list not used after all (a text that doesn't qualify), kept for
/// another text: apart from the loop over its words, which it would crowd.
#[cold]
#[inline(never)]
#[allow(clippy::vec_box)]
fn give_back(spare: &mut Vec<Box<Words>>, words: Box<Words>) {
    spare.extend(words.recycle());
}

/// Where a visual line starts or ends: a word, and a glyph in it.
type At = (usize, usize);

/// A visual line being measured (`Words::measure`), as `Compact`'s
/// `Visual`: its width, the height its words set, and whether it holds any.
#[derive(Clone, Copy, Default)]
struct Line {
    w: f32,
    height: Option<f32>,
    nonempty: bool,
}
/// The taller of two line heights a word may set (`Visual::add`'s).
fn taller(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, None) => a,
        (None, b) => b,
    }
}
/// A word's line height, as `Compact` packs it (its first glyph's).
fn height_of(word: &Shape) -> Option<f32> {
    word.glyphs
        .first()
        .and_then(|glyph| glyph.metrics_opt)
        .map(|metrics| metrics.line_height)
}
impl Line {
    /// Add the words from `start` to `end`, `width` wide and setting
    /// `height` (theirs, kept as they were read: `Words::measure`), as
    /// `Visual::add` does.
    fn add(&mut self, start: At, end: At, width: f32, height: Option<f32>) {
        if start == end {
            return;
        }
        self.nonempty = true;
        self.w += width;
        self.height = taller(self.height, height);
    }
    /// Done: counted into `sums`, where it holds any.
    fn push(&mut self, sums: &mut Sums, any: &mut bool, line_height: f32) {
        if self.nonempty {
            sums.widest = sums.widest.max(self.w);
            sums.height += self.height.unwrap_or(line_height);
            *any = true;
        }
        *self = Self::default();
    }
}

impl Words {
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
    /// a line of one span (word, else glyph), from word `from` (one a line
    /// starts at: the lines before are as they were): each one's start and
    /// end and width. The steps `Compact::size` takes.
    fn visual_lines(
        &self,
        width: Option<f32>,
        from: usize,
    ) -> smallvec::SmallVec<[(At, At, f32); 2]> {
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
        let mut fitting = (from, 0);
        let mut range_width = 0_f32;
        let mut before_last_blank = 0_f32;
        #[cfg(test)]
        worked(|w| w.lines += words.len() - from);
        for (i, (_, word)) in words.iter().enumerate().skip(from) {
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
    /// Its size wrapped at `width` (an empty paragraph `line_height` high)
    /// and the width up to which its lines break as they do there, as
    /// `Compact::size_reach` measures a compact of these words, step by
    /// step. Grown from a paragraph measured at `width` (`Measured::grown`),
    /// from the last checkpoint before its new words: where measuring stood
    /// there depends on the words before it alone, which are the same. A
    /// chunk measures its own words and a checkpoint's at most, in a line
    /// or across lines.
    pub fn measure(&mut self, width: Option<f32>, line_height: f32) -> ((f32, f32), f32) {
        let Words {
            words,
            font_size,
            stream,
            ..
        } = self;
        let stream = stream.get_or_insert_with(Default::default);
        let at = stream
            .measured
            .iter()
            .position(|measured| measured.width == width);
        if let Some(at) = at
            && stream.measured[at].grown.is_none()
        {
            return stream.measured[at].size;
        }
        let (mut checkpoints, grown) = match at {
            Some(at) => {
                let measured = stream.measured.remove(at);
                (measured.checkpoints, measured.grown)
            }
            None => (Vec::new(), None),
        };
        // The last checkpoint its kept words decide (a measure of the same
        // words, not grown, keeps none: measured afresh).
        let kept = grown.map_or(0, |at| checkpoints.partition_point(|(word, _)| *word <= at));
        checkpoints.truncate(kept);
        let (from, state) = checkpoints.last().copied().unwrap_or((
            0,
            Measuring {
                sums: Sums::default(),
                ..Measuring::default()
            },
        ));
        let limit = crate::text::wrap_width(width).unwrap_or(f32::INFINITY);
        let size = *font_size;
        #[cfg(test)]
        worked(|w| w.measured += words.len() - from);
        let Measuring {
            mut line,
            mut sums,
            mut any,
            mut fitting,
            mut range_width,
            mut before_last_blank,
            mut range_height,
            mut before_blank_height,
        } = state;
        for (i, (_, word)) in words.iter().enumerate().skip(from) {
            if i % CHECKPOINT == 0 && i > from {
                checkpoints.push((
                    i,
                    Measuring {
                        line,
                        sums,
                        any,
                        fitting,
                        range_width,
                        before_last_blank,
                        range_height,
                        before_blank_height,
                    },
                ));
            }
            let own = height_of(word);
            let asks = match word.blank {
                true => line.w + range_width,
                false => line.w + (range_width + word.width),
            };
            if line.w + (range_width + word.width) <= limit
                || (word.blank && line.w + range_width <= limit)
            {
                sums.fit = sums.fit.max(asks);
                if word.blank {
                    before_last_blank = range_width;
                    before_blank_height = range_height;
                }
                range_width += word.width;
                range_height = taller(range_height, own);
            } else if word.width > limit {
                sums.beyond = sums.beyond.min(asks).min(word.width);
                if range_width > 0. {
                    line.add(fitting, (i, 0), range_width, range_height);
                    line.push(&mut sums, &mut any, line_height);
                    range_width = 0.;
                    fitting = (i, 0);
                    range_height = None;
                }
                for (glyph, shaped) in word.glyphs.iter().enumerate() {
                    let advance = shaped.width(size);
                    let asks = line.w + (range_width + advance);
                    if asks <= limit {
                        sums.fit = sums.fit.max(asks);
                        range_width += advance;
                    } else {
                        sums.beyond = sums.beyond.min(asks);
                        // To glyph `glyph` of this word: it too, if any.
                        let height = match glyph {
                            0 => range_height,
                            _ => taller(range_height, own),
                        };
                        line.add(fitting, (i, glyph), range_width, height);
                        line.push(&mut sums, &mut any, line_height);
                        range_width = advance;
                        fitting = (i, glyph);
                        range_height = None;
                    }
                }
                range_height = taller(range_height, own);
            } else {
                sums.beyond = sums.beyond.min(asks);
                sums.fit = sums.fit.max(word.width);
                if range_width > 0. {
                    // A blank before the break is left out.
                    if i > 0 && words[i - 1].1.blank {
                        line.add(fitting, (i - 1, 0), before_last_blank, before_blank_height);
                    } else {
                        line.add(fitting, (i, 0), range_width, range_height);
                    }
                }
                line.push(&mut sums, &mut any, line_height);
                if word.blank {
                    range_width = 0.;
                    fitting = (i + 1, 0);
                    range_height = None;
                } else {
                    range_width = word.width;
                    fitting = (i, 0);
                    range_height = own;
                }
            }
        }
        line.add(fitting, (words.len(), 0), range_width, range_height);
        line.push(&mut sums, &mut any, line_height);
        let height = sums.height + if any { 0. } else { line_height };
        let reach = match width {
            Some(_) if sums.fit <= sums.widest && sums.beyond.is_finite() => {
                let reach = sums.beyond - 1e-3;
                reach - reach.abs() * 1e-6 - 1e-6
            }
            Some(_) if sums.fit <= sums.widest => f32::INFINITY,
            _ => 0.,
        };
        let size = ((sums.widest, height), reach);
        if stream.measured.len() == MEASURED {
            stream.measured.remove(0);
        }
        stream.measured.push(Measured {
            width,
            checkpoints,
            size,
            grown: None,
        });
        size
    }
    /// The lines a paragraph grown from these words' last layout keeps,
    /// its first `at` words the same (`grown`), laid out at `width`: each
    /// before the line whose start a later word decides, back to one that
    /// starts a word (a word broken between its glyphs is laid out again).
    /// A line ends where a word doesn't fit, the next starting at it or
    /// after it (a blank), so one whose next starts before `at` ends as it
    /// did. How many, and the word the rest is laid out from; none where
    /// its alignment places every line by the widest (centred or at the
    /// right with no wrap width).
    fn resume_line(&self, at: usize, width: Option<f32>) -> (usize, usize) {
        let left = matches!(
            self.align.unwrap_or(Align::Left),
            Align::Left | Align::Justified
        );
        let Some(stream) = &self.stream else {
            return (0, 0);
        };
        let bounds = &stream.bounds;
        if (width.is_none() && !left) || bounds.len() != self.lines.len() {
            return (0, 0);
        }
        let first = bounds
            .iter()
            .position(|(start, _)| start.0 >= at)
            .unwrap_or(bounds.len());
        let mut kept = first.saturating_sub(1);
        while kept > 0 && bounds[kept].0.1 != 0 {
            kept -= 1;
        }
        match bounds.get(kept) {
            Some(&((from, _), _)) if kept > 0 && from < self.words.len() => (kept, from),
            _ => (0, 0),
        }
    }
    /// Laid out at `width` (the wrap width given cosmic-text): its glyphs
    /// placed as `layout_to_buffer` places a left-to-right line's. Grown
    /// from a paragraph laid out at `width` (a stream's last state), from
    /// the first line its new words can change (`resume_line`), the lines
    /// before kept: a chunk lays out its own lines, however long the
    /// paragraph.
    pub fn lay_out(&mut self, width: Option<f32>) {
        if self.laid == Some(width) {
            return;
        }
        self.laid = Some(width);
        let grown = self.stream.as_mut().and_then(|stream| stream.grown.take());
        let (kept, from) = match grown {
            Some((laid, at)) if laid == width => self.resume_line(at, width),
            _ => (0, 0),
        };
        #[cfg(test)]
        worked(|w| w.resumed += usize::from(kept > 0));
        let visual = self.visual_lines(width, from);
        let font_size = self.font_size;
        // (Only lines placed from the left are kept: `resume_line`.)
        let line_width =
            width.unwrap_or_else(|| visual.iter().fold(0_f32, |w, line| w.max(line.2)));
        // Its lines from before are filled again, their lists kept; those
        // its words still decide stay as they are.
        let mut lines = std::mem::take(&mut self.lines);
        // Where its lines start and end, kept for a long paragraph only.
        let mut bounds = (self.words.len() >= STREAM_WORDS).then(|| {
            let stream = self.stream.get_or_insert_with(Default::default);
            let mut bounds = std::mem::take(&mut stream.bounds);
            bounds.truncate(kept);
            bounds
        });
        let mut made = kept;
        for (start, end, w) in visual {
            if let Some(bounds) = &mut bounds {
                bounds.push((start, end));
            }
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
            #[cfg(test)]
            worked(|w| w.placed += count);
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
            if let Some(bounds) = &mut bounds {
                bounds.clear();
                bounds.push(((0, 0), (0, 0)));
            }
        }
        lines.truncate(made);
        self.lines = lines;
        if let Some(stream) = &mut self.stream {
            stream.bounds = bounds.unwrap_or_default();
        }
    }
    /// Keep its lists for another text (`WordCache::words`): its words let
    /// go, its lines' glyph lists only if short.
    pub fn recycle(mut self: Box<Self>) -> Option<Box<Self>> {
        self.words.clear();
        self.glyphs = 0;
        if let Some(stream) = &mut self.stream {
            stream.bounds.clear();
            stream.grown = None;
            stream.measured.clear();
        }
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
