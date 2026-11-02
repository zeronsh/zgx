//! A long text held once and shared ([`Text`]): in chunks of a few
//! kilobytes, each knowing how many line breaks it holds, so an edit
//! rewrites the chunks it touches and not the text, a copy of it is a
//! pointer (`Clone`), and a line or the line of a byte is found in
//! `O(log n)`. An editor holding a long document, its layout and its
//! semantics share one.
use std::{
    borrow::Cow,
    collections::VecDeque,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// What a chunk is cut to (a long line lies across several).
const CHUNK: usize = 8 * 1024;
/// A chunk edited past this is cut again.
const MAX: usize = 2 * CHUNK;
// Its line breaks' places fit two bytes (an edit's chunk before it is cut
// is at most `MAX` and what was typed: a paste is cut as it is joined).
const _: () = assert!(MAX < u16::MAX as usize / 2);

#[derive(Clone, Debug)]
struct Chunk {
    text: Arc<str>,
    /// Where its line breaks are (a chunk is never 64 KiB long): two bytes
    /// a line, for finding a line in it without reading it.
    breaks: Arc<[u16]>,
}
impl Chunk {
    fn new(text: &str) -> Self {
        Self {
            breaks: text.match_indices('\n').map(|(at, _)| at as u16).collect(),
            text: text.into(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Chunks {
    chunks: Vec<Chunk>,
    /// Before each chunk, and one past the last: its bytes and line breaks.
    bytes: Vec<usize>,
    breaks: Vec<usize>,
    /// How many carriage returns it holds, so whether a text has any is
    /// known without reading it (`Text::has_returns`): counted where it is
    /// made, and in the chunks an edit makes again.
    returns: usize,
    /// Which text this is (`Text::id`): fresh when made and after every
    /// edit, shared by its clones. 0 only for the empty text `Default`
    /// makes, which every such text is.
    id: u64,
    /// Its last edits, the earliest first (`Text::edits_since`).
    log: EditLog,
}
/// A fresh text id: never one made before in this process.
fn fresh_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}
/// One edit of a text (`Text::edits_since`): at `start`, the `old_len`
/// bytes there became `inserted`, turning the text `before` into the next.
#[derive(Clone, Debug)]
pub(crate) struct TextDelta {
    /// The id of the text it was made to.
    pub(crate) before: u64,
    pub(crate) start: usize,
    pub(crate) old_len: usize,
    pub(crate) inserted: Arc<str>,
}
/// A text's last edits: at most `LOG_EDITS` of them, holding at most
/// `LOG_BYTES` inserted, the earliest dropped first. An edit inserting more
/// than that empties it: what was before can't be followed past it.
#[derive(Clone, Debug, Default)]
struct EditLog {
    edits: VecDeque<TextDelta>,
    bytes: usize,
    /// Whether a reader follows the text by its edits (`Text::keep_edits`):
    /// until one does, an edit keeps none, and costs only its id.
    followed: Followed,
}
/// `EditLog::followed`: set through a shared text (a reader holds the
/// text it took, not the producer's), and carried to the text an edit
/// makes of it.
#[derive(Debug, Default)]
struct Followed(std::sync::atomic::AtomicBool);
impl Clone for Followed {
    fn clone(&self) -> Self {
        Self(self.0.load(Ordering::Relaxed).into())
    }
}
impl Followed {
    fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    fn set(&self, followed: bool) {
        self.0.store(followed, Ordering::Relaxed);
    }
}
const LOG_EDITS: usize = 16;
const LOG_BYTES: usize = 64 * 1024;
impl EditLog {
    fn record(&mut self, edit: TextDelta) {
        if edit.inserted.len() > LOG_BYTES {
            self.edits.clear();
            self.bytes = 0;
            return;
        }
        self.bytes += edit.inserted.len();
        self.edits.push_back(edit);
        while self.edits.len() > LOG_EDITS || self.bytes > LOG_BYTES {
            let dropped = self.edits.pop_front().expect("over a bound, not empty");
            self.bytes -= dropped.inserted.len();
        }
    }
}
impl Chunks {
    /// The sums of what comes before each chunk, from `from` on.
    fn sum(&mut self, from: usize) {
        self.bytes.truncate(from + 1);
        self.breaks.truncate(from + 1);
        if self.bytes.is_empty() {
            self.bytes.push(0);
            self.breaks.push(0);
        }
        for chunk in &self.chunks[from..] {
            self.bytes
                .push(self.bytes.last().unwrap() + chunk.text.len());
            self.breaks
                .push(self.breaks.last().unwrap() + chunk.breaks.len());
        }
    }
}

/// `text` cut into chunks of about `CHUNK` bytes, each ending at a line
/// break where one is near (so a line is most often in one chunk), else at
/// a character boundary.
fn cut(text: &str) -> Vec<Chunk> {
    let mut chunks = Vec::with_capacity(text.len() / CHUNK + 1);
    let mut rest = text;
    while rest.len() > CHUNK {
        let end = rest.floor_char_boundary(CHUNK);
        // A line break in the last quarter, if there is one: end after it.
        // (Both bounds on character boundaries: a character may straddle
        // either.)
        let quarter = rest.floor_char_boundary(CHUNK * 3 / 4);
        let end = match rest[quarter..end].rfind('\n') {
            Some(at) => quarter + at + 1,
            None => end,
        };
        chunks.push(Chunk::new(&rest[..end]));
        rest = &rest[end..];
    }
    if !rest.is_empty() || chunks.is_empty() {
        chunks.push(Chunk::new(rest));
    }
    chunks
}

/// An immutable text, shared: cloning it copies a pointer; an edit
/// ([`Text::replace_range`]) changes this one only, copying the chunks it
/// touches and the list of them, never the text.
#[derive(Clone, Debug, Default)]
pub struct Text(Arc<Chunks>);
impl Text {
    /// `text` held in chunks.
    pub fn new(text: &str) -> Self {
        let mut chunks = Chunks {
            chunks: cut(text),
            returns: text.matches('\r').count(),
            id: fresh_id(),
            ..Default::default()
        };
        chunks.sum(0);
        Self(Arc::new(chunks))
    }
    /// Its length in bytes.
    pub fn len(&self) -> usize {
        *self.0.bytes.last().unwrap_or(&0)
    }
    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Whether it holds a carriage return, without reading it.
    pub fn has_returns(&self) -> bool {
        self.0.returns > 0
    }
    /// How many lines it has (one more than its line breaks).
    pub fn line_count(&self) -> usize {
        self.0.breaks.last().unwrap_or(&0) + 1
    }
    /// Which text this is: the same number for it and its clones, a new one
    /// for each text made or edited.
    pub fn id(&self) -> u64 {
        self.0.id
    }
    /// The edits that made this text from the one `id` names, earliest
    /// first, if its log reaches back to it: none for this text itself,
    /// `None` for a text it doesn't follow (its log dropped it, or it
    /// never was).
    pub(crate) fn edits_since(
        &self,
        id: u64,
    ) -> Option<std::collections::vec_deque::Iter<'_, TextDelta>> {
        let edits = &self.0.log.edits;
        let from = match id == self.0.id {
            true => edits.len(),
            false => edits.iter().position(|edit| edit.before == id)?,
        };
        Some(edits.range(from..))
    }
    /// What was inserted to make this text from the one `id` names, where
    /// (the byte it went in at, then), earliest first, if its log reaches
    /// back to that one and nothing was removed since: a reader that took
    /// that text follows this one by these alone (a reply streamed in,
    /// each token where it ends). `None` otherwise: read it whole.
    pub fn inserts_since(&self, id: u64) -> Option<impl Iterator<Item = (usize, &str)> + Clone> {
        let edits = self.edits_since(id)?;
        let inserts = edits.clone().all(|edit| edit.old_len == 0);
        inserts.then(|| edits.map(|edit| (edit.start, &*edit.inserted)))
    }
    /// Drops the edits it keeps for its readers ([`Text::inserts_since`])
    /// once a stream into it has stopped, staying the same text (its
    /// [`id`](Text::id)): a reader that took it goes on from here, and one
    /// that took an earlier text reads it whole. Only while nothing else
    /// holds it (a clone keeps them); whether it dropped any.
    /// It stops keeping edits too, until a reader asks again
    /// ([`keep_edits`](Text::keep_edits)), shared or not.
    pub fn forget_edits(&mut self) -> bool {
        self.0.log.followed.set(false);
        match Arc::get_mut(&mut self.0) {
            Some(chunks) if !chunks.log.edits.is_empty() => {
                chunks.log = EditLog::default();
                true
            }
            _ => false,
        }
    }
    /// Says a reader follows this text by its edits ([`inserts_since`](
    /// Text::inserts_since)): from now on its edits, and those of the
    /// texts edits make of it, are kept for it. A reader asks as it takes
    /// the text (an editor bound to it, `ui.markdown_view`); a text nothing
    /// follows keeps none, and an edit costs it only its new id.
    pub fn keep_edits(&self) {
        self.0.log.followed.set(true);
    }
    /// Whether `other` is this same text (a clone of it), not just equal.
    pub fn ptr_eq(&self, other: &Text) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    /// Its chunks, in order: the text, a piece at a time.
    pub fn chunks(&self) -> impl Iterator<Item = &str> {
        self.0.chunks.iter().map(|chunk| &*chunk.text)
    }
    /// The chunk byte `at` is in (the last at the end).
    fn chunk_at(&self, at: usize) -> usize {
        let chunks = &self.0;
        chunks
            .bytes
            .partition_point(|start| *start <= at)
            .saturating_sub(1)
            .min(chunks.chunks.len().saturating_sub(1))
    }
    /// Whether byte `at` is on a character boundary (or the end).
    pub fn is_char_boundary(&self, at: usize) -> bool {
        if at >= self.len() {
            return at == self.len();
        }
        let i = self.chunk_at(at);
        self.0.chunks[i].text.is_char_boundary(at - self.0.bytes[i])
    }
    /// The bytes of `range`: borrowed when they lie in one chunk.
    pub fn slice(&self, range: Range<usize>) -> Cow<'_, str> {
        let (start, end) = (range.start.min(self.len()), range.end.min(self.len()));
        if start >= end {
            return Cow::Borrowed("");
        }
        let (first, last) = (self.chunk_at(start), self.chunk_at(end - 1));
        let chunks = &self.0;
        if first == last {
            let base = chunks.bytes[first];
            return Cow::Borrowed(&chunks.chunks[first].text[start - base..end - base]);
        }
        let mut out = String::with_capacity(end - start);
        for i in first..=last {
            let (base, text) = (chunks.bytes[i], &*chunks.chunks[i].text);
            let from = start.saturating_sub(base);
            let to = (end - base).min(text.len());
            out.push_str(&text[from..to]);
        }
        Cow::Owned(out)
    }
    /// The whole text in one string (a copy, if it has more than a chunk).
    pub fn to_cow(&self) -> Cow<'_, str> {
        self.slice(0..self.len())
    }
    /// The line byte `at` is on (a line's end, before its break, is its own;
    /// the byte after the break, the next line's).
    pub fn line_of(&self, at: usize) -> usize {
        let at = at.min(self.len());
        let i = self.chunk_at(at);
        let chunks = &self.0;
        let local = at - chunks.bytes[i];
        chunks.breaks[i]
            + chunks.chunks[i]
                .breaks
                .partition_point(|b| (*b as usize) < local)
    }
    /// Where line `line` starts (the end, past the last line).
    pub fn line_start(&self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        let chunks = &self.0;
        // The chunk holding the line's break before it: the first whose
        // breaks reach `line`.
        let i = chunks.breaks.partition_point(|before| *before < line);
        if i == 0 || i > chunks.chunks.len() {
            return self.len();
        }
        let i = i - 1;
        let nth = line - chunks.breaks[i] - 1;
        match chunks.chunks[i].breaks.get(nth) {
            Some(at) => chunks.bytes[i] + *at as usize + 1,
            None => self.len(),
        }
    }
    /// Line `line`'s bytes, its break excluded.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let start = self.line_start(line);
        let end = if line + 1 < self.line_count() {
            self.line_start(line + 1) - 1
        } else {
            self.len()
        };
        start..end.max(start)
    }
    /// Line `line`, its break excluded: borrowed when in one chunk.
    pub fn line(&self, line: usize) -> Cow<'_, str> {
        self.slice(self.line_range(line))
    }
    /// The bytes this and `other` start with in common: the chunks they
    /// share skipped by pointer (an edit's copy shares all but its own).
    pub fn common_prefix(&self, other: &Text) -> usize {
        let (a, b) = (&self.0.chunks, &other.0.chunks);
        let shared = a
            .iter()
            .zip(b)
            .take_while(|(x, y)| Arc::ptr_eq(&x.text, &y.text))
            .count();
        let at = self.0.bytes.get(shared).copied().unwrap_or(0);
        at + prefix_of(
            a[shared..].iter().map(|c| c.text.as_bytes()),
            b[shared..].iter().map(|c| c.text.as_bytes()),
        )
    }
    /// The bytes this and `other` end with in common, after `prefix`.
    pub fn common_suffix(&self, other: &Text, prefix: usize) -> usize {
        let (a, b) = (&self.0.chunks, &other.0.chunks);
        let shared = a
            .iter()
            .rev()
            .zip(b.iter().rev())
            .take_while(|(x, y)| Arc::ptr_eq(&x.text, &y.text))
            .count();
        let room = (self.len() - prefix).min(other.len() - prefix);
        let bytes: usize = a[a.len() - shared..].iter().map(|c| c.text.len()).sum();
        if bytes >= room {
            return room;
        }
        bytes
            + suffix_of(
                a[..a.len() - shared]
                    .iter()
                    .rev()
                    .map(|c| c.text.as_bytes()),
                b[..b.len() - shared]
                    .iter()
                    .rev()
                    .map(|c| c.text.as_bytes()),
                room - bytes,
            )
    }
    /// This text edited to be `text`: the bytes they start and end with
    /// compared, those between replaced. It shares this one's chunks but
    /// the edited one's (a copy made by `Text::new` would copy them all).
    pub(crate) fn edited_to(&self, text: &str) -> Text {
        let bytes = text.as_bytes();
        let prefix = prefix_of(
            self.0.chunks.iter().map(|c| c.text.as_bytes()),
            std::iter::once(bytes),
        );
        let room = (self.len() - prefix).min(text.len() - prefix);
        let suffix = suffix_of(
            self.0.chunks.iter().rev().map(|c| c.text.as_bytes()),
            std::iter::once(bytes),
            room,
        );
        // Where both have a character boundary: the bytes past the common
        // ones differ, and with them what starts a character.
        let mut start = prefix;
        while !text.is_char_boundary(start) || !self.is_char_boundary(start) {
            start -= 1;
        }
        let (mut end, mut from) = (self.len() - suffix, text.len() - suffix);
        while !text.is_char_boundary(from) || !self.is_char_boundary(end) {
            end += 1;
            from += 1;
        }
        let mut edited = self.clone();
        if start != end || start != from {
            edited.replace_range(start..end, &text[start..from]);
        }
        edited
    }
    /// Its longest line's length in bytes.
    pub fn longest_line(&self) -> usize {
        let (mut longest, mut run) = (0, 0);
        for chunk in self.chunks() {
            let mut last = 0;
            for (at, _) in chunk.match_indices('\n') {
                longest = longest.max(run + at - last);
                run = 0;
                last = at + 1;
            }
            run += chunk.len() - last;
        }
        longest.max(run)
    }
    /// Replace `range` with `with`: the chunks it touches are made again
    /// (and cut, if they grew long), the rest shared with the text's other
    /// copies.
    pub fn replace_range(&mut self, range: Range<usize>, with: &str) {
        let len = self.len();
        let (start, end) = (
            range.start.min(len),
            range.end.min(len).max(range.start.min(len)),
        );
        let chunks = Arc::make_mut(&mut self.0);
        if chunks.chunks.is_empty() {
            chunks.chunks.push(Chunk::new(""));
            chunks.sum(0);
        }
        let first = chunks
            .bytes
            .partition_point(|s| *s <= start)
            .saturating_sub(1)
            .min(chunks.chunks.len() - 1);
        let last = chunks
            .bytes
            .partition_point(|s| *s < end)
            .saturating_sub(1)
            .max(first)
            .min(chunks.chunks.len() - 1);
        let base = chunks.bytes[first];
        let tail_base = chunks.bytes[last];
        let mut joined =
            String::with_capacity(chunks.bytes[last + 1] - base - (end - start) + with.len());
        joined.push_str(&chunks.chunks[first].text[..start - base]);
        joined.push_str(with);
        joined.push_str(&chunks.chunks[last].text[end - tail_base..]);
        let made = if joined.len() > MAX {
            cut(&joined)
        } else {
            vec![Chunk::new(&joined)]
        };
        let returns = |chunk: &Chunk| chunk.text.matches('\r').count();
        chunks.returns -= chunks.chunks[first..=last]
            .iter()
            .map(returns)
            .sum::<usize>();
        chunks.returns += made.iter().map(returns).sum::<usize>();
        chunks.chunks.splice(first..=last, made);
        // An empty chunk goes, unless it is the only one.
        if chunks.chunks.len() > 1 {
            chunks.chunks.retain(|chunk| !chunk.text.is_empty());
        }
        chunks.sum(first.min(chunks.chunks.len().saturating_sub(1)));
        // Another text now, following the one it was: the edit kept only
        // for a reader that follows it.
        let before = std::mem::replace(&mut chunks.id, fresh_id());
        if chunks.log.followed.get() {
            chunks.log.record(TextDelta {
                before,
                start,
                old_len: end - start,
                inserted: with.into(),
            });
        } else if !chunks.log.edits.is_empty() {
            // Edits kept before it no longer reach this text.
            chunks.log.edits.clear();
            chunks.log.bytes = 0;
        }
    }
    /// `with` added at the end, as `replace_range` would put it there.
    pub fn append(&mut self, with: &str) {
        let end = self.len();
        self.replace_range(end..end, with);
    }
}
impl Text {
    /// A handle to this text that does not keep it.
    pub fn downgrade(&self) -> WeakText {
        WeakText(Arc::downgrade(&self.0))
    }
}
/// A [`Text`] not kept alive by this handle ([`Text::downgrade`]).
#[derive(Clone, Debug, Default)]
pub struct WeakText(std::sync::Weak<Chunks>);
impl WeakText {
    /// The text, if anything still holds it.
    pub fn upgrade(&self) -> Option<Text> {
        self.0.upgrade().map(Text)
    }
}
/// Shared chunks compare equal at a pointer's glance.
impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other) || (self.len() == other.len() && self.common_prefix(other) == self.len())
    }
}
impl PartialEq<str> for Text {
    fn eq(&self, other: &str) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let mut at = 0;
        self.chunks().all(|chunk| {
            let same = other.as_bytes()[at..at + chunk.len()] == *chunk.as_bytes();
            at += chunk.len();
            same
        })
    }
}
impl From<&str> for Text {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}
impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.chunks().try_for_each(|chunk| f.write_str(chunk))
    }
}

