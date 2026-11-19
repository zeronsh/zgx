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

use crate::lint::Kind as Tk;

/// A token of a script as the rewrite reads it: its kind, its text (where it
/// lies in the source) and its line. Read as lint's tokenizer reads
/// (`lint::tokenize`), without copying any text: a load leaves its
/// allocator little to keep.
#[derive(Clone, Copy)]
struct Tok<'a> {
    kind: Tk,
    text: &'a str,
    line: u32,
}
impl Tok<'_> {
    fn is(&self, text: &str) -> bool {
        self.kind != Tk::Str && self.text == text
    }
    /// Where it starts in `source` (the source it was read from), in bytes.
    fn at(&self, source: &str) -> usize {
        self.text.as_ptr() as usize - source.as_ptr() as usize
    }
}

/// `source`'s tokens, comments left out: counted first, so their list is
/// made once, at its size.
fn lex(source: &str) -> Vec<Tok<'_>> {
    let mut count = 0;
    lex_each(source, |_| count += 1);
    let mut tokens = Vec::with_capacity(count);
    lex_each(source, |token| tokens.push(token));
    tokens
}

/// Each of `source`'s tokens, in order, to `each`.
fn lex_each<'a>(source: &'a str, mut each: impl FnMut(Tok<'a>)) {
    let bytes = source.as_bytes();
    let (mut i, mut line) = (0, 1u32);
    // The level of a long bracket opening at `at` (`[[` 0, `[==[` 2).
    let long_open = |at: usize| -> Option<usize> {
        if bytes.get(at) != Some(&b'[') {
            return None;
        }
        let level = bytes[at + 1..].iter().take_while(|b| **b == b'=').count();
        (bytes.get(at + 1 + level) == Some(&b'[')).then_some(level)
    };
    // Where a long bracket of `level` opened at `at` ends.
    let long_end = |at: usize, level: usize| -> usize {
        let body = at + level + 2;
        (body..bytes.len())
            .find(|&j| {
                bytes[j] == b']'
                    && bytes.get(j + 1 + level) == Some(&b']')
                    && bytes[j + 1..j + 1 + level].iter().all(|b| *b == b'=')
            })
            .map_or(bytes.len(), |j| j + level + 2)
    };
    while i < bytes.len() {
        let c = bytes[i];
        let start_line = line;
        if c.is_ascii_whitespace() {
            line += u32::from(c == b'\n');
            i += 1;
            continue;
        }
        if c == b'-' && bytes.get(i + 1) == Some(&b'-') {
            let end = match long_open(i + 2) {
                Some(level) => long_end(i + 2, level),
                None => bytes[i..]
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(bytes.len(), |n| i + n),
            };
            line += bytes[i..end].iter().filter(|b| **b == b'\n').count() as u32;
            i = end;
            continue;
        }
        let (kind, end) = if let Some(level) = long_open(i) {
            (Tk::Str, long_end(i, level))
        } else if c == b'"' || c == b'\'' || c == b'`' {
            let mut j = i + 1;
            let mut depth = 0;
            while j < bytes.len() {
                match bytes[j] {
                    b'\\' => j += 1,
                    b'{' if c == b'`' => depth += 1,
                    b'}' if c == b'`' && depth > 0 => depth -= 1,
                    b'\n' if c != b'`' => break,
                    q if q == c && depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            (Tk::Str, (j + 1).min(bytes.len()))
        } else if c.is_ascii_digit()
            || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let hex = bytes.get(i + 1).is_some_and(|c| matches!(c, b'x' | b'X'));
            let mut j = i;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric()
                    || bytes[j] == b'_'
                    || bytes[j] == b'.'
                    || (matches!(bytes[j], b'+' | b'-')
                        && !hex
                        && matches!(bytes[j - 1], b'e' | b'E')))
            {
                j += 1;
            }
            (Tk::Number, j)
        } else if let Some(first) = source[i..].chars().next()
            && (first.is_alphabetic() || first == '_')
        {
            // A name: from an alphabetic character or `_` on, as lint reads.
            let len = source[i..]
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .map(char::len_utf8)
                .sum::<usize>();
            (Tk::Name, i + len)
        } else if c >= 0x80 {
            // Any other character: itself.
            let len = source[i..].chars().next().map_or(1, char::len_utf8);
            (Tk::Punct, i + len)
        } else {
            const LONG: &[&str] = &[
                "...", "..=", "//=", "==", "~=", "<=", ">=", "..", "::", "->", "+=", "-=", "*=",
                "/=", "%=", "^=", "//",
            ];
            let len = LONG
                .iter()
                .find(|op| bytes[i..].starts_with(op.as_bytes()))
                .map_or(1, |op| op.len());
            (Tk::Punct, i + len)
        };
        // The end on a character's boundary (a string cut at the source's end).
        let end = (end..=bytes.len())
            .find(|&e| source.is_char_boundary(e))
            .unwrap_or(bytes.len());
        each(Tok {
            kind,
            text: &source[i..end],
            line: start_line,
        });
        line += source[i..end].bytes().filter(|b| *b == b'\n').count() as u32;
        i = end;
    }
}
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
    pub kind: &'static str,
    pub line: u32,
    pub fields: Vec<(Option<&'static str>, TValue)>,
}

