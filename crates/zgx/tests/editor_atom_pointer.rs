//! A text area's atoms under the pointer (`View::on_atom_press`,
//! `View::on_atom_hover`): a chip pressed in place, taken or not; a drag
//! from one selects as before; the atom the pointer is over, with its box
//! on the line under it, followed through a scroll.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::{View, prelude::*},
    input::{InputEvent, PointerButton},
    scene::Rect,
    widgets::{EditorHandle, Ui},
};

#[derive(Debug, Default)]
struct Heard {
    presses: Vec<(u64, f32, f32)>,
    hovers: Vec<Option<(u64, Rect)>>,
}

/// A text area of `text`, `width` wide and `height` tall, its `atoms`
/// (byte ranges, start and end) made, with listeners that take presses when `take` says.
fn field(
    text: &str,
    atoms: &[(usize, usize)],
    size: (f32, f32),
    listen: bool,
    take: bool,
) -> (Ui, EditorHandle, Vec<u64>, Rc<RefCell<Heard>>) {
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal(text.to_string());
    let heard: Rc<RefCell<Heard>> = Rc::default();
    let mut view: View = text_area("Notes", value)
        .id("field")
        .size(size.0, size.1)
        .text_size(10.)
        .line_height(20.)
        .text_wrap(true);
    if listen {
        let (press, hover) = (heard.clone(), heard.clone());
        view = view
            .on_atom_press(move |event| {
                press
                    .borrow_mut()
                    .presses
                    .push((event.atom.id, event.x, event.y));
                take
            })
            .on_atom_hover(move |event| {
                let told = event.atom.map(|atom| atom.id).zip(event.area);
                hover.borrow_mut().hovers.push(told);
            });
    }
    let root = ui.mount(column().size(400., 300.).child(view));
    ui.prepare_frame();
    let node = root.find("field").unwrap();
    let editor = ui.editor(node).unwrap();
    let ids = atoms
        .iter()
        .map(|&range| {
            let id = editor.new_atom_id();
            let mut all = editor.atoms();
            all.push(zgx::text_edit::Atom::new(id, range.0..range.1));
            editor.set_atoms(all);
            id
        })
        .collect();
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    (ui, editor, ids, heard)
}

fn middle(area: Rect) -> (f32, f32) {
    (area.x + area.width / 2., area.y + area.height / 2.)
}

fn boxes(editor: &EditorHandle, id: u64) -> Vec<Rect> {
    let atom = editor.atoms().into_iter().find(|a| a.id == id).unwrap();
    editor.range_boxes(atom.range)
}

fn press(ui: &mut Ui, (x, y): (f32, f32), to: (f32, f32)) {
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
    ui.dispatch(InputEvent::PointerMove { x: to.0, y: to.1 });
    ui.dispatch(InputEvent::PointerUp {
        x: to.0,
        y: to.1,
        button: PointerButton::Primary,
    });
    ui.prepare_frame();
}

fn move_to(ui: &mut Ui, (x, y): (f32, f32)) {
    ui.dispatch(InputEvent::PointerMove { x, y });
    ui.prepare_frame();
}

const TEXT: &str = "see @file.pdf and @notes now";

#[test]
fn a_press_on_a_chip_taken_keeps_the_caret_out_of_it() {
    let (mut ui, editor, ids, heard) = field(TEXT, &[(4, 13)], (300., 80.), true, true);
    ui.input.focus(&ui.scene, Some(editor.node));
    editor.set_selection(0, 0);
    ui.prepare_frame();
    let at = middle(boxes(&editor, ids[0])[0]);
    // A wobble within 4 points is still a press in place.
    press(&mut ui, at, (at.0 + 2., at.1 + 1.));
    let presses = &heard.borrow().presses;
    assert_eq!(presses.len(), 1, "{presses:?}");
    assert_eq!(presses[0].0, ids[0]);
    assert_eq!(editor.selection(), (0, 0), "the caret stays where it was");
}

#[test]
fn a_press_not_taken_puts_the_caret_as_before() {
    let (mut ui, editor, ids, heard) = field(TEXT, &[(4, 13)], (300., 80.), true, false);
    editor.set_selection(0, 0);
    let at = middle(boxes(&editor, ids[0])[0]);
    press(&mut ui, at, at);
    assert_eq!(heard.borrow().presses.len(), 1);
    let (anchor, focus) = editor.selection();
    assert_eq!(anchor, focus);
    assert!(
        (4..=13).contains(&focus),
        "the press placed the caret: {focus}"
    );
}

