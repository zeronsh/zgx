//! A drag held near a scroll view's edge scrolls it, as a browser's does:
//! up to 24 points a frame within 48 of the edge, per time (a faster display
//! scrolls as far a second), what is under the pointer hearing the drag over
//! it again after each step. Nothing is scheduled in the middle, past the
//! end of what scrolls, or once the drag leaves, drops or ends. The window's
//! own drag, another window's and files from outside scroll alike.
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};
use zgx::{
    compose::prelude::*,
    input::{DragPhase, InputEvent, PointerButton},
    reactive::Signal,
    widgets::Ui,
};

#[derive(Clone, Debug, PartialEq)]
struct Card;

fn pointer(ui: &Ui, event: InputEvent) {
    ui.input.dispatch(&ui.scene, event);
    ui.prepare_frame();
}
fn moved(ui: &Ui, x: f32, y: f32) {
    pointer(ui, InputEvent::PointerMove { x, y });
}
fn press(ui: &Ui, x: f32, y: f32, down: bool) {
    let button = PointerButton::Primary;
    pointer(
        ui,
        if down {
            InputEvent::PointerDown { x, y, button }
        } else {
            InputEvent::PointerUp { x, y, button }
        },
    );
}
/// Start a drag from the source at the top left.
fn start_drag(ui: &Ui) {
    press(ui, 20., 20., true);
    moved(ui, 35., 20.);
    assert!(ui.input.is_dragging());
}
/// Run the next interaction tick, if one is due; when it was.
fn tick(ui: &Ui) -> Option<Instant> {
    let due = ui.next_interaction_deadline()?;
    ui.advance_interactions(due).unwrap();
    Some(due)
}

/// A window 400 by 400: a drag source at the top left (0 to 40 down), under
/// it a scroll view 300 by 200 (40 to 240 down) of 600 points of rows, which
/// counts the drags and files it hears over it.
struct Page {
    ui: Ui,
    offset: Signal<f32>,
    over: Rc<Cell<u32>>,
}
fn page(content: f32) -> Page {
    let mut ui = Ui::new(400., 400.);
    let offset = ui.signal(0.);
    let over = Rc::new(Cell::new(0));
    let heard = over.clone();
    ui.mount(
        column().child(div().size(80., 40.).on_drag(|| Card)).child(
            scroll(offset.clone())
                .w(300.)
                .h(200.)
                .child(
                    column()
                        .w(300.)
                        .h(content)
                        .on_event(move |cx| match &cx.event {
                            InputEvent::Drag(drag) if drag.phase == DragPhase::Over => {
                                heard.set(heard.get() + 1)
                            }
                            InputEvent::FileHover { .. } => heard.set(heard.get() + 1),
                            _ => {}
                        }),
                ),
        ),
    );
    ui.prepare_frame();
    Page { ui, offset, over }
}
/// 10 points in from the scroll view's bottom edge.
const NEAR_BOTTOM: (f32, f32) = (150., 230.);

#[test]
fn a_drag_near_a_scroll_views_bottom_scrolls_it_and_is_over_what_came_under_it() {
    let Page { ui, offset, over } = page(600.);
    start_drag(&ui);
    moved(&ui, NEAR_BOTTOM.0, NEAR_BOTTOM.1);
    let heard = over.get();
    // Two ticks a 60 Hz frame apart: its full 24 points.
    tick(&ui).expect("near the edge");
    tick(&ui).expect("still near it");
    assert!((offset.get() - 24.).abs() < 0.01, "{}", offset.get());
    assert_eq!(over.get(), heard + 2, "over it again after each step");
    press(&ui, NEAR_BOTTOM.0, NEAR_BOTTOM.1, false);
    assert!(!ui.input.is_dragging());
    assert_eq!(ui.next_interaction_deadline(), None, "dropped: no more");
}

#[test]
fn a_drag_in_the_middle_of_a_scroll_view_schedules_nothing() {
    let Page { ui, offset, .. } = page(600.);
    start_drag(&ui);
    moved(&ui, 150., 140.);
    assert_eq!(ui.next_interaction_deadline(), None);
    // Just outside the band: nothing either.
    moved(&ui, 150., 240. - 49.);
    assert_eq!(ui.next_interaction_deadline(), None);
    // Into it from there.
    moved(&ui, 150., 240. - 30.);
    assert!(ui.next_interaction_deadline().is_some());
    moved(&ui, 150., 140.);
    tick(&ui);
    assert_eq!(ui.next_interaction_deadline(), None, "back in the middle");
    assert_eq!(offset.get(), 0.);
}