/// A field's value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TValue {
    /// A literal, as written.
    Lit(Literal),
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
    pub source: Rc<str>,
    /// Its templates, in the order its `__stamp` calls number them (from 1).
    pub templates: Rc<[TNode]>,
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
        static ON: bool =
            std::env::var("ZGX_TEMPLATES").is_ok_and(|v| v == "force" || v == "stamp");
    }
    ON.with(|on| *on)
}

/// What `code` compiles to: rewritten if templates are on and it has any.
/// The same bytes in every VM, so compiled once (`sources`) for all.
pub(crate) fn for_compile(code: &[u8]) -> ToCompile<'_> {
    match enabled().then(|| rewritten(code)).flatten() {
        Some(rewritten) => ToCompile::Rewritten(rewritten.source.clone()),
        None => ToCompile::AsWritten(code),
    }
}

/// What a source compiles from (`for_compile`): itself, or its rewrite,
/// shared with the load that registered it (not copied).
pub(crate) enum ToCompile<'a> {
    AsWritten(&'a [u8]),
    Rewritten(Rc<str>),
}
impl std::ops::Deref for ToCompile<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            ToCompile::AsWritten(code) => code,
            ToCompile::Rewritten(source) => source.as_bytes(),
        }
    }
}

/// A source's hash and length, and what it rewrote to (`LAST`).
type LastRewritten = (u64, usize, Option<Rc<Rewritten>>);

thread_local! {
    /// The last source rewritten, by its hash and length: a load rewrites
    /// it to register its templates and again to compile it (`for_compile`).
    /// Let go as idle (`trim`).
    static LAST: RefCell<Option<LastRewritten>> = const { RefCell::new(None) };
}

/// `code` rewritten (`rewrite`), once for a load's two uses.
fn rewritten(code: &[u8]) -> Option<Rc<Rewritten>> {
    let hash = {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        code.hash(&mut hasher);
        hasher.finish()
    };
    if let Some(found) = LAST.with_borrow(|last| {
        last.as_ref()
            .filter(|(h, len, _)| *h == hash && *len == code.len())
            .map(|(_, _, rewritten)| rewritten.clone())
    }) {
        return found;
    }
    let made = std::str::from_utf8(code)
        .ok()
        .and_then(rewrite)
        .map(Rc::new);
    LAST.set(Some((hash, code.len(), made.clone())));
    made
}

/// What `code` loads as in `lua`: `None`, as written, templates being off
/// or it having none; otherwise the rewritten source (or bytecode compiled
/// from it, `for_compile`), its templates made for `lua` and waiting for the
/// chunk to take them as it starts (`ui.__templates`). Called just before
/// the chunk is compiled and run.
pub(crate) fn register(lua: &Lua, code: &[u8]) -> Option<Rc<str>> {
    if !enabled() {
        return None;
    }
    let rewritten = rewritten(code)?;
    let registry = lua.app_data_ref::<Registry>()?;
    let chunk = Rc::new(Chunk::new(&rewritten.templates));
    CHUNKS.with_borrow_mut(|chunks| {
        chunks.retain(|chunk| chunk.strong_count() > 0);
        chunks.push(Rc::downgrade(&chunk));
    });
    *registry.next.borrow_mut() = Some((rewritten.key, chunk));
    Some(rewritten.source.clone())
}

/// `source` with its templates rewritten to `ui.__stamp` calls, or `None`
/// if it has none (or uses `ui` other than as `ui.<field>`). Its first code
/// line starts by taking its templates (`local __zgx_t =
/// ui.__templates(key) `), on that line, so every line stays where it was.
pub(crate) fn rewrite(source: &str) -> Option<Rewritten> {
    let mut scan = scan(source, false)?;
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
    // One that starts with a statement's keyword can't be: not compiled to
    // see.
    let expression = !scan.statement
        && mlua::chunk::Compiler::new()
            .compile(format!("return {source}"))
            .is_ok();
    let prologue = format!(
        "local {LOCAL} = ui.__templates({key}) {}",
        if expression { "return " } else { "" }
    );
    // Before its first token: after any comments (`--!strict` and the
    // like stay first).
    // (What comes before its first token is copied as it was.)
    scan.out.insert_str(scan.first, &prologue);
    Some(Rewritten {
        source: Rc::from(scan.out),
        templates: Rc::from(scan.templates),
        key,
    })
}

/// Whether `tokens` name `ui` only as `ui.<field>` (as a field of
/// something else too): otherwise a file is left as written.
fn ui_only_as_fields(tokens: &[Tok<'_>]) -> bool {
    tokens.iter().enumerate().all(|(i, token)| {
        token.kind != Tk::Name
            || token.text != "ui"
            || (i > 0 && (tokens[i - 1].is(".") || tokens[i - 1].is(":")))
            || tokens.get(i + 1).is_some_and(|t| t.is("."))
    })
}

/// A source rewritten: its text, its templates, and which tokens are a
/// template's own (not its holes').
struct Scan<'a> {
    source: &'a str,
    out: String,
    templates: Vec<TNode>,
    stamped: Vec<bool>,
    /// Where its first token is, in bytes (none: its end).
    first: usize,
    /// Whether it starts with a statement's keyword (`local`, `return`…):
    /// then it can't be an expression.
    statement: bool,
}

