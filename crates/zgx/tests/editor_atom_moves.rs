//! Text dragged within an editor carries its atoms (`TextEditor::move_text`),
//! as a web editor's chips go with a drag: one in the moved text arrives as
//! the same atom (id, wash, payload), one the text passes shifts, none is
//! lost, and one undo puts everything back (redo again), with no atom
//! reported gone. A range through a chip takes it whole; a drop inside one
//! goes to its nearer edge.
use std::sync::Arc;
use zgx::{
    scene::Color,
    text_edit::{Atom, AtomChange, AtomPayload, TextEditor},
};

/// "a @chip b c", the chip an atom with a wash and a payload: the editor, and
/// the atom as made.
fn chip_text() -> (TextEditor, Atom) {
    let mut editor = TextEditor::new("a  b c");
    editor.set_selection(2, 2);
    let payload: Arc<dyn AtomPayload> = Arc::new(String::from("file-7"));
    let id = editor
        .insert_atom("@chip", None, Some(Color(10, 20, 30, 255)), Some(payload))
        .expect("an atom");
    assert_eq!(editor.text(), "a @chip b c");
    let atom = editor.atoms()[0].clone();
    assert_eq!((atom.id, atom.range.clone()), (id, 2..7));
    editor.take_atom_changes();
    (editor, atom)
}

/// The atom as `atom` was, now at `range`.
fn at(atom: &Atom, range: std::ops::Range<usize>) -> Atom {
    Atom {
        range,
        ..atom.clone()
    }
}

#[test]
fn a_dragged_chip_arrives_as_the_same_atom() {
    let (mut editor, chip) = chip_text();
    // "@chip b" dropped after "c".
    assert!(editor.move_text(2..9, 11));
    assert_eq!(editor.text(), "a  c@chip b");
    assert_eq!(
        editor.atoms(),
        [at(&chip, 4..9)],
        "same id, wash and payload"
    );
    assert!(
        editor.take_atom_changes().is_empty(),
        "nothing reported lost or made: it moved"
    );
    assert_eq!(editor.selection().range(), 4..11, "the moved text selected");
}

#[test]
fn a_chip_the_text_passes_stays_in_place() {
    let (mut editor, chip) = chip_text();
    // "c" dropped before "a", across the chip.
    assert!(editor.move_text(10..11, 0));
    assert_eq!(editor.text(), "ca @chip b ");
    assert_eq!(editor.atoms(), [at(&chip, 3..8)]);
    assert_eq!(&editor.text()[3..8], "@chip");
    assert!(editor.take_atom_changes().is_empty());
    // And "a" dropped after "b", the other way across it.
    let (mut editor, chip) = chip_text();
    assert!(editor.move_text(0..1, 9));
    assert_eq!(editor.text(), " @chip ba c");
    assert_eq!(editor.atoms(), [at(&chip, 1..6)]);
    assert_eq!(&editor.text()[1..6], "@chip");
}

#[test]
fn one_undo_puts_the_atoms_back_and_redo_moves_them_again() {
    let (mut editor, chip) = chip_text();
    assert!(editor.move_text(2..9, 11));
    assert!(editor.undo());
    assert_eq!(editor.text(), "a @chip b c");
    assert_eq!(
        editor.atoms(),
        std::slice::from_ref(&chip),
        "as it was, the same atom"
    );
    assert!(
        editor.take_atom_changes().is_empty(),
        "no chip reported gone"
    );
    assert!(editor.redo());
    assert_eq!(editor.text(), "a  c@chip b");
    assert_eq!(editor.atoms(), [at(&chip, 4..9)]);
    assert!(editor.take_atom_changes().is_empty());
    // Across it, too.
    let (mut editor, chip) = chip_text();
    assert!(editor.move_text(10..11, 0));
    assert!(editor.undo());
    assert_eq!(
        (editor.text().into_owned(), editor.atoms().to_vec()),
        ("a @chip b c".into(), vec![chip.clone()])
    );
    assert!(editor.redo());
    assert_eq!(editor.atoms(), [at(&chip, 3..8)]);
}

#[test]
fn a_range_through_a_chip_takes_it_whole_and_a_drop_inside_one_leaves_it() {
    // "hip b": through the chip, which goes whole, as a web editor's would.
    let (mut editor, chip) = chip_text();
    assert!(editor.move_text(4..9, 11));
    assert_eq!(editor.text(), "a  c@chip b");
    assert_eq!(editor.atoms(), [at(&chip, 4..9)]);
    // "c" dropped inside the chip: at its nearer edge, the chip whole.
    let (mut editor, chip) = chip_text();
    assert!(editor.move_text(10..11, 6));
    assert_eq!(editor.text(), "a @chipc b ");
    assert_eq!(editor.atoms(), std::slice::from_ref(&chip));
    assert!(editor.take_atom_changes().is_empty());
}

#[test]
fn a_chip_deleted_is_still_reported_gone() {
    // The settling of moves leaves a real removal alone.
    let (mut editor, chip) = chip_text();
    editor.set_selection(2, 7);
    assert!(editor.insert(""));
    assert_eq!(editor.text(), "a  b c");
    let changes = editor.take_atom_changes();
    assert!(
        matches!(changes.as_slice(), [AtomChange::Removed(gone, ..)] if gone.id == chip.id),
        "{changes:?}"
    );
}
