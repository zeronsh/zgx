//! Markdown as scripts draw it: blocks of styled runs, parsed in Rust (from
//! Rust with [`parse`] and [`Incremental`], from scripts with
//! `ui.markdown`), so a script only lays out what it is given.
//!
//! A block is a table with `t`:
//! - `p` (`s`: runs), `h` (`l`: level, `s`), `code` (`lang`, `text`, and
//!   what [`Options::code`] adds), `ul` / `ol` (`start`, `items`: a list of
//!   block lists), `quote` (`b`), `hr`, `table` (`head`: cells, `rows`:
//!   rows of cells; a cell is runs), `img` (`src`, `alt`).
//!
//! A list item's own text (a tight list's, never a paragraph) is a `p`
//! with `tight`, which has no space under it.
//!
//! A run is `{ x = text, b?, i?, c? (code), s? (strike), a? (link),
//! f? (a repository file, with [`Options::paths`]) }`. A link whose address
//! is still streaming has `a` = [`PENDING_LINK_URL`]: style it as a link,
//! but it goes nowhere yet. Raw HTML is dropped.
//!
//! Streaming text parses incrementally ([`Incremental`]): an append parses
//! again only from the last top-level blocks, and the blocks before them
//! are the same values as before, so what reads them (a script's
//! `ui.each` over them, a reconciling [`crate::Shared`]) skips them at a
//! pointer's glance.
mod mend;
pub(crate) mod view;

pub use mend::PENDING_LINK_URL;

use crate::Value;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Parser, Tag, TagEnd};
use std::rc::Rc;

/// How text is read.
#[derive(Clone, Default)]
pub struct Options {
    /// Single newlines break lines (as people's messages show them), rather
    /// than joining them as CommonMark does.
    pub breaks: bool,
    /// Repository paths in text (`src/main.rs:12`), in code spans that are a
    /// path whole, and relative links that read as one, are files (`f`).
    pub paths: bool,
    /// More fields for a code block, from its language and text (coloured
    /// lines, say): `(name, value)` pairs.
    pub code: Option<CodeFields>,
}