/// The bytes two texts, given in pieces, start with in common: a run at a
/// time where they agree (a memory compare), byte by byte only where they
/// part.
fn prefix_of<'a>(
    mut a: impl Iterator<Item = &'a [u8]>,
    mut b: impl Iterator<Item = &'a [u8]>,
) -> usize {
    let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
    let mut same = 0;
    loop {
        if x.is_empty() {
            match a.next() {
                Some(next) => x = next,
                None => return same,
            }
            continue;
        }
        if y.is_empty() {
            match b.next() {
                Some(next) => y = next,
                None => return same,
            }
            continue;
        }
        let n = x.len().min(y.len());
        let run = agree(&x[..n], &y[..n]);
        same += run;
        if run < n {
            return same;
        }
        (x, y) = (&x[n..], &y[n..]);
    }
}
/// The bytes two texts end with in common, up to `limit`: their pieces
/// given last first.
fn suffix_of<'a>(
    mut a: impl Iterator<Item = &'a [u8]>,
    mut b: impl Iterator<Item = &'a [u8]>,
    limit: usize,
) -> usize {
    let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
    let mut same = 0;
    while same < limit {
        if x.is_empty() {
            match a.next() {
                Some(next) => x = next,
                None => break,
            }
            continue;
        }
        if y.is_empty() {
            match b.next() {
                Some(next) => y = next,
                None => break,
            }
            continue;
        }
        let n = x.len().min(y.len()).min(limit - same);
        let run = agree_back(&x[x.len() - n..], &y[y.len() - n..]);
        same += run;
        if run < n {
            break;
        }
        (x, y) = (&x[..x.len() - n], &y[..y.len() - n]);
    }
    same
}
/// How far two equally long runs agree from their start.
fn agree(a: &[u8], b: &[u8]) -> usize {
    let mut at = 0;
    while at + 64 <= a.len() && a[at..at + 64] == b[at..at + 64] {
        at += 64;
    }
    at + a[at..]
        .iter()
        .zip(&b[at..])
        .take_while(|(x, y)| x == y)
        .count()
}
/// How far two equally long runs agree from their end.
fn agree_back(a: &[u8], b: &[u8]) -> usize {
    let mut at = a.len();
    while at >= 64 && a[at - 64..at] == b[at - 64..at] {
        at -= 64;
    }
    a.len() - at
        + a[..at]
            .iter()
            .rev()
            .zip(b[..at].iter().rev())
            .take_while(|(x, y)| x == y)
            .count()
}
/// The bytes two texts, given in pieces, start with in common, and then
/// end with (after those).
pub(crate) fn common(a: &[&str], b: &[&str]) -> (usize, usize) {
    let bytes = |pieces: &[&str]| pieces.iter().map(|p| p.len()).sum::<usize>();
    let prefix = prefix_of(
        a.iter().map(|p| p.as_bytes()),
        b.iter().map(|p| p.as_bytes()),
    );
    let room = (bytes(a) - prefix).min(bytes(b) - prefix);
    let suffix = suffix_of(
        a.iter().rev().map(|p| p.as_bytes()),
        b.iter().rev().map(|p| p.as_bytes()),
        room,
    );
    (prefix, suffix)
}

