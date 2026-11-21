//! A text area's atoms (chips) through the system clipboard
//! (`scripts/clipboard_atoms_smoke.py`): "a" holds "see @file.pdf now", the
//! mention an atom named `file-1`; a copy out of it puts the atoms beside
//! the text (`application/x-zgx-atoms`), and a paste into "b", in this
//! process or another one's, makes them atoms again. Each text area prints
//! atoms as they change, and why: `ATOMS b edit [4..13:file-1]`. The
//! window's title is "zgx clipboard atoms", and the first argument after it.
use std::sync::Arc;
use zgx::{
    compose::prelude::*,
    text_edit::{Atom, AtomPayload},
    widgets::{AtomCodec, AtomEvent},
};
use zgx_desktop::{Application, WindowOptions};

/// An atom's name, as this app keeps it.
fn name(atom: &Atom) -> Option<&str> {
    let payload: &dyn AtomPayload = &**atom.payload.as_ref()?;
    payload
        .as_any()
        .downcast_ref::<String>()
        .map(String::as_str)
}

/// What a text area's atoms print as: each one's bytes and name.
fn print(field: &str, event: &AtomEvent<'_>) {
    let atoms: Vec<String> = event
        .atoms
        .iter()
        .map(|atom| format!("{:?}:{}", atom.range, name(atom).unwrap_or("?")))
        .collect();
    println!("ATOMS {field} {} [{}]", event.reason, atoms.join(", "));
}

/// Its atoms' names on the clipboard beyond the app, and back: `{"id":…}`.
fn codec() -> AtomCodec {
    AtomCodec {
        encode: Box::new(|atom| Some(format!("{{\"id\":{:?}}}", name(atom)?))),
        decode: Box::new(|json, _| {
            // The text, then each atom's range and name, as `copied` writes
            // them (no escapes in this app's names).
            let text_at = json.find("\"text\":\"")? + 8;
            let text_end = text_at + json[text_at..].find("\",\"atoms\"")?;
            let text = json[text_at..text_end]
                .replace("\\\"", "\"")
                .replace("\\\\", "\\");
            let mut atoms = Vec::new();
            for part in json[text_end..].split("{\"from\":").skip(1) {
                let from: usize = part.split(',').next()?.parse().ok()?;
                let to: usize = part
                    .split("\"to\":")
                    .nth(1)?
                    .split(',')
                    .next()?
                    .parse()
                    .ok()?;
                let id = part
                    .split("\"id\":\"")
                    .nth(1)?
                    .split('"')
                    .next()?
                    .to_owned();
                let mut atom = Atom::new(0, 0..0);
                atom.payload = Some(Arc::new(id));
                atoms.push((from..to, atom));
            }
            Some((text, atoms))
        }),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let suffix = std::env::args().nth(1).unwrap_or_default();
    let title = format!("zgx clipboard atoms{suffix}");
    Application::new()
        .window(WindowOptions {
            title,
            width: 420.,
            height: 240.,
            ..Default::default()
        })
        .run(move |cx| {
            let a = cx.ui.signal(String::from("see @file.pdf now"));
            let b = cx.ui.signal(String::new());
            let field = |label: &'static str, value| {
                text_area(label, value)
                    .w(380.)
                    .h(60.)
                    .on_atoms(move |event| print(label, event))
                    .atom_codec(codec())
            };
            cx.render(
                column()
                    .p(20.)
                    .gap(20.)
                    .child(field("a", a).atoms_when(|editor| {
                        let mut atom = Atom::new(editor.new_atom_id(), 4..13);
                        atom.payload = Some(Arc::new(String::from("file-1")));
                        vec![atom]
                    }))
                    .child(field("b", b)),
            );
        })
}