/// A code block's added fields, from its language and text.
pub type CodeFields = Rc<dyn Fn(&str, &str) -> Vec<(&'static str, Value)>>;

/// Markdown parsed into blocks.
pub fn parse(text: &str, options: &Options) -> Vec<Value> {
    parse_at(text, 0, options)
        .into_iter()
        .flat_map(|top| top.blocks)
        .collect()
}

/// Streaming markdown: [`set_text`](Self::set_text) with the text so far
/// parses only what an append can have changed (the last two top-level
/// blocks; a link reference definition anywhere means the whole text), and
/// [`blocks`](Self::blocks) shares the blocks before that with the last
/// answer. While a marker hangs at the end (`**bold`, a link's address half
/// there), the blocks show it closed, so the text does not restyle and
/// reflow when the closer arrives.
#[derive(Default)]
pub struct Incremental {
    options: Options,
    source: String,
    tops: Vec<Top>,
    /// The last top-level block's blocks as shown, mended; `None` when
    /// nothing hangs.
    display_tail: Option<Vec<Value>>,
    /// Link reference definitions act at a distance: whole parses only.
    full_only: bool,
    /// Bytes parsed by the last update: an append's work is its tail's.
    last_parse_bytes: usize,
    /// The last top-level block's mending so far, and where that block
    /// starts: an append to it scans only what came.
    mender: Option<(usize, mend::Mender)>,
    /// The long text (`Value::Text`) the source is, by its id (`Text::id`;
    /// 0 for none): its next version is followed by what it inserted.
    taken: u64,
    /// A text landing whole, parsed over time (`set_text_lazy`): top-level
    /// elements below this one may be pending, the next looked for from
    /// here down; 0 for none. (In the struct's padding: a parser that never
    /// parses lazily is the size it was.)
    pending_below: u32,
}

/// A top-level element: where it starts, and the blocks it made (one, or
/// none for raw HTML). One not parsed yet has its stand-in
/// (`pending_block`).
#[derive(Clone)]
struct Top {
    start: usize,
    blocks: Vec<Value>,
}
impl Top {
    fn pending(&self) -> bool {
        matches!(&self.blocks[..], [block]
            if matches!(block.field("t"), Some(Value::Str(t)) if &**t == PENDING))
    }
}
/// A stand-in block's `t`.
const PENDING: &str = "pending";

impl Incremental {
    /// A parser for markdown rendered with `options`, with no text yet.
    pub fn new(options: Options) -> Self {
        Self {
            options,
            ..Self::default()
        }
    }
    /// The text parsed so far.
    pub fn source(&self) -> &str {
        &self.source
    }
    /// Bytes parsed by the last [`set_text`](Self::set_text) (0: the same
    /// text).
    pub fn last_parse_bytes(&self) -> usize {
        self.last_parse_bytes
    }
    /// The text so far: an append takes the incremental path, anything else
    /// parses afresh.
    pub fn set_text(&mut self, text: &str) {
        self.taken = 0;
        if text.len() >= self.source.len() && text.starts_with(self.source.as_str()) {
            let delta = &text[self.source.len()..];
            if delta.is_empty() {
                self.last_parse_bytes = 0;
                return;
            }
            self.append(delta);
        } else {
            self.reset(text);
        }
    }
    /// The text a state holds. A long text (`Value::Text`) that grew from
    /// the one taken last, by inserts at its end (`state:append`), is
    /// parsed from those alone: the document is neither copied nor compared
    /// again. Anything else is [`set_text`](Self::set_text)'s.
    pub fn set_value(&mut self, value: &Value) {
        let Value::Text(text) = value else {
            return match value {
                Value::Str(text) => self.set_text(text),
                other => self.set_text(&other.to_string()),
            };
        };
        // Followed by its edits from here on.
        text.keep_edits();
        if self.taken != 0
            && let Some(inserts) = text.inserts_since(self.taken)
        {
            // Each at the end of the text before it.
            let mut end = self.source.len();
            let at_end = inserts.clone().all(|(at, inserted)| {
                let ok = at == end;
                end += inserted.len();
                ok
            });
            if at_end && end == text.len() {
                let mut inserts = inserts.map(|(_, inserted)| inserted);
                match (inserts.next(), inserts.next()) {
                    (None, _) => self.last_parse_bytes = 0,
                    (Some(one), None) => self.append(one),
                    (Some(first), Some(second)) => {
                        // A burst: parsed once, as one append.
                        let mut delta = format!("{first}{second}");
                        delta.extend(inserts);
                        self.append(&delta);
                    }
                }
                self.taken = text.id();
                return;
            }
        }
        self.set_text(&text.to_cow());
        self.taken = text.id();
    }
    /// [`set_text`](Self::set_text), except that a text of `threshold`
    /// bytes or more that is not an append (one landing whole) is only
    /// scanned: each top-level element is a block `{ t = "pending", hint,
    /// lines, from, to }` (what it is by its first token, its lines, its
    /// bytes, 1-based and inclusive) until
    /// [`parse_pending`](Self::parse_pending) parses it, the last first. The last element is
    /// parsed at once; an append goes on as ever, the pending ones before it
    /// staying pending.
    pub fn set_text_lazy(&mut self, text: &str, threshold: usize) {
        // (From nothing, the text lands whole.)
        let extends = !self.source.is_empty()
            && text.len() >= self.source.len()
            && text.starts_with(self.source.as_str());
        if extends || text.len() < threshold || has_link_defs(text) {
            return self.set_text(text);
        }
        self.taken = 0;
        self.mender = None;
        self.source = text.to_string();
        self.full_only = false;
        self.tops = scan_tops(text);
        for at in 0..self.tops.len() {
            // What it is, if it makes a block (raw HTML doesn't).
            if let Some(Value::Str(hint)) = self.tops[at].blocks.first().cloned() {
                let (from, to) = self.top_range(at);
                self.tops[at].blocks = vec![pending_block(text, from, to, hint)];
            }
        }
        self.pending_below = u32::try_from(self.tops.len()).unwrap_or(u32::MAX);
        self.last_parse_bytes = text.len();
        // The end, which shows first and which an append mends.
        if let Some(last) = self.tops.len().checked_sub(1) {
            self.parse_top(last);
        }
        self.remend();
    }
    /// Whether [`set_text_lazy`](Self::set_text_lazy) left blocks
    /// unparsed.
    pub fn has_pending(&self) -> bool {
        self.pending_below > 0
    }
    /// Whether block `index` (of [`blocks`](Self::blocks)) is a stand-in
    /// not parsed yet.
    pub fn block_pending(&self, index: usize) -> bool {
        self.top_of(index).is_some_and(|at| self.tops[at].pending())
    }
    /// The top-level element block `index` (of [`blocks`](Self::blocks)) is
    /// in.
    fn top_of(&self, index: usize) -> Option<usize> {
        let mut seen = 0;
        self.tops.iter().position(|top| {
            seen += top.blocks.len();
            seen > index
        })
    }
    /// Parse pending top-level elements, block `first` (of
    /// [`blocks`](Self::blocks), a search found something in it) before the
    /// others, then from the end down, until `budget` bytes of them are
    /// parsed: whether any remain.
    pub fn parse_pending(&mut self, budget: usize, first: Option<usize>) -> bool {
        let mut spent = 0;
        let mut first = first
            .and_then(|index| self.top_of(index))
            .filter(|&at| self.tops[at].pending());
        while self.pending_below > 0 && spent < budget {
            let at = match first.take() {
                Some(at) => at,
                None => {
                    let below = (self.pending_below as usize).min(self.tops.len());
                    let Some(at) = self.tops[..below].iter().rposition(Top::pending) else {
                        self.pending_below = 0;
                        break;
                    };
                    self.pending_below = at as u32 + 1;
                    at
                }
            };
            let (from, to) = self.top_range(at);
            spent += to - from;
            if !self.parse_top(at) {
                // Not one element alone after all: the whole text, as ever.
                let text = std::mem::take(&mut self.source);
                self.reset(&text);
                return false;
            }
        }
        self.pending_below > 0
    }
    /// The bytes of top-level element `at`: from its line's start to the
    /// next one's.
    fn top_range(&self, at: usize) -> (usize, usize) {
        let line = |at: usize| self.source[..at].rfind('\n').map_or(0, |i| i + 1);
        let from = line(self.tops[at].start);
        let to = match self.tops.get(at + 1) {
            Some(next) => line(next.start),
            None => self.source.len(),
        };
        (from, to.max(from))
    }
    /// Parse pending element `at` alone: false if its bytes don't make one
    /// element by themselves.
    fn parse_top(&mut self, at: usize) -> bool {
        if !self.tops[at].pending() {
            return true;
        }
        let (from, to) = self.top_range(at);
        let mut parsed = parse_at(&self.source[from..to], from, &self.options);
        if parsed.len() != 1 || parsed[0].blocks.len() > 1 {
            return false;
        }
        self.tops[at].blocks = std::mem::take(&mut parsed[0].blocks);
        true
    }
    fn reset(&mut self, text: &str) {
        self.pending_below = 0;
        self.mender = None;
        self.source = text.to_string();
        self.full_only = has_link_defs(text);
        self.tops = parse_at(text, 0, &self.options);
        self.last_parse_bytes = text.len();
        self.remend();
    }
    fn append(&mut self, delta: &str) {
        // The delta may finish a line begun earlier: look for definitions
        // from that line's start.
        let scan_from = self.source.rfind('\n').map_or(0, |i| i + 1);
        self.source.push_str(delta);
        if !self.full_only && has_link_defs(&self.source[scan_from..]) {
            self.full_only = true;
        }
        if self.full_only {
            self.tops = parse_at(&self.source, 0, &self.options);
            self.last_parse_bytes = self.source.len();
            self.remend();
            return;
        }
        // From the start of the second-to-last top-level block, at its
        // line's start (indentation keeps its meaning): a trailing paragraph
        // like `3` can become `3.` and join the list before it, but a block's
        // separation from the one before is settled by its own first bytes,
        // so two blocks back is always enough.
        let boundary = match self.tops.len() {
            0 | 1 => 0,
            n => self.tops[n - 2].start,
        };
        let boundary = self.source[..boundary].rfind('\n').map_or(0, |i| i + 1);
        let mut tail = parse_at(&self.source[boundary..], boundary, &self.options);
        self.last_parse_bytes = self.source.len() - boundary;
        let at = self.tops.partition_point(|t| t.start < boundary);
        // A block parsed again the same keeps its value.
        for (new, old) in tail.iter_mut().zip(&self.tops[at..]) {
            if new.start == old.start && new.blocks == old.blocks {
                new.blocks = old.blocks.clone();
            }
        }
        self.tops.truncate(at);
        self.tops.extend(tail);
        // Pending ones are above the tail parsed again.
        if self.pending_below > 0 {
            self.pending_below = self.pending_below.min(at as u32);
        }
        self.remend();
    }
    /// Mend hanging inline markers in the last top-level block, the only
    /// place they can hang (a blank line settles a block).
    fn remend(&mut self) {
        self.display_tail = None;
        let Some(last) = self.tops.last() else { return };
        // Code shows an open fence as it is; rules and tables have no
        // inline tail to mend.
        let kind = last.blocks.first().and_then(|b| b.field("t")).cloned();
        if matches!(&kind, Some(Value::Str(t)) if matches!(&**t, "code" | "hr" | "table")) {
            self.mender = None;
            return;
        }
        let start = last.start;
        // The same block grown (the source only grows between resets):
        // its mending goes on from where it stopped.
        let mender = match &mut self.mender {
            Some((at, mender)) if *at == start => mender,
            slot => &mut slot.insert((start, mend::Mender::default())).1,
        };
        let Some(mended) = mender.close(&self.source[start..]) else {
            return;
        };
        self.last_parse_bytes += mended.len();
        let tail = parse_at(&mended, start, &self.options);
        self.display_tail = Some(tail.into_iter().flat_map(|t| t.blocks).collect());
    }
    /// The blocks to show: those before the last top-level block are the
    /// same values as the last time they were asked for, while they last.
    pub fn blocks(&self) -> Vec<Value> {
        let stable = self.tops.len().saturating_sub(1);
        let mut out: Vec<Value> = self.tops[..stable]
            .iter()
            .flat_map(|t| t.blocks.iter().cloned())
            .collect();
        match &self.display_tail {
            Some(tail) => out.extend(tail.iter().cloned()),
            None => out.extend(
                self.tops[stable..]
                    .iter()
                    .flat_map(|t| t.blocks.iter().cloned()),
            ),
        }
        out
    }
}

/// `ui.markdown(text, { key?, breaks?, paths? })`: the blocks. With `key`
/// (a message's id), text that grows under the same key parses
/// incrementally, sharing its earlier blocks with the last answer; without,
/// the same text answers with the same blocks.
pub(crate) fn script(
    lua: &mlua::Lua,
    (text, opts): (mlua::LuaString, Option<mlua::Table>),
) -> mlua::Result<Value> {
    let text = text.to_str()?;
    let flag = |name: &str| -> mlua::Result<bool> {
        Ok(match &opts {
            Some(t) => t.get::<Option<bool>>(name)?.unwrap_or(false),
            None => false,
        })
    };
    let options = Options {
        breaks: flag("breaks")?,
        paths: flag("paths")?,
        code: None,
    };
    let key: Option<String> = match &opts {
        Some(t) => t.get("key")?,
        None => None,
    };
    // `lazy = true` (4 KB) or a byte count: a keyed text that long landing
    // whole is parsed over frames, the end first (`set_text_lazy`).
    let lazy = match &opts {
        Some(t) => match t.get::<mlua::Value>("lazy")? {
            mlua::Value::Boolean(true) => Some(LAZY_FROM),
            mlua::Value::Integer(n) => Some(n.max(0) as usize),
            mlua::Value::Number(n) if n.is_finite() => Some(n.max(0.) as usize),
            mlua::Value::Nil | mlua::Value::Boolean(false) => None,
            _ => {
                return Err(mlua::Error::runtime(
                    "ui.markdown `lazy` is true or a byte count",
                ));
            }
        },
        None => None,
    };
    let first: Option<usize> = match &opts {
        Some(t) => t.get::<Option<usize>>("first")?,
        None => None,
    };
    // (The VM registers its cache when it starts: `install`.)
    let Some(parsed) = lua.app_data_ref::<Parsed>() else {
        return Ok(Value::List(parse(&text, &options).into()));
    };
    let flags = (options.breaks, options.paths);
    Ok(match key {
        Some(key) => {
            let mut streams = parsed.streams.borrow_mut();
            let fresh = !matches!(streams.get(&key), Some((f, ..)) if *f == flags);
            if fresh {
                streams.insert(key.clone(), (flags, Incremental::new(options), None, None));
            }
            let (_, stream, last, kept) = streams.get_mut(&key).expect("inserted above");
            match lazy {
                Some(threshold) => stream.set_text_lazy(&text, threshold),
                None => stream.set_text(&text),
            }
            let pending = stream.has_pending();
            let blocks = Value::List(stream.blocks().into());
            // Its table from the last answer's, which holds the same blocks
            // but the end: only what's new is converted and weighed, not
            // the whole list each time a chunk lands (`value::derive`).
            if let Some(old) = last.replace(blocks.clone()) {
                // The last answer's table, if the sweep left none kept: a
                // script read it since.
                if kept.is_none() {
                    *kept = crate::value::table_of(lua, &old)?;
                }
                crate::value::derive(lua, &old, &blocks)?;
                *kept = crate::value::table_of(lua, &blocks)?;
            }
            touch(
                &mut parsed.stream_order.borrow_mut(),
                key.clone(),
                &mut streams,
            );
            drop(streams);
            if pending {
                follow_lazy(lua, key, first.and_then(|i| i.checked_sub(1)))?;
            }
            blocks
        }
        None => {
            let mut texts = parsed.texts.borrow_mut();
            let at = (text.to_string(), flags);
            if let Some(blocks) = texts.get(&at) {
                return Ok(blocks.clone());
            }
            let blocks = Value::List(parse(&text, &options).into());
            if texts.len() >= KEPT {
                texts.clear();
            }
            texts.insert(at, blocks.clone());
            blocks
        }
    })
}

/// A stream with blocks pending: the ticker parsing them over frames, and
/// the state that says they moved on, which the caller (a computed) reads to
/// run again as they come.
struct Ticker {
    /// Its VM (`Rc::as_ptr`) and stream.
    vm: *const crate::Vm,
    key: String,
    progress: zgx::reactive::Signal<u64>,
    /// The block to parse before the others (`first`, from 0).
    first: std::rc::Rc<std::cell::Cell<Option<usize>>>,
    _stop: crate::timers::Stop,
}
thread_local! {
    /// The streams parsing lazily, of every VM on the thread: none (and no
    /// room held) once they are done.
    static TICKERS: std::cell::RefCell<Vec<Ticker>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Stream `key` has blocks pending: follow them (the caller runs again as
/// they come in), ticking them in over frames, `first` before the others.
fn follow_lazy(lua: &mlua::Lua, key: String, first: Option<usize>) -> mlua::Result<()> {
    let env = crate::node::env(lua)?;
    let vm_ptr = env.vm.as_ptr();
    let found = TICKERS.with_borrow(|tickers| {
        tickers
            .iter()
            .find(|t| t.vm == vm_ptr && t.key == key)
            .map(|t| {
                if first.is_some() {
                    t.first.set(first);
                }
                t.progress.get();
            })
            .is_some()
    });
    if found {
        return Ok(());
    }
    let progress = env.runtime.signal(0_u64);
    progress.get();
    let asked = std::rc::Rc::new(std::cell::Cell::new(first));
    let (vm, tick, stream_key, wanted) =
        (env.vm.clone(), progress.clone(), key.clone(), asked.clone());
    let mut ticks = 0_u32;
    let stop = env.timers.start(LAZY_TICK, true, move || {
        let done = |tick: &zgx::reactive::Signal<u64>| {
            // Let go: no room held for the next.
            TICKERS.with_borrow_mut(|tickers| {
                tickers.retain(|t| !(t.vm == vm.as_ptr() && t.key == stream_key));
                if tickers.is_empty() {
                    *tickers = Vec::new();
                }
            });
            tick.set(tick.with_untracked(|n| n.wrapping_add(1)));
            false
        };
        let Some(live) = vm.upgrade() else {
            return done(&tick);
        };
        let Some(parsed) = live.lua.app_data_ref::<Parsed>() else {
            return done(&tick);
        };
        let first = wanted.take();
        let (more, asked) = match parsed.streams.borrow_mut().get_mut(&stream_key) {
            Some((_, stream, ..)) => {
                let more = stream.parse_pending(LAZY_BUDGET, first);
                (more, first.is_some_and(|i| !stream.block_pending(i)))
            }
            None => (false, false),
        };
        drop(parsed);
        if !more {
            return done(&tick);
        }
        ticks += 1;
        // Told when the blocks nearest the end are in (what shows first)
        // and when a block asked for is (and, above, when all are): each
        // telling builds the list's tables again, so not every tick.
        if ticks == 1 || asked {
            tick.set(tick.with_untracked(|n| n.wrapping_add(1)));
        }
        true
    });
    TICKERS.with_borrow_mut(|tickers| {
        tickers.push(Ticker {
            vm: vm_ptr,
            key,
            progress,
            first: asked,
            _stop: stop,
        })
    });
    Ok(())
}

/// `lazy = true`'s threshold: shorter texts parse at once.
const LAZY_FROM: usize = 4096;
/// Bytes a lazy text's ticker parses a tick, and how often it ticks: a
/// frame's share, about a frame apart.
const LAZY_BUDGET: usize = 8192;
const LAZY_TICK: std::time::Duration = std::time::Duration::from_millis(8);

/// Give a new VM its parsed markdown.
pub(crate) fn install(lua: &mlua::Lua) {
    lua.set_app_data(Parsed::default());
}

/// A VM at rest: streams not asked for lately go. (The texts parsed stay: a
/// chat opened again would parse them in its first frame.)
pub(crate) fn rest(lua: &mlua::Lua) {
    let Some(parsed) = lua.app_data_ref::<Parsed>() else {
        return;
    };
    let mut streams = parsed.streams.borrow_mut();
    let mut order = parsed.stream_order.borrow_mut();
    let now = std::time::Instant::now();
    while order
        .front()
        .is_some_and(|(_, used)| now.duration_since(*used) > STREAM_QUIET)
    {
        if let Some((old, _)) = order.pop_front() {
            streams.remove(&old);
        }
    }
    streams.shrink_to_fit();
    order.shrink_to_fit();
}

/// A stream left this long has ended, or parses again if it goes on.
const STREAM_QUIET: std::time::Duration = std::time::Duration::from_secs(60);

/// How many texts and streams a script's `ui.markdown` keeps.
const KEPT: usize = 256;

/// `breaks` and `paths`, as a script asked for them.
type Flags = (bool, bool);
type Texts = std::collections::HashMap<(String, Flags), Value>;
/// Each keyed stream: its flags, its parse, and its last answer (whose
/// table the next one's is derived from), with that table once a script
/// has it, kept so the host's idle sweep doesn't let it go (only scripts'
/// holding counts there): a stream pausing between chunks goes idle.
type Streams =
    std::collections::HashMap<String, (Flags, Incremental, Option<Value>, Option<mlua::Table>)>;

/// A VM's parsed markdown.
#[derive(Default)]
struct Parsed {
    texts: std::cell::RefCell<Texts>,
    streams: std::cell::RefCell<Streams>,
    /// Streams by last use, oldest first.
    stream_order: std::cell::RefCell<std::collections::VecDeque<(String, std::time::Instant)>>,
}
/// `key` was just used: last in the order, and the oldest streams go past
/// `KEPT`.
fn touch(
    order: &mut std::collections::VecDeque<(String, std::time::Instant)>,
    key: String,
    streams: &mut Streams,
) {
    let now = std::time::Instant::now();
    match order.back_mut() {
        Some((last, used)) if *last == key => *used = now,
        _ => {
            order.retain(|(k, _)| *k != key);
            order.push_back((key, now));
        }
    }
    while order.len() > KEPT {
        if let Some((old, _)) = order.pop_front() {
            streams.remove(&old);
        }
    }
}

/// `text`'s top-level elements, each with what it is by its first
/// token (`p`, `h`, `code`, `list`, `quote`, `table`, `hr`) as its one
/// block's stand-in, found with the parser's events alone (what makes the
/// blocks is most of a parse); raw HTML, which makes none, not pending.
fn scan_tops(text: &str) -> Vec<Top> {
    parse_or_scan(text, 0, &Options::default(), true)
}

/// The stand-in for a top-level element not parsed yet.
fn pending_block(text: &str, from: usize, to: usize, hint: Rc<str>) -> Value {
    let own = text[from..to].trim_end();
    let num = |n: usize| Value::Num(n as f64);
    map(vec![
        ("t", Value::Str(Rc::from(PENDING))),
        ("hint", Value::Str(hint)),
        ("lines", num(own.lines().count().max(1))),
        ("from", num(from + 1)),
        ("to", num(from + own.len())),
    ])
}

fn parser_flags() -> pulldown_cmark::Options {
    let mut flags = pulldown_cmark::Options::empty();
    flags.insert(pulldown_cmark::Options::ENABLE_TABLES);
    flags.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
    flags.insert(pulldown_cmark::Options::ENABLE_TASKLISTS);
    flags
}

/// Link reference definition lines (`[label]: destination`, up to three
/// spaces in), conservatively.
fn has_link_defs(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        line.len() - trimmed.len() <= 3 && trimmed.starts_with('[') && trimmed.contains("]:")
    })
}

/// `text` (which starts at `offset` in the whole) parsed into its top-level
/// elements.
fn parse_at(text: &str, offset: usize, options: &Options) -> Vec<Top> {
    parse_or_scan(text, offset, options, false)
}
/// `parse_at`, or with `scan` its top-level elements alone (`scan_tops`):
/// one loop over the parser's events, so the parser's iterator has the one
/// caller it is inlined into.
fn parse_or_scan(text: &str, offset: usize, options: &Options, scan: bool) -> Vec<Top> {
    let flags = parser_flags();
    let mut builder = Builder {
        frames: vec![Frame::Blocks(Vec::new())],
        runs: Vec::new(),
        styles: Vec::new(),
        links: Vec::new(),
        code: None,
        heading: None,
        image: None,
        picture: None,
        line: (0, 0),
        took: false,
        options,
        deeper: 0,
    };
    let mut tops = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (event, range) in Parser::new_ext(text, flags).into_offset_iter() {
        let opens = matches!(event, Event::Start(_));
        let closes = matches!(event, Event::End(_));
        if depth == 0 {
            start = range.start;
            if scan {
                let hint = match &event {
                    Event::Start(Tag::Paragraph) => Some("p"),
                    Event::Start(Tag::Heading { .. }) => Some("h"),
                    Event::Start(Tag::CodeBlock(_)) => Some("code"),
                    Event::Start(Tag::List(_)) => Some("list"),
                    Event::Start(Tag::BlockQuote(_)) => Some("quote"),
                    Event::Start(Tag::Table(_)) => Some("table"),
                    Event::Rule => Some("hr"),
                    _ => None,
                };
                // What it is stands in until `set_text_lazy` makes its block.
                tops.push(Top {
                    start: offset + start,
                    blocks: hint.map(|h| Value::Str(Rc::from(h))).into_iter().collect(),
                });
            }
        }
        if !scan {
            builder.event(event);
        }
        if opens {
            depth += 1;
        } else if closes {
            depth = depth.saturating_sub(1);
        }
        // A top-level element is done: what it made is its blocks.
        if depth == 0 && !scan {
            let blocks = match builder.frames.first_mut() {
                Some(Frame::Blocks(blocks)) => std::mem::take(blocks),
                _ => Vec::new(),
            };
            tops.push(Top {
                start: offset + start,
                blocks,
            });
        }
    }
    tops
}

#[derive(Default, Clone, Copy)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
}

