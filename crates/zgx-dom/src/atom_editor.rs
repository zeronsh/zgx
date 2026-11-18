//! A DOM text area with atoms (`ElementKind::Editor`'s `atoms`): a
//! contenteditable whose text is a model's (`TextEditor`), as the native
//! text area's is. The browser edits nothing itself: each `beforeinput`
//! (typing, deleting, a paste, undo) is prevented and done to the model,
//! and the text is drawn again from it, only the pieces that changed (its
//! text between atoms, its atoms as chips), the caret put back from the
//! model before the browser paints. An input method's composing goes into
//! the text as the browser shows it and into the model as it is committed.
//!
//! A text area without atoms is the browser's own field (`mount`), as it
//! was.
use crate::{
    mount::{Block, Event},
    ops::{self, Ops, Tag},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use zgx::{
    compose::__dom::DomAtoms,
    reactive::Runtime,
    scene::Color,
    text_edit::{Atom, TextEditor},
    widgets::{AtomEvent, AtomListener, AtomStyle, EditorValue},
};

/// A piece of the text as shown: its own text, or an atom's chip.
#[derive(Clone, PartialEq)]
enum Piece {
    Text(String),
    Chip { id: u64, label: String, css: String },
}

/// A piece as shown, and its nodes: a text's, or a chip's span and its
/// label's text.
struct Shown {
    piece: Piece,
    node: u32,
    label: Option<u32>,
}

/// A text area with atoms, mounted.
struct AtomEditor {
    root: u32,
    multiline: bool,
    model: RefCell<TextEditor>,
    value: EditorValue,
    on_atoms: RefCell<Option<AtomListener>>,
    wash: Option<Color>,
    style: Option<AtomStyle>,
    shown: RefCell<Vec<Shown>>,
    /// An input method composing in it: its text is the browser's until it
    /// commits, so nothing is shown again over it; `stale`: something was
    /// asked meanwhile, shown whole once it commits.
    composing: Cell<bool>,
    stale: Cell<bool>,
}

/// A text area's own options, beyond its atoms.
pub(crate) struct Field {
    pub(crate) label: String,
    pub(crate) value: EditorValue,
    pub(crate) multiline: bool,
    pub(crate) placeholder: Option<Box<dyn FnMut() -> String>>,
}

/// `at` in `text`, on a character's start (the browser's offsets and the
/// model's text could disagree for a moment).
fn floor_char(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

impl Drop for AtomEditor {
    fn drop(&mut self) {
        let ids: Vec<u32> = self
            .shown
            .get_mut()
            .iter()
            .flat_map(|shown| std::iter::once(shown.node).chain(shown.label))
            .collect();
        ops::with(|ops| ops.free(&ids));
    }
}

/// `color` as CSS.
fn css_color(color: Color) -> String {
    let Color(r, g, b, a) = color;
    format!("rgba({r},{g},{b},{:.3})", f32::from(a) / 255.)
}

impl AtomEditor {
    /// Its text and atoms as the pieces to show.
    fn pieces(&self) -> Vec<Piece> {
        let model = self.model.borrow();
        let text = model.text();
        let mut pieces = Vec::new();
        let mut at = 0;
        for atom in model.atoms() {
            if atom.range.start > at {
                pieces.push(Piece::Text(text[at..atom.range.start].to_owned()));
            }
            pieces.push(Piece::Chip {
                id: atom.id,
                label: text[atom.range.clone()].to_owned(),
                css: self.chip_css(atom),
            });
            at = atom.range.end;
        }
        if at < text.len() {
            pieces.push(Piece::Text(text[at..].to_owned()));
        }
        pieces
    }
    /// How atom `atom` shows: its wash, and its look beyond it (its own over
    /// the text area's).
    fn chip_css(&self, atom: &Atom) -> String {
        let mut css = String::from("border-radius:4px;");
        if let Some(wash) = atom.wash.or(self.wash) {
            css.push_str(&format!("background:{};", css_color(wash)));
        }
        let own = atom.style.as_deref();
        let base = self.style.as_ref();
        let color = own.and_then(|s| s.color).or(base.and_then(|s| s.color));
        let weight = own.and_then(|s| s.weight).or(base.and_then(|s| s.weight));
        let underline = own
            .and_then(|s| s.underline)
            .or(base.and_then(|s| s.underline));
        if let Some(color) = color {
            css.push_str(&format!("color:{};", css_color(color)));
        }
        if let Some(weight) = weight {
            css.push_str(&format!("font-weight:{weight};"));
        }
        if let Some(line) = underline {
            css.push_str("text-decoration:underline;");
            if let Some(color) = line.color {
                css.push_str(&format!("text-decoration-color:{};", css_color(color)));
            }
            if let Some(offset) = line.offset {
                css.push_str(&format!("text-underline-offset:{offset}px;"));
            }
        }
        css
    }
    /// A node showing `piece`, made.
    fn make(&self, ops: &mut Ops, piece: &Piece) -> Shown {
        match piece {
            Piece::Text(text) => {
                let node = ops.id();
                ops.op_str(ops::TEXT, node, text);
                Shown {
                    piece: piece.clone(),
                    node,
                    label: None,
                }
            }
            Piece::Chip { id, label, css } => {
                let node = ops.id();
                ops.op(&[ops::CREATE, node, Tag::Span as u32]);
                ops.op_str(ops::ATTR, node, "contenteditable");
                ops.str("false");
                ops.op_str(ops::ATTR, node, "data-atom");
                ops.str(&id.to_string());
                ops.op_str(ops::STYLE, node, css);
                let text = ops.id();
                ops.op_str(ops::TEXT, text, label);
                ops.op(&[ops::APPEND, node, text]);
                Shown {
                    piece: piece.clone(),
                    node,
                    label: Some(text),
                }
            }
        }
    }
    /// Show its text and atoms as the model has them: the pieces between
    /// the first and last that changed made again, the rest left (`whole`:
    /// every piece, as after an input method wrote into the text itself).
    /// Then its caret, from the model, where `select`.
    fn render(&self, whole: bool, select: bool) {
        // Over an input method's text: once it commits.
        if self.composing.get() {
            self.stale.set(true);
            return;
        }
        let whole = whole || self.stale.replace(false);
        let pieces = self.pieces();
        let mut shown = self.shown.borrow_mut();
        let (mut start, mut old_end, mut new_end) = (0, shown.len(), pieces.len());
        if !whole {
            while start < old_end && start < new_end && shown[start].piece == pieces[start] {
                start += 1;
            }
            while old_end > start
                && new_end > start
                && shown[old_end - 1].piece == pieces[new_end - 1]
            {
                old_end -= 1;
                new_end -= 1;
            }
        }
        ops::with(|ops| {
            let before = shown.get(old_end).map(|s| s.node);
            let gone: Vec<u32> = shown[start..old_end]
                .iter()
                .flat_map(|s| std::iter::once(s.node).chain(s.label))
                .collect();
            for s in &shown[start..old_end] {
                ops.op(&[ops::REMOVE, s.node]);
            }
            if whole {
                ops.op(&[ops::CLEAR, self.root]);
            }
            ops.free(&gone);
            let made: Vec<Shown> = pieces[start..new_end]
                .iter()
                .map(|piece| {
                    let made = self.make(ops, piece);
                    match before {
                        Some(before) => ops.op(&[ops::INSERT, self.root, made.node, before]),
                        None => ops.op(&[ops::APPEND, self.root, made.node]),
                    }
                    made
                })
                .collect();
            shown.splice(start..old_end, made);
        });
        drop(shown);
        if select {
            self.select();
        }
    }
    /// Its caret and selection put where the model has them, in the text's
    /// UTF-16 units (as the browser counts).
    fn select(&self) {
        let model = self.model.borrow();
        let text = model.text();
        let selection = model.selection();
        let units = |at: usize| text[..at.min(text.len())].encode_utf16().count() as u32;
        let (anchor, focus) = (units(selection.anchor), units(selection.focus));
        ops::with(|ops| ops.op(&[ops::SELECT, self.root, anchor, focus]));
    }
    /// Tell its listener what happened to its atoms, if anything did.
    fn tell(&self) {
        let (changes, atoms, reason, text) = {
            let mut model = self.model.borrow_mut();
            if !model.atom_changes_pending() {
                return;
            }
            (
                model.take_atom_changes(),
                model.atoms().to_vec(),
                model.atom_reason(),
                model.text().into_owned(),
            )
        };
        let listener = self.on_atoms.borrow_mut().take();
        if let Some(mut listener) = listener {
            listener(&AtomEvent::new(&changes, &atoms, reason, &text));
            let mut slot = self.on_atoms.borrow_mut();
            if slot.is_none() {
                *slot = Some(listener);
            }
        }
    }
    /// Do `kind` (a `beforeinput`'s input type, or `compositionend`) with
    /// `data` to the model, over `start..end` (the selection then, in
    /// bytes): its text set as the model's, shown again, its caret put back.
    fn edit(&self, kind: &str, data: &str, start: usize, end: usize) {
        match kind {
            // An input method begins: its text is the browser's until it ends.
            "compositionstart" => {
                self.composing.set(true);
                return;
            }
            // The browser wrote into the text itself (an edit it wouldn't let
            // be prevented): shown again whole from the model.
            "render" => {
                self.render(true, true);
                return;
            }
            "compositionend" => self.composing.set(false),
            _ => {}
        }
        let data = zgx::widgets::normalize_text(data, self.multiline);
        let changed = {
            let mut model = self.model.borrow_mut();
            let (start, end) = {
                let text = model.text();
                (floor_char(&text, start), floor_char(&text, end))
            };
            model.set_selection(start, end);
            match kind {
                "insertText" | "insertReplacementText" | "compositionend" => model.insert(&data),
                "insertFromPaste" | "insertFromDrop" => model.insert_pasted(&data),
                "insertParagraph" | "insertLineBreak" if self.multiline => model.insert("\n"),
                "deleteContentBackward" => model.delete_backward(false),
                "deleteWordBackward" => model.delete_backward(true),
                "deleteContentForward" => model.delete_next(false),
                "deleteWordForward" => model.delete_next(true),
                "deleteSoftLineBackward" | "deleteHardLineBackward" => model.delete_to_line_start(),
                "deleteSoftLineForward" | "deleteHardLineForward" => model.delete_to_line_end(),
                "insertTranspose" => model.transpose(),
                "deleteByCut" | "deleteByDrag" | "deleteContent" => model.insert(""),
                "historyUndo" => model.undo(),
                "historyRedo" => model.redo(),
                _ => false,
            }
        };
        if changed {
            let text = self.model.borrow().text().into_owned();
            self.value.set(text);
        }
        // After an input method, its text is the browser's: shown again whole.
        self.render(kind == "compositionend", true);
        if changed {
            self.tell();
        }
    }
}

/// Mount a text area with atoms at `id`: a contenteditable over a model of
/// `value`'s text, its atoms as `atoms` says.
pub(crate) fn mount(block: &mut Block, id: u32, runtime: &Runtime, field: Field, atoms: DomAtoms) {
    let Field {
        label,
        value,
        multiline,
        placeholder,
    } = field;
    ops::with(|ops| {
        ops.op_str(ops::ATTR, id, "contenteditable");
        ops.str("true");
        ops.op_str(ops::ATTR, id, "role");
        ops.str("textbox");
        ops.op_str(ops::ATTR, id, "aria-label");
        ops.str(&label);
        if multiline {
            ops.op_str(ops::ATTR, id, "aria-multiline");
            ops.str("true");
        }
    });
    let DomAtoms {
        when,
        on_atoms,
        wash,
        style,
    } = atoms;
    let editor = Rc::new(AtomEditor {
        root: id,
        multiline,
        model: RefCell::new(TextEditor::new(value.get())),
        value: value.clone(),
        on_atoms: RefCell::new(on_atoms),
        wash,
        style,
        shown: RefCell::default(),
        composing: Cell::new(false),
        stale: Cell::new(false),
    });
    // What it shows while empty, drawn by its style (`.ze[data-placeholder]`).
    if let Some(mut placeholder) = placeholder {
        block.effect(id, runtime, move || {
            let text = placeholder();
            ops::with(|ops| {
                ops.op_str(ops::ATTR, id, "data-placeholder");
                ops.str(&text);
            });
        });
    }
    // Its atoms as what they follow gives (`View::atoms_when`): first, so its
    // text shows with them from the start.
    if let Some(mut when) = when {
        let editor = editor.clone();
        block.effect(id, runtime, move || {
            let next = when(&mut editor.model.borrow_mut());
            let changed = editor.model.borrow_mut().set_atoms(next);
            if changed {
                editor.render(false, true);
                editor.tell();
            }
        });
    }
    // Its text as its value has it, set from outside (a draft restored).
    {
        let editor = editor.clone();
        block.effect(id, runtime, move || {
            let text = editor.value.get();
            let changed = {
                let mut model = editor.model.borrow_mut();
                *model.text() != *text && model.set_text(text)
            };
            editor.render(false, true);
            if changed {
                editor.tell();
            }
        });
    }
    {
        let editor = editor.clone();
        block.on(id, ops::ON_EDIT, move |event| {
            if let Event::Edit {
                kind,
                data,
                start,
                end,
            } = event
            {
                editor.edit(&kind, &data, start, end);
            }
        });
    }
    block.hold(editor);
}
