//! Templates without a compiler, as Solid stamps them: a block's structure
//! (its elements, classes and attributes, without its texts and values) is
//! captured as it mounts; the second time a shape is seen it becomes a
//! detached template, and from then on each block of that shape is one
//! `cloneNode(true)`, walked to its nodes by `firstChild` and
//! `nextSibling`, with only the texts, styles and attribute values that
//! differ from the template's written.

use crate::ops::{self, Captured, Ops};
use rustc_hash::FxHashMap as HashMap;
use std::cell::RefCell;

/// What each word of an operation is: an id or number, or a string (two
/// words: its UTF-16 offset and length).
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Word,
    Str,
}
use Field::{Str as S, Word as W};

fn layout(code: u32) -> Option<&'static [Field]> {
    Some(match code {
        ops::CREATE
        | ops::CLASS
        | ops::APPEND
        | ops::CHECKED
        | ops::CLONE
        | ops::FIRST
        | ops::NEXT
        | ops::CLASS_ADD => &[W, W],
        ops::TEXT
        | ops::SET_TEXT
        | ops::STYLE
        | ops::VALUE
        | ops::REMOVE_ATTR
        | ops::CLASS_NAME => &[W, S],
        ops::ATTR => &[W, S, S],
        ops::INSERT | ops::HANDLER | ops::NUMBER => &[W, W, W],
        ops::REMOVE | ops::CLEAR => &[W],
        ops::RULE => &[S],
        ops::PROP => &[W, S, W],
        ops::IMAGE => &[W, W, W, W, W],
        _ => return None,
    })
}

/// A string of the captured text, by its UTF-16 offset and length.
struct Strings<'a> {
    text: &'a str,
    /// The byte and UTF-16 offsets read up to.
    byte: usize,
    utf16: u32,
}
impl<'a> Strings<'a> {
    /// The string at UTF-16 `at` of length `len`: strings are read in the
    /// order they were written, so this only moves forward.
    fn get(&mut self, at: u32, len: u32) -> &'a str {
        let ascii = self.text.is_ascii();
        let advance = |from: usize, units: u32, text: &str| -> usize {
            if ascii {
                return from + units as usize;
            }
            let mut n = 0;
            let mut end = from;
            for c in text[from..].chars() {
                if n >= units {
                    break;
                }
                n += c.len_utf16() as u32;
                end += c.len_utf8();
            }
            end
        };
        if at != self.utf16 {
            self.byte = advance(self.byte, at - self.utf16, self.text);
            self.utf16 = at;
        }
        let start = self.byte;
        self.byte = advance(start, len, self.text);
        self.utf16 += len;
        &self.text[start..self.byte]
    }
}

/// One shape's template.
struct Template {
    /// The detached template's root.
    master: u32,
    /// How to reach each node from the clone's root: (op, node, from), in
    /// local indices (creation order).
    walk: Vec<(u32, u32, u32)>,
    /// The template's texts, styles and attribute values, by slot.
    values: Vec<String>,
}

#[derive(Default)]
struct Entry {
    seen: u32,
    template: Option<Template>,
}

#[derive(Default)]
struct Templates {
    shapes: HashMap<Vec<u32>, Entry>,
    /// Attribute names, numbered for shapes.
    names: HashMap<String, u32>,
    /// Spare buffers for capturing and reading.
    spare: Vec<Captured>,
    parsed: Parsed,
}

thread_local! {
    static TEMPLATES: RefCell<Templates> = RefCell::default();
}

/// Shapes kept at most: templates live for the page.
const SHAPES: usize = 512;
/// Blocks of more nodes are replayed, not templated: a page, not a row.
const NODES: usize = 256;

/// Buffers to capture a block into.
pub(crate) fn spare() -> Captured {
    TEMPLATES
        .with(|t| t.borrow_mut().spare.pop())
        .unwrap_or(Captured {
            words: Vec::new(),
            text: String::new(),
        })
}

/// A captured block's slots: its texts, styles and attribute values (the
/// attribute's name a byte range of the captured text).
#[derive(Clone, Copy)]
enum Slot {
    Text,
    Style,
    Attr(u32, u32),
}

