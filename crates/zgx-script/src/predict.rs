//! Heights of rows before they are built (`ui.list { predict = true }`).
//!
//! PROTOTYPE, to measure whether it pays. A list's rows are described in Luau
//! once; mounted rows are watched: what their builder read of their data, and
//! what text and boxes came of it. From those the list learns a plan that goes
//! from a row's data to its height with no Luau: tokens for what the data
//! chooses, solved from mounted rows' heights, and text measured on a thread
//! of its own into tables of height against width.
//!
//! A row the mounted rows don't determine stays an estimate: never a guess.
use crate::value::Value;
use mlua::Lua;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};
use zgx::reactive::Signal;
use zgx::rich_text::{RichText, TextRun};
use zgx::scene::{Layout, NodeId, NodeKind, Scene};
use zgx::text_layout::{FontStyle, TextOptions};

// ── Recording: what a row being captured reads ───────────────────────────────

thread_local! {
    /// Rows being captured now; zero outside them.
    static ON: Cell<u32> = const { Cell::new(0) };
    /// Proxies alive, each a table standing for a value.
    static PROXIES: Cell<u32> = const { Cell::new(0) };
    static PATHS: RefCell<FxHashMap<usize, Mark>> = RefCell::default();
    static STRINGS: RefCell<FxHashMap<Rc<str>, Mark>> = RefCell::default();
    static LISTS: RefCell<Vec<Weak<Predictor>>> = const { RefCell::new(Vec::new()) };
    static STATS: RefCell<Stats> = RefCell::default();
    static MISSES: RefCell<Vec<(usize, f32, f32)>> = const { RefCell::new(Vec::new()) };
}
#[derive(Clone)]
struct Mark {
    model: Rc<RefCell<Model>>,
    path: u32,
    row: usize,
}
fn identity(v: &Value) -> Option<usize> {
    match crate::value::identity(v) {
        0 => None,
        p => Some(p),
    }
}
/// Whether a row is being captured: all a list without `predict` ever asks.
#[inline]
pub(crate) fn recording() -> bool {
    ON.get() != 0
}
#[inline]
pub(crate) fn proxies_alive() -> bool {
    PROXIES.get() != 0
}
struct Held(Value);
impl mlua::UserData for Held {}
impl Drop for Held {
    fn drop(&mut self) {
        PROXIES.set(PROXIES.get().saturating_sub(1));
    }
}
/// Marks `v` and what's in it as paths under `path`, for the row at `row`.
fn register(model: &Rc<RefCell<Model>>, row: usize, path: u32, v: &Value) {
    if let Some(p) = identity(v) {
        PATHS.with(|m| {
            m.borrow_mut().insert(
                p,
                Mark {
                    model: model.clone(),
                    path,
                    row,
                },
            );
        });
    }
    match v {
        Value::List(l) => {
            let star = model.borrow_mut().star(path);
            for item in l.iter() {
                register(model, row, star, item);
            }
        }
        Value::Map(m) => {
            for (k, item) in m.iter() {
                let child = model.borrow_mut().child(path, k);
                register(model, row, child, item);
            }
        }
        Value::Str(s) => {
            STRINGS.with(|m| {
                m.borrow_mut().entry(s.clone()).or_insert(Mark {
                    model: model.clone(),
                    path,
                    row,
                });
            });
        }
        _ => {}
    }
}
/// Whether a row being captured holds this value.
pub(crate) fn marked(v: &Value) -> bool {
    identity(v).is_some_and(|p| PATHS.with(|m| m.borrow().contains_key(&p)))
}
/// A table a row gave back: the value it stands for.
pub(crate) fn given_back(table: &mlua::Table) -> Option<Value> {
    let mt = table.metatable()?;
    let held: mlua::AnyUserData = mt.raw_get("__held").ok()?;
    let held = held.borrow::<Held>().ok()?;
    Some(held.0.clone())
}
/// A text a marked string parsed to: its blocks marked in turn.
pub(crate) fn parsed(text: &str, breaks: bool, blocks: &Value) {
    let Some(mark) = STRINGS.with(|m| m.borrow().get(text).cloned()) else {
        return;
    };
    let md = {
        let mut model = mark.model.borrow_mut();
        let md = model.md(mark.path, breaks);
        model.touch_parsed(text);
        md
    };
    register(&mark.model, mark.row, md, blocks);
}
/// Stands a proxy in for `value` if it's marked.
pub(crate) fn proxy(value: &Value, lua: &Lua, frozen: bool) -> mlua::Result<Option<mlua::Value>> {
    let Some(p) = identity(value) else {
        return Ok(None);
    };
    let Some(mark) = PATHS.with(|m| m.borrow().get(&p).cloned()) else {
        return Ok(None);
    };
    let table = lua.create_table()?;
    let mt = lua.create_table()?;
    PROXIES.set(PROXIES.get() + 1);
    mt.raw_set("__held", lua.create_userdata(Held(value.clone()))?)?;
    let (v, m) = (value.clone(), mark.clone());
    mt.raw_set(
        "__index",
        lua.create_function(move |lua, (_, k): (mlua::Value, mlua::Value)| {
            let child = match (&v, &k) {
                (Value::List(l), mlua::Value::Integer(i)) => l
                    .get((*i - 1).max(0) as usize)
                    .cloned()
                    .unwrap_or(Value::Nil),
                (Value::List(l), mlua::Value::Number(i)) => l
                    .get((*i as i64 - 1).max(0) as usize)
                    .cloned()
                    .unwrap_or(Value::Nil),
                (Value::Map(_), mlua::Value::String(s)) => {
                    v.field(&s.to_string_lossy()).cloned().unwrap_or(Value::Nil)
                }
                _ => Value::Nil,
            };
            if recording() {
                match &k {
                    mlua::Value::String(s) => {
                        if let Ok(mut model) = m.model.try_borrow_mut() {
                            model.read_field(m.path, m.row, &s.to_string_lossy())
                        }
                    }
                    mlua::Value::Integer(_) | mlua::Value::Number(_) => {
                        if let Ok(mut model) = m.model.try_borrow_mut() {
                            model.read_star(m.path, m.row)
                        }
                    }
                    _ => {}
                }
            }
            crate::value::to_lua(&child, lua, frozen)
        })?,
    )?;
    let (v, m) = (value.clone(), mark.clone());
    mt.raw_set(
        "__len",
        lua.create_function(move |_, _: mlua::Value| {
            if recording() {
                if let Ok(mut model) = m.model.try_borrow_mut() {
                    model.read_len(m.path, m.row);
                }
            }
            Ok(match &v {
                Value::List(l) => l.len() as i64,
                _ => 0,
            })
        })?,
    )?;
    let (v, m) = (value.clone(), mark.clone());
    mt.raw_set(
        "__iter",
        lua.create_function(move |lua, _: mlua::Value| {
            let (v, m) = (v.clone(), m.clone());
            let at = Cell::new(0_usize);
            lua.create_function(move |lua, ()| -> mlua::Result<(mlua::Value, mlua::Value)> {
                let i = at.get();
                at.set(i + 1);
                match &v {
                    Value::List(l) => match l.get(i) {
                        Some(item) => {
                            if recording() {
                                if let Ok(mut model) = m.model.try_borrow_mut() {
                                    model.read_star(m.path, m.row);
                                }
                            }
                            Ok((
                                mlua::Value::Integer(i as i64 + 1),
                                crate::value::to_lua(item, lua, frozen)?,
                            ))
                        }
                        None => Ok((mlua::Value::Nil, mlua::Value::Nil)),
                    },
                    Value::Map(map) => match map.iter().nth(i) {
                        Some((k, item)) => {
                            if recording() {
                                if let Ok(mut model) = m.model.try_borrow_mut() {
                                    model.read_field(m.path, m.row, k);
                                }
                            }
                            Ok((
                                mlua::Value::String(lua.create_string(&**k)?),
                                crate::value::to_lua(item, lua, frozen)?,
                            ))
                        }
                        None => Ok((mlua::Value::Nil, mlua::Value::Nil)),
                    },
                    _ => Ok((mlua::Value::Nil, mlua::Value::Nil)),
                }
            })
        })?,
    )?;
    table.set_metatable(Some(mt))?;
    Ok(Some(mlua::Value::Table(table)))
}
/// A picture's size read while a row is captured.
pub(crate) fn picture(src: &str, w: u32, h: u32) {
    let marks: Vec<_> = PATHS
        .with(|m| m.borrow().values().next().cloned())
        .into_iter()
        .collect();
    for mark in marks {
        mark.model
            .borrow_mut()
            .pictures
            .insert(Rc::from(src), (w, h));
    }
}
/// A state read while a row is captured: keyed by a row, it can hold it open.
pub(crate) fn state_read(state: &Signal<Value>, value: &Value) {
    let Value::Map(_) = value else {
        return;
    };
    LISTS.with(|lists| {
        for list in lists.borrow().iter().filter_map(Weak::upgrade) {
            let mut model = list.model.borrow_mut();
            if !model.capturing.is_empty() && !model.states.iter().any(|s| s.same(state)) {
                model.states.push(state.clone());
            }
        }
    });
}

// ── What was learned ─────────────────────────────────────────────────────────

