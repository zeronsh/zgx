//! Templates: an interface written literally in a script (`ui.row { h = 28,
//! ui.text { … }, … }`), read once as the script loads, so that a row
//! function called for every item of a list makes only what differs.
//!
//! A *template* is a call of one of `KINDS` on a table constructor whose keys
//! are names, each value a literal (a number, a plain string, a boolean,
//! nil), another template, or anything else: a *hole*, an expression of the
//! script's own, kept as written. The load rewrites a template to
//! `ui.__stamp(id, hole, hole, …)`: the holes in the order the script would
//! evaluate them, each on the line it was written on, so a hole's error
//! names the line it did.
//!
//! This first step changes no node: `__stamp` builds the very tables the
//! script would have and calls the very constructors, so what a script makes
//! is what it made. It proves the rewrite (which expressions are holes,
//! their values and order, their lines) before anything relies on it. It is
//! off unless `ZGX_TEMPLATES=force`.
//!
//! What is left as written: a file that uses the name `ui` other than as
//! `ui.<field>` (a local, a parameter, an argument), computed keys, a
//! constructor called other than `ui.<kind> { … }`, and any kind not in
//! `KINDS`, whose constructors read or do more than build a node.

use crate::lint::{Kind as Tk, Token, tokenize};
use mlua::{Function, Lua, MultiValue, Table, Value};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    rc::Rc,
    sync::Arc,
};

/// The constructors a template may call: each builds a node from its table
/// and nothing else (`fill_from`, and for `text` its content), reading no
/// state and doing no work a later sibling could see.
pub(crate) const KINDS: &[&str] = &["column", "row", "div", "overlay", "button", "grid", "text"];

/// A template's node: its constructor, the line its call is on, and its
/// fields in the order written.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TNode {
    pub kind: String,
    pub line: u32,
    pub fields: Vec<(Option<String>, TValue)>,
}

/// A field's value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TValue {
    /// A literal, as written.
    Lit(String),
    /// The `n`th hole of the template.
    Hole(usize),
    /// The `n`th hole, a call (or `...`) ending a table's list: each of its
    /// values a field, as Luau has it (passed packed, `ui.__pack`).
    Spread(usize),
    Node(TNode),
}

/// A hole: its tokens, first to last, and whether it is a spread.
#[derive(Clone, Copy)]
struct HoleAt {
    first: usize,
    last: usize,
    spread: bool,
}

/// A script's source with its templates rewritten.
pub(crate) struct Rewritten {
    pub source: String,
    /// Its templates, in the order its `__stamp` calls number them (from 1).
    pub templates: Vec<TNode>,
    /// What its first line asks for its templates by (`ui.__templates`): a
    /// hash of the source as written.
    pub key: u64,
}

/// The local a rewritten chunk holds its templates in: its closures'
/// upvalue, so the templates last as long as the code that stamps them.
const LOCAL: &str = "__zgx_t";

/// Whether templates are on (`ZGX_TEMPLATES=force`; off otherwise).
pub(crate) fn enabled() -> bool {
    thread_local! {
        static ON: bool = std::env::var("ZGX_TEMPLATES").is_ok_and(|v| v == "force");
    }
    ON.with(|on| *on)
}

/// What `code` compiles to: rewritten if templates are on and it has any.
/// The same bytes in every VM, so compiled once (`sources`) for all.
pub(crate) fn for_compile(code: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    match enabled()
        .then(|| std::str::from_utf8(code).ok().and_then(rewrite))
        .flatten()
    {
        Some(rewritten) => std::borrow::Cow::Owned(rewritten.source.into_bytes()),
        None => std::borrow::Cow::Borrowed(code),
    }
}

/// What `code` loads as in `lua`: `None`, as written, templates being off
/// or it having none; otherwise the rewritten source (or bytecode compiled
/// from it, `for_compile`), its templates made for `lua` and waiting for the
/// chunk to take them as it starts (`ui.__templates`). Called just before
/// the chunk is compiled and run.
pub(crate) fn register(lua: &Lua, code: &[u8]) -> Option<Vec<u8>> {
    if !enabled() {
        return None;
    }
    let rewritten = rewrite(std::str::from_utf8(code).ok()?)?;
    let registry = lua.app_data_ref::<Registry>()?;
    let chunk = Chunk::new(lua, &rewritten.templates);
    *registry.next.borrow_mut() = Some((rewritten.key, Rc::new(chunk)));
    Some(rewritten.source.into_bytes())
}