#[test]
fn a_drag_stops_scrolling_at_the_end_of_what_scrolls() {
    let Page { ui, offset, .. } = page(600.);
    start_drag(&ui);
    moved(&ui, NEAR_BOTTOM.0, NEAR_BOTTOM.1);
    let mut ticks = 0;
    while tick(&ui).is_some() {
        ticks += 1;
        assert!(ticks < 100, "it stops");
    }
    assert_eq!(offset.get(), 400.);
    // 400 points at 12 a tick, and the one that found no room left.
    assert_eq!(ticks, 35);
    // Toward the top it scrolls again.
    moved(&ui, 150., 50.);
    tick(&ui).expect("near the top");
    assert!(offset.get() < 400.);
    // Nothing to scroll: none.
    let Page { ui, .. } = page(150.);
    start_drag(&ui);
    moved(&ui, NEAR_BOTTOM.0, NEAR_BOTTOM.1);
    assert_eq!(ui.next_interaction_deadline(), None);
}

#[test]
fn a_drag_scrolls_as_far_a_second_however_often_it_ticks() {
    let scrolled = |every: u64| {
        let Page { ui, offset, .. } = page(4000.);
        start_drag(&ui);
        moved(&ui, NEAR_BOTTOM.0, NEAR_BOTTOM.1);
        let start = tick(&ui).unwrap() - Duration::from_millis(8);
        let from = offset.get();
        let mut at = start;
        while at < start + Duration::from_millis(400) {
            at += Duration::from_millis(every);
            ui.advance_interactions(at).unwrap();
        }
        offset.get() - from
    };
    // 24 points a 16 ms frame, for the 392 ms after the first tick.
    let (fast, slow) = (scrolled(8), scrolled(16));
    assert!((fast - 588.).abs() < 0.1, "{fast}");
    assert!((slow - 588.).abs() < 0.1, "{slow}");
    // A stall makes up for 50 ms at most.
    let Page { ui, offset, .. } = page(4000.);
    start_drag(&ui);
    moved(&ui, NEAR_BOTTOM.0, NEAR_BOTTOM.1);
    let at = tick(&ui).unwrap();
    let before = offset.get();
    ui.advance_interactions(at + Duration::from_secs(2))
        .unwrap();
    assert!(
        (offset.get() - before - 75.).abs() < 0.01,
        "{}",
        offset.get()
    );
}

#[test]
fn a_drag_that_leaves_the_window_stops_scrolling() {
    let Page { ui, .. } = page(600.);
    start_drag(&ui);
    moved(&ui, NEAR_BOTTOM.0, NEAR_BOTTOM.1);
    assert!(ui.next_interaction_deadline().is_some());
    pointer(&ui, InputEvent::PointerLeave);
    assert!(ui.input.is_dragging());
    assert_eq!(ui.next_interaction_deadline(), None);
    // Back over it, near the edge again.
    moved(&ui, NEAR_BOTTOM.0, NEAR_BOTTOM.1);
    assert!(ui.next_interaction_deadline().is_some());
    pointer(
        &ui,
        InputEvent::KeyDown {
            key: zgx::input::Key::Escape,
            modifiers: Default::default(),
            repeat: false,
        },
    );
    assert!(!ui.input.is_dragging());
    assert_eq!(ui.next_interaction_deadline(), None, "cancelled");
}

#[test]
fn files_dragged_in_near_an_edge_scroll_as_the_windows_own_drag_does() {
    let Page { ui, offset, over } = page(600.);
    let hover = || {
        pointer(
            &ui,
            InputEvent::FileHover {
                x: NEAR_BOTTOM.0,
                y: NEAR_BOTTOM.1,
                path: "notes.txt".into(),
            },
        )
    };
    hover();
    let heard = over.get();
    tick(&ui).expect("near the edge");
    tick(&ui).expect("still near it");
    assert!((offset.get() - 24.).abs() < 0.01, "{}", offset.get());
    assert_eq!(over.get(), heard + 2, "the files over it again");
    pointer(&ui, InputEvent::FileHoverCancelled);
    assert_eq!(ui.next_interaction_deadline(), None, "they left");
    hover();
    pointer(
        &ui,
        InputEvent::FileDrop {
            x: NEAR_BOTTOM.0,
            y: NEAR_BOTTOM.1,
            path: "notes.txt".into(),
        },
    );
    assert_eq!(ui.next_interaction_deadline(), None, "they dropped");
}

#[test]
fn another_windows_drag_near_an_edge_scrolls_it() {
    let mut source = Ui::new(200., 100.);
    source.mount(div().size(80., 40.).on_drag(|| Card));
    source.prepare_frame();
    start_drag(&source);
    source.input.set_drag_away(&source.scene, true);
    let payload = source.input.drag_state().unwrap().payload;
    let Page { ui, offset, over } = page(600.);
    ui.input
        .drag_over(&ui.scene, &payload, NEAR_BOTTOM.0, NEAR_BOTTOM.1);
    let heard = over.get();
    tick(&ui).expect("near the edge");
    tick(&ui).expect("still near it");
    assert!((offset.get() - 24.).abs() < 0.01, "{}", offset.get());
    assert_eq!(over.get(), heard + 2);
    ui.input.drag_out(&ui.scene);
    assert_eq!(ui.next_interaction_deadline(), None, "it left");
}