/// Write a captured block into the frame: stamped from its shape's template
/// when there is one, as it was captured otherwise. `ids` are its nodes in
/// creation order (the root first).
pub(crate) fn stamp(ops: &mut Ops, captured: Captured, ids: &[u32]) {
    TEMPLATES.with(|templates| {
        let mut templates = templates.borrow_mut();
        let templates = &mut *templates;
        let mut parsed = std::mem::take(&mut templates.parsed);
        match shape(&captured, ids, &mut templates.names, &mut parsed) {
            Some(()) => {
                let entry = if templates.shapes.contains_key(&parsed.key) {
                    templates.shapes.get_mut(&parsed.key)
                } else if templates.shapes.len() < SHAPES {
                    Some(templates.shapes.entry(parsed.key.clone()).or_default())
                } else {
                    None
                };
                match entry {
                    Some(entry) => {
                        entry.seen += 1;
                        if entry.template.is_none() && entry.seen >= 2 {
                            entry.template = make(ops, &captured, &parsed, ids.len());
                        }
                        match &entry.template {
                            Some(template) => clone(ops, template, &parsed, &captured, ids),
                            None => replay(ops, &captured),
                        }
                    }
                    None => replay(ops, &captured),
                }
            }
            None => replay(ops, &captured),
        }
        templates.parsed = parsed;
        let mut captured = captured;
        captured.words.clear();
        captured.text.clear();
        templates.spare.push(captured);
    });
}

/// A captured block read as a shape.
#[derive(Default)]
struct Parsed {
    key: Vec<u32>,
    /// Its slots: which node, what kind and its value's byte range in the
    /// captured text.
    values: Vec<(u32, Slot, u32, u32)>,
    /// Its handlers: (node, kind, handler), in global ids.
    handlers: Vec<[u32; 3]>,
    /// Parent and children, in local indices, for the walk.
    appends: Vec<(u32, u32)>,
}

fn shape(
    captured: &Captured,
    ids: &[u32],
    names: &mut HashMap<String, u32>,
    parsed: &mut Parsed,
) -> Option<()> {
    if ids.len() < 2 || ids.len() > NODES {
        return None;
    }
    let local = |id: u32| ids.iter().position(|&x| x == id).map(|i| i as u32);
    let text = captured.text.as_str();
    let mut strings = Strings {
        text,
        byte: 0,
        utf16: 0,
    };
    // A string's byte range in the captured text.
    let mut range = |at: u32, len: u32| {
        let s = strings.get(at, len);
        let start = s.as_ptr() as usize - text.as_ptr() as usize;
        (start as u32, (start + s.len()) as u32)
    };
    let words = &captured.words;
    parsed.key.clear();
    parsed.values.clear();
    parsed.handlers.clear();
    parsed.appends.clear();
    let mut i = 0;
    while i < words.len() {
        let code = words[i];
        let w = &words[i + 1..];
        match code {
            ops::CREATE | ops::CLASS | ops::CLASS_ADD => {
                parsed.key.extend([code, local(w[0])?, w[1]]);
                i += 3;
            }
            ops::APPEND => {
                let (p, c) = (local(w[0])?, local(w[1])?);
                parsed.key.extend([code, p, c]);
                parsed.appends.push((p, c));
                i += 3;
            }
            ops::TEXT | ops::STYLE => {
                let node = local(w[0])?;
                parsed.key.extend([code, node]);
                let (start, end) = range(w[1], w[2]);
                let slot = if code == ops::TEXT {
                    Slot::Text
                } else {
                    Slot::Style
                };
                parsed.values.push((w[0], slot, start, end));
                i += 4;
            }
            ops::ATTR => {
                let node = local(w[0])?;
                let (name_start, name_end) = range(w[1], w[2]);
                let (start, end) = range(w[3], w[4]);
                let name = &text[name_start as usize..name_end as usize];
                let number = match names.get(name) {
                    Some(number) => *number,
                    None => {
                        let number = names.len() as u32;
                        names.insert(name.to_owned(), number);
                        number
                    }
                };
                parsed.key.extend([code, node, number]);
                parsed
                    .values
                    .push((w[0], Slot::Attr(name_start, name_end), start, end));
                i += 6;
            }
            ops::HANDLER => {
                parsed.key.extend([code, local(w[0])?, w[1]]);
                parsed.handlers.push([w[0], w[1], w[2]]);
                i += 4;
            }
            _ => return None,
        }
    }
    Some(())
}