/// `source` with its templates rewritten to `ui.__stamp` calls, or `None`
/// if it has none (or uses `ui` other than as `ui.<field>`). Its first code
/// line starts by taking its templates (`local __zgx_t =
/// ui.__templates(key) `), on that line, so every line stays where it was.
pub(crate) fn rewrite(source: &str) -> Option<Rewritten> {
    let mut scan = scan(source)?;
    if scan.templates.is_empty() {
        return None;
    }
    let key = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        // Exactly representable as a Luau number.
        hasher.finish() & ((1 << 52) - 1)
    };
    // A script that is an expression (`ui.column { … }`, no `return`) is
    // run as one (mlua's `eval`): it returns it still, from the same frame.
    let expression = mlua::chunk::Compiler::new()
        .compile(format!("return {source}"))
        .is_ok();
    let prologue = format!(
        "local {LOCAL} = ui.__templates({key}) {}",
        if expression { "return " } else { "" }
    );
    // Before its first token: after any comments (`--!strict` and the
    // like stay first).
    let at = scan
        .out
        .char_indices()
        .nth(scan.first)
        .map_or(scan.out.len(), |(at, _)| at);
    scan.out.insert_str(at, &prologue);
    Some(Rewritten {
        source: scan.out,
        templates: scan.templates,
        key,
    })
}

/// Whether `tokens` name `ui` only as `ui.<field>` (as a field of
/// something else too): otherwise a file is left as written.
fn ui_only_as_fields(tokens: &[Token]) -> bool {
    tokens.iter().enumerate().all(|(i, token)| {
        token.kind != Tk::Name
            || token.text != "ui"
            || (i > 0 && (tokens[i - 1].is(".") || tokens[i - 1].is(":")))
            || tokens.get(i + 1).is_some_and(|t| t.is("."))
    })
}

/// A source rewritten: its text, its templates, and which tokens are a
/// template's own (not its holes').
struct Scan {
    chars: Vec<char>,
    out: String,
    templates: Vec<TNode>,
    stamped: Vec<bool>,
    /// Where its first token is, in chars (none: its end).
    first: usize,
}

fn scan(source: &str) -> Option<Scan> {
    let (tokens, _) = tokenize(source);
    // `ui` named any other way (a local, a parameter, passed on): left alone.
    if !ui_only_as_fields(&tokens) {
        return None;
    }
    let chars: Vec<char> = source.chars().collect();
    let mut scan = Scan {
        first: tokens.first().map_or(chars.len(), |t| t.at),
        chars,
        out: String::with_capacity(source.len() + 48),
        templates: Vec::new(),
        stamped: vec![false; tokens.len()],
    };
    let stop = scan.chars.len();
    scan.emit(&tokens, 0, tokens.len(), 0, stop);
    Some(scan)
}

impl Scan {
    /// Tokens `from..to`, chars `start..stop`, written out with each
    /// template in them a stamp (the templates in its holes too).
    fn emit(&mut self, tokens: &[Token], from: usize, to: usize, start: usize, stop: usize) {
        let mut copied = start;
        let mut i = from;
        while i < to {
            let Some((node, holes, end)) = template_at(tokens, i).filter(|(_, _, end)| *end < to)
            else {
                i += 1;
                continue;
            };
            // Numbered before the templates in its holes.
            self.templates.push(node);
            let number = self.templates.len();
            self.out.extend(&self.chars[copied..tokens[i].at]);
            self.stamped[i..=end].fill(true);
            // `ui.__stamp(__zgx_t, n, …)`, each hole on its own line again.
            self.out.push_str(&format!("ui.__stamp({LOCAL}, {number}"));
            let mut line = tokens[i].line;
            for hole in &holes {
                while line < tokens[hole.first].line {
                    self.out.push('\n');
                    line += 1;
                }
                self.out.push_str(", ");
                let from = tokens[hole.first].at;
                let to = tokens[hole.last].at + tokens[hole.last].text.chars().count();
                line += self.chars[from..to].iter().filter(|c| **c == '\n').count() as u32;
                self.stamped[hole.first..=hole.last].fill(false);
                if hole.spread {
                    self.out.push_str("ui.__pack(");
                }
                self.emit(tokens, hole.first, hole.last + 1, from, to);
                if hole.spread {
                    self.out.push(')');
                }
            }
            while line < tokens[end].line {
                self.out.push('\n');
                line += 1;
            }
            self.out.push(')');
            copied = tokens[end].at + 1;
            i = end + 1;
        }
        self.out.extend(&self.chars[copied..stop]);
    }
}

