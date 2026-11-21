//! A text area's atoms on the clipboard: a copy or a cut of text with atoms
//! writes them as a type of zgx's own (`ATOMS_TYPE`) beside the text, and a
//! paste into a text area that takes atoms makes them again over the pasted
//! text. Within the app (a draft to another, one text area to the next) the
//! atoms come back as they were copied, payload, wash and look and all;
//! from elsewhere (another window of another app, a page), through the
//! target's `AtomCodec`, which reads what the source's wrote. A plain-text
//! target takes the text alone, the atoms' labels.
//!
//! What the type holds: `{"v":1,"text":"…","atoms":[{"from":0,"to":3,
//! "atom":{…}|null}]}`, `from` and `to` in characters of `text` (from 0,
//! half-open), `atom` what the source's codec wrote of it.
use crate::text_edit::Atom;
use std::{cell::RefCell, ops::Range, rc::Rc};

/// The clipboard type a text area's atoms go on as.
#[doc(hidden)]
pub const ATOMS_TYPE: &str = "application/x-zgx-atoms";

/// Atoms over a text, each with its range of it (`AtomCodec::decode`'s, in
/// characters; the app's own copy's, in bytes).
#[doc(hidden)]
pub type PlacedAtoms = Vec<(Range<usize>, Atom)>;
/// What an `AtomCodec` writes of an atom: a JSON object, or none.
type Encode = Box<dyn Fn(&Atom) -> Option<String>>;
/// What an `AtomCodec` reads of the clipboard's JSON, given the target's
/// atoms: the text and its atoms, or none.
type Decode = Box<dyn Fn(&str, &[Atom]) -> Option<(String, PlacedAtoms)>>;

/// How a text area's atoms go on the clipboard beyond the app, and come
/// back from it: a script's names, data and looks.
#[doc(hidden)]
pub struct AtomCodec {
    /// What goes on the clipboard of `atom` (a JSON object), or none.
    pub encode: Encode,
    /// The text and atoms in `ATOMS_TYPE`'s `json`, each atom's range in
    /// characters of the text, its id and range set when it is made (the
    /// target's `atoms`, to keep names apart); none if it reads none.
    pub decode: Decode,
}

thread_local! {
    /// The atoms the app last put on the clipboard, as they were (their
    /// ranges in bytes of the text), and what it wrote of them: a paste of
    /// that same text takes them back whole.
    static LAST: RefCell<Option<(String, PlacedAtoms)>> = const { RefCell::new(None) };
}

/// `text` as a JSON string.
fn json_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// What goes on the clipboard for `text` (the selection, from byte `start`
/// of the editor's text) and the `atoms` in it, none if it has none.
#[doc(hidden)]
pub fn copied(
    text: &str,
    start: usize,
    atoms: &[Atom],
    codec: Option<&AtomCodec>,
) -> Option<String> {
    let end = start + text.len();
    let inside: PlacedAtoms = atoms
        .iter()
        .filter(|atom| atom.range.start >= start && atom.range.end <= end)
        .map(|atom| {
            (
                atom.range.start - start..atom.range.end - start,
                atom.clone(),
            )
        })
        .collect();
    if inside.is_empty() {
        return None;
    }
    let chars = |byte: usize| text[..byte].chars().count();
    let mut json = String::from("{\"v\":1,\"text\":");
    json_string(text, &mut json);
    json.push_str(",\"atoms\":[");
    for (i, (range, atom)) in inside.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        json.push_str(&format!(
            "{{\"from\":{},\"to\":{},\"atom\":",
            chars(range.start),
            chars(range.end)
        ));
        match codec.and_then(|codec| (codec.encode)(atom)) {
            Some(own) => json.push_str(&own),
            None => json.push_str("null"),
        }
        json.push('}');
    }
    json.push_str("]}");
    LAST.with(|last| *last.borrow_mut() = Some((json.clone(), inside)));
    Some(json)
}

/// The atoms to make over `text` (what is pasted, as the target takes it),
/// from the clipboard's `json`: the app's own copy as it was, else what
/// the target's `codec` reads. Ranges in bytes of `text`. None where the
/// clipboard's text is not `text` (another app wrote it since, or the
/// target changed it as it took it).
#[doc(hidden)]
pub fn pasted(text: &str, json: &str, existing: &[Atom], codec: Option<&AtomCodec>) -> PlacedAtoms {
    let own = LAST.with(|last| {
        last.borrow()
            .as_ref()
            .filter(|(written, _)| written == json)
            .map(|(_, atoms)| atoms.clone())
    });
    if let Some(atoms) = own {
        let fits = atoms.iter().all(|(range, _)| {
            range.end <= text.len()
                && text.is_char_boundary(range.start)
                && text.is_char_boundary(range.end)
        });
        // (Its text is checked as the codec's is: a copy within the app
        // writes what it read, so the same JSON is the same text.)
        if fits && json_text_is(json, text) {
            return atoms;
        }
        return Vec::new();
    }
    let Some((written, atoms)) = codec.and_then(|codec| (codec.decode)(json, existing)) else {
        return Vec::new();
    };
    if written != text {
        return Vec::new();
    }
    let byte = |chars: usize| {
        text.char_indices()
            .nth(chars)
            .map_or(text.len(), |(at, _)| at)
    };
    atoms
        .into_iter()
        .filter(|(range, _)| range.start < range.end && range.end <= text.chars().count())
        .map(|(range, atom)| (byte(range.start)..byte(range.end), atom))
        .collect()
}

/// Whether `json` (as `copied` writes it) holds `text` as its text.
fn json_text_is(json: &str, text: &str) -> bool {
    let mut written = String::from("{\"v\":1,\"text\":");
    json_string(text, &mut written);
    written.push(',');
    json.starts_with(&written)
}

/// The app's own copy, forgotten (tests: a paste as from another app).
#[doc(hidden)]
pub fn forget_copied_atoms() {
    LAST.with(|last| *last.borrow_mut() = None);
}

/// A shared codec.
pub(crate) type Codec = Rc<AtomCodec>;