#[derive(Default)]
struct Path {
    children: Vec<(Rc<str>, u32)>,
    star: Option<u32>,
    md: Option<u32>,
    breaks: bool,
    /// The script asked this one's value or length.
    read: bool,
    len_read: bool,
    /// Classes seen across the rows, up to one past the vocabulary's most.
    seen: Vec<u64>,
    rows: u32,
    /// Words people wrote, not a tag from a few.
    long: bool,
    /// The same, as a survey under way finds them.
    next_seen: Vec<u64>,
    next_rows: u32,
    next_long: bool,
    /// A newline in what was read: its lines count.
    multi: bool,
    /// Text is drawn from it.
    slot: bool,
    /// A list of runs, with the field holding each run's text.
    runs: Option<Rc<str>>,
    /// Run fields seen drawn in one text, and those seen not.
    drawn_flags: Vec<Rc<str>>,
    apart_flags: Vec<Rc<str>>,
    /// A chip's name: its width counts, its value chooses nothing.
    name: bool,
    /// Parsed under these owners' signatures, or seen not to be.
    drawn: FxHashMap<u64, bool>,
}
const VOCABULARY: usize = 8;
const SHORT: usize = 16;
fn class(v: &Value) -> u64 {
    match v {
        Value::Nil => 1,
        Value::Bool(true) => 2,
        Value::Bool(false) => 3,
        Value::List(_) => 4,
        Value::Map(_) => 5,
        Value::Text(_) => 7,
        Value::Str(s) if s.is_empty() => 6,
        Value::Str(s) => {
            let mut h = FxHasher::default();
            s.hash(&mut h);
            h.finish() | 1 << 63
        }
        Value::Num(n) => n.to_bits() | 1 << 62,
        _ => 8,
    }
}
fn hash_of(s: &str) -> u64 {
    let mut h = FxHasher::default();
    s.hash(&mut h);
    h.finish()
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Tag {
    None,
    First,
    Next,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct TokenKey {
    path: u32,
    what: u64,
    tag: Tag,
    open: bool,
}
// What a token stands for beside a value's class.
const ELEMENT: u64 = 100;
const EACH: u64 = 101;
const EMPTY: u64 = 102;
const LINE: u64 = 103;
const OPENED: u64 = 104;
const CLAMP: u64 = 105;
// A row's place: its builder can ask it (`index`), and the data doesn't say.
const FIRST_ROW: u64 = 106;
const LAST_ROW: u64 = 107;
const APART: u64 = 1 << 40;
const PICTURES: u64 = 1 << 41;
#[derive(Clone, Default)]
struct Slot {
    chain: Vec<(f32, f32, f32)>,
    options: Option<TextOptions>,
    fonts: Vec<(u64, TextRun)>,
    singles: Vec<(u64, TextRun)>,
    cap: Option<f32>,
}
#[derive(Clone, Default)]
struct Wrap {
    list: u32,
    chain: Vec<(f32, f32, f32)>,
    gap: f32,
    items: FxHashMap<u64, (f32, f32, Option<Rc<str>>)>,
    font: Option<(FontStyle, f32)>,
}
/// A row's plan, kept: what its height is made of.
#[derive(Default)]
struct RowPlan {
    tokens: Vec<u32>,
    slots: Vec<(u32, u32)>,
    wrap: Vec<(u64, Option<u32>)>,
    open: bool,
}
struct PlanOut<'a> {
    tokens: Vec<u32>,
    slots: Vec<PlannedText>,
    wrap: Vec<(u64, Option<Rc<str>>)>,
    open: bool,
    states: &'a [Value],
    root: Rc<str>,
    unseen: bool,
    /// Tokens only: no slots made, for equations.
    light: bool,
}
struct PlannedText {
    slot: (u32, u64),
    text: Rc<str>,
    runs: Vec<(usize, usize, u64, Vec<u64>)>,
}
struct Equation {
    row: Value,
    key: Rc<str>,
    width: u32,
    real: f32,
    leaves: FxHashMap<u64, f32>,
    naturals: FxHashMap<u64, f32>,
    /// Its row of the system, and the structure it was made under.
    made: Option<(u64, Option<(Vec<u32>, f32)>)>,
    /// Whether it was the first row, and the last.
    place: (bool, bool),
}
#[derive(Default)]
struct Solved {
    values: Vec<f32>,
    /// A random mix of the directions the equations leave free: a row with
    /// a part along any has a part along this. One sum to ask, per row.
    probe: Vec<f64>,
    inconsistent: u32,
}
struct Capturing {
    value: Value,
    read: FxHashSet<u32>,
}
#[derive(Default)]
struct Model {
    paths: Vec<Path>,
    tokens: FxHashMap<TokenKey, u32>,
    token_count: u32,
    slots: FxHashMap<(u32, u64), u32>,
    slot_list: Vec<Slot>,
    wrap: Option<Wrap>,
    states: Vec<Signal<Value>>,
    state_row: bool,
    state_record: bool,
    equations: Vec<Equation>,
    solved: std::collections::BTreeMap<u32, Solved>,
    capturing: FxHashMap<usize, Capturing>,
    parsed: FxHashSet<u64>,
    pictures: FxHashMap<Rc<str>, (u32, u32)>,
    ctx: u64,
    version: u64,
    /// Bumped as a path is first read: what classes it holds counts from all rows.
    paths_version: u64,
    source_parsed: bool,
    /// Rows with some part of them held open by a state.
    open_prefixes: FxHashSet<Rc<str>>,
    /// Bumped as what a plan is made of changes: only then is a row planned
    /// again. A new equation changes values alone.
    structure: u64,
}
fn width_key(w: f32) -> u32 {
    (w * 4.).round() as u32
}
impl Model {
    fn path(&mut self, at: u32) -> &mut Path {
        &mut self.paths[at as usize]
    }
    fn child(&mut self, at: u32, key: &str) -> u32 {
        if let Some((_, c)) = self.paths[at as usize]
            .children
            .iter()
            .find(|(k, _)| &**k == key)
        {
            return *c;
        }
        let id = self.paths.len() as u32;
        self.paths.push(Path::default());
        self.paths[at as usize].children.push((Rc::from(key), id));
        id
    }
    fn star(&mut self, at: u32) -> u32 {
        if let Some(c) = self.paths[at as usize].star {
            return c;
        }
        let id = self.paths.len() as u32;
        self.paths.push(Path::default());
        self.paths[at as usize].star = Some(id);
        id
    }
    fn md(&mut self, at: u32, breaks: bool) -> u32 {
        if let Some(c) = self.paths[at as usize].md {
            return c;
        }
        let id = self.paths.len() as u32;
        self.paths.push(Path::default());
        self.path(at).md = Some(id);
        self.path(at).breaks = breaks;
        id
    }
    fn touch_parsed(&mut self, text: &str) {
        self.parsed.insert(hash_of(text));
    }
    fn mark_read(&mut self, path: u32, row: usize) {
        if let Some(c) = self.capturing.get_mut(&row) {
            c.read.insert(path);
        }
    }
    fn read_field(&mut self, at: u32, row: usize, key: &str) {
        let c = self.child(at, key);
        if !self.paths[c as usize].read {
            self.path(c).read = true;
            self.paths_version += 1;
            self.structure += 1;
        }
        self.mark_read(c, row);
    }
    fn read_star(&mut self, at: u32, row: usize) {
        let c = self.star(at);
        if !self.paths[c as usize].read {
            self.path(c).read = true;
            self.paths_version += 1;
            self.structure += 1;
        }
        self.mark_read(c, row);
    }
    fn read_len(&mut self, at: u32, row: usize) {
        if !self.paths[at as usize].len_read {
            self.path(at).len_read = true;
            self.paths_version += 1;
            self.structure += 1;
        }
        self.mark_read(at, row);
    }
    /// Words people wrote don't choose a shape; a short tag from a few does.
    fn vocab(&self, at: u32) -> bool {
        let p = &self.paths[at as usize];
        !p.slot
            && !p.name
            && !p.long
            && p.rows > 0
            && p.seen.len() <= VOCABULARY
            && (p.rows as usize) >= 3 * p.seen.len()
    }
    /// Across a row's data: how many classes each path read has held.
    fn survey(&mut self, v: &Value, at: u32) {
        let p = &mut self.paths[at as usize];
        p.next_rows += 1;
        if p.next_seen.len() <= VOCABULARY {
            let c = class(v);
            if !p.next_seen.contains(&c) {
                p.next_seen.push(c);
            }
        }
        if let Value::Str(s) = v {
            if s.len() > SHORT {
                p.next_long = true;
            }
            if !p.multi && p.md.is_none() && s.trim_end_matches('\n').contains('\n') {
                p.multi = true;
                self.structure += 1;
            }
        }
        match v {
            Value::Map(_) => {
                let n = self.paths[at as usize].children.len();
                for i in 0..n {
                    let (key, c) = self.paths[at as usize].children[i].clone();
                    if self.paths[c as usize].read {
                        let nil = Value::Nil;
                        let child = v.field(&key).unwrap_or(&nil);
                        self.survey(child, c);
                    }
                }
            }
            Value::List(l) => {
                if let Some(star) = self.paths[at as usize].star {
                    for item in l.iter() {
                        self.survey(item, star);
                    }
                }
            }
            Value::Str(s) => {
                if let Some(md) = self.paths[at as usize].md {
                    let breaks = self.paths[at as usize].breaks;
                    let parsed = parse(s, breaks);
                    self.survey(&parsed, md);
                }
            }
            _ => {}
        }
    }
    fn token(&mut self, path: u32, what: u64, tag: Tag, open: bool) -> u32 {
        let key = TokenKey {
            path,
            what,
            tag,
            open,
        };
        let next = self.token_count;
        let id = *self.tokens.entry(key).or_insert(next);
        if id == next {
            self.token_count += 1;
        }
        id
    }
    /// What beside a text tells it from the others at its path.
    fn signature(&self, record: &Value, at: u32, except: u32) -> u64 {
        let mut sum = self.ctx;
        for (key, c) in &self.paths[at as usize].children {
            if *c != except && self.paths[*c as usize].read && self.vocab(*c) {
                let v = record.field(key);
                if let Some(v @ (Value::Str(_) | Value::Num(_) | Value::Bool(_))) = v {
                    sum = sum.wrapping_add(hash_of(key).wrapping_mul(class(v) | 1));
                }
            }
        }
        sum
    }
    fn walk(&mut self, v: &Value, at: u32, first: bool, open: bool, out: &mut PlanOut) {
        let tag = if first { Tag::First } else { Tag::Next };
        match v {
            Value::Map(_) => {
                let mut open = open;
                if self.state_record && self.open_prefixes.contains(&out.root) {
                    if let Value::Map(m) = v {
                        for (_, x) in m.iter() {
                            if let Value::Str(s) = x {
                                let key = format!("{}:{s}", out.root);
                                if out
                                    .states
                                    .iter()
                                    .any(|m| matches!(m.field(&key), Some(Value::Bool(true))))
                                {
                                    open = true;
                                    let t = self.token(at, OPENED, Tag::None, false);
                                    out.tokens.push(t);
                                    break;
                                }
                            }
                        }
                    }
                }
                let n = self.paths[at as usize].children.len();
                for i in 0..n {
                    let (key, c) = self.paths[at as usize].children[i].clone();
                    if !self.paths[c as usize].read && self.paths[c as usize].star.is_none() {
                        continue;
                    }
                    let nil = Value::Nil;
                    let child = v.field(&key).unwrap_or(&nil);
                    let sig = self.signature(v, at, c);
                    if let Some(field) = self.paths[c as usize].runs.clone() {
                        if let Value::List(l) = child {
                            self.plan_runs(l, c, &field, sig, open, out);
                        }
                        continue;
                    }
                    let many = self.paths[c as usize].seen.len() > 1;
                    if many {
                        let what = match child {
                            Value::Nil | Value::Bool(_) | Value::List(_) | Value::Map(_) => {
                                Some((class(child), Tag::None))
                            }
                            Value::Str(s) if s.is_empty() => Some((class(child), Tag::None)),
                            x if self.vocab(c) => Some((class(x), tag)),
                            _ => None,
                        };
                        if let Some((what, t)) = what {
                            let t = self.token(c, what, t, open);
                            out.tokens.push(t);
                        }
                    }
                    match child {
                        Value::Str(s) => {
                            let md = self.paths[c as usize].md;
                            if self.paths[c as usize].slot {
                                let parsed_too = md.is_some();
                                let known = self.slots.contains_key(&(c, sig));
                                if !parsed_too || known {
                                    out.slots.push(PlannedText {
                                        slot: (c, sig),
                                        text: s.clone(),
                                        runs: vec![(0, s.len(), 0, Vec::new())],
                                    });
                                    if parsed_too {
                                        continue;
                                    }
                                }
                            } else if self.paths[c as usize].multi {
                                let lines = s.trim_end_matches('\n').lines().count().max(1);
                                let t = self.token(c, LINE, Tag::None, open);
                                for _ in 0..lines {
                                    out.tokens.push(t);
                                }
                            }
                            if let Some(md) = md {
                                if self.paths[c as usize].drawn.get(&sig) == Some(&false) {
                                    continue;
                                }
                                let breaks = self.paths[c as usize].breaks;
                                let parsed = parse(s, breaks);
                                let saved = std::mem::replace(&mut self.ctx, sig);
                                self.walk(&parsed, md, true, open, out);
                                self.ctx = saved;
                            }
                        }
                        Value::List(_) | Value::Map(_) => self.walk(child, c, first, open, out),
                        _ => {}
                    }
                }
            }
            Value::List(l) => {
                if self.paths[at as usize].len_read {
                    if l.is_empty() {
                        let t = self.token(at, EMPTY, Tag::None, open);
                        out.tokens.push(t);
                    }
                    let t = self.token(at, EACH, Tag::None, open);
                    for _ in l.iter() {
                        out.tokens.push(t);
                    }
                }
                if self.wrap.as_ref().is_some_and(|w| w.list == at) {
                    for item in l.iter() {
                        let wrap = self.wrap.as_ref().unwrap();
                        let class = item_class(item);
                        let name =
                            wrap.items
                                .get(&class)
                                .and_then(|e| e.2.as_ref())
                                .and_then(|f| match (item, item.field(f)) {
                                    // A list of words: the item is its own text.
                                    (Value::Str(s), _) => Some(s.clone()),
                                    (_, Some(Value::Str(s))) => Some(s.clone()),
                                    _ => None,
                                });
                        out.wrap.push((class, name));
                    }
                }
                // A tray: its pictures' sizes together choose its height.
                if !self.pictures.is_empty() {
                    let mut sum = 0_u64;
                    let mut any = false;
                    for item in l.iter() {
                        if let Value::Map(m) = item {
                            for (_, x) in m.iter() {
                                if let Value::Str(f) = x
                                    && let Some((w, h)) = self.pictures.get(f)
                                {
                                    any = true;
                                    sum = sum
                                        .wrapping_mul(31)
                                        .wrapping_add((*w as u64) << 16 | *h as u64);
                                }
                            }
                        }
                    }
                    if any {
                        let t = self.token(at, PICTURES | (sum & 0xff_ffff_ffff), Tag::None, open);
                        out.tokens.push(t);
                    }
                }
                let Some(star) = self.paths[at as usize].star else {
                    return;
                };
                if self.paths[star as usize].read
                    || !self.paths[star as usize].children.is_empty()
                    || self.paths[star as usize].star.is_some()
                {
                    for (n, item) in l.iter().enumerate() {
                        let tag = if n == 0 { Tag::First } else { Tag::Next };
                        let t = self.token(star, ELEMENT, tag, open);
                        out.tokens.push(t);
                        self.walk(item, star, n == 0, open, out);
                    }
                }
            }
            _ => {}
        }
    }
    fn plan_runs(
        &mut self,
        l: &[Value],
        at: u32,
        field: &Rc<str>,
        sig: u64,
        open: bool,
        out: &mut PlanOut,
    ) {
        let apart = self.paths[at as usize].apart_flags.clone();
        if !apart.is_empty() {
            let held = l
                .iter()
                .filter(|i| apart.iter().any(|f| i.field(f).is_some()))
                .count() as u64;
            if held > 0 {
                let t = self.token(at, APART + held, Tag::None, open);
                out.tokens.push(t);
                return;
            }
        }
        if out.light {
            // (For an equation: which texts, with their runs' spans skipped.)
        }
        let (mut text, mut runs) = (String::new(), Vec::with_capacity(l.len()));
        for item in l {
            let start = text.len();
            if let Some(Value::Str(x)) = item.field(field) {
                text.push_str(x);
            }
            let (flags, singles) = run_flags(item, field);
            runs.push((start, text.len(), flags, singles));
        }
        if !text.is_empty() {
            out.slots.push(PlannedText {
                slot: (at, sig),
                text: Rc::from(text),
                runs,
            });
        }
    }
    fn plan(
        &mut self,
        row: &Value,
        key: &Rc<str>,
        place: (bool, bool),
        states: &[Value],
        light: bool,
    ) -> (
        Vec<u32>,
        Vec<PlannedText>,
        Vec<(u64, Option<Rc<str>>)>,
        bool,
    ) {
        let open = self.state_row
            && states
                .iter()
                .any(|m| matches!(m.field(key), Some(Value::Bool(true))));
        let mut out = PlanOut {
            tokens: Vec::new(),
            slots: Vec::new(),
            wrap: Vec::new(),
            open,
            states,
            root: key.clone(),
            unseen: false,
            light,
        };
        self.ctx = 0;
        if place.0 {
            out.tokens.push(self.token(0, FIRST_ROW, Tag::None, false));
        }
        if place.1 {
            out.tokens.push(self.token(0, LAST_ROW, Tag::None, false));
        }
        self.walk(row, 0, true, false, &mut out);
        let _ = out.unseen;
        (out.tokens, out.slots, out.wrap, out.open)
    }
    fn states_now(&self) -> Vec<Value> {
        self.states
            .iter()
            .map(|s| s.with_untracked(Clone::clone))
            .collect()
    }
    /// Equations brought up to the structure, up to `budget`; the systems to
    /// solve when all are.
    fn systems(
        &mut self,
        states: &[Value],
        budget: Option<Duration>,
    ) -> Option<Vec<(u32, Vec<(Vec<u32>, f32)>)>> {
        let start = Instant::now();
        let mut equations = std::mem::take(&mut self.equations);
        let structure = self.structure;
        let mut whole = true;
        for e in equations.iter_mut() {
            if e.made.as_ref().is_none_or(|(made, _)| *made != structure) {
                if budget.is_some_and(|b| start.elapsed() >= b) {
                    whole = false;
                    break;
                }
                e.made = Some((structure, self.equation_row(e, states)));
            }
        }
        let systems = whole.then(|| {
            let mut widths: Vec<u32> = equations.iter().map(|e| e.width).collect();
            widths.sort_unstable();
            widths.dedup();
            widths
                .into_iter()
                .map(|w| {
                    let system = equations
                        .iter()
                        .filter(|e| e.width == w)
                        .filter_map(|e| e.made.as_ref().and_then(|(_, row)| row.clone()))
                        .collect();
                    (w, system)
                })
                .collect()
        });
        self.equations = equations;
        systems
    }
    fn token_names(&self) -> Vec<String> {
        fn walk(m: &Model, at: u32, name: String, out: &mut FxHashMap<u32, String>) {
            out.insert(at, name.clone());
            let p = &m.paths[at as usize];
            for (k, c) in &p.children {
                walk(m, *c, format!("{name}.{k}"), out);
            }
            if let Some(c) = p.star {
                walk(m, c, format!("{name}.*"), out);
            }
            if let Some(c) = p.md {
                walk(m, c, format!("{name}.md"), out);
            }
        }
        let mut paths = FxHashMap::default();
        walk(self, 0, String::new(), &mut paths);
        let mut names = vec![String::new(); self.token_count as usize];
        for (key, id) in &self.tokens {
            let what = match key.what {
                1 => "=nil".to_string(),
                2 => "=true".to_string(),
                3 => "=false".to_string(),
                4 => "=list".to_string(),
                5 => "=map".to_string(),
                6 => "=empty".to_string(),
                ELEMENT => "[el]".to_string(),
                EACH => "#each".to_string(),
                EMPTY => "#empty".to_string(),
                LINE => ":line".to_string(),
                OPENED => "@open".to_string(),
                CLAMP => "CLAMP".to_string(),
                w if w & APART != 0 && w < APART * 2 => format!("!apart{}", w - APART),
                w if w & PICTURES != 0 && w < PICTURES * 2 => format!("=pics{:x}", w - PICTURES),
                w => format!("={w:x}"),
            };
            let tag = match key.tag {
                Tag::None => "",
                Tag::First => "<first",
                Tag::Next => "<next",
            };
            names[*id as usize] = format!(
                "{}{}{what}{tag}",
                if key.open { "open|" } else { "" },
                paths.get(&key.path).cloned().unwrap_or_default()
            );
        }
        names
    }
    /// An equation's row of the system: None if it can't say yet.
    fn equation_row(&mut self, e: &Equation, states: &[Value]) -> Option<(Vec<u32>, f32)> {
        let (mut tokens, texts, wrap, open) = self.plan(&e.row, &e.key, e.place, states, true);
        let (mut total, mut capped, mut cap) = (0_f32, 0_f32, None);
        for t in &texts {
            let h = e.leaves.get(&hash_of(&t.text))?;
            match self
                .slots
                .get(&t.slot)
                .and_then(|s| self.slot_list[*s as usize].cap)
            {
                Some(c) => {
                    capped += h;
                    cap = Some(c);
                }
                None => total += h,
            }
        }
        if let Some(c) = cap
            && capped > c + 0.497
        {
            if !open {
                capped = c;
            }
            tokens.push(self.token(0, CLAMP, Tag::None, false));
        }
        let wrapped = self.wrap_height(&wrap, e.width as f32 / 4., |name| {
            e.naturals.get(&hash_of(name)).copied()
        })?;
        Some((tokens, e.real - total - capped - wrapped))
    }
    /// The wrapping row's height, its chips' widths from `natural`.
    fn wrap_height(
        &self,
        plan: &[(u64, Option<Rc<str>>)],
        width: f32,
        natural: impl Fn(&str) -> Option<f32>,
    ) -> Option<f32> {
        if plan.is_empty() {
            return Some(0.);
        }
        let wrap = self.wrap.as_ref()?;
        let room = through(&wrap.chain, width);
        let (mut used, mut tall, mut total) = (0_f32, 0_f32, 0_f32);
        for (class, name) in plan {
            let (c, h, _) = *wrap.items.get(class)?;
            let mut w = c;
            if let Some(name) = name {
                w += natural(name)?;
            }
            if used > 0. && used + wrap.gap + w > room {
                total += tall + wrap.gap;
                used = 0.;
                tall = 0.;
            }
            used += if used > 0. { wrap.gap } else { 0. } + w;
            tall = tall.max(h);
        }
        Some(total + tall)
    }
    /// The tokens' sum at `w`, if the equations determine these tokens there.
    fn tokens_at(&self, tokens: &[u32], w: f32) -> Option<f32> {
        let key = width_key(w);
        if let Some(s) = self.solved.get(&key) {
            return sum_in_span(s, tokens);
        }
        let below = self.solved.range(..key).next_back();
        let above = self.solved.range(key..).next();
        match (below, above) {
            (Some((a, x)), Some((b, y))) => {
                let (p, q) = (sum_in_span(x, tokens)?, sum_in_span(y, tokens)?);
                Some(p + (q - p) * (key - a) as f32 / (b - a) as f32)
            }
            (Some((_, x)), None) | (None, Some((_, x))) => sum_in_span(x, tokens),
            (None, None) => None,
        }
    }
}
fn sum_in_span(s: &Solved, tokens: &[u32]) -> Option<f32> {
    let (mut sum, mut along) = (0_f64, 0_f64);
    for t in tokens {
        sum += *s.values.get(*t as usize)? as f64;
        along += s.probe[*t as usize];
    }
    (along.abs() < 1e-4).then_some(sum as f32)
}
/// Solves, keeping which directions stay free.
fn eliminate(system: &[(Vec<u32>, f32)], n: usize) -> Solved {
    let (mut rows, mut pivots): (Vec<Vec<f64>>, Vec<usize>) = (Vec::new(), Vec::new());
    let mut inconsistent = 0;
    for (tokens, residual) in system {
        let mut r = vec![0_f64; n + 1];
        for t in tokens {
            r[*t as usize] += 1.;
        }
        r[n] = *residual as f64;
        for (row, &p) in rows.iter().zip(&pivots) {
            let f = r[p];
            if f != 0. {
                for c in 0..=n {
                    r[c] -= f * row[c];
                }
            }
        }
        // The largest coefficient: an inconsistent row mustn't grow through
        // the others.
        match (0..n)
            .filter(|&c| r[c].abs() > 1e-6)
            .max_by(|&a, &b| r[a].abs().total_cmp(&r[b].abs()))
        {
            Some(p) => {
                let d = r[p];
                for c in 0..=n {
                    r[c] /= d;
                }
                // Reduced, so each pivot stands in only its own row.
                for row in &mut rows {
                    let f = row[p];
                    if f != 0. {
                        for c in 0..=n {
                            row[c] -= f * r[c];
                        }
                    }
                }
                rows.push(r);
                pivots.push(p);
            }
            None => {
                if r[n].abs() > 0.05 {
                    inconsistent += 1;
                    if std::env::var_os("PREDICT_TRACE").is_some() {
                        eprintln!(
                            "CONFLICT off {:.2} residual {residual} tokens {tokens:?}",
                            r[n]
                        );
                    }
                }
            }
        }
    }
    let mut x = vec![0_f32; n];
    for (row, &p) in rows.iter().zip(&pivots) {
        x[p] = row[n] as f32;
    }
    // Each free direction, mixed in at a random weight. (A token no equation
    // had is free: a row with it isn't determined.)
    let mut pivot_at = vec![false; n];
    for &p in &pivots {
        pivot_at[p] = true;
    }
    let mut probe = vec![0_f64; n];
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    for c in 0..n {
        if pivot_at[c] {
            continue;
        }
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let weight = 1. + (seed >> 40) as f64 / (1_u64 << 24) as f64;
        probe[c] += weight;
        for (row, &p) in rows.iter().zip(&pivots) {
            if row[c] != 0. {
                probe[p] -= weight * row[c];
            }
        }
    }
    Solved {
        values: x,
        probe,
        inconsistent,
    }
}
fn through(chain: &[(f32, f32, f32)], w: f32) -> f32 {
    // Each box on the way down: its share, its cap, what it takes off.
    chain.iter().fold(w, |room, (share, cap, minus)| {
        ((room * share).min(*cap) - minus).max(0.)
    })
}
fn parse(text: &str, breaks: bool) -> Value {
    let options = crate::markdown::Options {
        breaks,
        ..Default::default()
    };
    Value::List(Rc::from(crate::markdown::parse(text, &options)))
}
/// A run's flags: which other fields it has, together and each.
fn run_flags(item: &Value, field: &str) -> (u64, Vec<u64>) {
    let Value::Map(m) = item else {
        return (0, Vec::new());
    };
    let (mut sum, mut each) = (0_u64, Vec::new());
    for (k, _) in m.iter() {
        if &**k != field {
            let h = hash_of(k);
            sum = sum.wrapping_add(h);
            each.push(h);
        }
    }
    (sum, each)
}
/// What tells a wrapping row's items apart: its fields, and those short.
fn item_class(v: &Value) -> u64 {
    let Value::Map(m) = v else {
        return 0;
    };
    let mut sum = 0_u64;
    for (k, x) in m.iter() {
        sum = sum.wrapping_add(hash_of(k));
        if k.len() <= 4
            && let Value::Str(s) = x
            && s.len() <= 8
        {
            sum = sum.wrapping_add(hash_of(s).rotate_left(7));
        }
    }
    sum
}

// ── Off the main thread ──────────────────────────────────────────────────────

/// `RichText` crosses to the worker: its text and runs are `Arc`s.
struct Job {
    table: u32,
    rich: RichText,
    natural: bool,
    room: f32,
}
struct Done {
    table: u32,
    steps: Box<[(f32, f32)]>,
    natural: f32,
    spent: Duration,
}
const WIDEST: f32 = 2400.;
const NARROWEST: f32 = 120.;
fn work(start: zgx::scene::TextWorkerStart, jobs: Receiver<Job>, done: Sender<Done>) {
    let mut worker = start();
    let mut since_clear = 0;
    loop {
        match jobs.recv_timeout(Duration::from_millis(250)) {
            Ok(job) => {
                let t = Instant::now();
                let mut steps: Vec<(f32, f32)> = Vec::new();
                let mut natural = 0.;
                if job.natural {
                    natural = worker.measure(&job.rich, None).0;
                } else {
                    // Each height, from the width that holds it: widest first.
                    let (mut w, mut upper) = (WIDEST.max(job.room), WIDEST.max(job.room));
                    while w >= NARROWEST {
                        let (widest, h) = worker.measure(&job.rich, Some(w));
                        if steps.last().is_none_or(|l| l.1 != h) {
                            steps.push((upper, h));
                        }
                        upper = widest.min(w);
                        w = upper - 0.01;
                    }
                }
                since_clear += 1;
                if since_clear >= 500 {
                    since_clear = 0;
                    worker.forget();
                }
                let sent = done.send(Done {
                    table: job.table,
                    steps: steps.into_boxed_slice(),
                    natural,
                    spent: t.elapsed(),
                });
                if sent.is_err() {
                    return;
                }
            }
            // Nothing asked a while: fonts and caches let go until it is.
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
fn lookup(steps: &[(f32, f32)], room: f32) -> Option<f32> {
    // The last row of the table (widest first) whose width it still reaches:
    // a line as wide as the room fits, by a hair of rounding.
    let at = steps.partition_point(|e| e.0 > room + 0.001);
    if at == 0 {
        steps.first()
    } else {
        steps.get(at - 1)
    }
    .map(|e| e.1)
}

// ── The list's side ──────────────────────────────────────────────────────────

#[derive(Default, Clone)]
pub struct Stats {
    pub rows: usize,
    pub rows_exact_known: usize,
    pub captures: u32,
    pub capture: Duration,
    pub capture_worst: Duration,
    pub plan: Duration,
    pub plan_worst_slice: Duration,
    pub solve: Duration,
    pub solve_worst: Duration,
    pub apply: Duration,
    pub apply_worst_slice: Duration,
    pub width_change_worst: Duration,
    pub worker: Duration,
    pub texts: u32,
    pub steps: usize,
    pub table_bytes: usize,
    pub conflicts: u32,
    pub tokens: u32,
    pub slots: usize,
    pub checked: u32,
    pub checked_exact: u32,
    pub first_tick: Option<Instant>,
    pub all_known_after: Option<Duration>,
    pub extras_built: u32,
    pub samples_exact: u32,
    pub samples_missed: u32,
    pub given_up: bool,
    pub jobs: u32,
    pub no_worker: u32,
    pub no_font: u32,
}
impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        write!(
            f,
            "rows {} known {} | captures {} ({:.1} ms, worst {:.2}) extras {} | solve {:.1} ms (worst {:.2}) | plan {:.1} ms (worst slice {:.2}) | apply {:.1} ms (worst slice {:.2}) width worst {:.2} | worker {:.1} ms, {} texts, {} steps, {:.2} MB | tokens {} slots {} conflicts {} | checked {} exact {} | samples exact {} missed {} given up {} | jobs {} no worker {} no font {} | all known after {:?}",
            self.rows,
            self.rows_exact_known,
            self.captures,
            ms(self.capture),
            ms(self.capture_worst),
            self.extras_built,
            ms(self.solve),
            ms(self.solve_worst),
            ms(self.plan),
            ms(self.plan_worst_slice),
            ms(self.apply),
            ms(self.apply_worst_slice),
            ms(self.width_change_worst),
            ms(self.worker),
            self.texts,
            self.steps,
            self.table_bytes as f64 / 1048576.,
            self.tokens,
            self.slots,
            self.conflicts,
            self.checked,
            self.checked_exact,
            self.samples_exact,
            self.samples_missed,
            self.given_up,
            self.jobs,
            self.no_worker,
            self.no_font,
            self.all_known_after.map(ms),
        )
    }
}
/// Every predicting list's numbers, added up, and the counters reset.
pub fn take_stats() -> Stats {
    STATS.with(|s| std::mem::take(&mut *s.borrow_mut()))
}
fn stats(f: impl FnOnce(&mut Stats)) {
    STATS.with(|s| f(&mut s.borrow_mut()));
}
const MOST_EXTRAS: usize = 4;
/// Samples missed, when they outnumber half those right, that end it.
const GIVE_UP_AFTER: u32 = 16;
/// Rows out of view checked in a row, exact, before it trusts what it learned.
const TRUSTED: u32 = 32;
const HOLDS: Duration = Duration::from_millis(50);
const RESTED: Duration = Duration::from_millis(250);
const MOST_WIDTHS: usize = 3;
const MOST_CAPTURES: u32 = 400;
pub(crate) struct Predictor {
    model: Rc<RefCell<Model>>,
    /// The rows now, from the list's data.
    items: Rc<dyn Fn() -> Value>,
    key_of: Rc<dyn Fn(usize) -> Option<Rc<str>>>,
    heights: zgx::compose::VariableHeights,
    width: Cell<f32>,
    row_plans: RefCell<Vec<Option<RowPlan>>>,
    plan_at: Cell<usize>,
    surveyed: Cell<u64>,
    survey_at: Cell<usize>,
    surveying: Cell<bool>,
    needs_solve: Cell<bool>,
    planned_version: Cell<u64>,
    mounted: RefCell<FxHashSet<usize>>,
    /// Rows measured at any time: what they say isn't a prediction.
    ever: RefCell<FxHashSet<usize>>,
    /// Rows checked at a width, each once.
    checked: RefCell<FxHashSet<(usize, u32)>>,
    /// Rows measured whose height hasn't yet held: long text comes in over a
    /// few frames. What it last was, and since when.
    settling: RefCell<FxHashMap<usize, (f32, Instant)>>,
    last_pass: Cell<Option<Instant>>,
    extras: RefCell<Vec<usize>>,
    tables: RefCell<Vec<Option<Box<[(f32, f32)]>>>>,
    naturals: RefCell<Vec<Option<f32>>>,
    table_of: RefCell<FxHashMap<u64, u32>>,
    natural_of: RefCell<FxHashMap<u64, u32>>,
    table_slot: RefCell<Vec<u32>>,
    start: RefCell<Option<zgx::scene::TextWorkerStart>>,
    make_start: RefCell<Option<Rc<dyn Fn() -> zgx::scene::TextWorkerStart>>>,
    jobs: RefCell<Option<Sender<Job>>>,
    done_tx: RefCell<Option<Sender<Done>>>,
    done_rx: RefCell<Option<Receiver<Done>>>,
    in_flight: Cell<u32>,
    wake: RefCell<Option<Rc<dyn Fn()>>>,
    dirty: Cell<bool>,
    spent_captures: Cell<u32>,
    apply_due: Cell<bool>,
    plan_finished: Cell<bool>,
    known: Cell<usize>,
    /// Rows out of view checked exact since the last miss, and where the
    /// next check falls.
    streak: Cell<u32>,
    /// A run of samples came back exact: its heights may be used.
    trusted: Cell<bool>,
    /// These rows don't fit what it can learn: it gives up, and they stay
    /// estimates until mounted, as a list without it.
    given_up: Cell<bool>,
    samples_exact: Cell<u32>,
    samples_missed: Cell<u32>,
    /// Something moved: there may be rows to learn from.
    extras_due: Cell<bool>,
    state_seen: RefCell<Vec<usize>>,
    solving: RefCell<Option<Receiver<(Vec<(u32, Solved)>, Duration)>>>,
    /// When the width last changed, when heights were last given.
    width_at: Cell<Option<Instant>>,
    applied_at: Cell<Option<Instant>>,
    results_waiting: Cell<bool>,
    sample_at: Cell<usize>,
    sampled: RefCell<FxHashSet<usize>>,
    /// Only watching, to compare with it off (`ZGX_PREDICT_PASSIVE`).
    passive: bool,
    /// What each row was when planned: on new data, only what changed is
    /// planned again.
    planned_rows: RefCell<Vec<usize>>,
}
impl Predictor {
    pub(crate) fn new(
        items: Rc<dyn Fn() -> Value>,
        key_of: Rc<dyn Fn(usize) -> Option<Rc<str>>>,
        heights: zgx::compose::VariableHeights,
    ) -> Rc<Self> {
        let mut model = Model::default();
        model.paths.push(Path::default());
        model.paths[0].read = true;
        let this = Rc::new(Self {
            model: Rc::new(RefCell::new(model)),
            items,
            key_of,
            heights,
            width: Cell::new(0.),
            row_plans: RefCell::default(),
            plan_at: Cell::new(0),
            surveyed: Cell::new(u64::MAX),
            survey_at: Cell::new(0),
            surveying: Cell::new(false),
            needs_solve: Cell::new(false),
            planned_version: Cell::new(u64::MAX),
            mounted: RefCell::default(),
            ever: RefCell::default(),
            checked: RefCell::default(),
            settling: RefCell::default(),
            last_pass: Cell::new(None),
            extras: RefCell::default(),
            tables: RefCell::default(),
            naturals: RefCell::default(),
            table_of: RefCell::default(),
            natural_of: RefCell::default(),
            table_slot: RefCell::default(),
            start: RefCell::default(),
            make_start: RefCell::default(),
            jobs: RefCell::default(),
            done_tx: RefCell::default(),
            done_rx: RefCell::default(),
            in_flight: Cell::new(0),
            wake: RefCell::default(),
            dirty: Cell::new(true),
            spent_captures: Cell::new(0),
            apply_due: Cell::new(false),
            plan_finished: Cell::new(false),
            known: Cell::new(0),
            streak: Cell::new(0),
            trusted: Cell::new(false),
            given_up: Cell::new(false),
            samples_exact: Cell::new(0),
            samples_missed: Cell::new(0),
            extras_due: Cell::new(true),
            state_seen: RefCell::default(),
            solving: RefCell::default(),
            width_at: Cell::new(None),
            applied_at: Cell::new(None),
            results_waiting: Cell::new(false),
            sample_at: Cell::new(0),
            sampled: RefCell::default(),
            passive: std::env::var_os("ZGX_PREDICT_PASSIVE").is_some(),
            planned_rows: RefCell::default(),
        });
        LISTS.with(|l| l.borrow_mut().push(Rc::downgrade(&this)));
        this
    }
    pub(crate) fn on_wake(&self, wake: Rc<dyn Fn()>) {
        *self.wake.borrow_mut() = Some(wake);
    }
    fn wake(&self) {
        let wake = self.wake.borrow().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
    /// The list's data changed: rows planned again where they did.
    pub(crate) fn data_changed(&self) {
        self.dirty.set(true);
        self.wake();
    }
    /// A row is being built at `index`: captured if the model can't say.
    pub(crate) fn building(&self, index: usize, item: &Value) {
        if self.passive {
            return;
        }
        self.mounted.borrow_mut().insert(index);
        if self.spent_captures.get() >= MOST_CAPTURES || !matches!(item, Value::Map(_)) {
            return;
        }
        let known = self
            .row_plans
            .borrow()
            .get(index)
            .and_then(|p| p.as_ref())
            .is_some_and(|plan| {
                let model = self.model.borrow();
                model.tokens_at(&plan.tokens, self.width.get()).is_some()
            });
        if known
            && self.model.borrow().equations.len() > 16
            && !self.sampled.borrow().contains(&index)
        {
            return;
        }
        let Some(p) = identity(item) else {
            return;
        };
        self.model.borrow_mut().capturing.insert(
            p,
            Capturing {
                value: item.clone(),
                read: FxHashSet::default(),
            },
        );
        ON.set(ON.get() + 1);
        register(&self.model, p, 0, item);
    }
    pub(crate) fn unmounted(&self, index: usize, item: usize) {
        self.mounted.borrow_mut().remove(&index);
        self.settling.borrow_mut().remove(&index);
        self.extras.borrow_mut().retain(|e| *e != index);
        // Gone before its height held: nothing learned from it.
        let _ = self.finish_capture(item);
    }
    fn finish_capture(&self, p: usize) -> Option<Capturing> {
        let c = self.model.borrow_mut().capturing.remove(&p)?;
        let on = ON.get().saturating_sub(1);
        ON.set(on);
        if on == 0 {
            PATHS.with(|m| *m.borrow_mut() = FxHashMap::default());
            STRINGS.with(|m| *m.borrow_mut() = FxHashMap::default());
        }
        Some(c)
    }
    #[allow(clippy::too_many_arguments)]
    fn capture(
        &self,
        scene: &Scene,
        node: NodeId,
        index: usize,
        value: &Value,
        width: f32,
        real: f32,
        read: Option<FxHashSet<u32>>,
    ) {
        let t = Instant::now();
        let exact = self
            .said(index, width)
            .is_some_and(|s| (s - real).abs() < 0.05);
        let key = (self.key_of)(index).unwrap_or_else(|| Rc::from(""));
        let mut model = self.model.borrow_mut();
        let model = &mut *model;
        let structure_was = model.structure;
        let (mut found, mut wraps) = (Vec::new(), Vec::new());
        leaves(scene, node, Vec::new(), 0., None, &mut found, &mut wraps);
        let text_of = |id: NodeId| match scene.kind(id) {
            NodeKind::Text { text, .. } => text.to_string(),
            NodeKind::RichText { text } => text.text().to_string(),
            _ => String::new(),
        };
        let mut strings = Vec::new();
        model.ctx = 0;
        model.source_parsed = true;
        collect_strings(model, value, 0, 0, &mut strings);
        // Parsed or not, by each string (reads are by path, not instance).
        for s in &strings {
            if s.field.is_none()
                && let Some(_) = model.paths[s.path as usize].md
            {
                let was = model.parsed.contains(&hash_of(&s.text));
                *model.paths[s.path as usize]
                    .drawn
                    .entry(s.sig)
                    .or_insert(was) |= was;
            }
        }
        let shown: Vec<(String, f32, f32)> = found
            .iter()
            .map(|(id, _, _)| {
                let b = scene.bounds(*id);
                (text_of(*id), b.height, b.width)
            })
            .collect();
        let mut eq_leaves = FxHashMap::default();
        let mut eq_naturals = FxHashMap::default();
        for (text, h, w) in &shown {
            if !text.is_empty() {
                *eq_leaves.entry(hash_of(text)).or_insert(0.) = *h;
                eq_naturals.insert(hash_of(text), *w);
            }
        }
        for (leaf, chain, cap) in &found {
            if !scene.style(*leaf).text_wrap {
                continue;
            }
            let text = text_of(*leaf);
            if text.is_empty() {
                continue;
            }
            let read_it = |path: u32| read.as_ref().is_none_or(|r| r.contains(&path));
            let candidate = strings
                .iter()
                .filter(|s| {
                    s.text == text
                        && read_it(s.path)
                        && !strings.iter().any(|o| {
                            o.field.as_ref().is_some_and(|f| {
                                model.paths[o.path as usize].star.is_some_and(|star| {
                                    model.paths[star as usize]
                                        .children
                                        .iter()
                                        .any(|(k, c)| k == f && *c == s.path)
                                })
                            })
                        })
                })
                .max_by_key(|s| s.field.is_some());
            let Some(s) = candidate else {
                continue;
            };
            let path = &mut model.paths[s.path as usize];
            let mut moved = !path.slot;
            path.slot = true;
            if let Some(f) = &s.field {
                moved |= path.runs.as_ref() != Some(f);
                path.runs = Some(f.clone());
            }
            for f in &s.flag_names {
                if !path.drawn_flags.contains(f) {
                    path.drawn_flags.push(f.clone());
                }
            }
            let drawn = path.drawn_flags.clone();
            let apart = path.apart_flags.len();
            path.apart_flags.retain(|f| !drawn.contains(f));
            moved |= apart != path.apart_flags.len();
            let next = model.slot_list.len() as u32;
            let id = *model.slots.entry((s.path, s.sig)).or_insert(next);
            if id == next {
                model.slot_list.push(Slot::default());
                moved = true;
            }
            let slot = &mut model.slot_list[id as usize];
            if slot.chain.is_empty() {
                slot.chain = chain.clone();
            }
            if cap.is_some() {
                moved |= slot.cap != *cap;
                slot.cap = *cap;
            }
            let fonts_were = slot.fonts.len();
            if moved {
                model.structure += 1;
            }
            let slot = &mut model.slot_list[id as usize];
            match scene.kind(*leaf) {
                NodeKind::RichText { text } => {
                    slot.options = Some(text.options());
                    for (start, _, flags, singles) in &s.runs {
                        if let Some(run) = text
                            .runs()
                            .iter()
                            .find(|r| r.range.start <= *start && *start < r.range.end)
                        {
                            if !slot.fonts.iter().any(|(f, _)| f == flags) {
                                slot.fonts.push((*flags, run.clone()));
                            }
                            for single in singles {
                                if !slot.singles.iter().any(|(f, _)| f == single) {
                                    slot.singles.push((*single, run.clone()));
                                }
                            }
                        }
                    }
                }
                NodeKind::Text { font_size, .. } => {
                    let run = TextRun {
                        font: scene.font(*leaf).clone(),
                        font_size: *font_size,
                        ..Default::default()
                    };
                    for (_, _, flags, _) in &s.runs {
                        if !slot.fonts.iter().any(|(f, _)| f == flags) {
                            slot.fonts.push((*flags, run.clone()));
                        }
                    }
                }
                _ => {}
            }
            if slot.fonts.len() != fonts_were {
                model.structure += 1;
            }
        }
        // A run list read but drawn as no one text: what its runs carry that
        // drawn ones don't draws it apart. Only where this row truly parsed
        // what holds it: reads are by path, not by each block.
        for s in &strings {
            let read_it = read.as_ref().is_some_and(|r| r.contains(&s.path));
            let is_runs = s.field.is_some() && s.field == model.paths[s.path as usize].runs;
            if !is_runs || !read_it || !s.source_parsed || shown.iter().any(|(t, ..)| *t == s.text)
            {
                continue;
            }
            let drawn = model.paths[s.path as usize].drawn_flags.clone();
            if std::env::var_os("PREDICT_TRACE").is_some() {
                eprintln!(
                    "APART row {index} height {real} flags {:?} drawn {:?} text [{}] shown {:?}",
                    s.flag_names,
                    drawn,
                    s.text.chars().take(70).collect::<String>(),
                    shown
                        .iter()
                        .take(40)
                        .map(|(t, h, _)| (t.chars().take(24).collect::<String>(), *h))
                        .collect::<Vec<_>>()
                );
            }
            for f in &s.flag_names {
                if !drawn.contains(f) && !model.paths[s.path as usize].apart_flags.contains(f) {
                    model.paths[s.path as usize].apart_flags.push(f.clone());
                    model.structure += 1;
                }
            }
        }
        // A wrapping row: what it lays out is one of the row's lists.
        if let Some((wrap_node, chain)) = wraps.first().cloned() {
            let kids = scene.children(wrap_node).to_vec();
            let mut lists = Vec::new();
            collect_lists(model, value, 0, &mut lists);
            if let Some((list, items)) = lists.into_iter().find(|(_, l)| l.len() == kids.len()) {
                let mut wrap = model.wrap.take().unwrap_or_default();
                let classes = wrap.items.len();
                let was_list = wrap.list;
                wrap.list = list;
                wrap.chain = chain;
                wrap.gap = scene.style(wrap_node).gap;
                let star = model.star(list);
                for (k, item) in kids.iter().zip(items.iter()) {
                    let b = scene.bounds(*k);
                    let (mut c, mut named) = (b.width, None);
                    if let Value::Str(x) = item {
                        for (leaf, _, _) in &found {
                            let lb = scene.bounds(*leaf);
                            let inside = lb.x >= b.x - 0.5
                                && lb.x + lb.width <= b.x + b.width + 0.5
                                && lb.y >= b.y - 0.5
                                && lb.y + lb.height <= b.y + b.height + 0.5;
                            if inside
                                && text_of(*leaf) == **x
                                && let NodeKind::Text { font_size, .. } = scene.kind(*leaf)
                            {
                                c -= lb.width;
                                wrap.font = Some((scene.font(*leaf).clone(), *font_size));
                                named = Some(Rc::from(""));
                                break;
                            }
                        }
                    }
                    if let Value::Map(fields) = item {
                        // The widest text of the item's own: what its width follows.
                        let mut best: Option<(Rc<str>, f32, FontStyle, f32)> = None;
                        for (f, x) in fields.iter() {
                            let Value::Str(x) = x else { continue };
                            for (leaf, _, _) in &found {
                                let lb = scene.bounds(*leaf);
                                let inside = lb.x >= b.x - 0.5
                                    && lb.x + lb.width <= b.x + b.width + 0.5
                                    && lb.y >= b.y - 0.5
                                    && lb.y + lb.height <= b.y + b.height + 0.5;
                                if inside
                                    && text_of(*leaf) == **x
                                    && let NodeKind::Text { font_size, .. } = scene.kind(*leaf)
                                    && best.as_ref().is_none_or(|e| lb.width > e.1)
                                {
                                    best = Some((
                                        f.clone(),
                                        lb.width,
                                        scene.font(*leaf).clone(),
                                        *font_size,
                                    ));
                                }
                            }
                        }
                        if let Some((f, w, font, size)) = best {
                            c -= w;
                            wrap.font = Some((font, size));
                            named = Some(f.clone());
                            let name_path = model.child(star, &f);
                            if !model.paths[name_path as usize].name {
                                model.structure += 1;
                            }
                            model.path(name_path).name = true;
                        }
                    }
                    let e = wrap
                        .items
                        .entry(item_class(item))
                        .or_insert((c, b.height, named));
                    // A row stretches its items to its tallest: the least seen is its own.
                    if b.height < e.1 {
                        e.1 = b.height;
                    }
                }
                if wrap.items.len() != classes || was_list != list {
                    model.structure += 1;
                }
                model.wrap = Some(wrap);
            }
        }
        model.equations.push(Equation {
            row: value.clone(),
            key,
            width: width_key(width),
            real,
            leaves: eq_leaves,
            naturals: eq_naturals,
            made: None,
            place: (index == 0, index + 1 == (self.items)().list().len()),
        });
        // Already right, and nothing new seen of its structure: nothing it
        // taught. (Keeps the system its rank, not the rows mounted.)
        if exact && model.structure == structure_was {
            model.equations.pop();
        } else {
            self.needs_solve.set(true);
        }
        // The widths most recently rested at.
        let mut widths: Vec<u32> = Vec::new();
        for e in model.equations.iter().rev() {
            if !widths.contains(&e.width) {
                widths.push(e.width);
            }
        }
        if widths.len() > MOST_WIDTHS {
            let keep = widths[..MOST_WIDTHS].to_vec();
            model.equations.retain(|e| keep.contains(&e.width));
            self.needs_solve.set(true);
        }
        let spent = t.elapsed();
        self.spent_captures.set(self.spent_captures.get() + 1);
        let (tokens, slots) = (model.token_count, model.slot_list.len());
        stats(|s| {
            s.captures += 1;
            s.capture += spent;
            s.capture_worst = s.capture_worst.max(spent);
            s.tokens = tokens;
            s.slots = slots;
        });
        self.dirty.set(true);
    }
    /// Whether rows are waiting for their heights to hold: the list passes
    /// again for them, as nothing else may.
    pub(crate) fn holding(&self) -> bool {
        if self.settling.borrow().is_empty() {
            return false;
        }
        let now = Instant::now();
        if self
            .last_pass
            .get()
            .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(16))
        {
            self.last_pass.set(Some(now));
            self.heights.pass_again();
        }
        true
    }
    /// One slice of the work, up to `budget`; true while there's more.
    pub(crate) fn tick(&self, budget: Option<Duration>) -> bool {
        if self.passive || self.given_up.get() {
            return false;
        }
        let t = Instant::now();
        stats(|s| {
            s.first_tick.get_or_insert(t);
        });
        let width = self.width.get();
        if width <= 0. || self.model.borrow().equations.is_empty() {
            return false;
        }
        let items = (self.items)();
        let rows = items.list().len();
        // Paths grown: what classes they hold counts from all rows.
        {
            let mut model = self.model.borrow_mut();
            if self.surveyed.get() != model.paths_version {
                self.surveyed.set(model.paths_version);
                for p in &mut model.paths {
                    p.next_seen.clear();
                    p.next_rows = 0;
                    p.next_long = false;
                }
                self.survey_at.set(0);
                self.surveying.set(true);
            }
            if self.surveying.get() {
                let start = Instant::now();
                let mut at = self.survey_at.get();
                while at < rows && budget.is_none_or(|b| start.elapsed() < b) {
                    model.survey(&items.list()[at], 0);
                    at += 1;
                }
                self.survey_at.set(at);
                let spent = start.elapsed();
                stats(|s| {
                    s.plan += spent;
                    s.plan_worst_slice = s.plan_worst_slice.max(spent);
                });
                if at < rows {
                    return true;
                }
                self.surveying.set(false);
                self.needs_solve.set(true);
                let was: Vec<(bool, bool)> = (0..model.paths.len() as u32)
                    .map(|i| (model.vocab(i), model.paths[i as usize].seen.len() > 1))
                    .collect();
                for p in &mut model.paths {
                    p.seen = std::mem::take(&mut p.next_seen);
                    p.rows = p.next_rows;
                    p.long = p.next_long;
                }
                let now: Vec<(bool, bool)> = (0..model.paths.len() as u32)
                    .map(|i| (model.vocab(i), model.paths[i as usize].seen.len() > 1))
                    .collect();
                if was != now {
                    model.structure += 1;
                }
            }
            // A solve finished: its values in.
            let finished = self
                .solving
                .borrow()
                .as_ref()
                .and_then(|rx| rx.try_recv().ok());
            if let Some((solved, spent)) = finished {
                *self.solving.borrow_mut() = None;
                let mut conflicts = 0;
                model.solved.clear();
                for (w, s) in solved {
                    conflicts += s.inconsistent;
                    model.solved.insert(w, s);
                }
                // Many rows at odds: they aren't a sum of parts.
                let equations = model.equations.len() as u32;
                if equations >= 16 && conflicts * 5 > equations {
                    self.give_up();
                }
                self.apply_due.set(true);
                stats(|s| {
                    s.solve += spent;
                    s.solve_worst = s.solve_worst.max(spent);
                    s.conflicts = conflicts;
                });
            }
            // The next, off the main thread: only its rows are made here.
            if self.needs_solve.get() && self.solving.borrow().is_none() {
                let states = model.states_now();
                let start = Instant::now();
                let systems = model.systems(&states, budget);
                let spent = start.elapsed();
                stats(|s| {
                    s.plan += spent;
                    s.plan_worst_slice = s.plan_worst_slice.max(spent);
                });
                match systems {
                    Some(systems) => {
                        if std::env::var_os("PREDICT_TRACE").is_some() {
                            let names = model.token_names();
                            eprintln!(
                                "TOKENS {}",
                                names
                                    .iter()
                                    .enumerate()
                                    .map(|(i, n)| format!("{i}={n}"))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            );
                            for e in &model.equations {
                                if let Some((_, Some((tokens, residual)))) = &e.made {
                                    eprintln!(
                                        "EQUATION {} real {} residual {residual} tokens {tokens:?}",
                                        e.key, e.real
                                    );
                                }
                            }
                        }
                        self.needs_solve.set(false);
                        let n = model.token_count as usize;
                        let (tx, rx) = channel();
                        *self.solving.borrow_mut() = Some(rx);
                        let _ = std::thread::Builder::new()
                            .name("zgx list solve".into())
                            .spawn(move || {
                                let t = Instant::now();
                                let solved: Vec<(u32, Solved)> = systems
                                    .iter()
                                    .map(|(w, system)| (*w, eliminate(system, n)))
                                    .collect();
                                let _ = tx.send((solved, t.elapsed()));
                            });
                    }
                    None => return true,
                }
            }
        }
        // Rows to plan again, from the first, when the model moved.
        let version = self.model.borrow().structure;
        let structure_moved = self.planned_version.get() != version;
        if self.dirty.replace(false) || structure_moved {
            self.planned_version.set(version);
            self.plan_at.set(0);
            let mut plans = self.row_plans.borrow_mut();
            let mut was = self.planned_rows.borrow_mut();
            // Rows the same as they were keep their plans, where they now are.
            if !structure_moved {
                let last = rows.saturating_sub(1);
                let mut kept: FxHashMap<usize, RowPlan> = FxHashMap::default();
                let old_last = was.len().saturating_sub(1);
                for (i, plan) in plans.drain(..).enumerate() {
                    if let (Some(plan), Some(p)) = (plan, was.get(i))
                        && i != old_last
                        && i != 0
                    {
                        kept.insert(*p, plan);
                    }
                }
                plans.resize_with(rows, || None);
                for (i, row) in items.list().iter().enumerate() {
                    if i != last
                        && i != 0
                        && let Some(plan) = kept.remove(&crate::value::identity(row))
                    {
                        plans[i] = Some(plan);
                    }
                }
            } else {
                plans.clear();
                plans.resize_with(rows, || None);
            }
            was.clear();
            was.extend(items.list().iter().map(crate::value::identity));
            drop((plans, was));
            self.apply_due.set(true);
        }
        let states = self.model.borrow().states_now();
        // A state keyed by a row, or a part of one, holds it open: looked at
        // again only when one is new, each key a row's own (`key:part`).
        let identities: Vec<usize> = states.iter().map(crate::value::identity).collect();
        if *self.state_seen.borrow() != identities {
            *self.state_seen.borrow_mut() = identities;
            let mut model = self.model.borrow_mut();
            let mut prefixes: FxHashSet<Rc<str>> = FxHashSet::default();
            let (mut row, mut record) = (false, false);
            for state in &states {
                if let Value::Map(m) = state {
                    for (k, _) in m.iter() {
                        match k.split_once(':') {
                            Some((a, _)) => {
                                prefixes.insert(Rc::from(a));
                                record = true;
                            }
                            None => row = true,
                        }
                    }
                }
            }
            if row != model.state_row
                || record != model.state_record
                || prefixes != model.open_prefixes
            {
                model.structure += 1;
            }
            model.state_row = row;
            model.state_record = record;
            model.open_prefixes = prefixes;
        }
        let mut plans = self.row_plans.borrow_mut();
        let mut at = self.plan_at.get();
        let start = Instant::now();
        while at < rows && budget.is_none_or(|b| start.elapsed() < b) {
            if plans[at].is_some() {
                at += 1;
                continue;
            }
            let key = (self.key_of)(at).unwrap_or_else(|| Rc::from(""));
            let row = &items.list()[at];
            let plan = self.plan_row(row, &key, (at == 0, at + 1 == rows), &states, width);
            plans[at] = Some(plan);
            at += 1;
        }
        if at == rows && self.plan_at.get() != rows {
            self.plan_finished.set(true);
        }
        self.plan_at.set(at);
        let planned = start.elapsed();
        stats(|s| {
            s.plan += planned;
            s.plan_worst_slice = s.plan_worst_slice.max(planned);
        });
        // What the worker finished.
        let mut got_any = false;
        if let Some(done) = self.done_rx.borrow().as_ref() {
            let mut tables = self.tables.borrow_mut();
            let mut naturals = self.naturals.borrow_mut();
            let drain = Instant::now();
            while let Ok(d) = done.try_recv() {
                self.in_flight.set(self.in_flight.get() - 1);
                let (steps, natural, spent) = (d.steps.len(), d.natural, d.spent);
                if tables.len() <= d.table as usize {
                    tables.resize_with(d.table as usize + 1, || None);
                }
                if naturals.len() <= d.table as usize {
                    naturals.resize(d.table as usize + 1, None);
                }
                tables[d.table as usize] = Some(d.steps);
                naturals[d.table as usize] = Some(natural);
                got_any = true;
                stats(|s| {
                    s.worker += spent;
                    s.texts += 1;
                    s.steps += steps;
                    s.table_bytes += steps * 8 + 24;
                });
                if budget.is_some_and(|b| drain.elapsed() > b) {
                    break;
                }
            }
        }
        drop(plans);
        // Heights, once the plans stand and their tables are in.
        let more_planning = self.plan_at.get() < rows;
        let apply_start = Instant::now();
        let mut known = self.known.get();
        if self.apply_due.get() {
            self.extras_due.set(true);
        }
        if got_any {
            self.results_waiting.set(true);
        }
        // Results give heights when the last is in, or a tenth of a second on:
        // each giving walks every row.
        let due = self.results_waiting.get()
            && (self.in_flight.get() == 0
                || self
                    .applied_at
                    .get()
                    .is_none_or(|t| t.elapsed() > Duration::from_millis(100)));
        if due || self.plan_finished.replace(false) {
            self.results_waiting.set(false);
            self.apply_due.set(true);
        }
        if !more_planning
            && self.trusted.get()
            && !self.given_up.get()
            && self.apply_due.replace(false)
        {
            known = self.apply(rows, width);
            self.known.set(known);
            self.applied_at.set(Some(Instant::now()));
        }
        let applied = apply_start.elapsed();
        stats(|s| {
            s.apply += applied;
            s.apply_worst_slice = s.apply_worst_slice.max(applied);
            s.rows = rows;
            if !more_planning && self.in_flight.get() == 0 {
                s.rows_exact_known = known;
                if known == rows && s.all_known_after.is_none() {
                    s.all_known_after = s.first_tick.map(|f| f.elapsed());
                }
            }
        });
        // What to build out of view, if a row still can't be said.
        if !more_planning
            && self.in_flight.get() == 0
            && self.spent_captures.get() < MOST_CAPTURES
            && self.extras.borrow().is_empty()
            && self.extras_due.replace(false)
        {
            self.pick_extras(rows, width);
        }
        more_planning
            || self.in_flight.get() > 0
            || self.solving.borrow().is_some()
            || self.needs_solve.get()
            || self.results_waiting.get()
    }
    fn plan_row(
        &self,
        row: &Value,
        key: &Rc<str>,
        place: (bool, bool),
        states: &[Value],
        room_width: f32,
    ) -> RowPlan {
        let mut model = self.model.borrow_mut();
        let (tokens, texts, wrap, open) = model.plan(row, key, place, states, false);
        let mut slots = Vec::with_capacity(texts.len());
        let mut unknown = Vec::new();
        for t in &texts {
            let Some(&slot_id) = model.slots.get(&t.slot) else {
                unknown.push(t.slot);
                continue;
            };
            let slot = &model.slot_list[slot_id as usize];
            let mut h = FxHasher::default();
            t.text.hash(&mut h);
            slot_id.hash(&mut h);
            let hash = h.finish();
            let mut table_of = self.table_of.borrow_mut();
            let table = match table_of.get(&hash) {
                Some(table) => *table,
                None => {
                    let table = self.next_table();
                    table_of.insert(hash, table);
                    self.table_slot.borrow_mut().push(slot_id);
                    match rich_of(slot, t) {
                        Some(rich) => self.send(Job {
                            table,
                            rich,
                            natural: false,
                            room: through(&slot.chain, room_width),
                        }),
                        None => stats(|s| s.no_font += 1),
                    }
                    table
                }
            };
            slots.push((slot_id, table));
        }
        let mut wrap_plan = Vec::with_capacity(wrap.len());
        for (class, name) in wrap {
            let name = name.map(|name| {
                let hash = hash_of(&name);
                let mut natural_of = self.natural_of.borrow_mut();
                match natural_of.get(&hash) {
                    Some(table) => *table,
                    None => {
                        let table = self.next_table();
                        natural_of.insert(hash, table);
                        self.table_slot.borrow_mut().push(u32::MAX);
                        if let Some((font, size)) = model.wrap.as_ref().and_then(|w| w.font.clone())
                        {
                            let run = TextRun {
                                font,
                                font_size: size,
                                range: 0..name.len(),
                                ..Default::default()
                            };
                            if let Ok(rich) = RichText::new(name.to_string(), vec![run]) {
                                self.send(Job {
                                    table,
                                    rich,
                                    natural: true,
                                    room: 0.,
                                });
                            }
                        }
                        table
                    }
                }
            });
            wrap_plan.push((class, name));
        }
        let mut tokens = tokens;
        for (path, sig) in unknown {
            // A text it has no box for: a token none has solved.
            let t = model.token(path, sig | 1 << 60, Tag::None, false);
            tokens.push(t);
        }
        RowPlan {
            tokens,
            slots,
            wrap: wrap_plan,
            open,
        }
    }
    fn next_table(&self) -> u32 {
        self.table_slot.borrow().len() as u32
    }
    fn send(&self, job: Job) {
        if self.jobs.borrow().is_none() || self.done_tx.borrow().is_none() {
            let make = self.make_start.borrow().clone();
            let start = self.start.borrow_mut().take().or_else(|| make.map(|m| m()));
            let Some(start) = start else {
                stats(|s| s.no_worker += 1);
                return;
            };
            let (tx, rx) = channel();
            let (done_tx, done_rx) = channel();
            *self.done_rx.borrow_mut() = Some(done_rx);
            *self.done_tx.borrow_mut() = Some(done_tx.clone());
            let _ = std::thread::Builder::new()
                .name("zgx list heights".into())
                .spawn(move || work(start, rx, done_tx));
            *self.jobs.borrow_mut() = Some(tx);
        }
        stats(|s| s.jobs += 1);
        self.in_flight.set(self.in_flight.get() + 1);
        let failed = self
            .jobs
            .borrow()
            .as_ref()
            .is_some_and(|tx| tx.send(job).is_err());
        if failed {
            // It let go after a quiet while: another, and the job again.
            self.in_flight.set(self.in_flight.get() - 1);
            *self.jobs.borrow_mut() = None;
        }
    }
    /// Every row with all it needs gets its height; how many have.
    fn apply(&self, rows: usize, width: f32) -> usize {
        let plans = self.row_plans.borrow();
        let tables = self.tables.borrow();
        let naturals = self.naturals.borrow();
        let model = self.model.borrow();
        let mounted = self.mounted.borrow();
        let rooms: Vec<f32> = model
            .slot_list
            .iter()
            .map(|s| through(&s.chain, width))
            .collect();
        let caps: Vec<Option<f32>> = model.slot_list.iter().map(|s| s.cap).collect();
        let mut known = 0;
        let mut updates = Vec::new();
        for (i, plan) in plans.iter().enumerate().take(rows) {
            let Some(plan) = plan else { continue };
            let Some(mut h) = height_of(&model, plan, &rooms, &caps, &tables, &naturals, width)
            else {
                continue;
            };
            h = h.max(1.);
            known += 1;
            if !mounted.contains(&i) {
                updates.push((i, h));
            }
        }
        drop((plans, tables, naturals, model, mounted));
        self.heights.set_heights(&updates);
        known
    }
    fn pick_extras(&self, rows: usize, width: f32) {
        let mut extras = self.extras.borrow_mut();
        if !extras.is_empty() {
            return;
        }
        let plans = self.row_plans.borrow();
        let model = self.model.borrow();
        let mounted = self.mounted.borrow();
        let checked = self.checked.borrow();
        // Tokens the equations leave free at this width, and those a row
        // chosen here will settle.
        let solved = model
            .solved
            .get(&width_key(width))
            .or(model.solved.values().next());
        let is_open =
            |t: u32| solved.is_none_or(|s| s.probe.get(t as usize).is_none_or(|p| p.abs() > 1e-9));
        let mut covered: FxHashSet<u32> = FxHashSet::default();
        for (i, plan) in plans.iter().enumerate().take(rows) {
            if extras.len() >= MOST_EXTRAS {
                break;
            }
            let Some(plan) = plan else { continue };
            if mounted.contains(&i)
                || checked.contains(&(i, width_key(width)))
                || model.tokens_at(&plan.tokens, width).is_some()
            {
                continue;
            }
            let new: Vec<u32> = plan
                .tokens
                .iter()
                .copied()
                .filter(|t| is_open(*t) && !covered.contains(t))
                .collect();
            if new.is_empty() {
                continue;
            }
            covered.extend(new);
            extras.push(i);
        }
        // None the equations leave open: a spread of rows, to find what they
        // have not seen read. (By a large stride, so rows far apart.)
        if extras.is_empty() && self.streak.get() < TRUSTED && rows > 0 {
            // (Sharing no factor with the rows, so it reaches them all.)
            let mut stride = (rows as f64 * 0.618_033_9) as usize | 1;
            while (2..=stride).any(|d| stride % d == 0 && rows % d == 0) {
                stride += 2;
            }
            let mut tries = 0;
            while extras.len() < MOST_EXTRAS && tries < rows {
                let i = self.sample_at.get() % rows;
                self.sample_at.set(i + stride);
                tries += 1;
                if !mounted.contains(&i)
                    && !checked.contains(&(i, width_key(width)))
                    && plans.get(i).is_some_and(Option::is_some)
                {
                    self.sampled.borrow_mut().insert(i);
                    extras.push(i);
                }
            }
        }
        let any = !extras.is_empty();
        drop((extras, plans, model, mounted, checked));
        if any {
            stats(|s| s.extras_built += 1);
            self.heights.pass_again();
        }
    }
    /// Stops learning; what it has said stays till rows mount.
    fn give_up(&self) {
        self.given_up.set(true);
        self.trusted.set(false);
        self.extras.borrow_mut().clear();
        self.spent_captures.set(MOST_CAPTURES);
        stats(|s| s.given_up = true);
    }
    /// What it says of a row now, if it can.
    fn said(&self, index: usize, width: f32) -> Option<f32> {
        let plans = self.row_plans.borrow();
        let plan = plans.get(index)?.as_ref()?;
        let tables = self.tables.borrow();
        let naturals = self.naturals.borrow();
        let model = self.model.borrow();
        let rooms: Vec<f32> = model
            .slot_list
            .iter()
            .map(|s| through(&s.chain, width))
            .collect();
        let caps: Vec<Option<f32>> = model.slot_list.iter().map(|s| s.cap).collect();
        height_of(&model, plan, &rooms, &caps, &tables, &naturals, width)
    }
    /// How the worker comes: from the scene, when a row is first measured.
    fn have_worker(&self, scene: &Scene) {
        if self.start.borrow().is_none() && self.make_start.borrow().is_none() {
            *self.start.borrow_mut() = scene.text_worker();
        }
    }
    /// Until all it can do is done: for a test or a measurement.
    fn settle(&self) {
        while self.tick(None) {
            std::thread::sleep(Duration::from_micros(200));
        }
    }
}
fn height_of(
    model: &Model,
    plan: &RowPlan,
    rooms: &[f32],
    caps: &[Option<f32>],
    tables: &[Option<Box<[(f32, f32)]>>],
    naturals: &[Option<f32>],
    width: f32,
) -> Option<f32> {
    let (mut total, mut capped, mut cap) = (0_f32, 0_f32, None);
    for (slot, table) in &plan.slots {
        let steps = tables.get(*table as usize)?.as_ref()?;
        let h = lookup(steps, rooms[*slot as usize])?;
        match caps[*slot as usize] {
            Some(c) => {
                capped += h;
                cap = Some(c);
            }
            None => total += h,
        }
    }
    let mut clamped = None;
    if let Some(c) = cap
        && capped > c + 0.497
    {
        if !plan.open {
            capped = c;
        }
        let key = TokenKey {
            path: 0,
            what: CLAMP,
            tag: Tag::None,
            open: false,
        };
        let mut all = plan.tokens.clone();
        all.push(*model.tokens.get(&key)?);
        clamped = Some(all);
    }
    let tokens = model.tokens_at(clamped.as_deref().unwrap_or(&plan.tokens), width)?;
    let wrapped = if plan.wrap.is_empty() {
        0.
    } else {
        let wrap = model.wrap.as_ref()?;
        let room = through(&wrap.chain, width);
        let (mut used, mut tall, mut all) = (0_f32, 0_f32, 0_f32);
        for (class, name) in &plan.wrap {
            let (c, h, _) = *wrap.items.get(class)?;
            let mut w = c;
            if let Some(name) = name {
                w += naturals.get(*name as usize).copied().flatten()?;
            }
            if used > 0. && used + wrap.gap + w > room {
                all += tall + wrap.gap;
                used = 0.;
                tall = 0.;
            }
            used += if used > 0. { wrap.gap } else { 0. } + w;
            tall = tall.max(h);
        }
        all + tall
    };
    Some(total + capped + tokens + wrapped)
}
fn rich_of(slot: &Slot, t: &PlannedText) -> Option<RichText> {
    let mut runs = Vec::with_capacity(t.runs.len());
    for (a, b, flags, singles) in &t.runs {
        let font = slot
            .fonts
            .iter()
            .find(|(f, _)| f == flags)
            .map(|(_, r)| r)
            .or_else(|| {
                singles
                    .iter()
                    .find_map(|s| slot.singles.iter().find(|(f, _)| f == s).map(|(_, r)| r))
            })
            .or_else(|| slot.fonts.iter().find(|(f, _)| *f == 0).map(|(_, r)| r))
            .or_else(|| slot.fonts.first().map(|(_, r)| r))?;
        let mut run = font.clone();
        run.range = *a..*b;
        runs.push(run);
    }
    let rich = RichText::new(t.text.to_string(), runs).ok()?;
    Some(match slot.options {
        Some(o) => rich.with_options(o),
        None => rich,
    })
}
struct Found {
    path: u32,
    sig: u64,
    text: String,
    runs: Vec<(usize, usize, u64, Vec<u64>)>,
    field: Option<Rc<str>>,
    flag_names: Vec<Rc<str>>,
    /// The markdown it came of was parsed for this row, or it came of none.
    source_parsed: bool,
}
fn collect_strings(model: &mut Model, v: &Value, at: u32, sig: u64, out: &mut Vec<Found>) {
    let parsed_above = model.source_parsed;
    collect_in(model, v, at, sig, out);
    model.source_parsed = parsed_above;
}
fn collect_in(model: &mut Model, v: &Value, at: u32, sig: u64, out: &mut Vec<Found>) {
    match v {
        Value::Str(s) => {
            out.push(Found {
                path: at,
                sig,
                text: s.to_string(),
                runs: vec![(0, s.len(), 0, Vec::new())],
                field: None,
                flag_names: Vec::new(),
                source_parsed: model.source_parsed,
            });
            if let Some(md) = model.paths[at as usize].md {
                let breaks = model.paths[at as usize].breaks;
                let parsed = parse(s, breaks);
                let saved = std::mem::replace(&mut model.ctx, sig);
                let was =
                    std::mem::replace(&mut model.source_parsed, model.parsed.contains(&hash_of(s)));
                collect_in(model, &parsed, md, 0, out);
                model.source_parsed = was;
                model.ctx = saved;
            }
        }
        Value::List(l) => {
            if let Some(Value::Map(first)) = l.first() {
                for (k, _) in first.iter() {
                    if !l.iter().all(|i| matches!(i.field(k), Some(Value::Str(_)))) {
                        continue;
                    }
                    let (mut text, mut runs, mut names) = (String::new(), Vec::new(), Vec::new());
                    for i in l.iter() {
                        let start = text.len();
                        if let Some(Value::Str(x)) = i.field(k) {
                            text.push_str(x);
                        }
                        let (flags, singles) = run_flags(i, k);
                        runs.push((start, text.len(), flags, singles));
                        if let Value::Map(m) = i {
                            for (n, _) in m.iter() {
                                if **n != **k && !names.contains(n) {
                                    names.push(n.clone());
                                }
                            }
                        }
                    }
                    out.push(Found {
                        path: at,
                        sig,
                        text,
                        runs,
                        field: Some(k.clone()),
                        flag_names: names,
                        source_parsed: model.source_parsed,
                    });
                }
            }
            let star = model.star(at);
            for item in l.iter() {
                collect_in(model, item, star, 0, out);
            }
        }
        Value::Map(m) => {
            for (k, item) in m.iter() {
                let c = model.child(at, k);
                let sig = model.signature(v, at, c);
                collect_in(model, item, c, sig, out);
            }
        }
        _ => {}
    }
}
fn collect_lists(model: &mut Model, v: &Value, at: u32, out: &mut Vec<(u32, Rc<[Value]>)>) {
    match v {
        Value::Map(m) => {
            for (k, item) in m.iter() {
                let c = model.child(at, k);
                collect_lists(model, item, c, out);
            }
        }
        Value::List(l) => {
            out.push((at, l.clone()));
            let star = model.star(at);
            for item in l.iter() {
                collect_lists(model, item, star, out);
            }
        }
        _ => {}
    }
}
type Chain = Vec<(f32, f32, f32)>;
fn leaves(
    scene: &Scene,
    id: NodeId,
    chain: Chain,
    siblings: f32,
    cap: Option<f32>,
    out: &mut Vec<(NodeId, Chain, Option<f32>)>,
    wraps: &mut Vec<(NodeId, Chain)>,
) {
    let st = scene.style(id);
    let contents = scene.is_contents(id);
    let mut chain = chain;
    if !contents {
        let percent = st
            .layout_options
            .as_deref()
            .and_then(|o| match o.max_width {
                Some(zgx::layout::Length::Percent(v)) => Some(v),
                _ => None,
            });
        let share = st.width_percent.or(percent).unwrap_or(1.);
        // (A row's own box is as wide as the list: no cap on what it holds.)
        let most = if chain.is_empty() {
            f32::MAX
        } else {
            st.max_width.or(st.width).unwrap_or(f32::MAX)
        };
        chain.push((share, most, siblings + st.margin.left + st.margin.right));
    }
    if st
        .layout_options
        .as_deref()
        .is_some_and(|o| o.wrap.is_some())
    {
        wraps.push((id, chain.clone()));
    }
    let cap = st.max_height.filter(|c| *c < 1e5).or(cap);
    if matches!(
        scene.kind(id),
        NodeKind::Text { .. } | NodeKind::RichText { .. }
    ) {
        out.push((id, chain, cap));
        return;
    }
    let pad = st
        .padding_edges
        .unwrap_or(zgx::scene::Insets::all(st.padding));
    let border = match scene.kind(id) {
        NodeKind::Panel { quad, .. } => quad.border_width,
        _ => 0.,
    };
    if !contents {
        chain.push((1., f32::MAX, pad.left + pad.right + border * 2.));
    }
    let row = matches!(
        scene.kind(id),
        NodeKind::Container(Layout::Row)
            | NodeKind::Panel {
                layout: Layout::Row,
                ..
            }
    );
    let kids: Vec<_> = scene
        .children(id)
        .iter()
        .copied()
        .filter(|k| !scene.style(*k).absolute)
        .collect();
    for k in kids.iter().copied() {
        let beside = if row {
            kids.iter()
                .filter(|o| **o != k)
                .map(|o| {
                    let os = scene.style(*o);
                    if os.flex_grow == 0. {
                        scene.bounds(*o).width + os.margin.left + os.margin.right
                    } else {
                        0.
                    }
                })
                .sum::<f32>()
                + st.gap * (kids.len().saturating_sub(1)) as f32
        } else {
            0.
        };
        leaves(scene, k, chain.clone(), beside, cap, out, wraps);
    }
}
impl zgx::virtual_list::RowObserver for Predictor {
    fn width(&self, width: f32) {
        if width > 0. && self.width.replace(width) != width {
            self.width_at.set(Some(Instant::now()));
            self.apply_due.set(true);
            self.wake();
        }
    }
    fn measured(&self, scene: &Scene, row: NodeId, index: usize, width: f32, height: f32) {
        if self.passive || self.given_up.get() {
            return;
        }
        self.have_worker(scene);
        let items = (self.items)();
        let Some(value) = items.list().get(index).cloned() else {
            return;
        };
        self.ever.borrow_mut().insert(index);
        // A width passing by teaches nothing: only one it has rested at.
        if self.width_at.get().is_some_and(|t| t.elapsed() < RESTED) {
            self.settling
                .borrow_mut()
                .insert(index, (height, Instant::now()));
            self.wake();
            return;
        }
        // Only a height that has held is the row's own.
        let already = self.checked.borrow().contains(&(index, width_key(width)));
        if already {
            self.settling.borrow_mut().remove(&index);
            self.extras.borrow_mut().retain(|e| *e != index);
            return;
        }
        {
            let mut settling = self.settling.borrow_mut();
            let now = Instant::now();
            let e = settling.entry(index).or_insert((height, now));
            if e.0 != height {
                *e = (height, now);
            }
            if now.duration_since(e.1) < HOLDS {
                drop(settling);
                self.wake();
                return;
            }
            settling.remove(&index);
        }
        self.extras.borrow_mut().retain(|e| *e != index);
        self.extras_due.set(true);
        // Being captured: done now its layout stands. Else, checked.
        let captured = identity(&value).and_then(|p| self.finish_capture(p));
        self.checked.borrow_mut().insert((index, width_key(width)));
        if let Some(said) = self.said(index, width)
            && (said - height).abs() >= 0.05
        {
            MISSES.with(|m| m.borrow_mut().push((index, said, height)));
        }
        if self.sampled.borrow_mut().remove(&index) {
            let said = self.said(index, width);
            if said.is_some_and(|s| (s - height).abs() < 0.05) {
                self.streak.set(self.streak.get() + 1);
                self.samples_exact.set(self.samples_exact.get() + 1);
                stats(|s| s.samples_exact += 1);
                if self.streak.get() >= TRUSTED {
                    self.trusted.set(true);
                }
            } else {
                self.streak.set(0);
                self.samples_missed.set(self.samples_missed.get() + 1);
                stats(|s| s.samples_missed += 1);
            }
            let (right, missed) = (self.samples_exact.get(), self.samples_missed.get());
            if missed >= GIVE_UP_AFTER && missed * 2 > right {
                self.give_up();
            }
            self.apply_due.set(true);
            self.wake();
        }
        if let Some(c) = captured {
            // A row it already had right teaches nothing new but what it read.
            self.capture(scene, row, index, &c.value, width, height, Some(c.read));
            self.wake();
            return;
        }
        // Each row once at each width: what predicted it against what it is.
        let plans = self.row_plans.borrow();
        let Some(plan) = plans.get(index).and_then(|p| p.as_ref()) else {
            drop(plans);
            if self.spent_captures.get() < MOST_CAPTURES {
                self.capture(scene, row, index, &value, width, height, None);
                self.wake();
            }
            return;
        };
        let tables = self.tables.borrow();
        let naturals = self.naturals.borrow();
        let model = self.model.borrow();
        let rooms: Vec<f32> = model
            .slot_list
            .iter()
            .map(|s| through(&s.chain, width))
            .collect();
        let caps: Vec<Option<f32>> = model.slot_list.iter().map(|s| s.cap).collect();
        let predicted = height_of(&model, plan, &rooms, &caps, &tables, &naturals, width);
        drop((model, naturals, tables, plans));
        let exact = predicted.is_some_and(|p| (p - height).abs() < 0.05);
        stats(|s| {
            s.checked += 1;
            s.checked_exact += exact as u32;
        });
        if !exact && self.spent_captures.get() < MOST_CAPTURES {
            self.capture(scene, row, index, &value, width, height, None);
            self.wake();
        }
    }
    fn extra_rows(&self, rows: &mut Vec<usize>) {
        rows.extend_from_slice(&self.extras.borrow());
    }
}
/// Runs every predicting list's work to its end: for tests and measurements.
pub fn settle() {
    let lists: Vec<_> = LISTS.with(|l| l.borrow().iter().filter_map(Weak::upgrade).collect());
    for list in lists {
        list.settle();
    }
}
/// Whole-list details, for a test's message.
pub fn describe() -> String {
    let lists: Vec<_> = LISTS.with(|l| l.borrow().iter().filter_map(Weak::upgrade).collect());
    lists
        .iter()
        .map(|list| {
            let m = list.model.borrow();
            format!(
                "extras {:?} settling {:?} mounted {} on {} paths {} tokens {} slots {} equations {} widths {} rows planned {} capturing {} state row {} record {}",
                list.extras.borrow(),
                list.settling.borrow().iter().map(|(i, (h, t))| (*i, *h, t.elapsed().as_millis())).collect::<Vec<_>>(),
                list.mounted.borrow().len(),
                ON.get(),
                m.paths.len(),
                m.token_count,
                m.slot_list.len(),
                m.equations.len(),
                m.solved.len(),
                list.row_plans.borrow().iter().filter(|p| p.is_some()).count(),
                m.capturing.len(),
                m.state_row,
                m.state_record,
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}
#[allow(dead_code)]
fn _send_check() {
    fn send<T: Send>() {}
    send::<Job>();
    send::<Done>();
    let _: Option<Arc<str>> = None;
}

/// The largest predicting list's heights now, and which rows are mounted.
pub fn heights() -> Vec<(f32, bool)> {
    let lists: Vec<_> = LISTS.with(|l| l.borrow().iter().filter_map(Weak::upgrade).collect());
    let Some(list) = lists.iter().max_by_key(|l| l.heights.len()) else {
        return Vec::new();
    };
    let (mounted, ever) = (list.mounted.borrow(), list.ever.borrow());
    (0..list.heights.len())
        .map(|i| {
            (
                list.heights.row_height(i).unwrap_or(0.),
                mounted.contains(&i) || ever.contains(&i),
            )
        })
        .collect()
}
/// Captures and rows known so far, and rows still settling: while they move,
/// it isn't done.
pub fn counts() -> (u32, usize, usize) {
    let holding = LISTS.with(|l| {
        l.borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .map(|list| {
                list.holding();
                list.settling.borrow().len() + list.extras.borrow().len()
            })
            .sum()
    });
    STATS.with(|s| {
        let s = s.borrow();
        (s.captures, s.rows_exact_known, holding)
    })
}
/// The learned paths, as a tree: for a test's message.
pub fn describe_paths() -> String {
    fn walk(m: &Model, at: u32, name: &str, out: &mut String) {
        let p = &m.paths[at as usize];
        let flags = format!(
            "{}{}{}{}{}{}{}",
            if p.read { " read" } else { "" },
            if p.len_read { " len" } else { "" },
            if p.slot { " SLOT" } else { "" },
            p.runs
                .as_ref()
                .map(|r| format!(" runs={r}"))
                .unwrap_or_default(),
            if p.multi { " multi" } else { "" },
            if p.apart_flags.is_empty() {
                String::new()
            } else {
                format!(" apart={:?}", p.apart_flags)
            },
            if p.seen.is_empty() {
                String::new()
            } else {
                format!(
                    " seen={} rows={} long={} vocab={}",
                    p.seen.len(),
                    p.rows,
                    p.long,
                    m.vocab(at)
                )
            },
        );
        if p.read || p.len_read || p.slot {
            out.push_str(&format!("{name}{flags}\n"));
        }
        for (k, c) in &p.children {
            walk(m, *c, &format!("{name}.{k}"), out);
        }
        if let Some(c) = p.star {
            walk(m, c, &format!("{name}.*"), out);
        }
        if let Some(c) = p.md {
            walk(m, c, &format!("{name}.md"), out);
        }
    }
    let lists: Vec<_> = LISTS.with(|l| l.borrow().iter().filter_map(Weak::upgrade).collect());
    let mut out = String::new();
    if let Some(list) = lists.last() {
        let m = list.model.borrow();
        walk(&m, 0, "", &mut out);
        for ((path, sig), id) in &m.slots {
            let s = &m.slot_list[*id as usize];
            out.push_str(&format!(
                "slot path {path} sig {sig:x} chain {} fonts {} cap {:?}\n",
                s.chain.len(),
                s.fonts.len(),
                s.cap
            ));
        }
    }
    out
}
/// What the largest list's model says of each row now, for comparing with
/// what its heights hold.
pub fn said_now() -> Vec<Option<f32>> {
    let lists: Vec<_> = LISTS.with(|l| l.borrow().iter().filter_map(Weak::upgrade).collect());
    let Some(list) = lists.iter().max_by_key(|l| l.heights.len()) else {
        return Vec::new();
    };
    let w = list.width.get();
    (0..list.heights.len()).map(|i| list.said(i, w)).collect()
}

/// A slice of every predicting list's work, as a window's task does between
/// frames: for measuring without one.
pub fn tick_all(budget: Duration) -> bool {
    let lists: Vec<_> = LISTS.with(|l| l.borrow().iter().filter_map(Weak::upgrade).collect());
    let mut more = false;
    for list in lists {
        more |= list.tick(Some(budget));
        more |= list.holding();
    }
    more
}
/// The largest list's total height and how many rows it has.
pub fn content() -> (f32, usize) {
    let lists: Vec<_> = LISTS.with(|l| l.borrow().iter().filter_map(Weak::upgrade).collect());
    lists
        .iter()
        .max_by_key(|l| l.heights.len())
        .map_or((0., 0), |l| (l.heights.content_height(), l.heights.len()))
}
/// Where the largest list's row `index` starts.
pub fn row_offset(index: usize) -> f32 {
    let lists: Vec<_> = LISTS.with(|l| l.borrow().iter().filter_map(Weak::upgrade).collect());
    lists
        .iter()
        .max_by_key(|l| l.heights.len())
        .map_or(0., |l| l.heights.row_offset(index))
}

/// Rows measured unlike what was said of them since last asked: each one's
/// index, what was said and what it is.
pub fn take_misses() -> Vec<(usize, f32, f32)> {
    MISSES.with(|m| std::mem::take(&mut *m.borrow_mut()))
}
