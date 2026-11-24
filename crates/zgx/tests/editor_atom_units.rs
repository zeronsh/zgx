//! A text area's atoms as a chat composer's mentions need them: one unit to
//! the caret, a selection, Backspace and Delete; carried whole by copy, cut
//! and paste, between text areas; one undo step to make or take; and an
//! input method's text beside one leaving it whole. Every desktop host
//! edits through these paths (keys, `Ui::edit_command`, an input method's
//! composition and replacements).
use std::cell::RefCell;
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    platform::{FilePicker, Pending, Platform},
    text_edit::Atom,
    widgets::{ATOMS_TYPE, EditCommand, EditorHandle, Ui},
};

/// A clipboard in memory, with the app's own types beside the text.
#[derive(Default)]
struct Clipboard {
    text: RefCell<Option<String>>,
    types: RefCell<Vec<(String, String)>>,
}
impl Platform for Clipboard {
    fn clipboard_text(&self) -> Option<String> {
        self.text.borrow().clone()
    }
    fn set_clipboard_text(&self, text: String) -> bool {
        *self.text.borrow_mut() = Some(text);
        self.types.borrow_mut().clear();
        true
    }
    fn set_clipboard_rich_text(&self, text: String, _: String) -> bool {
        self.set_clipboard_text(text)
    }
    fn set_clipboard_typed(
        &self,
        text: String,
        _: Option<String>,
        types: &[(String, String)],
    ) -> bool {
        *self.text.borrow_mut() = Some(text);
        *self.types.borrow_mut() = types.to_vec();
        true
    }
    fn clipboard_types(&self) -> Vec<String> {
        self.types.borrow().iter().map(|(t, _)| t.clone()).collect()
    }
    fn clipboard_typed(&self, mime: &str) -> Option<String> {
        let types = self.types.borrow();
        types
            .iter()
            .find(|(t, _)| t == mime)
            .map(|(_, v)| v.clone())
    }
    fn open_files(&self, _: FilePicker) -> Pending<Result<Vec<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn save_file(&self, _: FilePicker) -> Pending<Result<Option<std::path::PathBuf>, String>> {
        Box::pin(async { Ok(None) })
    }
}

/// Two text areas that take atoms, "a" ("see  now", focused) and "b"
/// (empty), and a plain one, "c".
fn all_fields() -> (Ui, EditorHandle, EditorHandle, EditorHandle) {
    let mut ui = Ui::new(400., 300.);
    let a = ui.signal(String::from("see  now"));
    let b = ui.signal(String::new());
    let c = ui.signal(String::new());
    let view = ui.mount(
        column()
            .size(400., 300.)
            .child(text_area("A", a).id("a").w(300.).h(80.).on_atoms(|_| {}))
            .child(text_area("B", b).id("b").w(300.).h(80.).on_atoms(|_| {}))
            .child(text_area("C", c).id("c").w(300.).h(80.)),
    );
    ui.prepare_frame();
    let [a, b, c] = ["a", "b", "c"].map(|id| view.find(id).unwrap());
    ui.input.focus(&ui.scene, Some(a));
    let [a, b, c] = [a, b, c].map(|node| ui.editor(node).unwrap());
    (ui, a, b, c)
}

/// `all_fields` without the plain one.
fn fields() -> (Ui, EditorHandle, EditorHandle) {
    let (ui, a, b, _) = all_fields();
    (ui, a, b)
}

/// "see @file.pdf now", the atom on "@file.pdf" (4..13).
fn with_atom() -> (Ui, EditorHandle, EditorHandle, u64) {
    let (ui, a, b) = fields();
    a.set_selection(4, 4);
    let id = a.insert_atom("@file.pdf", None, None, None).unwrap();
    ui.prepare_frame();
    (ui, a, b, id)
}

fn key(ui: &mut Ui, key: Key, modifiers: Modifiers) {
    ui.dispatch(InputEvent::KeyDown {
        key,
        modifiers,
        repeat: false,
    });
    ui.prepare_frame();
}

fn text(editor: &EditorHandle) -> String {
    editor.editor.borrow().text().into_owned()
}

#[test]
fn a_selection_over_part_of_an_atom_takes_all_of_it() {
    let (_ui, a, _, _) = with_atom();
    // From before it into its first half ("se|e @fi|le.pdf").
    a.set_selection(2, 6);
    assert_eq!(a.selection(), (2, 13), "the selection takes the whole atom");
    // Its last word ("pdf"), as a double click selects it.
    a.set_selection(10, 13);
    assert_eq!(a.selection(), (4, 13), "a word inside it selects all of it");
    // A caret alone goes to the nearer edge.
    a.set_selection(6, 6);
    assert_eq!(a.selection(), (4, 4));
}

