//! A text area's scrollbar (`View::scrollbar`): the one scroll views show, at its box's edge, following and
//! moving the editor's own scroll.
use zgx::{
    compose::prelude::*,
    input::{InputEvent, PointerButton},
    scene::{NodeId, NodeKind, Rect},
    semantics::Role,
    widgets::{EditorHandle, Ui},
};

fn bar(ui: &Ui) -> NodeId {
    ui.semantics
        .borrow()
        .iter()
        .find(|(_, n)| n.role == Role::ScrollBar)
        .unwrap()
        .0
}
/// The thumb's place and opacity (0 to 255).
fn thumb(ui: &Ui) -> (Rect, u8) {
    let scene = ui.scene.borrow();
    let thumb = *scene.children(bar(ui)).last().unwrap();
    let NodeKind::Quad(quad) = scene.kind(thumb) else {
        panic!("a quad thumb");
    };
    (scene.bounds(thumb), quad.fill.3)
}
fn notes(ui: &mut Ui, scrollbar: bool) -> EditorHandle {
    let text: String = (0..100)
        .map(|i| format!("line {i} of a note, long enough to wrap in a narrow text area\n"))
        .collect();
    let value = ui.signal(text);
    let mut view = text_area("Notes", value)
        .w(200.)
        .h(120.)
        .text_size(10.)
        .line_height(20.)
        .text_wrap(true);
    if scrollbar {
        view = view.scrollbar(true);
    }
    let view = ui.mount(view);
    ui.prepare_frame();
    ui.editor(view.node()).unwrap()
}
fn wheel(ui: &mut Ui, delta_y: f32) {
    ui.dispatch(InputEvent::Scroll {
        x: 50.,
        y: 50.,
        delta_x: 0.,
        delta_y,
    });
    ui.prepare_frame();
}
fn down(ui: &mut Ui, x: f32, y: f32) {
    ui.dispatch(InputEvent::PointerDown {
        x,
        y,
        button: PointerButton::Primary,
    });
}
fn up(ui: &mut Ui, x: f32, y: f32) {
    ui.dispatch(InputEvent::PointerUp {
        x,
        y,
        button: PointerButton::Primary,
    });
}

#[test]
fn a_text_areas_scrollbar_sits_at_its_edge_and_takes_no_room() {
    let mut ui = Ui::new(500., 400.);
    let editor = notes(&mut ui, true);
    let (view, content) = editor.scroll_extent();
    let field = ui.scene.borrow().bounds(editor.node);
    assert_eq!((field.width, field.height), (200., 120.));
    assert!(
        view > 0. && view < 120.,
        "its viewport, inside its padding: {view}"
    );
    assert!(content > 2_000., "a hundred lines: {content}");
    let track = ui.scene.borrow().bounds(bar(&ui));
    // The box's right edge and whole height, over its padding.
    assert_eq!(track.x + track.width, field.x + field.width);
    assert_eq!((track.y, track.height), (field.y, field.height));
    assert!(track.width > 0. && track.width <= 12.);
    // The text wraps where it does without a scrollbar.
    let mut plain_ui = Ui::new(500., 400.);
    let plain = notes(&mut plain_ui, false);
    assert_eq!(plain.scroll_extent(), (view, content));
    assert!(
        plain_ui
            .semantics
            .borrow()
            .iter()
            .all(|(_, n)| n.role != Role::ScrollBar)
    );
}

#[test]
fn the_thumb_follows_the_editors_scroll() {
    let mut ui = Ui::new(500., 400.);
    let editor = notes(&mut ui, true);
    let (view, content) = editor.scroll_extent();
    let limit = content - view;
    let (rest, _) = thumb(&ui);
    assert_eq!(editor.scroll_offset(), 0.);
    assert!(rest.height > 0. && rest.height < 120.);
    wheel(&mut ui, 300.);
    assert_eq!(editor.scroll_offset(), 300.);
    let (moved, _) = thumb(&ui);
    assert!(moved.y > rest.y, "{moved:?} below {rest:?}");
    // As far along its track as the text is along its scroll.
    let travel = 120. - rest.height - 2. * (rest.y - ui.scene.borrow().bounds(bar(&ui)).y);
    let along = (moved.y - rest.y) / travel;
    assert!(
        (along - 300. / limit).abs() < 0.01,
        "{along} of {}",
        300. / limit
    );
    // The app's own scroll moves it too, clamped to what there is.
    editor.set_scroll_offset(1e9);
    ui.prepare_frame();
    assert_eq!(editor.scroll_offset(), limit);
    let (end, _) = thumb(&ui);
    assert!(end.y > moved.y);
    editor.scroll_to_line(0, zgx::compose::RowAlign::Start);
    ui.prepare_frame();
    assert_eq!(editor.scroll_offset(), 0.);
    assert_eq!(thumb(&ui).0.y, rest.y);
}

#[test]
fn the_scrollbar_scrolls_the_editor() {
    let mut ui = Ui::new(500., 400.);
    let editor = notes(&mut ui, true);
    let (view, content) = editor.scroll_extent();
    let limit = content - view;
    let track = ui.scene.borrow().bounds(bar(&ui));
    let x = track.x + track.width / 2.;
    // A press on the track below the thumb pages (or jumps) down; the caret stays where it was.
    let caret = editor.selection();
    down(&mut ui, x, track.y + 100.);
    up(&mut ui, x, track.y + 100.);

    ui.prepare_frame();
    let paged = editor.scroll_offset();
    assert!(paged > 0., "{paged}");
    assert_eq!(editor.selection(), caret);
    assert_ne!(
        ui.focused_editor().map(|e| e.node),
        Some(editor.node),
        "the press was the bar's"
    );
    // Dragging the thumb to the foot of the track scrolls to the end.
    let (at, _) = thumb(&ui);
    let grab = at.y + at.height / 2.;
    down(&mut ui, x, grab);
    ui.dispatch(InputEvent::PointerMove { x, y: grab + 20. });
    ui.prepare_frame();
    let dragged = editor.scroll_offset();
    assert!(dragged > paged, "{dragged} past {paged}");
    ui.dispatch(InputEvent::PointerMove { x, y: 390. });
    ui.prepare_frame();
    assert_eq!(editor.scroll_offset(), limit);
    up(&mut ui, x, 390.);
    // And back to the top.
    let (at, _) = thumb(&ui);
    down(&mut ui, x, at.y + at.height / 2.);
    ui.dispatch(InputEvent::PointerMove { x, y: 0. });
    up(&mut ui, x, 0.);
    ui.prepare_frame();
    assert_eq!(editor.scroll_offset(), 0.);
}

#[test]
fn an_editors_scrollbar_is_an_overlay_where_scrollbars_are() {
    let mut ui = Ui::new(500., 400.);
    let editor = notes(&mut ui, true);
    assert!(thumb(&ui).1 > 0, "shown at rest");
    ui.set_overlay_scrollbars(true);
    ui.prepare_frame();
    assert_eq!(thumb(&ui).0.height, 0., "hidden until the text scrolls");
    wheel(&mut ui, 60.);
    assert_eq!(editor.scroll_offset(), 60.);
    let (shown, alpha) = thumb(&ui);
    assert!(
        shown.height > 0. && alpha > 0,
        "shown as the user scrolls: {shown:?} {alpha}"
    );
}