#[derive(Default)]
struct Run {
    text: String,
    style: Style,
    code: bool,
    link: Option<String>,
    file: Option<String>,
    /// A picture among the words: its source, its text the alt's.
    img: Option<String>,
}

/// Where a link goes: a page, or (with `paths`) a file in the repository.
#[derive(Clone, PartialEq)]
enum Link {
    Url(String),
    File(String),
}

fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// A path segment's characters (`[\w@.-]`).
fn segment_char(c: char) -> bool {
    word(c) || c == '@' || c == '.' || c == '-'
}

/// `s` without a trailing `:line` or `:line:column`.
fn strip_position(s: &str) -> &str {
    let mut out = s;
    for _ in 0..2 {
        if let Some(i) = out.rfind(':') {
            let digits = &out[i + 1..];
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                out = &out[..i];
                continue;
            }
        }
        break;
    }
    out
}

/// Whether `s` is, whole, a repository path:
/// `(?:[\w@.-]+/)+[\w@.-]+\.[a-z\d]{1,8}(?::\d+(?::\d+)?)?`.
fn is_path(s: &str) -> bool {
    let path = strip_position(s);
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2
        || segments
            .iter()
            .any(|seg| seg.is_empty() || !seg.chars().all(segment_char))
    {
        return false;
    }
    let last = segments[segments.len() - 1];
    match last.rfind('.') {
        Some(dot) if dot > 0 => {
            let ext = &last[dot + 1..];
            (1..=8).contains(&ext.len())
                && ext
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        }
        _ => false,
    }
}

