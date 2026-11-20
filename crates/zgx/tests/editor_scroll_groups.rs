//! A text area's scroll groups (`LineInset::scroll_group`): a table wider than the note, its cells' lines in one
//! group, scrolls sideways alone, its text and marks cut to the group's frame.
use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::InputEvent,
    scene::{ClipInset, NodeId, NodeKind},
    widgets::{EditorHandle, GroupFrame, LineInset, LineMark, Ui},
};

/// Four 300-point columns (lines 1 to 4) in a 680-point note, between ordinary lines.
const TEXT: &str = "before\nc1 one\nc2 two\nc3 three\nc4 four\nafter";
/// The table's row, down the window (each line 20 points).
const ROW_Y: f32 = 30.;

fn note(grouped: bool) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(800., 400.);
    let value = ui.signal(TEXT.to_string());
    let view = ui.mount(
        text_area("Note", value)
            .w(680.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_insets(Some(Rc::new(move |i| match i {
        1..=4 => LineInset {
            left: 300. * (i - 1) as f32,
            width: Some(300.),
            joins_next: i < 4,
            scroll_group: grouped.then_some(7),
            group_width: grouped.then_some(680.),
            ..LineInset::default()
        },
        _ => LineInset::default(),
    })));
    // Each cell's border, from the area's left edge.
    editor.set_line_marks(Some(Rc::new(|i| match i {
        1..=4 => vec![LineMark {
            x: 300. * (i - 1) as f32,
            width: 300.,
            border_width: 1.,
            ..LineMark::default()
        }],
        _ => Vec::new(),
    })));
    ui.input.focus(&ui.scene, Some(view.node()));
    ui.prepare_frame();
    (ui, editor)
}

/// The text node showing `text` under the editor.
fn line_node(ui: &Ui, editor: &EditorHandle, text: &str) -> NodeId {
    let scene = ui.scene.borrow();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if matches!(scene.kind(id), NodeKind::Text { text: t, .. } if &**t == text) {
            return id;
        }
        pending.extend_from_slice(scene.children(id));
    }
    panic!("no line {text:?}");
}
/// Where `text`'s line shows across, and its clip.
fn shown(ui: &Ui, editor: &EditorHandle, text: &str) -> (f32, Option<ClipInset>) {
    let id = line_node(ui, editor, text);
    let scene = ui.scene.borrow();
    (scene.transform(id).x, scene.clip_inset(id))
}
/// The cell borders' quads: where each shows across, and its clip, left to right as made.
fn borders(ui: &Ui, editor: &EditorHandle) -> Vec<(f32, Option<ClipInset>)> {
    let scene = ui.scene.borrow();
    let mut found = Vec::new();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if matches!(scene.kind(id), NodeKind::Quad(q) if q.border_width == 1.) {
            found.push((scene.transform(id).x, scene.clip_inset(id)));
        }
        pending.extend_from_slice(scene.children(id));
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found
}
fn wheel(ui: &mut Ui, x: f32, y: f32, delta_x: f32, delta_y: f32, shift: Option<bool>) {
    let event = InputEvent::Scroll {
        x,
        y,
        delta_x,
        delta_y,
    };
    match shift {
        Some(shift) => {
            zgx::input::mouse_wheel(shift, || ui.dispatch(event));
        }
        None => {
            ui.dispatch(event);
        }
    }
    ui.prepare_frame();
}

/// The group's lines and marks show only inside its frame: the third column cut at the frame's right edge, the
/// fourth cut whole.
#[test]
fn a_groups_lines_and_marks_show_only_inside_its_frame() {
    let (ui, editor) = note(true);
    assert_eq!(editor.group_extent(7), Some((680., 1200.)));
    assert_eq!(
        shown(&ui, &editor, "c1 one"),
        (0., Some(ClipInset::px(0., 0., 0., 0.)))
    );
    assert_eq!(
        shown(&ui, &editor, "c3 three"),
        (600., Some(ClipInset::px(0., 220., 0., 0.)))
    );
    assert_eq!(
        shown(&ui, &editor, "c4 four"),
        (900., Some(ClipInset::px(0., 300., 0., 0.)))
    );
    assert_eq!(
        shown(&ui, &editor, "before"),
        (0., None),
        "a line outside the group isn't cut"
    );
    let marks = borders(&ui, &editor);
    assert_eq!(marks.len(), 4, "{marks:?}");
    assert_eq!(marks[2], (600., Some(ClipInset::px(0., 220., 0., 0.))));
    assert_eq!(marks[3].1, Some(ClipInset::px(0., 300., 0., 0.)));
}

/// A mouse wheel with Shift held over the table scrolls the table, not the note; so does a trackpad's sideways
/// motion. Over an ordinary line, the table doesn't move.
#[test]
fn shift_wheel_and_sideways_motion_scroll_the_group() {
    let (mut ui, editor) = note(true);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = heard.clone();
    editor.on_group_scroll(move |group, x| log.borrow_mut().push((group, x)));
    wheel(&mut ui, 100., ROW_Y, 0., 120., Some(true));
    assert_eq!(editor.group_scroll(7), 120.);
    assert_eq!(editor.scroll_offset().1, 0., "the note didn't move down");
    assert_eq!(shown(&ui, &editor, "c3 three").0, 480.);
    let marks = borders(&ui, &editor);
    assert_eq!(marks[2].0, 480., "the marks move with their lines");
    wheel(&mut ui, 100., ROW_Y, 40., 0., None);
    assert_eq!(editor.group_scroll(7), 160.);
    // Over the line above the table: nothing of the table moves.
    wheel(&mut ui, 100., 10., 40., 0., None);
    assert_eq!(editor.group_scroll(7), 160.);
    assert_eq!(*heard.borrow(), vec![(7, 120.), (7, 160.)]);
}

