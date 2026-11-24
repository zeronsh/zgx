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
    collections::HashMap,
    rc::{Rc, Weak},
};
use zgx::{
    compose::__dom::DomAtoms,
    reactive::Runtime,
    scene::Color,
    text_edit::{Atom, TextEditor},
    widgets::{AtomCodec, AtomEvent, AtomListener, AtomStyle, EditorValue},
};

/// A piece of the text as shown: its own text, or an atom's chip.
#[derive(Clone, PartialEq)]
enum Piece {
    Text(String),
    Chip {
        id: u64,
        label: String,
        css: String,
        /// What a reader reads for it: its name, or its text.
        spoken: String,
        /// Its icon's look (a span before its label), and its well's (a span
        /// around it), if it has them.
        icon: Option<(String, Option<String>)>,
    },
}

/// A piece as shown, and its nodes: a text's, or a chip's span and its
/// label's text.
struct Shown {
    piece: Piece,
    node: u32,
    label: Option<u32>,
    /// A chip's icon's span, and its well's.
    icon: Option<u32>,
    well: Option<u32>,
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
    /// How its atoms go on the clipboard beyond the page.
    codec: Option<AtomCodec>,
}

thread_local! {
    /// The text areas with atoms mounted, by node: what a script edits
    /// through its element (`text_edits`).
    static EDITORS: RefCell<HashMap<u32, Weak<AtomEditor>>> = RefCell::default();
}

/// The text area with atoms mounted at `node`, as scripts edit it
/// (`NodeHandle::text_edits`): none for other nodes.
pub(crate) fn text_edits(node: u32) -> Option<Rc<dyn zgx::widgets::TextEdits>> {
    let editor = EDITORS.with(|editors| editors.borrow().get(&node)?.upgrade())?;
    Some(Rc::new(Edits(editor)))
}

/// A text area with atoms, as scripts edit it: each edit its model's, the
/// text shown and told as a person's edit is.
struct Edits(Rc<AtomEditor>);
impl zgx::widgets::TextEdits for Edits {
    fn text(&self) -> String {
        self.0.model.borrow().text().into_owned()
    }
    fn atoms(&self) -> Vec<Atom> {
        self.0.model.borrow().atoms().to_vec()
    }
    fn insert_atom(
        &self,
        label: &str,
        range: Option<std::ops::Range<usize>>,
        wash: Option<Color>,
        payload: Option<std::sync::Arc<dyn zgx::text_edit::AtomPayload>>,
    ) -> Option<u64> {
        // At the caret: the page's, which moves without an edit (a click).
        if range.is_none()
            && let Some(at) = crate::js::zgx_edit_selection(self.0.root)
            && let [start, end] = *at
        {
            let mut model = self.0.model.borrow_mut();
            let (start, end) = {
                let text = model.text();
                (
                    floor_char(&text, start as usize),
                    floor_char(&text, end as usize),
                )
            };
            model.set_selection(start, end);
        }
        let id = self
            .0
            .model
            .borrow_mut()
            .insert_atom(label, range, wash, payload)?;
        self.0.changed(true);
        Some(id)
    }
    fn set_atom_name(&self, id: u64, name: Option<std::sync::Arc<str>>) -> bool {
        let found = self.0.model.borrow_mut().set_atom_name(id, name);
        if found {
            self.0.render(false, false);
        }
        found
    }
    fn undo(&self) -> bool {
        let done = self.0.model.borrow_mut().undo();
        self.0.changed(done);
        done
    }
    fn redo(&self) -> bool {
        let done = self.0.model.borrow_mut().redo();
        self.0.changed(done);
        done
    }
    fn can_undo(&self) -> bool {
        self.0.model.borrow().can_undo()
    }
    fn can_redo(&self) -> bool {
        self.0.model.borrow().can_redo()
    }
}

/// A text area's own options, beyond its atoms.
pub(crate) struct Field {
    pub(crate) label: String,
    pub(crate) value: EditorValue,
    pub(crate) multiline: bool,
    pub(crate) placeholder: Option<Box<dyn FnMut() -> String>>,
}