fn scan(source: &str, report: bool) -> Option<Scan<'_>> {
    // No `ui.<kind>` in it: nothing to read.
    if !source
        .match_indices("ui.")
        .any(|(at, _)| KINDS.iter().any(|kind| source[at + 3..].starts_with(kind)))
    {
        return None;
    }
    let tokens = lex(source);
    // `ui` named any other way (a local, a parameter, passed on): left alone.
    if !ui_only_as_fields(&tokens) {
        return None;
    }
    let mut scan = Scan {
        first: tokens.first().map_or(source.len(), |t| t.at(source)),
        statement: tokens.first().is_some_and(|t| {
            t.kind == Tk::Name
                && matches!(
                    t.text,
                    "local"
                        | "function"
                        | "return"
                        | "if"
                        | "for"
                        | "while"
                        | "do"
                        | "repeat"
                        | "export"
                        | "type"
                )
        }),
        source,
        out: String::with_capacity(source.len() + 64),
        templates: Vec::new(),
        // Which tokens are a template's own: for `report` alone.
        stamped: if report {
            vec![false; tokens.len()]
        } else {
            Vec::new()
        },
    };
    scan.emit(&tokens, 0, tokens.len(), 0, source.len());
    Some(scan)
}

impl Scan<'_> {
    /// Tokens `from..to`, bytes `start..stop`, written out with each
    /// template in them a stamp (the templates in its holes too).
    fn emit(&mut self, tokens: &[Tok<'_>], from: usize, to: usize, start: usize, stop: usize) {
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
            self.out
                .push_str(&self.source[copied..tokens[i].at(self.source)]);
            if let Some(stamped) = self.stamped.get_mut(i..=end) {
                stamped.fill(true);
            }
            // `ui.__stamp(__zgx_t, n, …)`, each hole on its own line again.
            self.out.push_str(&format!("ui.__stamp({LOCAL}, {number}"));
            let mut line = tokens[i].line;
            for hole in &holes {
                while line < tokens[hole.first].line {
                    self.out.push('\n');
                    line += 1;
                }
                self.out.push_str(", ");
                let from = tokens[hole.first].at(self.source);
                let to = tokens[hole.last].at(self.source) + tokens[hole.last].text.len();
                line += self.source[from..to]
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count() as u32;
                if let Some(stamped) = self.stamped.get_mut(hole.first..=hole.last) {
                    stamped.fill(false);
                }
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
            copied = tokens[end].at(self.source) + 1;
            i = end + 1;
        }
        self.out.push_str(&self.source[copied..stop]);
    }
}