/// What the rewrite makes of `source`'s constructors (`ui.<kind> { … }` or
/// `ui.<kind>(…)`): how many it stamps, and each it leaves as written with
/// why (the corpus test's report).
pub(crate) fn report(source: &str) -> (usize, Vec<String>) {
    let (tokens, _) = tokenize(source);
    let stamped = scan(source).map(|scan| scan.stamped);
    let mut count = 0;
    let mut left = Vec::new();
    for i in 0..tokens.len() {
        let constructor = tokens[i].kind == Tk::Name
            && tokens[i].text == "ui"
            && !(i > 0 && (tokens[i - 1].is(".") || tokens[i - 1].is(":")))
            && tokens.get(i + 1).is_some_and(|t| t.is("."))
            && tokens.get(i + 2).is_some_and(|t| t.kind == Tk::Name)
            && tokens.get(i + 3).is_some_and(|t| t.is("{") || t.is("("));
        if !constructor {
            continue;
        }
        if stamped.as_ref().is_some_and(|s| s[i]) {
            count += 1;
            continue;
        }
        let kind = tokens[i + 2].text.as_str();
        let why = if stamped.is_none() {
            "`ui` used as a value in its file"
        } else if !KINDS.contains(&kind) {
            "a kind not listed"
        } else if !tokens[i + 3].is("{") {
            "called with ( )"
        } else {
            "a computed key or a field it can't read"
        };
        left.push(format!("ui.{kind}: {why}"));
    }
    (count, left)
}

/// The template starting at token `i` (`ui . kind {`), its holes as token
/// ranges, and the index of its closing `}`.
/// A template found: its tree, its holes as token ranges, and the index of
/// its closing `}`.
type Found = (TNode, Vec<HoleAt>, usize);

fn template_at(tokens: &[Token], i: usize) -> Option<Found> {
    let mut holes = Vec::new();
    let (node, end) = node_at(tokens, i, &mut holes)?;
    if !in_order(tokens, i, end, &holes) {
        return None;
    }
    Some((node, holes, end))
}

/// Whether a stamp, which evaluates all of a template's holes before any of
/// its constructors, makes what the script would: a text template's
/// constructor (`ui.text { "{count}" }`) finds its state by key as it runs
/// (`node::template`), so a hole after it that may make state (a call) must
/// run after it, as written. Such a template is left as written; the
/// templates within it are found on their own.
fn in_order(tokens: &[Token], i: usize, end: usize, holes: &[HoleAt]) -> bool {
    let mut text_template = false;
    let mut holes = holes.iter().peekable();
    let mut k = i;
    while k <= end {
        if let Some(hole) = holes.next_if(|h| h.first == k) {
            if text_template && may_call(tokens, hole.first, hole.last) {
                return false;
            }
            k = hole.last + 1;
            continue;
        }
        let token = &tokens[k];
        text_template |= token.kind == Tk::Str
            && token
                .text
                .find('{')
                .is_some_and(|open| token.text[open..].contains('}'));
        k += 1;
    }
    true
}

/// Whether the expression in tokens `first..=last` may call a function as
/// it is evaluated: a call (`f(…)`, `f "…"`, `f {…}`, `a:m()`). A function
/// literal runs nothing until called.
fn may_call(tokens: &[Token], first: usize, last: usize) -> bool {
    if tokens[first].kind == Tk::Name && tokens[first].text == "function" {
        return false;
    }
    (first + 1..=last).any(|k| {
        let before = &tokens[k - 1];
        let callee = (before.kind == Tk::Name && !is_keyword(&before.text))
            || before.is(")")
            || before.is("]")
            || (before.kind == Tk::Str && k - 1 > first);
        callee && (tokens[k].is("(") || tokens[k].is("{") || tokens[k].kind == Tk::Str)
    })
}

/// Luau's keywords that may stand before `(`, `{` or a string without
/// calling anything.
fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "and"
            | "or"
            | "not"
            | "return"
            | "then"
            | "else"
            | "elseif"
            | "if"
            | "in"
            | "do"
            | "function"
    )
}

