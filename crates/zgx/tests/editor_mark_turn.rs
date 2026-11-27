//! A text area's line mark turned (`LineMark::rotate`: a toggle's chevron
//! opening): shown at its angle from the first, easing to a new one over
//! `motion.duration.fast`, and nothing scheduled once it settles.
use std::{cell::Cell, f32::consts::FRAC_PI_2, rc::Rc, time::Duration};
use zgx::{
    clock::VirtualClock,
    compose::prelude::*,
    scene::{Color, NodeId, NodeKind},
    theme::tok,
    widgets::{EditorHandle, LineMark, Ui},
};

/// A text area whose first line has a mark turned `angle` degrees (border
/// width 3, to find it), and one never turned (border width 4).
fn note(angle: Rc<Cell<f32>>) -> (Ui, EditorHandle) {
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal("toggle\nbody\nmore".to_string());
    let view = ui.mount(
        text_area("Note", value)
            .w(300.)
            .h(200.)
            .text_size(14.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_marks(Some(Rc::new(move |i, _, _| match i {
        0 => vec![
            LineMark::new(0., 0., 12., 12., Color(40, 40, 40, 255))
                .border_width(3.)
                .rotate(angle.get()),
            LineMark::new(20., 0., 12., 12., Color(0, 0, 0, 0)).border_width(4.),
        ],
        _ => Vec::new(),
    })));
    ui.prepare_frame();
    (ui, editor)
}

/// The mark with border width `width`.
fn mark(ui: &Ui, editor: &EditorHandle, width: f32) -> NodeId {
    let scene = ui.scene.borrow();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if matches!(scene.kind(id), NodeKind::Quad(q) if q.border_width == width) {
            return id;
        }
        pending.extend_from_slice(scene.children(id));
    }
    panic!("no mark of border {width}");
}

fn turned(ui: &Ui, editor: &EditorHandle, width: f32) -> f32 {
    let id = mark(ui, editor, width);
    ui.scene.borrow().local_transform(id).rotate
}

#[test]
fn a_mark_shows_at_its_angle_from_the_first() {
    let _clock = VirtualClock::new();
    let (ui, editor) = note(Rc::new(Cell::new(90.)));
    assert!((turned(&ui, &editor, 3.) - FRAC_PI_2).abs() < 1e-4);
    assert_eq!(turned(&ui, &editor, 4.), 0., "the other is not turned");
    assert_eq!(ui.next_interaction_deadline(), None, "nothing to ease");
}

#[test]
fn a_new_angle_eases_there_and_then_nothing_is_scheduled() {
    let clock = VirtualClock::new();
    let angle = Rc::new(Cell::new(0.));
    let (ui, editor) = note(angle.clone());
    assert_eq!(turned(&ui, &editor, 3.), 0.);
    let fast = ui
        .theme()
        .root()
        .number_untracked(tok::MOTION_DURATION_FAST);
    assert!(fast > 0., "a duration to ease over: {fast}");
    // Opened: it starts turning, and asks to be woken for the next step.
    angle.set(90.);
    editor.refresh();
    ui.prepare_frame();
    let next = ui.next_interaction_deadline().expect("a step to come");
    // Halfway through its time: past halfway, easing out.
    clock.advance(Duration::from_secs_f32(fast / 2000.), || {});
    ui.advance_interactions(clock.now().max(next)).unwrap();
    let mid = turned(&ui, &editor, 3.);
    assert!(
        mid > FRAC_PI_2 / 2. && mid < FRAC_PI_2,
        "partway, past half: {mid}"
    );
    // Its time over: at its angle, and nothing more scheduled.
    clock.advance(Duration::from_secs_f32(fast / 1000. + 0.05), || {});
    while let Some(due) = ui.next_interaction_deadline() {
        assert!(due <= clock.now(), "no step past the end: {due:?}");
        ui.advance_interactions(clock.now()).unwrap();
    }
    assert!((turned(&ui, &editor, 3.) - FRAC_PI_2).abs() < 1e-4);
    assert_eq!(ui.next_interaction_deadline(), None, "settled: idle");
    // A mark never turned has no transform.
    assert_eq!(turned(&ui, &editor, 4.), 0.);
}

/// A line break above a turned mark moves it a line down: it stays at its
/// angle, easing from nowhere.
#[test]
fn an_edit_above_a_turned_mark_turns_nothing() {
    let _clock = VirtualClock::new();
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal("plain\n> toggle\nbody".to_string());
    let view = ui.mount(
        text_area("Note", value)
            .w(300.)
            .h(200.)
            .text_size(14.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_marks(Some(Rc::new(|_, line: &str, _| {
        let angle = if line.starts_with('>') { 90. } else { 0. };
        vec![
            LineMark::new(0., 0., 12., 12., Color(0, 0, 0, 0))
                .border_width(if angle > 0. { 3. } else { 4. })
                .rotate(angle),
        ]
    })));
    ui.prepare_frame();
    assert!((turned(&ui, &editor, 3.) - FRAC_PI_2).abs() < 1e-4);
    ui.input.focus(&ui.scene, Some(view.node()));
    editor.set_selection(0, 0);
    ui.dispatch(zgx::input::InputEvent::Text("\n".into()));
    ui.prepare_frame();
    assert!(
        (turned(&ui, &editor, 3.) - FRAC_PI_2).abs() < 1e-4,
        "still at its angle, a line down"
    );
}

/// A plain mark where a turned one was, among its line's marks, shows as it
/// is: the other's turn is not its own to ease back from.
#[test]
fn a_plain_mark_in_a_turned_ones_place_does_not_turn() {
    let clock = VirtualClock::new();
    let turned_shows = Rc::new(Cell::new(true));
    let mut ui = Ui::new(400., 300.);
    let value = ui.signal("toggle\nbody".to_string());
    let view = ui.mount(
        text_area("Note", value)
            .w(300.)
            .h(200.)
            .text_size(14.)
            .line_height(20.)
            .text_wrap(true),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    let shows = turned_shows.clone();
    editor.set_line_marks(Some(Rc::new(move |i, _, _| {
        let plain = LineMark::new(20., 0., 12., 12., Color(0, 0, 0, 0)).border_width(4.);
        let turned = LineMark::new(0., 0., 12., 12., Color(0, 0, 0, 0));
        match (i, shows.get()) {
            (0, true) => vec![turned.border_width(3.).rotate(90.), plain],
            (0, false) => vec![plain],
            _ => Vec::new(),
        }
    })));
    ui.prepare_frame();
    assert!((turned(&ui, &editor, 3.) - FRAC_PI_2).abs() < 1e-4);
    assert_eq!(turned(&ui, &editor, 4.), 0.);
    turned_shows.set(false);
    editor.refresh();
    ui.prepare_frame();
    assert_eq!(turned(&ui, &editor, 4.), 0., "at once");
    clock.advance(Duration::from_millis(20), || {});
    editor.refresh();
    ui.prepare_frame();
    assert_eq!(turned(&ui, &editor, 4.), 0., "nor a frame on");
    assert_eq!(ui.next_interaction_deadline(), None, "nothing to ease");
}