#[test]
fn shift_arrows_select_an_atom_whole() {
    let (mut ui, a, _, _) = with_atom();
    a.set_selection(13, 13);
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    key(&mut ui, Key::ArrowLeft, shift);
    assert_eq!(a.selection(), (13, 4));
    key(&mut ui, Key::ArrowRight, shift);
    key(&mut ui, Key::ArrowRight, shift);
    assert_eq!(a.selection(), (13, 14));
}

#[test]
fn copy_and_paste_carry_an_atom_between_text_areas() {
    let (ui, a, b, _) = with_atom();
    let clipboard = Clipboard::default();
    a.set_selection(0, 17);
    assert!(ui.edit_command(EditCommand::Copy, &clipboard));
    assert_eq!(
        clipboard.clipboard_text().as_deref(),
        Some("see @file.pdf now")
    );
    let json = clipboard
        .clipboard_typed(ATOMS_TYPE)
        .expect("its atoms beside the text");
    assert!(json.contains("\"from\":4,\"to\":13"), "{json}");
    ui.input.focus(&ui.scene, Some(b.node));
    ui.prepare_frame();
    assert!(ui.edit_command(EditCommand::Paste, &clipboard));
    ui.prepare_frame();
    assert_eq!(text(&b), "see @file.pdf now");
    let atoms = b.atoms();
    assert_eq!(atoms.len(), 1, "the pasted atom is an atom: {atoms:?}");
    assert_eq!(atoms[0].range, 4..13);
    // Pasted again after its text: a second, its own id.
    assert!(ui.edit_command(EditCommand::Paste, &clipboard));
    let atoms = b.atoms();
    assert_eq!(atoms.len(), 2);
    assert_eq!(atoms[1].range, 21..30);
    assert_ne!(atoms[0].id, atoms[1].id);
    // One undo takes the paste and its atom.
    assert!(b.undo());
    assert_eq!(b.atoms().len(), 1);
    assert_eq!(text(&b), "see @file.pdf now");
}

#[test]
fn a_paste_keeps_the_atoms_payload_and_wash() {
    let (ui, a, b) = fields();
    let payload: std::sync::Arc<dyn zgx::text_edit::AtomPayload> = std::sync::Arc::new(7u32);
    a.set_selection(4, 4);
    a.insert_atom(
        "@x",
        None,
        Some(zgx::scene::Color(1, 2, 3, 255)),
        Some(payload.clone()),
    )
    .unwrap();
    let clipboard = Clipboard::default();
    a.set_selection(4, 6);
    assert!(ui.edit_command(EditCommand::Cut, &clipboard));
    assert!(a.atoms().is_empty(), "cut takes it");
    ui.input.focus(&ui.scene, Some(b.node));
    ui.prepare_frame();
    assert!(ui.edit_command(EditCommand::Paste, &clipboard));
    let atoms = b.atoms();
    assert_eq!(atoms.len(), 1);
    let pasted = atoms[0].payload.as_ref().expect("its payload");
    assert!(std::sync::Arc::ptr_eq(pasted, &payload));
    assert_eq!(atoms[0].wash, Some(zgx::scene::Color(1, 2, 3, 255)));
}

#[test]
fn a_paste_whose_text_changed_since_takes_no_atoms() {
    let (ui, a, b, _) = with_atom();
    let clipboard = Clipboard::default();
    a.set_selection(0, 17);
    assert!(ui.edit_command(EditCommand::Copy, &clipboard));
    // Another app wrote the text and left the type (as a stale one can).
    *clipboard.text.borrow_mut() = Some("other words".into());
    ui.input.focus(&ui.scene, Some(b.node));
    assert!(ui.edit_command(EditCommand::Paste, &clipboard));
    assert_eq!(text(&b), "other words");
    assert!(b.atoms().is_empty());
}

#[test]
fn a_plain_text_area_takes_the_label_alone() {
    let (ui, a, _, c) = all_fields();
    a.set_selection(4, 4);
    a.insert_atom("@file.pdf", None, None, None).unwrap();
    let clipboard = Clipboard::default();
    a.set_selection(0, 17);
    assert!(ui.edit_command(EditCommand::Copy, &clipboard));
    ui.input.focus(&ui.scene, Some(c.node));
    ui.prepare_frame();
    assert!(ui.edit_command(EditCommand::Paste, &clipboard));
    assert_eq!(text(&c), "see @file.pdf now");
    assert!(c.atoms().is_empty(), "a plain text area takes no atoms");
}