fn node_at(tokens: &[Token], i: usize, holes: &mut Vec<HoleAt>) -> Option<(TNode, usize)> {
    let ui = tokens.get(i)?;
    if ui.kind != Tk::Name || ui.text != "ui" {
        return None;
    }
    if i > 0 && (tokens[i - 1].is(".") || tokens[i - 1].is(":")) {
        return None;
    }
    if !tokens.get(i + 1)?.is(".") {
        return None;
    }
    let kind = tokens.get(i + 2)?;
    if kind.kind != Tk::Name || !KINDS.contains(&kind.text.as_str()) {
        return None;
    }
    if !tokens.get(i + 3)?.is("{") {
        return None;
    }
    // Holes are numbered in the order they're met; a node that turns out
    // not to be a template gives back the holes it took.
    let taken = holes.len();
    let mut fields = Vec::new();
    let mut k = i + 4;
    loop {
        let token = tokens.get(k)?;
        if token.is("}") {
            break;
        }
        if token.is("[") {
            holes.truncate(taken);
            return None;
        }
        let (key, from) = if token.kind == Tk::Name && tokens.get(k + 1).is_some_and(|t| t.is("="))
        {
            (Some(token.text.clone()), k + 2)
        } else {
            (None, k)
        };
        let to = expression_end(tokens, from)?;
        if to <= from {
            holes.truncate(taken);
            return None;
        }
        let value = value_of(tokens, from, to - 1, holes);
        fields.push((key, value));
        k = to;
        let next = tokens.get(k)?;
        if next.is(",") || next.is(";") {
            k += 1;
        } else if !next.is("}") {
            holes.truncate(taken);
            return None;
        }
    }
    // A call or `...` last in the list gives all its values to the table.
    if let Some((None, value)) = fields.last_mut()
        && let TValue::Hole(n) = *value
        && multiple(tokens, holes[n].first, holes[n].last)
    {
        holes[n].spread = true;
        *value = TValue::Spread(n);
    }
    Some((
        TNode {
            kind: kind.text.clone(),
            line: ui.line,
            fields,
        },
        k,
    ))
}

/// The value of the expression in tokens `first..=last`: a literal, a
/// template or a hole.
fn value_of(tokens: &[Token], first: usize, last: usize, holes: &mut Vec<HoleAt>) -> TValue {
    let one = &tokens[first];
    if first == last {
        let plain_string = one.kind == Tk::Str
            && (one.text.starts_with('"') || one.text.starts_with('\''))
            && !one.text.contains('\\');
        if one.kind == Tk::Number
            || plain_string
            || (one.kind == Tk::Name && matches!(one.text.as_str(), "true" | "false" | "nil"))
        {
            return TValue::Lit(one.text.clone());
        }
    }
    if last == first + 1 && one.is("-") && tokens[last].kind == Tk::Number {
        return TValue::Lit(format!("-{}", tokens[last].text));
    }
    if let Some((node, end)) = node_at(tokens, first, holes)
        && end == last
    {
        return TValue::Node(node);
    }
    holes.push(HoleAt {
        first,
        last,
        spread: false,
    });
    TValue::Hole(holes.len() - 1)
}

/// Whether the expression in tokens `first..=last` may give more than one
/// value: a call (`f(…)`, `f "…"`, `f {…}`) or `...`. Taking one for a
/// spread costs a table, never a wrong answer.
fn multiple(tokens: &[Token], first: usize, last: usize) -> bool {
    let end = &tokens[last];
    end.is("...") || (last > first && (end.is(")") || end.is("}") || end.kind == Tk::Str))
}