/// A text past this many bytes is held in chunks ([`TextValue::Long`]); one
/// edited below half of it, in a string again.
pub const LONG: usize = 64 * 1024;

/// A text to keep and bind an editor to (`text_area` takes a
/// `Signal<TextValue>` as well as a `Signal<String>`): a short one as a
/// string, a long one as a [`Text`]. An editor bound to a long document
/// then holds it once with its binding, and an edit's new value costs the
/// chunks it touched, not a copy of the document.
#[derive(Clone, Debug)]
pub enum TextValue {
    /// A short text.
    Flat(String),
    /// A long text, shared.
    Long(Text),
}
impl TextValue {
    /// Its length in bytes.
    pub fn len(&self) -> usize {
        match self {
            TextValue::Flat(text) => text.len(),
            TextValue::Long(text) => text.len(),
        }
    }
    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// All of it in one string: borrowed while short, a copy of a long one.
    pub fn to_cow(&self) -> Cow<'_, str> {
        match self {
            TextValue::Flat(text) => Cow::Borrowed(text),
            TextValue::Long(text) => text.to_cow(),
        }
    }
    /// Its pieces, in order.
    pub fn chunks(&self) -> impl Iterator<Item = &str> {
        let (flat, long) = match self {
            TextValue::Flat(text) => (Some(text.as_str()), None),
            TextValue::Long(text) => (None, Some(text.chunks())),
        };
        flat.into_iter().chain(long.into_iter().flatten())
    }
    /// The long text it holds, shared.
    pub fn as_text(&self) -> Option<&Text> {
        match self {
            TextValue::Flat(_) => None,
            TextValue::Long(text) => Some(text),
        }
    }
    /// Whether `other` is this same long text (a clone of it), not just
    /// equal.
    pub fn ptr_eq(&self, other: &TextValue) -> bool {
        matches!((self, other), (TextValue::Long(a), TextValue::Long(b)) if a.ptr_eq(b))
    }
}
impl Default for TextValue {
    fn default() -> Self {
        TextValue::Flat(String::new())
    }
}
/// In chunks if long.
impl From<String> for TextValue {
    fn from(text: String) -> Self {
        if text.len() > LONG {
            TextValue::Long(Text::new(&text))
        } else {
            TextValue::Flat(text)
        }
    }
}
impl From<&str> for TextValue {
    fn from(text: &str) -> Self {
        if text.len() > LONG {
            TextValue::Long(Text::new(text))
        } else {
            TextValue::Flat(text.to_owned())
        }
    }
}
impl From<Text> for TextValue {
    fn from(text: Text) -> Self {
        TextValue::Long(text)
    }
}
/// Equal texts are equal however they are held.
impl PartialEq for TextValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (TextValue::Flat(a), TextValue::Flat(b)) => a == b,
            (TextValue::Long(a), TextValue::Long(b)) => a == b,
            (TextValue::Flat(s), TextValue::Long(t)) | (TextValue::Long(t), TextValue::Flat(s)) => {
                *t == **s
            }
        }
    }
}
impl PartialEq<str> for TextValue {
    fn eq(&self, other: &str) -> bool {
        match self {
            TextValue::Flat(text) => text == other,
            TextValue::Long(text) => *text == *other,
        }
    }
}
impl std::fmt::Display for TextValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.chunks().try_for_each(|chunk| f.write_str(chunk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (start, old length, inserted) of the edits since `id`.
    fn since(text: &Text, id: u64) -> Option<Vec<(usize, usize, String)>> {
        Some(
            text.edits_since(id)?
                .map(|e| (e.start, e.old_len, e.inserted.to_string()))
                .collect(),
        )
    }

    #[test]
    fn a_text_and_its_clones_share_an_id_and_each_edit_makes_another() {
        let a = Text::new("hello");
        let b = a.clone();
        assert_eq!(a.id(), b.id());
        let mut c = a.clone();
        c.append(" world");
        assert_ne!(c.id(), a.id());
        assert_eq!(a.to_cow(), "hello", "the original is as it was");
        assert_ne!(Text::new("hello").id(), a.id(), "an equal text is another");
    }

    #[test]
    fn a_text_grown_by_inserts_lists_them_and_one_cut_lists_none() {
        let start = Text::new("ab");
        start.keep_edits();
        let mut text = start.clone();
        text.append("c");
        text.replace_range(0..0, "<");
        let inserts: Vec<_> = text.inserts_since(start.id()).unwrap().collect();
        assert_eq!(inserts, [(2, "c"), (0, "<")]);
        assert_eq!(text.inserts_since(text.id()).unwrap().count(), 0);
        text.replace_range(0..1, "");
        assert!(text.inserts_since(start.id()).is_none(), "a removal");
        assert!(
            text.inserts_since(Text::new("x").id()).is_none(),
            "not followed"
        );
    }

    #[test]
    fn a_text_forgets_its_edits_staying_itself_unless_shared() {
        let start = Text::new("ab");
        start.keep_edits();
        let mut text = start.clone();
        text.append("c");
        let id = text.id();
        let held = text.clone();
        assert!(!text.forget_edits(), "a clone keeps them");
        drop(held);
        assert!(text.forget_edits());
        assert_eq!(text.id(), id, "the same text");
        assert_eq!(text.inserts_since(id).unwrap().count(), 0);
        assert!(text.inserts_since(start.id()).is_none(), "read whole");
        assert!(!text.forget_edits(), "none left");
    }

    #[test]
    fn a_text_nothing_follows_keeps_no_edits_and_one_followed_does() {
        let start = Text::new("ab");
        let mut text = start.clone();
        text.append("c");
        assert_ne!(text.id(), start.id(), "another text");
        assert!(text.inserts_since(start.id()).is_none(), "none kept");
        // A reader takes it: what follows is kept, for it.
        let taken = text.clone();
        taken.keep_edits();
        text.append("d");
        let inserts: Vec<_> = text.inserts_since(taken.id()).unwrap().collect();
        assert_eq!(inserts, [(3, "d")]);
        // Forgotten (its host idle): none kept again, shared or not.
        let mut idle = text.clone();
        idle.forget_edits();
        text.append("e");
        assert!(text.inserts_since(taken.id()).is_none());
    }

    #[test]
    fn a_text_lists_the_edits_since_one_it_follows() {
        let start = Text::new("ab");
        start.keep_edits();
        let mut text = start.clone();
        text.append("c");
        let middle = text.id();
        text.replace_range(0..1, "A");
        assert_eq!(
            since(&text, start.id()),
            Some(vec![(2, 0, "c".into()), (0, 1, "A".into())])
        );
        assert_eq!(since(&text, middle), Some(vec![(0, 1, "A".into())]));
        assert_eq!(since(&text, text.id()), Some(vec![]));
        assert_eq!(
            since(&text, Text::new("ab").id()),
            None,
            "not one it follows"
        );
    }

    #[test]
    fn a_texts_log_keeps_its_last_edits_within_its_bounds() {
        let start = Text::new("");
        start.keep_edits();
        let mut text = start.clone();
        let mut ids = vec![start.id()];
        for i in 0..20 {
            text.append(&i.to_string());
            ids.push(text.id());
        }
        // Sixteen edits back, and no further.
        assert_eq!(since(&text, ids[4]).map(|e| e.len()), Some(16));
        assert_eq!(since(&text, ids[3]), None);
        // An insert past the bytes kept leaves nothing to follow across it.
        let before = text.id();
        text.append(&"x".repeat(LOG_BYTES + 1));
        assert_eq!(since(&text, before), None);
        text.append("y");
        assert_eq!(since(&text, ids[20]), None);
        // Inserts within it drop the earliest to fit.
        let mut text = Text::new("");
        text.keep_edits();
        let first = text.id();
        text.append(&"a".repeat(LOG_BYTES / 2));
        let second = text.id();
        text.append(&"b".repeat(LOG_BYTES / 2));
        text.append("c");
        assert_eq!(since(&text, first), None, "the earliest dropped to fit");
        assert_eq!(since(&text, second).map(|e| e.len()), Some(2));
    }

    /// A text edited to another (`edited_to`) holds it, whatever the edit:
    /// appended, cut, replaced across chunks, or between characters that
    /// share their first bytes. Its counts are the other's own, and the
    /// chunks the edit missed are shared.
    #[test]
    fn a_text_edited_to_another_is_it() {
        let line = "a line of the document so far, é候😀\n";
        let base = line.repeat(3 * CHUNK / line.len());
        let mut cases = vec![
            format!("{base}token "),
            base[..base.len() - line.len()].to_owned(),
            format!("x{base}"),
            base.replacen("so far", "SO FAR\r\nand on", 2),
            String::new(),
            base.replace('é', "è"),
            base.replacen('😀', "😁", 1),
        ];
        let middle = base.len() / 2;
        let middle = (middle..).find(|at| base.is_char_boundary(*at)).unwrap();
        cases.push(format!("{}{}", &base[..middle], &base[middle + 4..]));
        let old = Text::new(&base);
        for case in cases {
            let edited = old.edited_to(&case);
            assert_eq!(edited.to_cow(), case);
            let made = Text::new(&case);
            assert_eq!(edited.line_count(), made.line_count());
            assert_eq!(edited.has_returns(), made.has_returns());
            for line in 0..made.line_count() {
                assert_eq!(edited.line_range(line), made.line_range(line));
            }
        }
        let appended = old.edited_to(&format!("{base}token "));
        assert!(Arc::ptr_eq(
            &old.0.chunks[0].text,
            &appended.0.chunks[0].text
        ));
    }

    /// A 2-, 3- and 4-byte character at every byte around where chunks are
    /// cut (the last quarter's start, a chunk's end, the next chunk's),
    /// with and without line breaks: the text cut, sliced and edited there
    /// is the text, and every chunk ends on a character boundary.
    #[test]
    fn a_character_straddling_any_cut_is_kept_whole() {
        for wide in ["é", "候", "😀"] {
            for around in [CHUNK * 3 / 4, CHUNK, 2 * CHUNK, 2 * CHUNK + CHUNK * 3 / 4] {
                for at in around - 5..around + 5 {
                    for newline in [false, true] {
                        let mut plain = "a".repeat(at);
                        plain.push_str(wide);
                        plain.push_str(&"b".repeat(3 * CHUNK - at));
                        if newline {
                            plain.insert(CHUNK * 3 / 4 + 40, '\n');
                        }
                        let mut text = Text::new(&plain);
                        assert_eq!(text.to_cow(), plain, "{wide} at {at}");
                        assert!(text.chunks().all(|chunk| !chunk.is_empty()));
                        let start = plain.find(wide).unwrap();
                        let end = start + wide.len();
                        assert_eq!(text.slice(start..end), wide);
                        // Edited where it is: removed, and put back twice.
                        text.replace_range(start..end, "");
                        plain.replace_range(start..end, "");
                        assert_eq!(text.to_cow(), plain, "{wide} removed at {at}");
                        let twice = wide.repeat(2);
                        text.replace_range(start..start, &twice);
                        plain.replace_range(start..start, &twice);
                        assert_eq!(text.to_cow(), plain, "{wide} put back at {at}");
                    }
                }
            }
        }
    }

    fn lines(n: usize) -> String {
        (0..n)
            .map(|i| format!("line {i} {}\n", "x".repeat(i % 50)))
            .collect()
    }

    /// Lines, slices and edits read as the same string's do, across chunks.
    #[test]
    fn a_text_in_chunks_reads_and_edits_as_its_string_does() {
        let mut plain = lines(3_000);
        let mut text = Text::new(&plain);
        assert!(text.0.chunks.len() > 10);
        let check = |text: &Text, plain: &str| {
            assert_eq!(text.len(), plain.len());
            assert!(*text == *plain);
            let starts: Vec<usize> = std::iter::once(0)
                .chain(plain.match_indices('\n').map(|(at, _)| at + 1))
                .collect();
            assert_eq!(text.line_count(), starts.len());
            for (line, start) in starts.iter().enumerate().step_by(97) {
                assert_eq!(text.line_start(line), *start, "line {line}");
                assert_eq!(text.line_of(*start), line);
                let end = plain[*start..]
                    .find('\n')
                    .map_or(plain.len(), |i| start + i);
                assert_eq!(text.line(line), &plain[*start..end]);
            }
            for (a, b) in [(0, 10), (8190, 8200), (100, 20_000), (0, plain.len())] {
                let (a, b) = (a.min(plain.len()), b.min(plain.len()));
                assert_eq!(text.slice(a..b), &plain[a..b]);
            }
        };
        check(&text, &plain);
        let before = text.clone();
        for (start, end, with) in [
            (0, 0, "head\n"),
            (5000, 5010, ""),
            (8100, 8300, "a line\nacross\nchunks"),
            (20_000, 20_000, &"long ".repeat(10_000)),
            (1000, 60_000, "x"),
        ] {
            let (start, end) = (start.min(plain.len()), end.min(plain.len()));
            plain.replace_range(start..end, with);
            text.replace_range(start..end, with);
            check(&text, &plain);
        }
        // The copy taken before the edits is as it was.
        assert!(before == *lines(3_000));
        assert!(!before.ptr_eq(&text));
    }

    /// Two copies of a text, one edited, find what they share as two
    /// strings would: the chunks the edit did not touch by pointer.
    #[test]
    fn copies_find_what_they_start_and_end_with() {
        let plain = lines(5_000);
        let original = Text::new(&plain);
        for (start, end, with) in [
            (0, 0, "x"),
            (40_000, 40_010, "abc\n"),
            (plain.len(), plain.len(), "!"),
        ] {
            let mut edited = original.clone();
            edited.replace_range(start..end, with);
            let mut other = plain.clone();
            other.replace_range(start..end, with);
            let prefix = plain
                .bytes()
                .zip(other.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            let room = (plain.len() - prefix).min(other.len() - prefix);
            let suffix = plain
                .bytes()
                .rev()
                .zip(other.bytes().rev())
                .take(room)
                .take_while(|(a, b)| a == b)
                .count();
            assert_eq!(original.common_prefix(&edited), prefix);
            assert_eq!(original.common_suffix(&edited, prefix), suffix);
        }
        assert_eq!(
            original.longest_line(),
            plain.lines().map(str::len).max().unwrap()
        );
    }

    /// An edit copies the chunks it touches and a list of pointers, not the
    /// text: a copy of the text shares the rest.
    #[test]
    fn an_edit_shares_the_chunks_it_does_not_touch() {
        let original = Text::new(&lines(20_000));
        let mut edited = original.clone();
        edited.replace_range(100..100, "x");
        let shared = original
            .0
            .chunks
            .iter()
            .zip(&edited.0.chunks)
            .filter(|(a, b)| Arc::ptr_eq(&a.text, &b.text))
            .count();
        assert!(shared >= original.0.chunks.len() - 1, "{shared}");
    }

    /// Texts in pieces cut in different places compare as the strings they
    /// make; a value is equal to its text however it is held.
    #[test]
    fn pieces_compare_as_their_strings_and_values_as_their_text() {
        let plain = lines(4_000);
        let long = Text::new(&plain);
        let mut edited = plain.clone();
        edited.replace_range(30_000..30_010, "changed!");
        let halves = [&edited[..12_345], &edited[12_345..]];
        let (prefix, suffix) = common(&long.chunks().collect::<Vec<_>>(), &halves);
        let expect_prefix = plain
            .bytes()
            .zip(edited.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let expect_suffix = plain
            .bytes()
            .rev()
            .zip(edited.bytes().rev())
            .take(plain.len().min(edited.len()) - expect_prefix)
            .take_while(|(a, b)| a == b)
            .count();
        assert_eq!((prefix, suffix), (expect_prefix, expect_suffix));
        assert_eq!(common(&[&plain], &[&plain]), (plain.len(), 0));
        assert_eq!(common(&["abc"], &["abd"]), (2, 0));
        assert_eq!(common(&["xab", "c"], &["y", "abc"]), (0, 3));
        // Values: a long one in chunks, a short one a string, equal by text.
        let value = TextValue::from(plain.as_str());
        assert!(value.as_text().is_some() && value == TextValue::Flat(plain.clone()));
        let other = TextValue::from(edited.as_str());
        assert!(value == *plain.as_str() && value != other);
        assert!(TextValue::from("short").as_text().is_none());
        assert!(value.ptr_eq(&value.clone()) && !value.ptr_eq(&TextValue::from(plain.as_str())));
        assert_eq!(value.to_string(), plain);
    }

    /// Whether a text holds a carriage return follows its edits, wherever
    /// they cut its chunks, without reading it.
    #[test]
    fn a_text_knows_whether_it_holds_a_carriage_return() {
        let plain = "line of text\n".repeat(4 * CHUNK / 13);
        let mut text = Text::new(&plain);
        assert!(!text.has_returns());
        let at = 2 * CHUNK + 7;
        text.replace_range(at..at, "\r");
        assert!(text.has_returns());
        text.replace_range(0..0, &"x".repeat(CHUNK));
        assert!(text.has_returns(), "the chunk holding it cut again");
        let at = text.to_cow().find('\r').unwrap();
        text.replace_range(at..at + 1, "");
        assert!(!text.has_returns());
        assert!(Text::new("a\r\nb").has_returns() && !Text::new("").has_returns());
    }
}