#[test]
fn the_innermost_scroll_view_near_its_edge_scrolls_and_none_past_its_end() {
    let mut ui = Ui::new(400., 400.);
    let (outer, inner) = (ui.signal(0.), ui.signal(0.));
    ui.mount(
        column().child(div().size(80., 40.).on_drag(|| Card)).child(
            scroll(outer.clone())
                .w(300.)
                .h(300.)
                .child(
                    scroll(inner.clone())
                        .w(300.)
                        .h(100.)
                        .child(div().w(300.).h(160.)),
                )
                .child(div().h(800.)),
        ),
    );
    ui.prepare_frame();
    start_drag(&ui);
    // 10 in from the inner view's bottom (40 + 100), mid the outer view.
    moved(&ui, 150., 130.);
    while tick(&ui).is_some() {}
    assert_eq!((inner.get(), outer.get()), (60., 0.));
    // Near the outer view's bottom (40 + 300), it scrolls.
    moved(&ui, 150., 330.);
    tick(&ui).expect("near the outer edge");
    assert!(outer.get() > 0.);
}

#[test]
fn a_drag_near_a_horizontal_scroll_views_right_edge_scrolls_it_across() {
    let mut ui = Ui::new(400., 400.);
    let offset = ui.signal(0.);
    ui.mount(
        column().child(div().size(80., 40.).on_drag(|| Card)).child(
            scroll_x(offset.clone())
                .w(300.)
                .h(100.)
                .child(div().w(900.).h(100.)),
        ),
    );
    ui.prepare_frame();
    start_drag(&ui);
    moved(&ui, 290., 90.);
    tick(&ui).expect("near the right edge");
    tick(&ui).expect("still near it");
    assert!((offset.get() - 24.).abs() < 0.01, "{}", offset.get());
}

/// A window with a drag source of text at the top left and, under it (40 to
/// 240 down), a text area 300 by 200 of `lines` lines that scrolls itself;
/// a box round it counts the drags it hears over the text area.
fn note(ui: &mut Ui, lines: usize, over: Rc<Cell<u32>>) -> zgx::compose::ViewHandle {
    let text: String = (0..lines).map(|i| format!("line {i}\n")).collect();
    let value = ui.signal(text);
    ui.mount(
        column()
            .child(
                div()
                    .size(80., 40.)
                    .on_drag(|| zgx::input::DraggedText::new("dropped")),
            )
            .child(
                div()
                    .on_event(move |cx| {
                        // Once a route: on its way in, not out again.
                        if let InputEvent::Drag(drag) = &cx.event
                            && drag.phase == DragPhase::Over
                            && cx.phase == zgx::input::EventPhase::Capture
                        {
                            over.set(over.get() + 1);
                        }
                    })
                    .child(
                        text_area("note", value)
                            .id("note")
                            .size(300., 200.)
                            .drop_caret(false),
                    ),
            ),
    )
}

#[test]
fn a_drag_near_a_text_areas_bottom_scrolls_its_text_and_is_over_it_again() {
    let mut ui = Ui::new(400., 400.);
    let over = Rc::new(Cell::new(0));
    let view = note(&mut ui, 80, over.clone());
    ui.prepare_frame();
    let editor = ui.editor(view.find("note").unwrap()).unwrap();
    assert_eq!(editor.scroll_offset(), (0., 0.));
    start_drag(&ui);
    // 10 in from its bottom edge.
    moved(&ui, 150., 230.);
    let heard = over.get();
    tick(&ui).expect("near the edge");
    tick(&ui).expect("still near it");
    let (_, down) = editor.scroll_offset();
    assert!((down - 24.).abs() < 0.01, "{down}");
    assert_eq!(over.get(), heard + 2, "over it again after each step");
    // Toward the top: back up.
    moved(&ui, 150., 50.);
    tick(&ui).expect("near the top");
    assert!(editor.scroll_offset().1 < down);
    press(&ui, 150., 50., false);
    assert_eq!(ui.next_interaction_deadline(), None, "dropped: no more");
}

#[test]
fn a_drag_near_a_text_areas_edge_schedules_nothing_when_its_text_fits() {
    let mut ui = Ui::new(400., 400.);
    let view = note(&mut ui, 2, Rc::new(Cell::new(0)));
    ui.prepare_frame();
    start_drag(&ui);
    moved(&ui, 150., 230.);
    // The text area takes the drag, so the spring-loading delay alone is due
    // (it would spring open a destination the drag rests on).
    assert!(ui.input.next_spring_deadline().is_some());
    assert_eq!(
        ui.next_interaction_deadline(),
        ui.input.next_spring_deadline()
    );
    let editor = ui.editor(view.find("note").unwrap()).unwrap();
    assert_eq!(editor.scroll_offset(), (0., 0.));
}