/// What the rewrite makes of `source`'s constructors (`ui.<kind> { … }` or
/// `ui.<kind>(…)`): how many it stamps, and each it leaves as written with
/// why (the corpus test's report).
pub(crate) fn report(source: &str) -> (usize, Vec<String>) {
    let tokens = lex(source);
    let stamped = scan(source, true).map(|scan| scan.stamped);
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
        let kind = tokens[i + 2].text;
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

fn template_at(tokens: &[Tok<'_>], i: usize) -> Option<Found> {
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
fn in_order(tokens: &[Tok<'_>], i: usize, end: usize, holes: &[HoleAt]) -> bool {
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
fn may_call(tokens: &[Tok<'_>], first: usize, last: usize) -> bool {
    if tokens[first].kind == Tk::Name && tokens[first].text == "function" {
        return false;
    }
    (first + 1..=last).any(|k| {
        let before = &tokens[k - 1];
        let callee = (before.kind == Tk::Name && !is_keyword(before.text))
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

fn node_at(tokens: &[Tok<'_>], i: usize, holes: &mut Vec<HoleAt>) -> Option<(TNode, usize)> {
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
    if kind.kind != Tk::Name || !KINDS.contains(&kind.text) {
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
            (Some(intern(token.text)), k + 2)
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
            kind: KINDS.iter().copied().find(|k| *k == kind.text)?,
            line: ui.line,
            fields,
        },
        k,
    ))
}

/// The value of the expression in tokens `first..=last`: a literal, a
/// template or a hole.
fn value_of(tokens: &[Tok<'_>], first: usize, last: usize, holes: &mut Vec<HoleAt>) -> TValue {
    let one = &tokens[first];
    if first == last {
        let plain_string = one.kind == Tk::Str
            && (one.text.starts_with('"') || one.text.starts_with('\''))
            && !one.text.contains('\\');
        if one.kind == Tk::Number
            || plain_string
            || (one.kind == Tk::Name && matches!(one.text, "true" | "false" | "nil"))
        {
            return TValue::Lit(Literal::read(one.text));
        }
    }
    if last == first + 1 && one.is("-") && tokens[last].kind == Tk::Number {
        return TValue::Lit(match Literal::read(tokens[last].text) {
            Literal::Number(n) => Literal::Number(-n),
            _ => Literal::Luau(format!("-{}", tokens[last].text).into()),
        });
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
fn multiple(tokens: &[Tok<'_>], first: usize, last: usize) -> bool {
    let end = &tokens[last];
    end.is("...") || (last > first && (end.is(")") || end.is("}") || end.kind == Tk::Str))
}

/// Where the expression starting at `from` ends: the index of the `,`,
/// `;` or `}` after it, at its own depth (brackets, and the blocks of
/// function literals and the like).
fn expression_end(tokens: &[Tok<'_>], from: usize) -> Option<usize> {
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
            match token.text {
                "(" | "{" | "[" => brackets += 1,
                ")" | "]" => brackets -= 1,
                "}" if brackets == 0 && open.is_empty() => return Some(k),
                "}" => brackets -= 1,
                "," | ";" if brackets == 0 && open.is_empty() => return Some(k),
                _ => {}
            }
            // After an operator, `=`, `,` or an opening bracket comes an
            // expression; after a closing bracket, no.
            next_is_expression = !matches!(token.text, ")" | "]" | "}");
        } else if token.kind == Tk::Name {
            match token.text {
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
    /// Its stamps' caller, read at its first (`Caller`).
    caller: RefCell<Option<Rc<Caller>>>,
}

/// A template: its tree, and a plan for each of its nodes (in order, each
/// before the nodes within it).
struct Compiled {
    /// Its chunk's trees, shared with the rewrite that read them, and
    /// which is its own.
    nodes: Rc<[TNode]>,
    at: usize,
    plans: Vec<Plan>,
}
impl Compiled {
    fn node(&self) -> &TNode {
        &self.nodes[self.at]
    }
}

/// How a stamp makes one of a template's nodes.
struct Plan {
    /// Its own fields' literals, in their order: made at its first stamp
    /// (`Plan::literals`), let go as idle (`trim`).
    literals: RefCell<Option<Rc<[Value]>>>,
    /// The nodes it is made of, itself included: the plan after its last
    /// is `size` on.
    size: usize,
    /// Made from a prototype of its literals (`node::proto`) and a table of
    /// its holes and children alone, not a whole table: every node whose
    /// constructor reads only what `fill_from` walks.
    fast: bool,
    /// No hole in it or within it: made once, and shared after.
    constant: bool,
    proto: RefCell<Option<Rc<crate::node::Proto>>>,
    made: RefCell<Option<Rc<crate::node::Node>>>,
    /// Stamps it has been made by: a prototype only once it is made again
    /// (most of an app's interface is made once, and keeps nothing for it).
    stamped: std::cell::Cell<u32>,
    /// Its named fields that a stamp gives (holes and nodes), and all its
    /// named fields: the sizes of the tables it makes.
    named: usize,
    named_all: usize,
}

/// Plans for `node` and the nodes within it, in order, into `out`.
fn plans_of(node: &TNode, out: &mut Vec<Plan>) -> bool {
    let at = out.len();
    let keyed = |name: &str| node.fields.iter().any(|(key, _)| *key == Some(name));
    // What a constructor reads of its table beyond `fill_from`'s walk, or
    // what that walk finishes with that a second walk wouldn't redo.
    let fast = !keyed("variants")
        && !keyed("role")
        && !node
            .fields
            .iter()
            .any(|(key, _)| key.is_some_and(|k| k.starts_with("layout")))
        // A button's `variant` adds its look where the table sets none
        // (`button_look`): only over a table of literals alone.
        && !(node.kind == "button"
            && keyed("variant")
            && node
                .fields
                .iter()
                .any(|(key, value)| key.is_some() && !matches!(value, TValue::Lit(_))));
    out.push(Plan {
        literals: RefCell::default(),
        size: 1,
        fast,
        constant: false,
        proto: RefCell::default(),
        made: RefCell::default(),
        stamped: std::cell::Cell::new(0),
        named: node
            .fields
            .iter()
            .filter(|(key, value)| key.is_some() && !matches!(value, TValue::Lit(_)))
            .count(),
        named_all: node.fields.iter().filter(|(key, _)| key.is_some()).count(),
    });
    let mut constant = true;
    for (_, value) in &node.fields {
        match value {
            TValue::Lit(_) => {}
            TValue::Node(child) => constant &= plans_of(child, out),
            TValue::Hole(_) | TValue::Spread(_) => constant = false,
        }
    }
    let size = out.len() - at;
    let plan = &mut out[at];
    plan.size = size;
    plan.constant = constant && fast;
    constant
}

/// A literal as the rewrite read it (`value_of`): its value made from this
/// at a stamp (`Literal::value`), no text kept for the common ones.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Literal {
    Number(f64),
    /// A quoted string's characters (no escapes: `value_of` takes none).
    Text(Box<str>),
    True,
    False,
    Nil,
    /// Any other number as written (hex, `_`, an exponent), read by Luau.
    Luau(Box<str>),
}

impl Literal {
    /// The literal token `text` reads as.
    fn read(text: &str) -> Self {
        let quoted = text.len() >= 2
            && (text.starts_with('"') && text.ends_with('"')
                || text.starts_with('\'') && text.ends_with('\''));
        match text {
            "true" => Literal::True,
            "false" => Literal::False,
            "nil" => Literal::Nil,
            _ if quoted => Literal::Text(text[1..text.len() - 1].into()),
            _ if text.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                && text.bytes().filter(|b| *b == b'.').count() <= 1 =>
            {
                text.parse::<f64>()
                    .map_or_else(|_| Literal::Luau(text.into()), Literal::Number)
            }
            _ => Literal::Luau(text.into()),
        }
    }
    /// Its value, as Luau reads it.
    fn value(&self, lua: &Lua) -> Value {
        match self {
            Literal::Number(n) => Value::Number(*n),
            Literal::Text(text) => lua.create_string(&**text).map_or(Value::Nil, Value::String),
            Literal::True => Value::Boolean(true),
            Literal::False => Value::Boolean(false),
            Literal::Nil => Value::Nil,
            Literal::Luau(text) => lua
                .load(format!("return {text}"))
                .eval()
                .unwrap_or(Value::Nil),
        }
    }
}

/// A field's name, kept once for the process: the names scripts give
/// fields are few, and every template's tree shares them.
fn intern(name: &str) -> &'static str {
    thread_local! {
        static NAMES: RefCell<std::collections::HashSet<&'static str>> =
            RefCell::new(std::collections::HashSet::new());
    }
    NAMES.with_borrow_mut(|names| {
        if let Some(known) = names.get(name) {
            return *known;
        }
        let kept: &'static str = Box::leak(name.into());
        names.insert(kept);
        kept
    })
}

impl Plan {
    /// Its node's literals, as Luau reads them: kept from its second stamp
    /// after it loaded or went idle, while it is used; made for the one
    /// stamp alone before.
    fn literals(&self, lua: &Lua, node: &TNode) -> Rc<[Value]> {
        if let Some(literals) = self.literals.borrow().as_ref() {
            return literals.clone();
        }
        let literals: Rc<[Value]> = node
            .fields
            .iter()
            .filter_map(|(_, value)| match value {
                TValue::Lit(literal) => Some(literal.value(lua)),
                _ => None,
            })
            .collect();
        if self.stamped.get() > 1 {
            *self.literals.borrow_mut() = Some(literals.clone());
        }
        literals
    }
}

/// Whether stamps make nodes by their plans (`ZGX_TEMPLATES=force`), or
/// each from a whole table as the script would (`=stamp`: the rewrite
/// alone, to compare).
fn planned() -> bool {
    thread_local! {
        static ON: bool = std::env::var("ZGX_TEMPLATES").is_ok_and(|v| v == "force");
    }
    ON.with(|on| *on)
}

thread_local! {
    /// Templates alive on this thread (their chunks not yet freed).
    static LIVE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// The chunks alive on this thread, for `trim` and `bytes`.
    static CHUNKS: RefCell<Vec<std::rc::Weak<Chunk>>> = const { RefCell::new(Vec::new()) };
}

/// What the templates alive on this thread keep, as idle: their prototypes
/// and the nodes made once, made again at their next stamp.
pub(crate) fn trim() {
    LAST.set(None);
    CHUNKS.with_borrow_mut(|chunks| {
        chunks.retain(|chunk| chunk.strong_count() > 0);
        for chunk in chunks.iter().filter_map(std::rc::Weak::upgrade) {
            for plan in chunk.templates.iter().flat_map(|t| &t.plans) {
                plan.proto.take();
                plan.made.take();
                plan.literals.take();
            }
        }
    });
}

/// What the templates alive on this thread keep in Rust, in bytes: their
/// trees, plans, literals' values, prototypes and nodes made once.
pub(crate) fn bytes() -> usize {
    fn tree(node: &TNode) -> usize {
        size_of::<TNode>()
            + node.fields.capacity() * size_of::<(Option<&str>, TValue)>()
            + node
                .fields
                .iter()
                .map(|(_, value)| match value {
                    TValue::Lit(Literal::Text(text) | Literal::Luau(text)) => text.len(),
                    TValue::Node(child) => tree(child),
                    _ => 0,
                })
                .sum::<usize>()
    }
    let plan = |plan: &Plan| {
        plan.literals
            .borrow()
            .as_ref()
            .map_or(0, |l| l.len() * size_of::<Value>())
            + plan
                .proto
                .borrow()
                .as_deref()
                .map_or(0, crate::node::proto_bytes)
            + plan
                .made
                .borrow()
                .as_deref()
                .map_or(0, crate::node::tree_bytes)
    };
    CHUNKS.with_borrow(|chunks| {
        chunks
            .iter()
            .filter_map(std::rc::Weak::upgrade)
            .map(|chunk| {
                size_of::<Chunk>()
                    + chunk.templates.capacity() * size_of::<Compiled>()
                    + chunk
                        .templates
                        .iter()
                        .map(|t| {
                            tree(t.node())
                                + t.plans.capacity() * size_of::<Plan>()
                                + t.plans.iter().map(plan).sum::<usize>()
                        })
                        .sum::<usize>()
            })
            .sum()
    })
}

/// Templates alive on this thread: the memory test's count.
pub(crate) fn live() -> usize {
    LIVE.with(std::cell::Cell::get)
}

impl Chunk {
    fn new(nodes: &Rc<[TNode]>) -> Self {
        LIVE.with(|live| live.set(live.get() + nodes.len()));
        Self {
            templates: nodes
                .iter()
                .enumerate()
                .map(|(at, node)| {
                    let mut plans = Vec::new();
                    plans_of(node, &mut plans);
                    Compiled {
                        nodes: nodes.clone(),
                        at,
                        plans,
                    }
                })
                .collect(),
            caller: RefCell::default(),
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

/// A stamp's tables and nodes are made with the VM's memory limit set
/// aside (`create_table_relaxed`, `create_userdata_relaxed`): counted, not
/// checked. So each stamp checks first that the VM is under its limit, and
/// fails as an allocation past it would: a script stamping in a loop (a
/// plugin's, untrusted) goes past its limit by one stamp at most.
fn over_limit(lua: &Lua) -> mlua::Result<()> {
    if lua.over_memory_limit() {
        return Err(mlua::Error::MemoryError("not enough memory".into()));
    }
    Ok(())
}

/// Install `ui.__stamp` on `ui`, its constructors those `ui` has now.
pub(crate) fn install(lua: &Lua, ui: &Table) -> mlua::Result<()> {
    // Off, a script loads as written: no stamp to take, nothing installed.
    if !enabled() {
        return Ok(());
    }
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
            over_limit(lua)?;
            let packed = lua.create_table_relaxed(args.len(), 1)?;
            packed.raw_set_field_relaxed("n", args.len())?;
            for (i, value) in args.into_iter().enumerate() {
                packed.raw_set_relaxed(i + 1, value)?;
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
        lua.create_function(move |lua, mut args: MultiValue| {
            over_limit(lua)?;
            let chunk = match args.pop_front() {
                Some(Value::UserData(chunk)) => chunk.borrow::<ChunkData>()?.0.clone(),
                _ => return Err(mlua::Error::runtime("ui.__stamp: no templates")),
            };
            let number = match args.pop_front() {
                Some(Value::Integer(n)) => n as usize,
                Some(Value::Number(n)) => n as usize,
                _ => 0,
            };
            let compiled = number
                .checked_sub(1)
                .and_then(|at| chunk.templates.get(at))
                .ok_or_else(|| mlua::Error::runtime("ui.__stamp: an unknown template"))?;
            // Read where they came: no copy of them made.
            let holes: &[Value] = args.make_contiguous();
            STAMPS.with(|stamps| stamps.set(stamps.get() + 1));
            // The chunk calling the stamp, its nodes' caller: read at its first.
            let caller = chunk.caller.borrow().clone();
            let caller = match caller {
                Some(caller) => caller,
                None => {
                    let caller = Rc::new(
                        lua.inspect_stack(1, |d| {
                            let source = d.source();
                            Caller {
                                source: Rc::from(source.source.as_deref().unwrap_or("")),
                                short: Rc::from(source.short_src.as_deref().unwrap_or("")),
                            }
                        })
                        .unwrap_or(Caller {
                            source: Rc::from(""),
                            short: Rc::from(""),
                        }),
                    );
                    *chunk.caller.borrow_mut() = Some(caller.clone());
                    caller
                }
            };
            let stamp = Stamp {
                lua,
                caller: &caller,
                plans: &compiled.plans,
                holes,
                constructors: &constructors,
                planned: planned(),
                // A node made once says nothing of where it is used again.
                shared: planned() && !crate::node::locating(lua),
            };
            stamp.build(compiled.node(), &mut 0, true)?.value(lua)
        })?,
    )
}

/// A stamp's caller: its chunk's `source` and short name. Every stamp of
/// a chunk has the same (its `__zgx_t` is the chunk's own), read at its
/// first; the line of the call is read only for an error (`as_written`).
struct Caller {
    source: Rc<str>,
    short: Rc<str>,
}

/// A constructor's `error`, as the script's own call on `line` would have
/// raised it: its message as it was, its traceback naming `line` where
/// the stamp's call is.
fn as_written(lua: &Lua, error: mlua::Error, caller: &Caller, line: u32) -> mlua::Error {
    let mlua::Error::CallbackError { traceback, cause } = error else {
        return error;
    };
    let mut root = cause;
    while let mlua::Error::CallbackError { cause, .. } = &*root {
        root = cause.clone();
    }
    // Where the chunk's frame is: the stamp's call.
    let called = lua
        .inspect_stack(1, |d| d.current_line().unwrap_or(0) as u32)
        .unwrap_or(0);
    if line == called {
        // The stamp's own traceback is the one the call would have had.
        return Arc::unwrap_or_clone(root);
    }
    // This traceback in place of the stamp's (which, naming the stamp's
    // line, isn't found in it: `CallbackError`'s display shows it whole).
    let traceback = traceback.replacen(
        &format!("\t{}:{called}:", caller.short),
        &format!("\t{}:{line}:", caller.short),
        1,
    );
    mlua::Error::CallbackError {
        traceback,
        cause: root,
    }
}

/// A node a stamp made: by its plan (`Rc<Node>`, no userdata yet), or by
/// its constructor from a whole table (the script's value).
enum Made {
    Node(Rc<crate::node::Node>),
    Value(Value),
}

impl Made {
    /// As the script holds it.
    fn value(self, lua: &Lua) -> mlua::Result<Value> {
        match self {
            Made::Node(node) => crate::node::wrap(lua, node).map(Value::UserData),
            Made::Value(value) => Ok(value),
        }
    }
    /// As a child of the node being made.
    fn child(self) -> crate::node::Child {
        match self {
            Made::Node(node) => crate::node::Child::Node(node),
            Made::Value(value) => crate::node::Child::Value(value),
        }
    }
}

/// One stamp's making: its template's plans and holes, and its caller.
struct Stamp<'a> {
    lua: &'a Lua,
    caller: &'a Caller,
    plans: &'a [Plan],
    holes: &'a [Value],
    constructors: &'a HashMap<&'static str, Function>,
    /// Nodes made by their plans (`planned`), not each from a whole table.
    planned: bool,
    /// A node with no hole made once and shared.
    shared: bool,
}

impl Stamp<'_> {
    /// The node `node` makes, `plan` (its index in `plans`) moved past it
    /// and the nodes within it: what its constructor makes of the table the
    /// script would have built, as if called from the node's own line. A
    /// constructor's error is that line's.
    fn build(&self, node: &TNode, plan: &mut usize, root: bool) -> mlua::Result<Made> {
        let at = *plan;
        let this = &self.plans[at];
        *plan += 1;
        let again = this.stamped.get() > 0;
        this.stamped.set(this.stamped.get().saturating_add(1));
        // Its first stamp by the whole table, as written: a prototype (and
        // what it keeps) only for a node made again.
        if self.planned && this.fast && again {
            if self.shared
                && this.constant
                && let Some(made) = this.made.borrow().clone()
            {
                *plan = at + this.size;
                return Ok(Made::Node(made));
            }
            if let Some(proto) = self.proto(node, this) {
                match self.planned_node(node, this, &proto, plan) {
                    // A stamp's own node, not shared: its userdata made from
                    // it by `node_ud`, as every node's is.
                    Ok(made) if root && !(self.shared && this.constant) => {
                        let made = self.at(node, || crate::node::node_ud(self.lua, made))?;
                        return Ok(Made::Value(Value::UserData(made)));
                    }
                    Ok(made) => {
                        let made = self.at(node, || Ok(crate::node::node_rc(self.lua, made)))?;
                        if self.shared && this.constant {
                            *this.made.borrow_mut() = Some(made.clone());
                        }
                        return Ok(Made::Node(made));
                    }
                    // Made again as the script would: its error is then
                    // the constructor's, with the script's traceback.
                    Err(_) => *plan = at + 1,
                }
            }
        }
        self.whole(node, this, plan).map(Made::Value)
    }

    /// `node`'s prototype: made from its literal fields at its first stamp
    /// (`node::proto`), then kept. `None` if they fail: the whole table
    /// then says so, as the script's call would.
    fn proto(&self, node: &TNode, plan: &Plan) -> Option<Rc<crate::node::Proto>> {
        if let Some(proto) = plan.proto.borrow().as_ref() {
            return Some(proto.clone());
        }
        let made = (|| {
            let table = self.lua.create_table_relaxed(0, node.fields.len())?;
            let literals = plan.literals(self.lua, node);
            let (mut literals, mut at) = (literals.iter(), 1);
            for (key, value) in &node.fields {
                match (key, value) {
                    (Some(name), TValue::Lit(_)) => {
                        table.raw_set_field_relaxed(name, literals.next().cloned())?
                    }
                    (None, TValue::Lit(_)) => {
                        let literal = literals.next().cloned();
                        // A text's content, its first element: read as it is made.
                        if node.kind == "text" && at == 1 {
                            table.raw_set_relaxed(1, literal)?;
                        }
                        at += 1;
                    }
                    (None, _) => at += 1,
                    _ => {}
                }
            }
            self.at(node, || crate::node::proto(self.lua, node.kind, &table))
        })();
        let proto = Rc::new(made.ok().flatten()?);
        *plan.proto.borrow_mut() = Some(proto.clone());
        Some(proto)
    }

    /// `node` made from `proto` and what its literals don't give: its named
    /// holes (walked from a table of them alone, if it has any), a text's
    /// content, and its children, the nodes within it made by their plans.
    fn planned_node(
        &self,
        node: &TNode,
        this: &Plan,
        proto: &crate::node::Proto,
        plan: &mut usize,
    ) -> mlua::Result<crate::node::Node> {
        let text = node.kind == "text";
        let mut named: Option<Table> = None;
        let mut content = None;
        let mut items = Vec::new();
        let literals = this.literals(self.lua, node);
        let (mut literals, mut at) = (literals.iter(), 1_i64);
        // A positional value at `at`: a text's content first, else a child.
        let mut put = |at: &mut i64, child: crate::node::Child| -> mlua::Result<()> {
            if text && *at == 1 {
                content = Some(match child {
                    crate::node::Child::Node(node) => {
                        Value::UserData(crate::node::wrap(self.lua, node)?)
                    }
                    crate::node::Child::Value(value) => value,
                });
            } else {
                items.push((*at, child));
            }
            *at += 1;
            Ok(())
        };
        for (key, value) in &node.fields {
            match (key, value) {
                // In its prototype.
                (Some(_), TValue::Lit(_)) => {
                    literals.next();
                }
                (Some(name), TValue::Hole(n)) => {
                    let value = self.holes.get(*n).cloned().unwrap_or(Value::Nil);
                    let table = match &named {
                        Some(table) => table,
                        None => named.insert(self.lua.create_table_relaxed(0, this.named)?),
                    };
                    table.raw_set_field_relaxed(name, value)?;
                }
                (Some(name), TValue::Node(child)) => {
                    let value = self.build(child, plan, false)?.value(self.lua)?;
                    let table = match &named {
                        Some(table) => table,
                        None => named.insert(self.lua.create_table_relaxed(0, this.named)?),
                    };
                    table.raw_set_field_relaxed(name, value)?;
                }
                (None, TValue::Lit(_)) => {
                    let literal = literals.next().cloned().unwrap_or(Value::Nil);
                    if text && at == 1 {
                        // A text's content written out: in its prototype.
                        at += 1;
                    } else {
                        put(&mut at, crate::node::Child::Value(literal))?;
                    }
                }
                (None, TValue::Hole(n)) => {
                    let value = self.holes.get(*n).cloned().unwrap_or(Value::Nil);
                    put(&mut at, crate::node::Child::Value(value))?;
                }
                (_, TValue::Spread(n)) => {
                    if let Some(Value::Table(packed)) = self.holes.get(*n) {
                        let count: usize = packed.raw_get("n")?;
                        for i in 1..=count {
                            put(&mut at, crate::node::Child::Value(packed.raw_get(i)?))?;
                        }
                    }
                }
                (None, TValue::Node(child)) => {
                    let child = self.build(child, plan, false)?.child();
                    put(&mut at, child)?;
                }
            }
        }
        self.at(node, || {
            crate::node::made(self.lua, node.kind, proto, named.as_ref(), content, items)
        })
    }

    /// `node` made from the whole table the script would have built, by
    /// its constructor.
    fn whole(&self, node: &TNode, this: &Plan, plan: &mut usize) -> mlua::Result<Value> {
        let positional = node.fields.len() - this.named_all;
        let table = self.lua.create_table_relaxed(positional, this.named_all)?;
        let literals = this.literals(self.lua, node);
        let (mut literals, mut at) = (literals.iter(), 1);
        for (key, value) in &node.fields {
            let value = match value {
                TValue::Lit(_) => literals.next().cloned().unwrap_or(Value::Nil),
                TValue::Hole(n) => self.holes.get(*n).cloned().unwrap_or(Value::Nil),
                TValue::Spread(n) => {
                    if let Some(Value::Table(packed)) = self.holes.get(*n) {
                        let count: usize = packed.raw_get("n")?;
                        for i in 1..=count {
                            table.raw_set_relaxed(at, packed.raw_get::<Value>(i)?)?;
                            at += 1;
                        }
                    }
                    continue;
                }
                TValue::Node(child) => self.build(child, plan, false)?.value(self.lua)?,
            };
            match key {
                Some(name) => table.raw_set_field_relaxed(name, value)?,
                None => {
                    table.raw_set_relaxed(at, value)?;
                    at += 1;
                }
            }
        }
        let constructor = self
            .constructors
            .get(node.kind)
            .ok_or_else(|| mlua::Error::runtime("ui.__stamp: an unknown kind"))?;
        self.at(node, || constructor.call(table))
    }

    /// `make`, as a call from `node`'s own line: what it reads of its caller
    /// (`STAMPING`), and its error.
    fn at<T>(&self, node: &TNode, make: impl FnOnce() -> mlua::Result<T>) -> mlua::Result<T> {
        let caller = self.caller;
        let before = STAMPING.with(|s| {
            s.borrow_mut()
                .replace((caller.source.clone(), caller.short.clone(), node.line))
        });
        let made = make();
        STAMPING.with(|s| *s.borrow_mut() = before);
        made.map_err(|error| as_written(self.lua, error, caller, node.line))
    }
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
            &*rewritten.source,
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
    fn the_rewrite_reads_tokens_as_lint_does() {
        // Every script in the repository, and the hard cases.
        let mut sources: Vec<String> = vec![
            "local s = [==[ a ]] b ]==] --[[ c\n d ]] x = `a {b {c}} d` .. 'e\\'' -- f\n1e-5 0x1F 1_000 .5 a...b ::"
                .into(),
            "local é = 1\nreturn ui.text { \"ü\", é }".into(),
        ];
        fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() && !name.starts_with("target") && !name.starts_with('.') {
                    walk(&path, out);
                } else if name.ends_with(".luau")
                    && let Ok(text) = std::fs::read_to_string(&path)
                {
                    out.push(text);
                }
            }
        }
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".."),
            &mut sources,
        );
        assert!(sources.len() > 100, "{}", sources.len());
        for source in &sources {
            let (theirs, _) = crate::lint::tokenize(source);
            let ours = lex(source);
            assert_eq!(theirs.len(), ours.len(), "{source}");
            // A byte offset's column, in characters from 1, as lint counts.
            let column = |at: usize| {
                let start = source[..at].rfind('\n').map_or(0, |n| n + 1);
                source[start..at].chars().count() as u32 + 1
            };
            for (t, o) in theirs.iter().zip(&ours) {
                assert!(
                    t.kind == o.kind && t.text == o.text,
                    "{} {:?}",
                    t.text,
                    o.text
                );
                assert_eq!(
                    (t.line, t.column),
                    (o.line, column(o.at(source))),
                    "{}",
                    t.text
                );
            }
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