#[test]
fn a_plain_text_target_gets_the_label() {
    let (ui, a, _, _) = with_atom();
    let clipboard = Clipboard::default();
    a.set_selection(4, 13);
    assert!(ui.edit_command(EditCommand::Copy, &clipboard));
    assert_eq!(clipboard.clipboard_text().as_deref(), Some("@file.pdf"));
}

#[test]
fn cut_takes_an_atom_whole_and_one_undo_brings_it_back() {
    let (ui, a, _, id) = with_atom();
    let clipboard = Clipboard::default();
    a.set_selection(2, 6);
    assert!(ui.edit_command(EditCommand::Cut, &clipboard));
    assert!(a.atoms().is_empty());
    assert!(!text(&a).contains("file"), "{}", text(&a));
    assert!(a.undo());
    assert_eq!(text(&a), "see @file.pdf now");
    assert_eq!(a.atoms(), [Atom::new(id, 4..13)]);
}

#[test]
fn typed_trigger_text_and_its_atom_undo_one_step_at_a_time() {
    let (mut ui, a, _) = fields();
    a.set_selection(8, 8);
    for c in [" ", "@", "q", "u", "e"] {
        ui.dispatch(InputEvent::Text(c.into()));
        ui.prepare_frame();
    }
    assert_eq!(text(&a), "see  now @que");
    a.insert_atom("@quarterly.pdf", Some(9..13), None, None)
        .unwrap();
    ui.prepare_frame();
    assert_eq!(text(&a), "see  now @quarterly.pdf");
    assert!(a.undo());
    assert_eq!(
        text(&a),
        "see  now @que",
        "one undo: the atom, the typed text back"
    );
    assert!(a.atoms().is_empty());
    assert_eq!(a.selection(), (13, 13));
    assert!(a.redo());
    assert_eq!(a.atoms().len(), 1);
    // Backspace over it, then undo: one step each.
    key(&mut ui, Key::Backspace, Modifiers::default());
    assert_eq!(text(&a), "see  now ");
    assert!(a.undo());
    assert_eq!(text(&a), "see  now @quarterly.pdf");
    assert_eq!(a.atoms().len(), 1);
}

/// An input method's events as macOS, X11 and Wayland hosts send them
/// (winit's `Ime::Preedit` and `Ime::Commit`).
fn ime(ui: &mut Ui, preedit: &[&str], commit: &str) {
    for text in preedit {
        ui.dispatch(InputEvent::ImePreedit {
            text: text.to_string(),
            cursor: Some((text.len(), text.len())),
        });
        ui.prepare_frame();
    }
    ui.dispatch(InputEvent::ImePreedit {
        text: String::new(),
        cursor: None,
    });
    ui.dispatch(InputEvent::ImeCommit(commit.into()));
    ui.prepare_frame();
}

#[test]
fn a_composition_beside_an_atom_commits_and_leaves_it_whole() {
    let (mut ui, a, _, id) = with_atom();
    // Right after it.
    a.set_selection(13, 13);
    ime(&mut ui, &["に", "にほ"], "日本");
    assert_eq!(text(&a), "see @file.pdf日本 now");
    assert_eq!(a.atoms(), [Atom::new(id, 4..13)]);
    assert_eq!(a.selection(), (19, 19));
    // Right before it.
    a.set_selection(4, 4);
    ime(&mut ui, &["か"], "火");
    assert_eq!(text(&a), "see 火@file.pdf日本 now");
    assert_eq!(a.atoms(), [Atom::new(id, 7..16)]);
    assert_eq!(a.selection(), (7, 7));
    // One undo takes the committed text alone.
    assert!(a.undo());
    assert_eq!(text(&a), "see @file.pdf日本 now");
    assert_eq!(a.atoms(), [Atom::new(id, 4..13)]);
}

#[test]
fn an_input_methods_replacement_through_an_atom_never_splits_it() {
    // macOS's insertText:replacementRange:, a reconversion over Windows' text
    // store, Wayland's delete_surrounding_text: a range from the text before
    // into the atom.
    let (_ui, a, _, _) = with_atom();
    assert!(a.replace_text(2..6, "X"));
    let now = text(&a);
    let atoms = a.atoms();
    let whole = atoms.len() == 1 && now[atoms[0].range.clone()] == *"@file.pdf";
    assert!(
        whole || (atoms.is_empty() && !now.contains("le.pdf")),
        "the atom kept whole or gone whole: {now} {atoms:?}"
    );
}