/// Where the expression starting at `from` ends: the index of the `,`,
/// `;` or `}` after it, at its own depth (brackets, and the blocks of
/// function literals and the like).
fn expression_end(tokens: &[Token], from: usize) -> Option<usize> {
    /// What is open: a block closed by `end` or `until` (an `if`
    /// statement's too), or an `if` expression, closed by its `else`.
    #[derive(PartialEq)]
    enum Open {
        Block,
        IfExpression,
    }
    let mut open = Vec::new();
    let mut brackets = 0i32;
    // An expression comes next (after an `if` expression's `then`/`else`).
    let mut expression_next = true;
    let mut k = from;
    loop {
        let token = tokens.get(k)?;
        let mut next_is_expression = false;
        if token.kind == Tk::Punct {
            match token.text.as_str() {
                "(" | "{" | "[" => brackets += 1,
                ")" | "]" => brackets -= 1,
                "}" if brackets == 0 && open.is_empty() => return Some(k),
                "}" => brackets -= 1,
                "," | ";" if brackets == 0 && open.is_empty() => return Some(k),
                _ => {}
            }
            // After an operator, `=`, `,` or an opening bracket comes an
            // expression; after a closing bracket, no.
            next_is_expression = !matches!(token.text.as_str(), ")" | "]" | "}");
        } else if token.kind == Tk::Name {
            match token.text.as_str() {
                // A condition is an expression, whichever `if` it is.
                "if" => {
                    open.push(if expression_next {
                        Open::IfExpression
                    } else {
                        Open::Block
                    });
                    next_is_expression = true;
                }
                "function" | "do" | "repeat" => open.push(Open::Block),
                "end" => {
                    open.pop()?;
                }
                "until" => {
                    open.pop()?;
                    next_is_expression = true;
                }
                "elseif" | "while" => next_is_expression = true,
                "then" => next_is_expression = open.last() == Some(&Open::IfExpression),
                "else" if open.last() == Some(&Open::IfExpression) => {
                    open.pop();
                    next_is_expression = true;
                }
                "return" | "and" | "or" | "not" | "in" => next_is_expression = true,
                _ => {}
            }
        }
        if brackets < 0 {
            return None;
        }
        expression_next = next_is_expression;
        k += 1;
    }
}

/// A VM's chunk waiting to take its templates (`register`, then
/// `ui.__templates` as the chunk starts): its key and its templates.
#[derive(Default)]
pub(crate) struct Registry {
    next: RefCell<Option<(u64, Rc<Chunk>)>>,
}

/// A chunk's templates, held by the chunk (its `__zgx_t`) and so by every
/// closure of it that stamps: freed with the last of them.
struct Chunk {
    templates: Vec<Compiled>,
}

/// A template, its literals made once.
struct Compiled {
    node: TNode,
    /// Its literals, in the order its fields name them.
    literals: Vec<Value>,
}