#[test]
fn a_drag_from_a_chip_selects_as_before() {
    let (mut ui, editor, ids, heard) = field(TEXT, &[(4, 13)], (300., 80.), true, true);
    editor.set_selection(0, 0);
    let at = middle(boxes(&editor, ids[0])[0]);
    press(&mut ui, at, (at.0 + 60., at.1));
    assert!(heard.borrow().presses.is_empty(), "a drag is no press");
    let (anchor, focus) = editor.selection();
    assert_ne!(anchor, focus, "the drag selected text");
}

#[test]
fn hover_enters_changes_and_leaves() {
    let (mut ui, editor, ids, heard) = field(TEXT, &[(4, 13), (18, 24)], (300., 80.), true, true);
    let (first, second) = (boxes(&editor, ids[0])[0], boxes(&editor, ids[1])[0]);
    move_to(&mut ui, middle(first));
    move_to(&mut ui, (first.x + 1., first.y + 2.));
    move_to(&mut ui, middle(second));
    // Between them, on "and": no atom.
    move_to(
        &mut ui,
        (first.x + first.width + 12., first.y + first.height / 2.),
    );
    assert_eq!(
        heard.borrow().hovers,
        [Some((ids[0], first)), Some((ids[1], second)), None],
        "told on entering, on moving to another and on leaving, once each"
    );
    // Off the text area: nothing more to tell.
    move_to(&mut ui, (390., 290.));
    assert_eq!(heard.borrow().hovers.len(), 3);
}

#[test]
fn hover_leaving_the_text_area_from_a_chip_tells_none() {
    let (mut ui, editor, ids, heard) = field(TEXT, &[(4, 13)], (300., 80.), true, true);
    move_to(&mut ui, middle(boxes(&editor, ids[0])[0]));
    move_to(&mut ui, (390., 290.));
    assert_eq!(
        heard.borrow().hovers.last(),
        Some(&None),
        "{:?}",
        heard.borrow()
    );
}

#[test]
fn a_wrapped_chip_gives_the_box_on_the_line_under_the_pointer() {
    // Narrow: the chip's two words wrap onto two lines.
    let text = "see @quarterly figures.pdf now";
    let (mut ui, editor, ids, heard) = field(text, &[(4, 26)], (110., 120.), true, true);
    let pieces = boxes(&editor, ids[0]);
    assert!(pieces.len() >= 2, "the chip wraps: {pieces:?}");
    let last = *pieces.last().unwrap();
    move_to(&mut ui, middle(last));
    assert_eq!(heard.borrow().hovers, [Some((ids[0], last))]);
    move_to(&mut ui, middle(pieces[0]));
    assert_eq!(
        heard.borrow().hovers.last(),
        Some(&Some((ids[0], pieces[0]))),
        "the same chip's box on the other line"
    );
}

#[test]
fn the_hovered_box_follows_a_scroll() {
    let text = format!(
        "{}see @file.pdf now\n{}",
        "line\n".repeat(3),
        "line\n".repeat(30)
    );
    let start = "line\n".len() * 3 + 4;
    let (mut ui, editor, ids, heard) =
        field(&text, &[(start, start + 9)], (300., 120.), true, true);
    let before = boxes(&editor, ids[0])[0];
    let at = (before.x + 2., before.y + before.height - 2.);
    move_to(&mut ui, at);
    assert_eq!(heard.borrow().hovers, [Some((ids[0], before))]);
    // A small wheel step moves the chip up under the still pointer.
    ui.dispatch(InputEvent::Scroll {
        x: at.0,
        y: at.1,
        delta_x: 0.,
        delta_y: 6.,
    });
    ui.prepare_frame();
    let after = boxes(&editor, ids[0])[0];
    assert!(after.y < before.y, "it scrolled: {before:?} → {after:?}");
    let hovers = &heard.borrow().hovers;
    assert_eq!(hovers.len(), 2, "told again: {hovers:?}");
    assert_eq!(
        hovers[1],
        if (after.y..=after.y + after.height).contains(&at.1) {
            Some((ids[0], after))
        } else {
            None
        }
    );
}

#[test]
fn a_text_area_without_the_listeners_is_as_before() {
    let (mut ui, editor, ids, heard) = field(TEXT, &[(4, 13)], (300., 80.), false, true);
    editor.set_selection(0, 0);
    let at = middle(boxes(&editor, ids[0])[0]);
    move_to(&mut ui, at);
    press(&mut ui, at, at);
    let (anchor, focus) = editor.selection();
    assert!(anchor == focus && (4..=13).contains(&focus));
    let heard = heard.borrow();
    assert!(heard.presses.is_empty() && heard.hovers.is_empty());
}