/// `text` split into plain pieces and repository paths (the paths standing
/// alone: after a space, `(` or `[`, before a space or `,.;)]`).
fn split_paths(text: &str) -> Vec<(&str, bool)> {
    let mut out = Vec::new();
    let mut plain = 0;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        let after_boundary = i == 0 || matches!(chars[i - 1].1, ' ' | '\t' | '\n' | '(' | '[');
        if !after_boundary || !(segment_char(c)) {
            i += 1;
            continue;
        }
        // The longest stretch that may be a path, then back off to the
        // longest prefix that is one and ends where a path may end.
        let mut j = i;
        while j < chars.len()
            && (segment_char(chars[j].1) || chars[j].1 == '/' || chars[j].1 == ':')
        {
            j += 1;
        }
        let mut found = None;
        let mut k = j;
        while k > i {
            let end = if k < chars.len() {
                chars[k].0
            } else {
                text.len()
            };
            let ends_well = k == chars.len()
                || matches!(chars[k].1, ' ' | '\t' | '\n' | ',' | '.' | ';' | ')' | ']');
            if ends_well && is_path(&text[start..end]) {
                found = Some((k, end));
                break;
            }
            k -= 1;
        }
        match found {
            Some((k, end)) => {
                if plain < start {
                    out.push((&text[plain..start], false));
                }
                out.push((&text[start..end], true));
                plain = end;
                i = k;
            }
            None => i = j.max(i + 1),
        }
    }
    if plain < text.len() {
        out.push((&text[plain..], false));
    }
    out
}

/// A link's destination: with `paths`, a relative href without a query
/// that reads as a path (`src/a.rs`, `README.md:12`) is a file.
fn link_of(href: &str, paths: bool) -> Link {
    if !paths {
        return Link::Url(href.to_string());
    }
    let lower = href.to_ascii_lowercase();
    let scheme = href.find("://").is_some_and(|i| {
        i > 0
            && href[..i]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    });
    if scheme
        || href == PENDING_LINK_URL
        || lower.starts_with("mailto:")
        || lower.starts_with("data:")
        || lower.starts_with("javascript:")
        || href.starts_with('/')
        || href.contains('?')
    {
        return Link::Url(href.to_string());
    }
    let path = href.split('#').next().unwrap_or("");
    let first_ok = path
        .chars()
        .next()
        .is_some_and(|c| !matches!(c, '/' | '`' | ':' | '\\') && !c.is_whitespace());
    let rest_ok = path
        .chars()
        .all(|c| !matches!(c, '`' | '\\') && !c.is_whitespace());
    let stripped = strip_position(path);
    if first_ok && rest_ok && !stripped.contains(':') {
        Link::File(stripped.to_string())
    } else {
        Link::Url(href.to_string())
    }
}

fn map(fields: Vec<(&str, Value)>) -> Value {
    let mut fields: Vec<(Rc<str>, Value)> = fields.into_iter().map(|(k, v)| (key(k), v)).collect();
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    Value::Map(fields.into())
}

thread_local! {
    /// The field names blocks and runs are made with (`map`), each made
    /// once: a document's every block and run named its fields anew, an
    /// allocation each. At most `KEYS` (a code block's own fields add some).
    static NAMES: std::cell::RefCell<Vec<Rc<str>>> = const { std::cell::RefCell::new(Vec::new()) };
}
const KEYS: usize = 64;
/// `name` as the field name blocks share (`NAMES`).
fn key(name: &str) -> Rc<str> {
    NAMES.with_borrow_mut(|names| {
        if let Some(kept) = names.iter().find(|kept| &***kept == name) {
            return kept.clone();
        }
        let made: Rc<str> = Rc::from(name);
        if names.len() < KEYS {
            names.push(made.clone());
        }
        made
    })
}

