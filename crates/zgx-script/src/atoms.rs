//! A text area's atoms in scripts (`atoms`, `on_atoms`, `el:insert_atom`):
//! ranges of its text that edit as one unit, each named by a string and
//! carrying a plain value. The editor keeps them (`zgx::text_edit::Atom`);
//! what a script adds rides in the atom's payload, so it moves with the atom
//! through edits and comes back with it on undo.
use crate::value::Value;
use std::sync::{Arc, atomic::AtomicU64, atomic::Ordering};
use zgx::{
    scene::Color,
    text_edit::{Atom, AtomPayload},
    widgets::EditorHandle,
};

/// What a script keeps with an atom: its name, its `data` as JSON, and the
/// `bg` it asked for, as it gave them (so a list read back is the list given).
pub(crate) struct ScriptAtom {
    pub key: String,
    pub data: Option<String>,
    pub bg: Option<String>,
}

/// A name for an atom made without one.
pub(crate) fn fresh_key() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!("atom-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// `value` as JSON, for what a script keeps with an atom: its plain values.
pub(crate) fn value_json(value: &Value) -> Option<String> {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Nil => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Num(n) if n.is_finite() => out.push_str(&n.to_string()),
            Value::Num(_) => out.push_str("null"),
            Value::Str(s) => out.push_str(&serde_json::to_string(&**s).unwrap_or_default()),
            Value::List(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            Value::Map(fields) => {
                out.push('{');
                for (i, (key, item)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(&**key).unwrap_or_default());
                    out.push(':');
                    write(item, out);
                }
                out.push('}');
            }
            _ => out.push_str("null"),
        }
    }
    if matches!(value, Value::Nil) {
        return None;
    }
    let mut out = String::new();
    write(value, &mut out);
    Some(out)
}

/// A payload for an atom named `key`.
pub(crate) fn payload(key: String, data: &Value, bg: Option<String>) -> Arc<dyn AtomPayload> {
    Arc::new(ScriptAtom {
        key,
        data: value_json(data),
        bg,
    })
}

/// What a script kept with `atom`, if it made it.
pub(crate) fn script_atom(atom: &Atom) -> Option<&ScriptAtom> {
    // (Through the `Arc`: it is itself `Any`, and would answer for the payload.)
    let payload: &dyn AtomPayload = &**atom.payload.as_ref()?;
    payload.as_any().downcast_ref::<ScriptAtom>()
}

/// Character offset `chars` (from 0) of `text` as a byte.
pub(crate) fn byte_of_char(text: &str, chars: usize) -> Option<usize> {
    if chars == 0 {
        return Some(0);
    }
    let mut count = 0;
    for (at, _) in text.char_indices() {
        if count == chars {
            return Some(at);
        }
        count += 1;
    }
    (count == chars).then_some(text.len())
}

/// Byte `byte` of `text` in characters.
pub(crate) fn char_of_byte(text: &str, byte: usize) -> usize {
    let mut at = byte.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    text[..at].chars().count()
}

/// One atom as a script reads it: `{ id, from, to, data?, style? }`, `from`
/// and `to` in characters from 0, half-open.
pub(crate) fn item(atom: &Atom, chars: (usize, usize)) -> Value {
    let mut fields: Vec<(&str, Value)> = vec![
        ("from", Value::Num(chars.0 as f64)),
        ("to", Value::Num(chars.1 as f64)),
    ];
    match script_atom(atom) {
        Some(script) => {
            fields.push(("id", Value::Str(script.key.as_str().into())));
            if let Some(data) = script
                .data
                .as_deref()
                .and_then(|json| crate::json::parse(json).ok())
            {
                fields.push(("data", data));
            }
            if let Some(bg) = &script.bg {
                fields.push((
                    "style",
                    Value::record([("bg", Value::Str(bg.as_str().into()))]),
                ));
            }
        }
        None => fields.push(("id", Value::Str(format!("atom-{}", atom.id).into()))),
    }
    Value::record(fields)
}

/// `item` with the `label` it had when it went (a removed atom's text).
pub(crate) fn with_label(item: Value, label: &str) -> Value {
    match item {
        Value::Map(fields) => {
            let mut all: Vec<(std::rc::Rc<str>, Value)> = fields.to_vec();
            all.push((std::rc::Rc::from("label"), Value::Str(label.into())));
            Value::record(all)
        }
        other => other,
    }
}

/// A list of atoms as a script reads it, from a text's characters (`char_of`).
pub(crate) fn list(atoms: &[Atom], char_of: impl Fn(usize) -> usize) -> Value {
    let items: Vec<Value> = atoms
        .iter()
        .map(|atom| item(atom, (char_of(atom.range.start), char_of(atom.range.end))))
        .collect();
    Value::List(items.into())
}

/// The atoms a script's list `value` asks for (`{ id?, from, to, data?,
/// style? }`, characters from 0), on `handle`'s text: one already there under
/// the same name and data and wash is kept as it is, so a list read back
/// from the editor changes nothing.
pub(crate) fn from_list(
    value: &Value,
    handle: &EditorHandle,
    color_of: &dyn Fn(&str) -> Option<Color>,
) -> Vec<Atom> {
    let Value::List(items) = value else {
        return Vec::new();
    };
    let text = handle.editor.borrow().text().into_owned();
    let current = handle.atoms();
    let mut out = Vec::with_capacity(items.len());
    for item in items.iter() {
        let number = |key: &str| match item.field(key) {
            Some(Value::Num(n)) if n.is_finite() && *n >= 0. => Some(*n as usize),
            _ => None,
        };
        let (Some(from), Some(to)) = (number("from"), number("to")) else {
            continue;
        };
        let (Some(start), Some(end)) = (byte_of_char(&text, from), byte_of_char(&text, to)) else {
            continue;
        };
        let key = match item.field("id") {
            Some(Value::Str(id)) => id.to_string(),
            _ => fresh_key(),
        };
        let data = item.field("data").cloned().unwrap_or(Value::Nil);
        let bg = match item.field("style").and_then(|style| style.field("bg")) {
            Some(Value::Str(bg)) => Some(bg.to_string()),
            _ => None,
        };
        let json = value_json(&data);
        let same = current.iter().find(|atom| {
            script_atom(atom).is_some_and(|s| s.key == key && s.data == json && s.bg == bg)
        });
        let atom = match same {
            Some(old) => Atom {
                range: start..end,
                ..old.clone()
            },
            None => {
                let id = current
                    .iter()
                    .find(|atom| script_atom(atom).is_some_and(|s| s.key == key))
                    .map_or_else(|| handle.new_atom_id(), |atom| atom.id);
                Atom {
                    id,
                    range: start..end,
                    wash: bg.as_deref().and_then(color_of),
                    payload: Some(payload(key, &data, bg)),
                }
            }
        };
        out.push(atom);
    }
    out
}