/// `bytes` in base64 (a data URL's).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
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
            .flat_map(|shown| {
                std::iter::once(shown.node)
                    .chain(shown.label)
                    .chain(shown.icon)
                    .chain(shown.well)
            })
            .collect();
        ops::with(|ops| ops.free(&ids));
        let root = self.root;
        // (Its slot may be another's by now: a node mounted again.)
        let _ = EDITORS.try_with(|editors| {
            let mut editors = editors.borrow_mut();
            if editors
                .get(&root)
                .is_some_and(|weak| weak.strong_count() == 0)
            {
                editors.remove(&root);
            }
        });
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
            let label = text[atom.range.clone()].to_owned();
            pieces.push(Piece::Chip {
                id: atom.id,
                spoken: atom.name.as_deref().unwrap_or(&label).to_owned(),
                label,
                css: self.chip_css(atom),
                icon: self.icon_css(atom),
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
        let own = atom.style.as_deref();
        let base = self.style.as_ref();
        // Its wash's corners and the room inside it, as the desktop's chip;
        // selected whole, as the model selects it (a drag into it, a double
        // click on a word of it).
        let radius = own.and_then(|s| s.radius).or(base.and_then(|s| s.radius));
        let padding = own.and_then(|s| s.padding).or(base.and_then(|s| s.padding));
        let mut css = format!(
            "border-radius:{}px;user-select:all;-webkit-user-select:all;",
            radius.unwrap_or(4.)
        );
        if let Some([left, right, y]) = padding {
            css.push_str(&format!("padding:{y}px {right}px {y}px {left}px;"));
        }
        if let Some(wash) = atom.wash.or(self.wash) {
            css.push_str(&format!("background:{};", css_color(wash)));
        }
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
    /// How atom `atom`'s icon shows (its own over the text area's), as the
    /// style of a span before its label: the picture at its size, its gap
    /// after it, a tinted one (or one in the text's colour) as a mask over
    /// that colour, and its well's background, padding and corners. None
    /// for an atom without one. The span holds no text: the chip's text is
    /// its label alone, as the editor's offsets count it.
    fn icon_css(&self, atom: &Atom) -> Option<(String, Option<String>)> {
        let own = atom.style.as_deref().and_then(|s| s.icon.as_ref());
        let icon = own.or(self.style.as_ref().and_then(|s| s.icon.as_ref()))?;
        let url = format!(
            "url(\"data:image/svg+xml;base64,{}\")",
            base64(icon.svg.bytes())
        );
        let size = icon.size.max(0.);
        let gap = format!("margin-right:{}px;", icon.gap.max(0.));
        let mut css =
            format!("display:inline-block;width:{size}px;height:{size}px;vertical-align:-0.15em;");
        let paint = match icon.svg.tint() {
            Some(tint) => Some(css_color(tint)),
            None if icon.svg.is_current_color() => Some("currentColor".to_owned()),
            None => None,
        };
        match paint {
            Some(paint) => css.push_str(&format!(
                "background-color:{paint};-webkit-mask:{url} center/contain no-repeat;\
                 mask:{url} center/contain no-repeat;"
            )),
            None => css.push_str(&format!("background:{url} center/contain no-repeat;")),
        }
        // Its well: a span of its own around it, so a mask clips the picture
        // alone; the gap after the well.
        let well = icon.bg.map(|well| {
            format!(
                "display:inline-block;line-height:0;vertical-align:-0.15em;padding:{}px;\
                 border-radius:{}px;background:{};{gap}",
                icon.pad.max(0.),
                icon.radius.max(0.),
                css_color(well)
            )
        });
        if well.is_none() {
            css.push_str(&gap);
        }
        Some((css, well))
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
                    icon: None,
                    well: None,
                }
            }
            Piece::Chip {
                id,
                label,
                css,
                spoken,
                icon,
            } => {
                let node = ops.id();
                ops.op(&[ops::CREATE, node, Tag::Span as u32]);
                ops.op_str(ops::ATTR, node, "contenteditable");
                ops.str("false");
                // One unit to a reader, read by its name (or its text).
                ops.op_str(ops::ATTR, node, "role");
                ops.str("img");
                ops.op_str(ops::ATTR, node, "aria-label");
                ops.str(spoken);
                ops.op_str(ops::ATTR, node, "data-atom");
                ops.str(&id.to_string());
                ops.op_str(ops::STYLE, node, css);
                // Its icon, before its label.
                let (mut icon_node, mut well_node) = (None, None);
                if let Some((look, well)) = icon {
                    let span = ops.id();
                    ops.op(&[ops::CREATE, span, Tag::Span as u32]);
                    ops.op_str(ops::STYLE, span, look);
                    let outer = match well {
                        Some(well) => {
                            let around = ops.id();
                            ops.op(&[ops::CREATE, around, Tag::Span as u32]);
                            ops.op_str(ops::STYLE, around, well);
                            ops.op(&[ops::APPEND, around, span]);
                            well_node = Some(around);
                            around
                        }
                        None => span,
                    };
                    ops.op_str(ops::ATTR, outer, "aria-hidden");
                    ops.str("true");
                    ops.op(&[ops::APPEND, node, outer]);
                    icon_node = Some(span);
                }
                let text = ops.id();
                ops.op_str(ops::TEXT, text, label);
                ops.op(&[ops::APPEND, node, text]);
                Shown {
                    piece: piece.clone(),
                    node,
                    label: Some(text),
                    icon: icon_node,
                    well: well_node,
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
                .flat_map(|s| {
                    std::iter::once(s.node)
                        .chain(s.label)
                        .chain(s.icon)
                        .chain(s.well)
                })
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
    /// A copy or a cut of `start..end` (bytes, the browser's selection): its
    /// text, and its atoms beside it, handed to the page's copy
    /// (`zgx_copy_atoms`), the model's selection widened as it widens it.
    fn copy(&self, start: usize, end: usize) {
        let (text, json) = {
            let mut model = self.model.borrow_mut();
            let (start, end) = {
                let text = model.text();
                (floor_char(&text, start), floor_char(&text, end))
            };
            model.set_selection(start, end);
            let range = model.selection().range();
            let text = model.selected_text().into_owned();
            let json =
                zgx::widgets::copied_atoms(&text, range.start, model.atoms(), self.codec.as_ref());
            (text, json)
        };
        crate::js::zgx_copy_atoms(&text, json.as_deref());
    }
    /// After an edit made other than by a `beforeinput` (a script's): its
    /// value set where its text `changed`, shown again, its caret put back,
    /// and its listener told.
    fn changed(&self, changed: bool) {
        if changed {
            let text = self.model.borrow().text().into_owned();
            self.value.set(text);
        }
        self.render(false, true);
        if changed {
            self.tell();
        }
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
        if kind == "copy" {
            return self.copy(start, end);
        }
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
                // A paste with a copy's atoms beside its text: `text\0json`.
                "insertFromPasteAtoms" => {
                    let (text, json) = data.split_once('\0').unwrap_or((&data, ""));
                    let atoms =
                        zgx::widgets::pasted_atoms(text, json, model.atoms(), self.codec.as_ref());
                    model.insert_pasted_atoms(text, atoms)
                }
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
        codec,
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
        codec,
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
    EDITORS.with(|editors| editors.borrow_mut().insert(id, Rc::downgrade(&editor)));
    block.hold(editor);
}