thread_local! {
    /// Templates alive on this thread (their chunks not yet freed).
    static LIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Templates alive on this thread: the memory test's count.
pub(crate) fn live() -> usize {
    LIVE.with(std::cell::Cell::get)
}

impl Chunk {
    fn new(lua: &Lua, nodes: &[TNode]) -> Self {
        LIVE.with(|live| live.set(live.get() + nodes.len()));
        Self {
            templates: nodes
                .iter()
                .map(|node| {
                    let mut literals = Vec::new();
                    literals_of(lua, node, &mut literals);
                    Compiled {
                        node: node.clone(),
                        literals,
                    }
                })
                .collect(),
        }
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        LIVE.with(|live| live.set(live.get() - self.templates.len()));
    }
}

/// A chunk's templates, as its `__zgx_t` holds them.
struct ChunkData(Rc<Chunk>);
impl mlua::UserData for ChunkData {}

fn literals_of(lua: &Lua, node: &TNode, out: &mut Vec<Value>) {
    for (_, value) in &node.fields {
        match value {
            // What the rewrite reads as a literal is one Luau reads.
            TValue::Lit(text) => out.push(
                lua.load(format!("return {text}"))
                    .eval()
                    .unwrap_or(Value::Nil),
            ),
            TValue::Node(child) => literals_of(lua, child, out),
            TValue::Hole(_) | TValue::Spread(_) => {}
        }
    }
}

/// A stamp's caller: its chunk's source, its short name, and the line.
type Stamping = Option<(Rc<str>, Rc<str>, u32)>;

thread_local! {
    /// Stamps made on this thread (the corpus test's proof that it ran).
    static STAMPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Stamps made on this thread so far.
pub(crate) fn stamps() -> u64 {
    STAMPS.with(std::cell::Cell::get)
}

thread_local! {
    /// While a stamp makes a node: the chunk calling it (its `source`, as
    /// `@path`, and its short name) and the line the node's call is written
    /// on. What a constructor reads of its caller (`node::module_prefix`,
    /// `call_site`, `location`) it reads here then, as it would have of the
    /// script's own call.
    static STAMPING: RefCell<Stamping> = const { RefCell::new(None) };
}

/// The caller a stamp's constructor stands in for (`STAMPING`): the chunk's
/// source, its short name, and the node's line.
pub(crate) fn stamping() -> Option<(String, String, u32)> {
    STAMPING.with(|s| {
        s.borrow()
            .as_ref()
            .map(|(source, short, line)| (source.to_string(), short.to_string(), *line))
    })
}

/// Install `ui.__stamp` on `ui`, its constructors those `ui` has now.
pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    let mut constructors = HashMap::new();
    for kind in KINDS {
        constructors.insert(*kind, ui.get::<Function>(*kind)?);
    }
    let constructors = Rc::new(constructors);
    lua.set_app_data(Registry::default());
    // A spread's values, however many (nils kept, by `n`).
    ui.set(
        "__pack",
        lua.create_function(|lua, args: MultiValue| {
            let packed = lua.create_table_with_capacity(args.len(), 1)?;
            packed.raw_set("n", args.len())?;
            for (i, value) in args.into_iter().enumerate() {
                packed.raw_set(i + 1, value)?;
            }
            Ok(packed)
        })?,
    )?;
    // A chunk's first statement: its templates, made for this VM as it
    // loaded (`register`).
    ui.set(
        "__templates",
        lua.create_function(|lua, key: u64| {
            let next = lua
                .app_data_ref::<Registry>()
                .and_then(|registry| registry.next.borrow_mut().take());
            match next {
                Some((registered, chunk)) if registered == key => Ok(ChunkData(chunk)),
                _ => Err(mlua::Error::runtime(
                    "ui.__templates: this chunk's templates weren't made as it loaded",
                )),
            }
        })?,
    )?;
    ui.set(
        "__stamp",
        lua.create_function(move |lua, args: MultiValue| {
            let mut args = args.into_iter();
            let chunk = match args.next() {
                Some(Value::UserData(chunk)) => chunk.borrow::<ChunkData>()?.0.clone(),
                _ => return Err(mlua::Error::runtime("ui.__stamp: no templates")),
            };
            let number = match args.next() {
                Some(Value::Integer(n)) => n as usize,
                Some(Value::Number(n)) => n as usize,
                _ => 0,
            };
            let compiled = number
                .checked_sub(1)
                .and_then(|at| chunk.templates.get(at))
                .ok_or_else(|| mlua::Error::runtime("ui.__stamp: an unknown template"))?;
            let holes: Vec<Value> = args.collect();
            STAMPS.with(|stamps| stamps.set(stamps.get() + 1));
            // The chunk calling the stamp: its nodes' caller.
            let caller = lua
                .inspect_stack(1, |d| {
                    let source = d.source();
                    Caller {
                        source: Rc::from(source.source.as_deref().unwrap_or("")),
                        short: Rc::from(source.short_src.as_deref().unwrap_or("")),
                        line: d.current_line().unwrap_or(0) as u32,
                    }
                })
                .unwrap_or_else(|| Caller {
                    source: Rc::from(""),
                    short: Rc::from(""),
                    line: 0,
                });
            let mut literals = compiled.literals.iter();
            build(
                lua,
                &caller,
                &compiled.node,
                &holes,
                &mut literals,
                &constructors,
            )
        })?,
    )
}

/// A stamp's caller: its chunk's `source` and short name, and the line of
/// the stamp's call (where the chunk's frame is while it runs).
struct Caller {
    source: Rc<str>,
    short: Rc<str>,
    line: u32,
}

/// A constructor's `error`, as the script's own call on `line` would have
/// raised it: its message as it was, its traceback naming `line` where
/// the stamp's call is.
fn as_written(error: mlua::Error, caller: &Caller, line: u32) -> mlua::Error {
    let mlua::Error::CallbackError { traceback, cause } = error else {
        return error;
    };
    let mut root = cause;
    while let mlua::Error::CallbackError { cause, .. } = &*root {
        root = cause.clone();
    }
    if line == caller.line {
        // The stamp's own traceback is the one the call would have had.
        return Arc::unwrap_or_clone(root);
    }
    // This traceback in place of the stamp's (which, naming the stamp's
    // line, isn't found in it: `CallbackError`'s display shows it whole).
    let traceback = traceback.replacen(
        &format!("\t{}:{}:", caller.short, caller.line),
        &format!("\t{}:{line}:", caller.short),
        1,
    );
    mlua::Error::CallbackError {
        traceback,
        cause: root,
    }
}

/// The node `node` makes: its table as the script would have built it,
/// given to its constructor as if called from the node's own line. A
/// constructor's error is that line's.
fn build<'a>(
    lua: &Lua,
    caller: &Caller,
    node: &TNode,
    holes: &[Value],
    literals: &mut std::slice::Iter<'a, Value>,
    constructors: &HashMap<&'static str, Function>,
) -> mlua::Result<Value> {
    let table = lua.create_table()?;
    let mut at = 1;
    for (key, value) in &node.fields {
        let value = match value {
            TValue::Lit(_) => literals.next().cloned().unwrap_or(Value::Nil),
            TValue::Hole(n) => holes.get(*n).cloned().unwrap_or(Value::Nil),
            TValue::Spread(n) => {
                if let Some(Value::Table(packed)) = holes.get(*n) {
                    let count: usize = packed.raw_get("n")?;
                    for i in 1..=count {
                        table.raw_set(at, packed.raw_get::<Value>(i)?)?;
                        at += 1;
                    }
                }
                continue;
            }
            TValue::Node(child) => build(lua, caller, child, holes, literals, constructors)?,
        };
        match key {
            Some(name) => table.raw_set(name.as_str(), value)?,
            None => {
                table.raw_set(at, value)?;
                at += 1;
            }
        }
    }
    let constructor = constructors
        .get(node.kind.as_str())
        .ok_or_else(|| mlua::Error::runtime("ui.__stamp: an unknown kind"))?;
    let before = STAMPING.with(|s| {
        s.borrow_mut()
            .replace((caller.source.clone(), caller.short.clone(), node.line))
    });
    let made = constructor.call(table);
    STAMPING.with(|s| *s.borrow_mut() = before);
    made.map_err(|error| as_written(error, caller, node.line))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holes_of(source: &str) -> Vec<String> {
        let rewritten = rewrite(source).expect("a template");
        rewritten.source.lines().map(str::to_owned).collect()
    }

    #[test]
    fn a_literal_tree_becomes_one_stamp_with_its_holes_in_order() {
        let source = "return ui.row { h = 28, bg = f(), ui.text { tostring(x), w = 80 }, ui.text { \"x\" } }";
        let rewritten = rewrite(source).expect("a template");
        assert_eq!(rewritten.templates.len(), 1);
        let node = &rewritten.templates[0];
        assert_eq!(
            rewritten.source,
            format!(
                "local __zgx_t = ui.__templates({}) return ui.__stamp(__zgx_t, 1, f(), tostring(x))",
                rewritten.key
            )
        );
        assert_eq!(node.kind, "row");
        assert_eq!(node.fields.len(), 4);
    }

    #[test]
    fn holes_stay_on_their_lines() {
        let lines =
            holes_of("return ui.column {\n  gap = 4,\n  ui.text { a },\n  ui.text { b },\n}");
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert!(lines[2].contains(", a"), "{lines:?}");
        assert!(lines[3].contains(", b"), "{lines:?}");
        assert!(lines[4].ends_with(')'), "{lines:?}");
    }

    #[test]
    fn a_function_hole_keeps_its_commas() {
        let source = "return ui.button { on_click = function(a, b) return a, b end, \"go\" }";
        let rewritten = rewrite(source).expect("a template");
        assert!(rewritten.source.contains("function(a, b) return a, b end"));
    }

    #[test]
    fn a_call_ending_the_list_gives_all_its_values() {
        let spread = |source: &str| {
            let rewritten = rewrite(source).expect("a template");
            rewritten.source.contains("ui.__pack(")
        };
        assert!(spread("return ui.column { table.unpack(rows) }"));
        assert!(spread("return ui.column { gap = 4, f(a, b) }"));
        assert!(spread("return ui.column { ... }"));
        assert!(spread("return ui.column { f \"x\" }"));
        // Cut to one value: not last, or a named field last.
        assert!(!spread("return ui.column { f(), g }"));
        assert!(!spread("return ui.column { f(), gap = 4 }"));
        assert!(!spread("return ui.column { rows }"));
    }

    #[test]
    fn an_if_expression_is_a_hole_and_an_if_statement_a_block() {
        let rewritten = rewrite(
            "return ui.text { label, size = if big then 17 else 12, color = if a then \"x\" elseif b then if c then \"y\" else \"z\" else \"w\" }",
        )
        .expect("a template");
        assert_eq!(rewritten.templates.len(), 1, "{}", rewritten.source);
        assert!(rewritten.source.contains(", if big then 17 else 12,"));
        // In a function hole, an `if` statement's `end` closes it.
        let rewritten = rewrite(
            "return ui.button { \"go\", on_click = function()\n  if x then y() else z() end\n  local v = if a then 1 else 2\nend, gap = 2 }",
        )
        .expect("a template");
        assert_eq!(rewritten.templates.len(), 1, "{}", rewritten.source);
        assert_eq!(rewritten.templates[0].fields.len(), 3);
    }

    #[test]
    fn a_template_in_a_hole_is_a_stamp_too() {
        let source = "return ui.column {\n  gap = 2,\n  ui.each(xs, function(x)\n    return ui.row { ui.text { x } }\n  end),\n}";
        let rewritten = rewrite(source).expect("a template");
        assert_eq!(rewritten.templates.len(), 2, "{}", rewritten.source);
        assert_eq!(rewritten.source.matches("ui.__stamp(").count(), 2);
        assert_eq!(
            rewritten.source.lines().count(),
            source.lines().count(),
            "{}",
            rewritten.source
        );
        let inner = rewritten.source.lines().nth(3).unwrap();
        assert!(inner.contains("return ui.__stamp("), "{inner}");
        assert!(inner.contains(", x)"), "{inner}");
    }

    #[test]
    fn the_report_counts_stamps_and_says_why_the_rest_are_not() {
        let (stamped, left) = report(
            "return ui.column { ui.text { \"a\" }, ui.scroll { h = 1 }, ui.row { [k] = 1 }, ui.text(\"b\") }",
        );
        assert_eq!(stamped, 2);
        assert_eq!(
            left,
            [
                "ui.scroll: a kind not listed",
                "ui.row: a computed key or a field it can't read",
                "ui.text: called with ( )",
            ]
        );
        let (stamped, left) = report("local u = ui\nreturn ui.text { \"a\" }");
        assert_eq!(stamped, 0);
        assert_eq!(left, ["ui.text: `ui` used as a value in its file"]);
    }

    #[test]
    fn its_first_code_line_takes_its_templates() {
        // After comments, on the first code line; an expression returned.
        let rewritten = rewrite("--!strict\n-- a note\nlocal x = 1\nreturn ui.text { \"a\" }")
            .expect("a template");
        let lines: Vec<&str> = rewritten.source.lines().collect();
        assert_eq!(lines[0], "--!strict");
        assert!(
            lines[2].starts_with(&format!(
                "local __zgx_t = ui.__templates({}) local x",
                rewritten.key
            )),
            "{lines:?}"
        );
        let rewritten = rewrite("ui.column { ui.text { \"a\" } }").expect("a template");
        assert!(
            rewritten.source.contains(") return ui.__stamp(__zgx_t, 1)"),
            "{}",
            rewritten.source
        );
    }

    #[test]
    fn a_text_template_keeps_its_place_before_a_call_after_it() {
        // `make()` could make state `n` the text names: the row is left as
        // written, and the text alone is a stamp, in its place.
        let rewritten = rewrite("return ui.row { ui.text { \"{n}\" }, make() }").expect("the text");
        assert_eq!(rewritten.templates.len(), 1);
        assert_eq!(rewritten.templates[0].kind, "text");
        assert!(
            rewritten
                .source
                .contains("return ui.row { ui.__stamp(__zgx_t, 1), make() }")
        );
        // Before the text, or no call after it: one stamp for the row.
        for source in [
            "return ui.row { make(), ui.text { \"{n}\" } }",
            "return ui.row { ui.text { \"{n}\" }, label, theme.x, function() f() end }",
            "return ui.row { ui.text { \"plain\" }, make() }",
        ] {
            let rewritten = rewrite(source).expect("a template");
            assert_eq!(rewritten.templates[0].kind, "row", "{source}");
        }
    }

    #[test]
    fn what_is_left_as_written() {
        // `ui` as a local, a computed key, a kind not listed, a call
        // without a table.
        assert!(rewrite("local ui = x\nreturn ui.row { }").is_none());
        assert!(rewrite("return ui.row { [k] = 1 }").is_none());
        assert!(rewrite("return ui.scroll { h = 1 }").is_none());
        assert!(rewrite("return ui.text(\"x\")").is_none());
        assert!(rewrite("return f(ui)").is_none());
    }
}