/// Build a template of `parsed`'s shape from `captured`, if its nodes form
/// one tree under its root.
fn make(ops: &mut Ops, captured: &Captured, parsed: &Parsed, n: usize) -> Option<Template> {
    let mut children: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut parented = vec![false; n];
    for &(p, c) in &parsed.appends {
        if parented[c as usize] || c == 0 {
            return None;
        }
        parented[c as usize] = true;
        children[p as usize].push(c);
    }
    let mut walk = Vec::with_capacity(n - 1);
    let mut stack = vec![0_u32];
    while let Some(node) = stack.pop() {
        let kids = &children[node as usize];
        for (j, &child) in kids.iter().enumerate() {
            if j == 0 {
                walk.push((ops::FIRST, child, node));
            } else {
                walk.push((ops::NEXT, child, kids[j - 1]));
            }
        }
        // Children in order: the first's subtree walked first.
        stack.extend(kids.iter().rev());
    }
    if walk.len() != n - 1 {
        return None;
    }
    // The template: the captured block replayed onto nodes of its own,
    // never freed, without its handlers.
    let fresh: Vec<u32> = (0..n).map(|_| ops.id()).collect();
    let ids: Vec<u32> = {
        // Captured ids in creation order: the order CREATE and TEXT met them.
        let mut ids = Vec::with_capacity(n);
        let mut i = 0;
        let w = &captured.words;
        while i < w.len() {
            let fields = layout(w[i])?;
            if matches!(w[i], ops::CREATE | ops::TEXT) {
                ids.push(w[i + 1]);
            }
            i += 1 + fields
                .iter()
                .map(|f| if *f == S { 2 } else { 1 })
                .sum::<usize>();
        }
        ids
    };
    if ids.len() != n {
        return None;
    }
    let map = |id: u32| ids.iter().position(|&x| x == id).map(|at| fresh[at]);
    replay_mapped(ops, captured, Some(&map))?;
    Some(Template {
        master: fresh[0],
        walk,
        values: parsed
            .values
            .iter()
            .map(|&(_, _, start, end)| captured.text[start as usize..end as usize].to_owned())
            .collect(),
    })
}

/// Stamp a block of `template`'s shape: clone, walk, then what differs.
fn clone(ops: &mut Ops, template: &Template, parsed: &Parsed, captured: &Captured, ids: &[u32]) {
    ops.op(&[ops::CLONE, ids[0], template.master]);
    for &(code, node, from) in &template.walk {
        ops.op(&[code, ids[node as usize], ids[from as usize]]);
    }
    let text = captured.text.as_str();
    for (&(node, slot, start, end), master) in parsed.values.iter().zip(&template.values) {
        let value = &text[start as usize..end as usize];
        if value == master {
            continue;
        }
        match slot {
            Slot::Text => ops.op_str(ops::SET_TEXT, node, value),
            Slot::Style => ops.op_str(ops::STYLE, node, value),
            Slot::Attr(name_start, name_end) => {
                ops.op_str(
                    ops::ATTR,
                    node,
                    &text[name_start as usize..name_end as usize],
                );
                ops.str(value);
            }
        }
    }
    for handler in &parsed.handlers {
        ops.op(&[ops::HANDLER, handler[0], handler[1], handler[2]]);
    }
}

/// Write `captured` into the frame as it was captured.
fn replay(ops: &mut Ops, captured: &Captured) {
    let _ = replay_mapped(ops, captured, None);
}

/// Write `captured` into the frame, its node ids mapped (handlers left out
/// when mapping onto a template), its strings moved after the frame's.
fn replay_mapped(
    ops: &mut Ops,
    captured: &Captured,
    map: Option<&dyn Fn(u32) -> Option<u32>>,
) -> Option<()> {
    let identity = map.is_none();
    ops.room();
    let (base, _) = ops.text_at(&captured.text);
    let w = &captured.words;
    let mut out = Vec::with_capacity(w.len());
    let mut i = 0;
    while i < w.len() {
        let code = w[i];
        let fields = layout(code)?;
        let skip = !identity && code == ops::HANDLER;
        let start = out.len();
        out.push(code);
        i += 1;
        for (k, field) in fields.iter().enumerate() {
            match field {
                Field::Str => {
                    out.push(w[i] + base);
                    out.push(w[i + 1]);
                    i += 2;
                }
                Field::Word => {
                    // Which words are node ids: the first of every op, and
                    // APPEND's and INSERT's others.
                    let is_node = k == 0 || matches!(code, ops::APPEND | ops::INSERT);
                    out.push(match map {
                        Some(map) if is_node => map(w[i])?,
                        _ => w[i],
                    });
                    i += 1;
                }
            }
        }
        if skip {
            out.truncate(start);
        }
    }
    // Raw: an early apply here would drop the text `base` points into.
    ops.op_raw(&out);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test as test;

    // The crate's unit tests apply DOM operations: in a browser.
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[test]
    fn a_replay_in_a_long_frame_keeps_its_text() {
        ops::with(|ops| {
            // A frame past the early-apply length: class names defined,
            // harmless to apply.
            while ops.words.len() < ops::EARLY {
                ops.define(&[ops::CLASS_NAME, 0], "x");
            }
            let id = ops.id();
            let captured = Captured {
                words: vec![ops::CREATE, id, 0, ops::ATTR, id, 0, 4, 4, 5],
                text: "nameValue".into(),
            };
            replay(ops, &captured);
            // The attribute's strings still lie where its words say.
            let n = ops.words.len();
            let (name, value) = (&ops.words[n - 4..n - 2], &ops.words[n - 2..]);
            let at = |w: &[u32]| &ops.text[w[0] as usize..(w[0] + w[1]) as usize];
            assert_eq!((at(name), at(value)), ("name", "Value"));
            ops.apply();
            ops.free(&[id]);
        });
    }
}