fn runs_value(runs: &[Run]) -> Value {
    Value::List(
        runs.iter()
            .filter(|r| !r.text.is_empty() || r.img.is_some())
            .map(|r| {
                let mut f = vec![("x", Value::Str(r.text.as_str().into()))];
                if r.style.bold {
                    f.push(("b", Value::Bool(true)));
                }
                if r.style.italic {
                    f.push(("i", Value::Bool(true)));
                }
                if r.style.strike {
                    f.push(("s", Value::Bool(true)));
                }
                if r.code {
                    f.push(("c", Value::Bool(true)));
                }
                if let Some(link) = &r.link {
                    f.push(("a", Value::Str(link.as_str().into())));
                }
                if let Some(file) = &r.file {
                    f.push(("f", Value::Str(file.as_str().into())));
                }
                if let Some(img) = &r.img {
                    f.push(("img", Value::Str(img.as_str().into())));
                }
                map(f)
            })
            .collect(),
    )
}

/// What is being built at each level of nesting.
enum Frame {
    /// Blocks of a container (the document, a list item, a quote).
    Blocks(Vec<Value>),
    List {
        ordered: bool,
        start: u64,
        items: Vec<Value>,
    },
    Table {
        head: Vec<Value>,
        rows: Vec<Value>,
        row: Vec<Value>,
        in_head: bool,
    },
}

struct Builder<'a> {
    frames: Vec<Frame>,
    /// Inline runs of the paragraph, heading or cell being read.
    runs: Vec<Run>,
    styles: Vec<Style>,
    links: Vec<Link>,
    code: Option<(String, String)>,
    heading: Option<u8>,
    image: Option<(String, String)>,
    /// A picture read, whose place is known by what follows it: alone on
    /// its line (only blanks before it since `line`, a line's end after) it
    /// is a block; among words, a run. Its source, alt, whether only
    /// blanks came before it on its line, and the page it links to.
    picture: Option<(String, String, bool, Option<String>)>,
    /// Where the line being read starts in `runs`: a run, and a byte in it.
    line: (usize, usize),
    /// A picture took its paragraph's place (or its last line's): the
    /// paragraph leaves none empty behind.
    took: bool,
    options: &'a Options,
    /// Quotes, lists and items open beyond `NESTING`: read into the deepest
    /// container kept, as its blocks.
    deeper: usize,
}

/// How deeply quotes and lists nest at most (in frames: a list level is
/// two, its list and item). Text nesting thousands deep (`>>>>…`, `- - -
/// …`) would make values whose drop, conversion and display overflow the
/// stack; what nests deeper shows as the deepest level's blocks.
const NESTING: usize = 64;