/// Scrolled programmatically or by the wheel, a group stays between 0 and how far its lines reach past its frame.
#[test]
fn a_groups_scroll_is_clamped() {
    let (mut ui, editor) = note(true);
    editor.set_group_scroll(7, 10_000.);
    assert_eq!(editor.group_scroll(7), 520.);
    ui.prepare_frame();
    assert_eq!(
        shown(&ui, &editor, "c4 four"),
        (380., Some(ClipInset::px(0., 0., 0., 0.)))
    );
    wheel(&mut ui, 100., ROW_Y, 200., 0., None);
    assert_eq!(editor.group_scroll(7), 520.);
    editor.set_group_scroll(7, -5.);
    assert_eq!(editor.group_scroll(7), 0.);
    wheel(&mut ui, 100., ROW_Y, -50., 0., None);
    assert_eq!(editor.group_scroll(7), 0.);
}

/// The caret moved into a column the frame cuts: the group moves to show it.
#[test]
fn a_caret_moved_into_a_clipped_cell_reveals_it() {
    let (mut ui, editor) = note(true);
    let at = TEXT.find("four").unwrap();
    ui.dispatch(InputEvent::SetTextSelection {
        anchor: at,
        focus: at,
    });
    ui.prepare_frame();
    let x = editor.group_scroll(7);
    assert!(x > 900. - 680. && x <= 520., "revealed: {x}");
    let (left, _) = shown(&ui, &editor, "c4 four");
    assert!(left < 680., "the fourth column shows: {left}");
}

/// Without a group, nothing of this is there: no line or mark is cut, and the handle's group is empty.
#[test]
fn without_a_group_nothing_is_cut() {
    let (ui, editor) = note(false);
    assert_eq!(editor.group_extent(7), None);
    for line in ["before", "c1 one", "c3 three", "c4 four"] {
        assert_eq!(shown(&ui, &editor, line).1, None, "{line}");
    }
    assert_eq!(shown(&ui, &editor, "c3 three").0, 600.);
    assert!(borders(&ui, &editor).iter().all(|(_, clip)| clip.is_none()));
}

/// A frame the app sets (a table's cells' borders outside their text) applies at once, no text changing: the
/// extent, the clip and the clamp read it; a narrower one brings the offset back to its limit, and an empty one
/// clears it.
#[test]
fn a_frame_the_app_sets_applies_at_once() {
    let (ui, editor) = note(true);
    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = heard.clone();
    editor.on_group_scroll(move |group, x| log.borrow_mut().push((group, x)));
    editor.set_group_frame(
        7,
        GroupFrame {
            left: Some(0.),
            width: Some(600.),
            content: Some(1250.),
        },
    );
    assert_eq!(editor.group_extent(7), Some((600., 1250.)));
    ui.prepare_frame();
    // The third column starts where the frame ends: cut whole.
    assert_eq!(
        shown(&ui, &editor, "c3 three"),
        (600., Some(ClipInset::px(0., 300., 0., 0.)))
    );
    editor.set_group_scroll(7, 10_000.);
    assert_eq!(editor.group_scroll(7), 650.);
    // The table narrows: the offset comes back to the new limit, and the listener hears.
    editor.set_group_frame(
        7,
        GroupFrame {
            left: Some(0.),
            width: Some(600.),
            content: Some(900.),
        },
    );
    assert_eq!(editor.group_scroll(7), 300.);
    assert_eq!(*heard.borrow(), vec![(7, 650.), (7, 300.)]);
    // Cleared: as its lines say.
    editor.set_group_frame(7, GroupFrame::default());
    assert_eq!(editor.group_extent(7), Some((680., 1200.)));
    assert_eq!(editor.group_scroll(7), 300.);
}

/// A frame starting right of the first cell's text cuts that text at its left edge; a wheel left of the frame
/// isn't the group's.
#[test]
fn a_frames_left_edge_cuts_and_bounds_the_group() {
    let (mut ui, editor) = note(true);
    editor.set_group_frame(
        7,
        GroupFrame {
            left: Some(50.),
            ..GroupFrame::default()
        },
    );
    ui.prepare_frame();
    assert_eq!(editor.group_extent(7), Some((680., 1150.)));
    assert_eq!(
        shown(&ui, &editor, "c1 one"),
        (0., Some(ClipInset::px(0., 0., 0., 50.)))
    );
    wheel(&mut ui, 20., ROW_Y, 40., 0., None);
    assert_eq!(
        editor.group_scroll(7),
        0.,
        "left of the frame: not the group's"
    );
    wheel(&mut ui, 100., ROW_Y, 40., 0., None);
    assert_eq!(editor.group_scroll(7), 40.);
}