impl Builder<'_> {
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_text(&mut self, text: &str, code: bool) {
        if let Some(image) = &mut self.image {
            image.1.push_str(text);
            return;
        }
        let paths = self.options.paths;
        match self.links.last().cloned() {
            Some(Link::Url(url)) => self.push_run(text, code, Some(url), None),
            Some(Link::File(path)) => self.push_run(text, false, None, Some(path)),
            // Text outside links and code: paths in it are files.
            None if !code && paths => {
                for (piece, path) in split_paths(text) {
                    let file = path.then(|| strip_position(piece).to_string());
                    self.push_run(piece, false, None, file);
                }
            }
            // Code that is a path, whole, is a file too.
            None if code && paths && is_path(text) => {
                self.push_run(text, false, None, Some(strip_position(text).to_string()))
            }
            None => self.push_run(text, code, None, None),
        }
    }

    fn push_run(&mut self, text: &str, code: bool, link: Option<String>, file: Option<String>) {
        let style = self.style();
        if let Some(last) = self.runs.last_mut() {
            let same = !code
                && !last.code
                && file.is_none()
                && last.file.is_none()
                && last.img.is_none()
                && last.link == link
                && {
                    let s = last.style;
                    s.bold == style.bold && s.italic == style.italic && s.strike == style.strike
                };
            if same {
                last.text.push_str(text);
                return;
            }
        }
        self.runs.push(Run {
            text: text.to_string(),
            style,
            code,
            link,
            file,
            img: None,
        });
    }

    /// A line starts here (a paragraph's, or after a break).
    fn mark_line(&mut self) {
        self.line = match self.runs.last() {
            Some(last) => (self.runs.len() - 1, last.text.len()),
            None => (0, 0),
        };
    }

    /// Whether only blanks have been read since the line started.
    fn blank_line(&self) -> bool {
        let (run, byte) = self.line;
        self.runs.iter().enumerate().skip(run).all(|(i, r)| {
            let text = if i == run {
                r.text.get(byte..).unwrap_or("")
            } else {
                &r.text
            };
            r.img.is_none() && text.trim().is_empty()
        })
    }

    /// The picture read (`picture`), placed by `event`, what follows it:
    /// alone on its line, the words before it are a paragraph and it a
    /// block (the words after it the next paragraph); else a run among the
    /// words. Whether the event was the line's end it took.
    fn place_picture(&mut self, event: &Event) -> bool {
        // Inline tags closing after it (a link around a badge) leave its
        // place to what follows them.
        if matches!(
            event,
            Event::End(TagEnd::Link | TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough)
        ) {
            return false;
        }
        let Some((src, alt, blank, link)) = self.picture.take() else {
            return false;
        };
        let ends = matches!(
            event,
            Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph | TagEnd::Item)
        );
        let blocks = matches!(self.frames.last(), Some(Frame::Blocks(_)));
        if !(blank && ends && blocks && self.heading.is_none()) {
            self.runs.push(Run {
                text: alt,
                style: self.style(),
                code: false,
                link,
                file: None,
                img: Some(src),
            });
            return false;
        }
        // The words before its line, as their paragraph.
        let (run, byte) = self.line;
        self.runs.truncate(run + 1);
        if let Some(last) = self.runs.get_mut(run) {
            last.text.truncate(byte);
        }
        if let Some(last) = self.runs.last_mut() {
            last.text.truncate(last.text.trim_end().len());
        }
        if self
            .runs
            .iter()
            .any(|r| !r.text.is_empty() || r.img.is_some())
        {
            let runs = self.take_runs();
            self.block(map(vec![("t", "p".into()), ("s", runs)]));
        }
        self.runs.clear();
        self.block(map(vec![
            ("t", "img".into()),
            ("src", src.as_str().into()),
            ("alt", alt.as_str().into()),
        ]));
        self.took = true;
        self.line = (0, 0);
        matches!(event, Event::SoftBreak | Event::HardBreak)
    }

    fn take_runs(&mut self) -> Value {
        let mut runs = std::mem::take(&mut self.runs);
        // Trailing line breaks inside a block are not shown.
        if let Some(last) = runs.last_mut() {
            while last.text.ends_with('\n') {
                last.text.pop();
            }
        }
        runs_value(&runs)
    }

    /// A tight item's text so far, before a block inside the item (a
    /// nested list): it is the item's first paragraph.
    fn flush_tight(&mut self) {
        if self.runs.is_empty() || self.code.is_some() || self.heading.is_some() {
            return;
        }
        let runs = self.take_runs();
        if let Some(Frame::Blocks(blocks)) = self.frames.last_mut() {
            blocks.push(map(vec![
                ("t", "p".into()),
                ("s", runs),
                ("tight", Value::Bool(true)),
            ]));
        }
    }

    fn block(&mut self, block: Value) {
        // Loose content directly in a list (tight lists have no
        // paragraphs) belongs to the item being read: dropped here.
        if let Some(Frame::Blocks(blocks)) = self.frames.last_mut() {
            blocks.push(block);
        }
    }

    fn event(&mut self, event: Event) {
        // A picture's place, by what follows it (its line's end, taken).
        if self.picture.is_some() && self.place_picture(&event) {
            return;
        }
        if let Event::Start(Tag::List(_) | Tag::CodeBlock(_) | Tag::BlockQuote(_) | Tag::Table(_))
        | Event::Rule = &event
        {
            self.flush_tight();
        }
        match &event {
            Event::Start(Tag::BlockQuote(_) | Tag::List(_))
                if self.deeper > 0 || self.frames.len() >= NESTING =>
            {
                self.deeper += 1;
                return;
            }
            Event::Start(Tag::Item) if self.deeper > 0 => {
                self.deeper += 1;
                return;
            }
            Event::End(TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item)
                if self.deeper > 0 =>
            {
                self.flush_tight();
                self.deeper -= 1;
                return;
            }
            _ => {}
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    self.heading = Some(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    })
                }
                Tag::BlockQuote(_) => self.frames.push(Frame::Blocks(Vec::new())),
                Tag::CodeBlock(kind) => {
                    let lang = match kind {
                        CodeBlockKind::Fenced(lang) => {
                            lang.split_whitespace().next().unwrap_or("").to_string()
                        }
                        CodeBlockKind::Indented => String::new(),
                    };
                    self.code = Some((lang, String::new()));
                }
                Tag::List(start) => self.frames.push(Frame::List {
                    ordered: start.is_some(),
                    start: start.unwrap_or(1),
                    items: Vec::new(),
                }),
                Tag::Item => self.frames.push(Frame::Blocks(Vec::new())),
                Tag::Table(_) => self.frames.push(Frame::Table {
                    head: Vec::new(),
                    rows: Vec::new(),
                    row: Vec::new(),
                    in_head: false,
                }),
                Tag::TableHead => {
                    if let Some(Frame::Table { in_head, .. }) = self.frames.last_mut() {
                        *in_head = true;
                    }
                }
                Tag::Emphasis => {
                    let mut s = self.style();
                    s.italic = true;
                    self.styles.push(s);
                }
                Tag::Strong => {
                    let mut s = self.style();
                    s.bold = true;
                    self.styles.push(s);
                }
                Tag::Strikethrough => {
                    let mut s = self.style();
                    s.strike = true;
                    self.styles.push(s);
                }
                Tag::Link { dest_url, .. } => {
                    let link = link_of(&dest_url, self.options.paths);
                    self.links.push(link)
                }
                Tag::Image { dest_url, .. } => {
                    self.image = Some((dest_url.to_string(), String::new()))
                }
                Tag::Paragraph => {
                    self.took = false;
                    self.line = (0, 0);
                }
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => {
                    // None is left empty where a picture took its place.
                    if !(self.took && self.runs.is_empty()) {
                        let runs = self.take_runs();
                        self.block(map(vec![("t", "p".into()), ("s", runs)]));
                    }
                    self.took = false;
                }
                TagEnd::Heading(_) => {
                    let runs = self.take_runs();
                    let level = self.heading.take().unwrap_or(1);
                    self.block(map(vec![
                        ("t", "h".into()),
                        ("l", Value::Num(level as f64)),
                        ("s", runs),
                    ]));
                }
                TagEnd::BlockQuote(_) => {
                    if let Some(Frame::Blocks(blocks)) = self.frames.pop() {
                        self.block(map(vec![
                            ("t", "quote".into()),
                            ("b", Value::List(blocks.into())),
                        ]));
                    }
                }
                TagEnd::CodeBlock => {
                    if let Some((lang, mut text)) = self.code.take() {
                        if text.ends_with('\n') {
                            text.pop();
                        }
                        let mut fields = vec![
                            ("t", "code".into()),
                            ("lang", lang.as_str().into()),
                            ("text", text.as_str().into()),
                        ];
                        if let Some(code) = &self.options.code {
                            fields.extend(code(&lang, &text));
                        }
                        self.block(map(fields));
                    }
                }
                TagEnd::Item => {
                    // A tight item's text never became a paragraph.
                    self.flush_tight();
                    if let Some(Frame::Blocks(blocks)) = self.frames.pop()
                        && let Some(Frame::List { items, .. }) = self.frames.last_mut()
                    {
                        items.push(Value::List(blocks.into()));
                    }
                }
                TagEnd::List(_) => {
                    if let Some(Frame::List {
                        ordered,
                        start,
                        items,
                    }) = self.frames.pop()
                    {
                        self.block(map(vec![
                            ("t", if ordered { "ol" } else { "ul" }.into()),
                            ("start", Value::Num(start as f64)),
                            ("items", Value::List(items.into())),
                        ]));
                    }
                }
                TagEnd::TableCell => {
                    let runs = self.take_runs();
                    if let Some(Frame::Table { row, .. }) = self.frames.last_mut() {
                        row.push(runs);
                    }
                }
                TagEnd::TableHead => {
                    if let Some(Frame::Table {
                        head, row, in_head, ..
                    }) = self.frames.last_mut()
                    {
                        *head = std::mem::take(row);
                        *in_head = false;
                    }
                }
                TagEnd::TableRow => {
                    if let Some(Frame::Table { rows, row, .. }) = self.frames.last_mut() {
                        rows.push(Value::List(std::mem::take(row).into()));
                    }
                }
                TagEnd::Table => {
                    if let Some(Frame::Table { head, rows, .. }) = self.frames.pop() {
                        self.block(map(vec![
                            ("t", "table".into()),
                            ("head", Value::List(head.into())),
                            ("rows", Value::List(rows.into())),
                        ]));
                    }
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    self.styles.pop();
                }
                TagEnd::Link => {
                    self.links.pop();
                }
                TagEnd::Image => {
                    // Placed by what follows it (`place_picture`).
                    if let Some((src, alt)) = self.image.take() {
                        let blank = self.blank_line();
                        let link = match self.links.last() {
                            Some(Link::Url(url)) => Some(url.clone()),
                            _ => None,
                        };
                        self.picture = Some((src, alt, blank, link));
                    }
                }
                _ => {}
            },
            Event::Text(text) => match &mut self.code {
                Some((_, code)) => code.push_str(&text),
                None => self.push_text(&text, false),
            },
            Event::Code(text) => self.push_text(&text, true),
            Event::InlineMath(text) | Event::DisplayMath(text) => self.push_text(&text, true),
            Event::SoftBreak => {
                let breaks = self.options.breaks;
                self.push_text(if breaks { "\n" } else { " " }, false);
                self.mark_line();
            }
            Event::HardBreak => {
                self.push_text("\n", false);
                self.mark_line();
            }
            Event::Rule => self.block(map(vec![("t", "hr".into())])),
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::TaskListMarker(done) => self.push_text(if done { "☑ " } else { "☐ " }, false),
            Event::FootnoteReference(name) => self.push_text(&format!("[{name}]"), false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(blocks: &[Value]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| match b.field("t") {
                Some(Value::Str(t)) => t.to_string(),
                _ => "?".into(),
            })
            .collect()
    }
    fn plain() -> Options {
        Options::default()
    }
    fn paths() -> Options {
        Options {
            paths: true,
            ..Options::default()
        }
    }
    fn same(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Map(a), Value::Map(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// A picture on a line of its own in a paragraph (only blanks beside
    /// it) is a block of its own there, the words before and after it
    /// their own paragraphs, and none left empty. Among words it is a run
    /// keeping its source (`img`), its text its alt.
    #[test]
    fn a_picture_on_a_line_of_its_own_is_a_picture_and_among_words_a_run() {
        let cases: [(&str, &[&str]); 8] = [
            ("Here it is:\n![shot](a.png)", &["p", "img"]),
            ("![a](a.png)\n![b](b.png)", &["img", "img"]),
            ("![a](a.png)\n\nAfter.", &["img", "p"]),
            ("Before\n![a](a.png)\nAfter", &["p", "img", "p"]),
            ("Before  \n  ![a](a.png)  ", &["p", "img"]),
            ("[![badge](b.svg)](https://example.com)", &["img"]),
            ("see ![shot](a.png) here", &["p"]),
            ("- ![a](a.png)\n- text", &["ul"]),
        ];
        for (source, want) in cases {
            assert_eq!(kinds(&parse(source, &plain())), want, "{source:?}");
        }
        let text = |v: Option<&Value>| match v {
            Some(Value::Str(s)) => Some(s.to_string()),
            _ => None,
        };
        // The words before keep their text, without the line's break.
        let blocks = parse("Here it is:\n![shot](a.png)", &plain());
        let words = blocks[0].field("s").unwrap();
        let Value::List(runs) = words else { panic!() };
        assert_eq!(text(runs[0].field("x")).as_deref(), Some("Here it is:"));
        assert_eq!(text(blocks[1].field("src")).as_deref(), Some("a.png"));
        assert_eq!(text(blocks[1].field("alt")).as_deref(), Some("shot"));
        // Among words: a run with its source, its text the alt.
        let blocks = parse("see ![shot](a.png) here", &plain());
        let Some(Value::List(runs)) = blocks[0].field("s") else {
            panic!()
        };
        let texts: Vec<_> = runs
            .iter()
            .map(|r| (text(r.field("x")), text(r.field("img"))))
            .collect();
        let want = [("see ", None), ("shot", Some("a.png")), (" here", None)]
            .map(|(x, img)| (Some(x.to_string()), img.map(str::to_string)));
        assert_eq!(texts, want);
        // In a list item, alone: the item's block.
        let blocks = parse("- ![a](a.png)\n- text", &plain());
        let Some(Value::List(items)) = blocks[0].field("items") else {
            panic!()
        };
        let Value::List(first) = &items[0] else {
            panic!()
        };
        assert_eq!(kinds(first), ["img"]);
        // In a table's cell: a run (a cell holds runs).
        let blocks = parse("| a |\n|---|\n| ![p](p.png) |", &plain());
        let Some(Value::List(rows)) = blocks[0].field("rows") else {
            panic!()
        };
        let Value::List(row) = &rows[0] else { panic!() };
        let Value::List(cell) = &row[0] else { panic!() };
        assert_eq!(text(cell[0].field("img")).as_deref(), Some("p.png"));
    }

    #[test]
    fn nesting_thousands_deep_shows_as_the_deepest_level_kept() {
        /// How deeply blocks nest, and the text in them.
        fn walk(value: &Value, depth: usize, text: &mut String) -> usize {
            match value {
                Value::List(items) => items
                    .iter()
                    .map(|v| walk(v, depth + 1, text))
                    .max()
                    .unwrap_or(depth),
                Value::Map(fields) => fields
                    .iter()
                    .map(|(k, v)| {
                        if &**k == "x"
                            && let Value::Str(s) = v
                        {
                            text.push_str(s);
                        }
                        walk(v, depth + 1, text)
                    })
                    .max()
                    .unwrap_or(depth),
                _ => depth,
            }
        }
        for source in [
            format!("{} deep", ">".repeat(20_000)),
            format!("{}deep", "- ".repeat(20_000)),
            (0..300)
                .map(|i| format!("{}- deep\n", "  ".repeat(i)))
                .collect(),
        ] {
            let blocks = parse(&source, &plain());
            let mut text = String::new();
            let depth = walk(&Value::List(blocks.into()), 0, &mut text);
            assert!(depth < 4 * NESTING, "{depth}");
            assert!(text.contains("deep"), "{text}");
        }
    }

    #[test]
    fn reads_blocks_and_runs() {
        let blocks = parse(
            "Comet is a **multi-device** tool.\n\n- one\n- two `x`\n\n```rust\nfn a() {}\n```",
            &plain(),
        );
        assert_eq!(kinds(&blocks), ["p", "ul", "code"]);
        let runs = blocks[0].field("s").unwrap().list();
        assert_eq!(runs.len(), 3);
        assert!(runs[1].field("b").is_some());
        let items = blocks[1].field("items").unwrap().list();
        assert_eq!(items.len(), 2);
        let second = items[1].list()[0].field("s").unwrap().list();
        assert!(second[1].field("c").is_some(), "inline code");
        assert_eq!(
            blocks[2].field("lang").map(|v| v.to_json()),
            Some("\"rust\"".into())
        );
    }

    #[test]
    fn breaks_keep_line_breaks() {
        let blocks = parse(
            "one\ntwo",
            &Options {
                breaks: true,
                ..plain()
            },
        );
        let runs = blocks[0].field("s").unwrap().list();
        assert_eq!(
            runs[0].field("x").map(|v| v.to_json()),
            Some("\"one\\ntwo\"".into())
        );
    }

    #[test]
    fn raw_html_is_dropped() {
        let blocks = parse(
            "before <b>bold</b> after\n\n<div>\nhidden\n</div>\n\nlast",
            &plain(),
        );
        let text: Vec<String> = blocks
            .iter()
            .flat_map(|b| b.field("s").map(|s| s.list().to_vec()).unwrap_or_default())
            .filter_map(|r| r.field("x").map(|x| x.to_json()))
            .collect();
        assert_eq!(text, ["\"before bold after\"", "\"last\""]);
    }

    #[test]
    fn repository_paths_are_files_with_paths() {
        let text = "See src/main.rs:12, `apps/notes/ui/theme.luau` and [the guide](docs/guide.md) or [site](https://a.b/c.md).";
        let blocks = parse(text, &paths());
        let runs = blocks[0].field("s").unwrap().list();
        let files: Vec<String> = runs
            .iter()
            .filter_map(|r| r.field("f").map(|f| f.to_json()))
            .collect();
        assert_eq!(
            files,
            [
                "\"src/main.rs\"",
                "\"apps/notes/ui/theme.luau\"",
                "\"docs/guide.md\""
            ]
        );
        assert!(
            runs.iter().all(|r| r.field("c").is_none()),
            "a path in code is a file, not code"
        );
        assert!(
            runs.iter().any(|r| r.field("a").is_some()),
            "a web link stays a link"
        );
        assert!(
            split_paths("v1.2 and e.g. or a/b")
                .iter()
                .all(|(_, path)| !path),
            "no extension, no path"
        );
        // Without `paths`, nothing is a file.
        let blocks = parse(text, &plain());
        let runs = blocks[0].field("s").unwrap().list();
        assert!(runs.iter().all(|r| r.field("f").is_none()));
    }

    #[test]
    fn tight_items_keep_their_text_before_a_nested_list() {
        let blocks = parse("- one\n  - two\n- three", &plain());
        let items = blocks[0].field("items").unwrap().list();
        let first = items[0].list();
        assert_eq!(kinds(first), ["p", "ul"]);
        assert!(first[0].field("tight").is_some());
    }

    #[test]
    fn code_blocks_take_the_options_fields() {
        let options = Options {
            code: Some(Rc::new(|lang: &str, _text: &str| {
                if lang == "rust" {
                    vec![("lines", Value::from("coloured"))]
                } else {
                    Vec::new()
                }
            })),
            ..plain()
        };
        let blocks = parse("```rust\nfn main() {}\n```\n\n```\nplain\n```", &options);
        assert!(blocks[0].field("lines").is_some());
        assert!(blocks[1].field("lines").is_none());
    }

    /// Streams `text` in chunks of `step` bytes (on char boundaries).
    fn stream(text: &str, step: usize, options: &Options) -> Incremental {
        let mut parser = Incremental::new(options.clone());
        let mut end = 0;
        while end < text.len() {
            end = (end + step).min(text.len());
            while !text.is_char_boundary(end) {
                end += 1;
            }
            parser.set_text(&text[..end]);
        }
        parser
    }

    const CORPUS: &str = "# Title\n\nIntro with **bold** and `code` and a [link](https://x.dev).\n\n- one\n- two\n  - nested\n- three\n\n1. first\n2. second\n\n3\n\n> quoted\n> more\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nLast paragraph, src/main.rs:3.\n";

    #[test]
    fn streaming_ends_where_a_whole_parse_does() {
        for step in [1, 2, 3, 7, 16, 64] {
            for options in [plain(), paths()] {
                let parser = stream(CORPUS, step, &options);
                assert_eq!(
                    parser.blocks(),
                    parse(CORPUS, &options),
                    "streamed {step} bytes at a time"
                );
            }
        }
    }

    #[test]
    fn appends_parse_only_the_tail_and_share_the_rest() {
        let mut parser = Incremental::new(plain());
        let mut text = String::new();
        for i in 0..200 {
            text.push_str(&format!("Paragraph {i} with some words.\n\n"));
        }
        parser.set_text(&text);
        let before = parser.blocks();
        text.push_str("A new one, streaming");
        parser.set_text(&text);
        assert!(
            parser.last_parse_bytes() < 200,
            "only the tail: {} bytes",
            parser.last_parse_bytes()
        );
        let after = parser.blocks();
        assert_eq!(after.len(), before.len() + 1);
        assert!(
            before[..before.len() - 1]
                .iter()
                .zip(&after)
                .all(|(a, b)| same(a, b)),
            "the blocks before the tail are the same values"
        );
    }

    #[test]
    fn a_long_text_streamed_in_is_parsed_by_its_inserts() {
        let mut long = String::new();
        for i in 0..200 {
            long.push_str(&format!("Paragraph {i} with some words.\n\n"));
        }
        let mut text = zgx::text::Text::new(&long);
        let mut parser = Incremental::new(plain());
        parser.set_value(&Value::Text(text.clone()));
        let mut whole = long.clone();
        // A token at a time: each parses the tail alone.
        for (i, chunk) in CORPUS.as_bytes().chunks(7).enumerate() {
            let chunk = std::str::from_utf8(chunk).unwrap();
            text.append(chunk);
            whole.push_str(chunk);
            parser.set_value(&Value::Text(text.clone()));
            assert!(
                parser.last_parse_bytes() < 400,
                "token {i}: only the tail, not {} bytes",
                parser.last_parse_bytes()
            );
        }
        assert_eq!(parser.source(), whole);
        assert_eq!(parser.blocks(), parse(&whole, &plain()));
        // The same text again: nothing to do.
        parser.set_value(&Value::Text(text.clone()));
        assert_eq!(parser.last_parse_bytes(), 0);
        // A burst between reads: parsed once, from its inserts.
        for token in ["One ", "more ", "line.\n"] {
            text.append(token);
            whole.push_str(token);
        }
        parser.set_value(&Value::Text(text.clone()));
        assert!(parser.last_parse_bytes() < 400);
        assert_eq!(parser.blocks(), parse(&whole, &plain()));
        // More edits than a text keeps: read whole, and followed again after.
        for i in 0..40 {
            let token = format!("w{i} ");
            text.append(&token);
            whole.push_str(&token);
        }
        parser.set_value(&Value::Text(text.clone()));
        assert_eq!(parser.blocks(), parse(&whole, &plain()));
        text.append("tail");
        whole.push_str("tail");
        parser.set_value(&Value::Text(text.clone()));
        assert!(parser.last_parse_bytes() < 400);
        assert_eq!(parser.blocks(), parse(&whole, &plain()));
        // An edit before the end: read whole.
        text.replace_range(0..9, "Section");
        whole.replace_range(0..9, "Section");
        parser.set_value(&Value::Text(text.clone()));
        assert_eq!(parser.source(), whole);
        assert_eq!(parser.blocks(), parse(&whole, &plain()));
        // A plain string between: the long text that follows is read whole.
        parser.set_value(&Value::from("short"));
        text.append(" end");
        whole.push_str(" end");
        parser.set_value(&Value::Text(text.clone()));
        assert_eq!(parser.blocks(), parse(&whole, &plain()));
    }

    #[test]
    fn hanging_markers_show_closed_while_streaming() {
        let mut parser = Incremental::new(plain());
        parser.set_text("Some **bold");
        let blocks = parser.blocks();
        let runs = blocks[0].field("s").unwrap().list();
        assert!(
            runs.iter()
                .any(|r| r.field("b").is_some() && r.field("x") == Some(&Value::from("bold"))),
            "bold before its closer"
        );
        parser.set_text("See [docs](https://x.dev/lo");
        let blocks = parser.blocks();
        let runs = blocks[0].field("s").unwrap().list();
        assert_eq!(
            runs.last().and_then(|r| r.field("a")),
            Some(&Value::from(PENDING_LINK_URL)),
            "a link going nowhere yet"
        );
    }

    /// A long reply: every kind of top-level element, raw HTML among them.
    fn long_reply() -> String {
        let mut text = String::new();
        for n in 0..180 {
            text.push_str(&match n % 9 {
                0 => format!("## Part {n}\n\n"),
                1 => format!("A paragraph {n} with **bold**, `code` and a [link](https://example.com/{n}).\nIt runs on a second line.\n\n"),
                2 => "- one\n- two\n\n  more of two\n- three\n\n".to_string(),
                3 => "```rust\nfn main() {\n\n    println!(\"hi\");\n}\n```\n\n".to_string(),
                4 => "> quoted, *then*\n> more\n\n".to_string(),
                5 => "| a | b |\n|---|---|\n| 1 | 2 |\n\n".to_string(),
                6 => "---\n\n".to_string(),
                7 => "<div>raw</div>\n\n".to_string(),
                _ => format!("1. first {n}\n2. second\n\n"),
            });
        }
        text
    }
    fn pending_count(stream: &Incremental) -> usize {
        stream
            .blocks()
            .iter()
            .filter(|b| b.field("t") == Some(&Value::from("pending")))
            .count()
    }
    fn lazy(text: &str) -> Incremental {
        let mut stream = Incremental::new(plain());
        stream.set_text_lazy(text, 4096);
        stream
    }
    fn field<'a>(block: &'a Value, name: &str) -> Option<&'a Value> {
        block.field(name)
    }

    #[test]
    fn a_long_text_landing_whole_is_its_blocks_pending_then_the_same_blocks() {
        let text = long_reply();
        assert!(text.len() > 4096);
        let eager = parse(&text, &plain());
        let mut stream = lazy(&text);
        let first = stream.blocks();
        // One entry a block, from the start: the count is known at once.
        assert_eq!(first.len(), eager.len());
        assert!(pending_count(&stream) > 0);
        // The end parsed at once; the rest stand-ins saying where they are.
        assert_eq!(first.last(), eager.last());
        for (got, want) in first.iter().zip(&eager) {
            if field(got, "t") != Some(&Value::from("pending")) {
                assert_eq!(got, want);
                continue;
            }
            let num = |name| match field(got, name) {
                Some(Value::Num(n)) => *n as usize,
                other => panic!("{name}: {other:?}"),
            };
            let (from, to) = (num("from"), num("to"));
            let own = &text[from - 1..to];
            assert_eq!(own, own.trim(), "its bytes, trimmed");
            assert_eq!(num("lines"), own.lines().count());
            let hint = match field(want, "t") {
                Some(Value::Str(t)) => match &**t {
                    "ul" | "ol" => "list",
                    "quote" => "quote",
                    t => t,
                }
                .to_string(),
                other => panic!("{other:?}"),
            };
            assert_eq!(field(got, "hint"), Some(&Value::from(hint.as_str())));
        }
        // Tail first: a little budget parses the last pending ones.
        let pending_before = pending_count(&stream);
        assert!(stream.parse_pending(1, None));
        assert_eq!(pending_count(&stream), pending_before - 1);
        let after = stream.blocks();
        let last_pending = first
            .iter()
            .rposition(|b| field(b, "t") == Some(&Value::from("pending")))
            .unwrap();
        assert_eq!(after[last_pending], eager[last_pending]);
        // Blocks parsed before are the same tables.
        assert!(same(after.last().unwrap(), first.last().unwrap()));
        while stream.parse_pending(4096, None) {}
        assert_eq!(pending_count(&stream), 0);
        let done = stream.blocks();
        assert_eq!(done, eager);
        assert!(same(&done[last_pending], &after[last_pending]));
    }

    #[test]
    fn a_block_asked_for_is_parsed_first() {
        let text = long_reply();
        let eager = parse(&text, &plain());
        let mut stream = lazy(&text);
        stream.parse_pending(1, Some(3));
        assert_eq!(stream.blocks()[3], eager[3]);
        assert_eq!(
            field(&stream.blocks()[2], "t"),
            Some(&Value::from("pending"))
        );
    }

    #[test]
    fn an_append_to_a_lazy_text_goes_on_as_ever() {
        let text = long_reply();
        let mut stream = lazy(&text);
        let count = stream.blocks().len();
        let longer = format!("{text}And **then** some more.");
        stream.set_text_lazy(&longer, 4096);
        // The append parsed only its end; the pending stay pending.
        assert!(stream.last_parse_bytes() < 1000);
        assert!(pending_count(&stream) > 0);
        assert_eq!(stream.blocks().len(), count + 1);
        while stream.parse_pending(4096, None) {}
        assert_eq!(stream.blocks(), parse(&longer, &plain()));
    }

    #[test]
    fn short_texts_and_link_definitions_parse_at_once() {
        let mut stream = Incremental::new(plain());
        stream.set_text_lazy("A short **reply**.", 4096);
        assert_eq!(pending_count(&stream), 0);
        let text = format!("{}[a]: https://example.com\n", long_reply());
        stream.set_text_lazy(&text, 4096);
        assert_eq!(pending_count(&stream), 0);
        assert_eq!(stream.blocks(), parse(&text, &plain()));
    }
}
