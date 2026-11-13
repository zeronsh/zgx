use std::{cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers, PointerButton},
    widgets::Ui,
};
#[derive(Clone, Debug, PartialEq)]
struct Card(u32);
fn pointer(ui: &Ui, event: InputEvent) {
    ui.input.dispatch(&ui.scene, event);
    ui.prepare_frame();
}
fn down(ui: &Ui, x: f32, y: f32) {
    pointer(
        ui,
        InputEvent::PointerDown {
            x,
            y,
            button: PointerButton::Primary,
        },
    );
}
fn moved(ui: &Ui, x: f32, y: f32) {
    pointer(ui, InputEvent::PointerMove { x, y });
}
fn up(ui: &Ui, x: f32, y: f32) {
    pointer(
        ui,
        InputEvent::PointerUp {
            x,
            y,
            button: PointerButton::Primary,
        },
    );
}
#[test]
fn typed_drop_preview_and_click_suppression() {
    let mut ui = Ui::new(400., 200.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let dropped = log.clone();
    let ended = log.clone();
    let clicked = log.clone();
    let view = ui.mount(
        row()
            .gap(20.)
            .child(
                button()
                    .child(text("Card"))
                    .id("source")
                    .size(100., 80.)
                    .on_click(move || clicked.borrow_mut().push("click".to_owned()))
                    .on_drag(|| Card(7))
                    .drag_preview(|value: &Card| {
                        div()
                            .id("preview")
                            .size(100., 80.)
                            .child(text(format!("{}", value.0)))
                    })
                    .on_drag_end(move |_: &Card, accepted| {
                        ended.borrow_mut().push(format!("end:{accepted}"))
                    }),
            )
            .child(
                div()
                    .id("destination")
                    .size(100., 80.)
                    .on_drop(move |card: &Card, _| {
                        dropped.borrow_mut().push(format!("drop:{}", card.0))
                    }),
            ),
    );
    ui.prepare_frame();
    let original = ui.scene.borrow().children(ui.root()).len();
    down(&ui, 20., 20.);
    moved(&ui, 22., 20.);
    assert!(!ui.input.is_dragging());
    up(&ui, 22., 20.);
    assert_eq!(&*log.borrow(), &["click"]);
    log.borrow_mut().clear();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    assert!(ui.input.is_dragging());
    assert_eq!(ui.scene.borrow().children(ui.root()).len(), original + 1);
    moved(&ui, 150., 20.);
    up(&ui, 150., 20.);
    assert_eq!(&*log.borrow(), &["drop:7", "end:true"]);
    assert!(!ui.input.is_dragging());
    assert_eq!(ui.scene.borrow().children(ui.root()).len(), original);
    assert!(view.is_mounted());
}
/// A preview is built with what its source's tree provides: a component in it asks for the frame
/// clock the window provides, rather than finding none (a panic, the red banner).
#[test]
fn a_drag_preview_sees_the_services_its_source_has() {
    let mut ui = Ui::new(400., 200.);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    ui.mount(provide(
        zgx::frame::FrameClock::new(),
        div()
            .id("source")
            .size(100., 80.)
            .on_drag(|| Card(1))
            .drag_preview(move |_: &Card| {
                let log = log.clone();
                component(move |cx| {
                    // Panics without the provider.
                    let _frames = cx.frames();
                    log.borrow_mut().push("preview");
                    div().id("preview").size(40., 40.)
                })
            }),
    ));
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    assert!(ui.input.is_dragging());
    assert_eq!(&*seen.borrow(), &["preview"]);
    up(&ui, 35., 20.);
}
#[test]
fn a_drag_starts_past_the_systems_drag_threshold() {
    let mut ui = Ui::new(400., 200.);
    let clicks = Rc::new(RefCell::new(0));
    let clicked = clicks.clone();
    ui.mount(
        button()
            .size(100., 80.)
            .on_click(move || *clicked.borrow_mut() += 1)
            .on_drag(|| Card(1)),
    );
    ui.prepare_frame();
    // zgx's own: 4 points.
    down(&ui, 20., 20.);
    moved(&ui, 30., 20.);
    assert!(ui.input.is_dragging());
    up(&ui, 30., 20.);
    // A desktop's, raised (GNOME's `drag-threshold`): 10 points is a click.
    ui.input.set_drag_threshold(24.);
    down(&ui, 20., 20.);
    moved(&ui, 30., 20.);
    assert!(!ui.input.is_dragging());
    up(&ui, 30., 20.);
    assert_eq!(*clicks.borrow(), 1);
    down(&ui, 20., 20.);
    moved(&ui, 50., 20.);
    assert!(ui.input.is_dragging());
    up(&ui, 50., 20.);
}
#[test]
fn rejected_outside_and_escape_drags_end_without_drop() {
    let mut ui = Ui::new(400., 200.);
    let ends = Rc::new(RefCell::new(Vec::new()));
    let log = ends.clone();
    let view = ui.mount(
        row()
            .gap(20.)
            .child(
                div()
                    .id("source")
                    .size(100., 80.)
                    .on_drag(|| Card(7))
                    .on_drag_end(move |_: &Card, accepted| log.borrow_mut().push(accepted)),
            )
            .child(
                div()
                    .size(100., 80.)
                    .on_drop_when(|_: &Card| false, |_: &Card, _| panic!("rejected drop")),
            ),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 150., 20.);
    up(&ui, 150., 20.);
    down(&ui, 20., 20.);
    moved(&ui, 300., 150.);
    up(&ui, 300., 150.);
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    pointer(
        &ui,
        InputEvent::KeyDown {
            key: Key::Escape,
            modifiers: Modifiers::default(),
            repeat: false,
        },
    );
    assert_eq!(&*ends.borrow(), &[false, false, false]);
    assert!(!ui.input.is_dragging());
    view.unmount();
}
#[test]
fn a_drag_outlives_its_removed_source_and_drops_where_it_led() {
    let mut ui = Ui::new(400., 200.);
    let value = Rc::new(());
    let weak = Rc::downgrade(&value);
    let log = Rc::new(RefCell::new(Vec::new()));
    let (dropped, ended) = (log.clone(), log.clone());
    let source = ui.mount(
        div()
            .size(100., 80.)
            .on_drag(move || value.clone())
            .drag_preview(|_: &Rc<()>| div().size(40., 40.))
            .on_drag_end(move |_: &Rc<()>, accepted| {
                ended.borrow_mut().push(format!("end:{accepted}"))
            }),
    );
    let target = ui.mount(
        div()
            .absolute()
            .left(200.)
            .size(100., 80.)
            .on_drop(move |_: &Rc<()>, _| dropped.borrow_mut().push("drop".to_owned())),
    );
    ui.prepare_frame();
    let children = ui.scene.borrow().children(ui.root()).len();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    assert!(ui.input.is_dragging());
    // Gone (moved elsewhere by what the drag did): the drag goes on, its
    // preview following the pointer.
    source.unmount();
    ui.prepare_frame();
    assert!(ui.input.is_dragging(), "the drag outlives its source");
    let preview = ui
        .input
        .drag_state()
        .and_then(|d| d.preview)
        .expect("its preview");
    moved(&ui, 230., 30.);
    let at = ui.scene.borrow().bounds(preview);
    assert_eq!((at.x, at.y), (230. - 20., 30. - 20.), "the preview follows");
    up(&ui, 230., 30.);
    assert_eq!(
        &*log.borrow(),
        &["drop", "end:true"],
        "dropped, and its source's listeners hear it end"
    );
    assert!(!ui.input.is_dragging());
    assert!(weak.upgrade().is_none(), "the payload goes with the drag");
    assert_eq!(
        ui.scene.borrow().children(ui.root()).len(),
        children - 1,
        "and the preview"
    );
    target.unmount();
}
/// What a removed source's listeners hold goes once the drag ends, and may
/// reach input as it goes.
#[test]
fn a_removed_sources_listeners_drop_after_the_drag_lets_input_go() {
    struct Touches(zgx::input::InputDispatcher, Rc<RefCell<bool>>);
    impl Drop for Touches {
        fn drop(&mut self) {
            *self.1.borrow_mut() = self.0.is_dragging();
        }
    }
    let mut ui = Ui::new(400., 200.);
    let dropped_while_dragging = Rc::new(RefCell::new(true));
    let held = Touches(ui.input.clone(), dropped_while_dragging.clone());
    let source = ui.mount(div().size(100., 80.).on_drag(|| Card(1)).on_drag_end(
        move |_: &Card, _| {
            let _ = &held;
        },
    ));
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    source.unmount();
    ui.prepare_frame();
    up(&ui, 230., 30.);
    assert!(!ui.input.is_dragging());
    assert!(
        !*dropped_while_dragging.borrow(),
        "dropped as the drag ended"
    );
}
#[test]
fn nearest_typed_destination_wins_and_removed_target_cannot_receive_drop() {
    let mut ui = Ui::new(400., 200.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let parent = log.clone();
    let child = log.clone();
    let source = ui.mount(div().size(80., 80.).on_drag(|| Card(1)));
    let target = ui.mount(
        div()
            .absolute()
            .left(120.)
            .size(160., 100.)
            .on_drop(move |_: &Card, _| parent.borrow_mut().push("parent"))
            .child(
                div()
                    .id("inner")
                    .size(80., 80.)
                    .on_drop(move |_: &Card, _| child.borrow_mut().push("child")),
            ),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 140., 20.);
    up(&ui, 140., 20.);
    assert_eq!(&*log.borrow(), &["child"]);
    down(&ui, 20., 20.);
    moved(&ui, 140., 20.);
    target.unmount();
    up(&ui, 140., 20.);
    assert_eq!(&*log.borrow(), &["child"]);
    assert!(!ui.input.is_dragging());
    source.unmount();
}
#[test]
fn a_hidden_source_keeps_its_drag_and_a_disabled_one_cancels_it() {
    let mut ui = Ui::new(400., 200.);
    let ends = Rc::new(RefCell::new(Vec::new()));
    let log = ends.clone();
    let source = ui.mount(
        div()
            .size(100., 80.)
            .on_drag(|| Card(1))
            .drag_preview(|_: &Card| div().size(20., 20.))
            .on_drag_end(move |_: &Card, accepted| log.borrow_mut().push(accepted)),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    let set_visible = |ui: &Ui, visible: bool| {
        let mut style = ui.scene.borrow().style(source.node());
        style.layout_options = Some(std::sync::Arc::new(zgx::layout::LayoutOptions {
            visible: Some(visible),
            ..Default::default()
        }));
        ui.scene.borrow_mut().set_style(source.node(), style);
        ui.prepare_frame();
    };
    // Hidden (its tab no longer shown): the drag goes on.
    set_visible(&ui, false);
    assert!(ui.input.is_dragging());
    assert_eq!(
        ui.scene.borrow().children(ui.root()).len(),
        2,
        "the preview stays"
    );
    moved(&ui, 60., 20.);
    assert!(ui.input.is_dragging());
    // Shown and disabled: it can no longer take input, so the drag ends.
    set_visible(&ui, true);
    ui.set_disabled(source.node(), true);
    ui.prepare_frame();
    assert!(!ui.input.is_dragging());
    assert_eq!(&*ends.borrow(), &[false]);
    assert_eq!(
        ui.scene.borrow().children(ui.root()).len(),
        1,
        "its preview goes"
    );
}
#[test]
fn grouped_external_paths_route_once_to_nearest_handler() {
    let mut ui = Ui::new(300., 200.);
    let received = Rc::new(RefCell::new(Vec::new()));
    let output = received.clone();
    ui.mount(
        div()
            .size(200., 100.)
            .on_files_drop(move |paths, _| output.borrow_mut().push(paths.to_vec())),
    );
    ui.prepare_frame();
    pointer(
        &ui,
        InputEvent::FilesDrop {
            x: 20.,
            y: 20.,
            paths: vec!["/tmp/a".into(), "/tmp/b".into()].into(),
        },
    );
    assert_eq!(
        &*received.borrow(),
        &[vec![
            std::path::PathBuf::from("/tmp/a"),
            std::path::PathBuf::from("/tmp/b")
        ]]
    );
}
#[test]
fn nearest_nested_source_wins_capture() {
    let mut ui = Ui::new(400., 200.);
    let got = Rc::new(RefCell::new(Vec::new()));
    let result = got.clone();
    ui.mount(
        row()
            .gap(20.)
            .child(
                div()
                    .size(100., 80.)
                    .on_drag(|| Card(99))
                    .child(div().size(80., 60.).on_drag(|| Card(1))),
            )
            .child(
                div()
                    .size(100., 80.)
                    .on_drop(move |card: &Card, _| result.borrow_mut().push(card.0)),
            ),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 150., 20.);
    up(&ui, 150., 20.);
    assert_eq!(&*got.borrow(), &[1]);
}
#[test]
fn disabled_source_cancellation_releases_preview_and_notifies_end() {
    let mut ui = Ui::new(400., 200.);
    let ends = Rc::new(RefCell::new(Vec::new()));
    let results = ends.clone();
    let source = ui.mount(
        div()
            .size(100., 80.)
            .on_drag(|| Card(1))
            .drag_preview(|_: &Card| div().size(20., 20.))
            .on_drag_end(move |_: &Card, accepted| results.borrow_mut().push(accepted)),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    ui.set_disabled(source.node(), true);
    ui.prepare_frame();
    assert!(!ui.input.is_dragging());
    assert_eq!(&*ends.borrow(), &[false]);
    assert_eq!(ui.scene.borrow().children(ui.root()).len(), 1);
}
#[test]
fn preview_keeps_grab_point_or_fixed_cursor_offset() {
    for cursor_offset in [None, Some((12., 12.))] {
        let mut ui = Ui::new(400., 300.);
        let source = div().id("source").size(100., 80.).on_drag(|| Card(1));
        let preview = |_: &Card| div().size(100., 80.);
        let source = match cursor_offset {
            None => source.drag_preview(preview),
            Some(offset) => source.drag_preview_at_cursor(offset, preview),
        };
        let view = ui.mount(row().p(30.).child(source));
        ui.prepare_frame();
        let original = ui.scene.borrow().children(ui.root()).len();
        // Press 20,30 inside the source at 30,30, then drag.
        down(&ui, 50., 60.);
        moved(&ui, 100., 90.);
        assert!(ui.input.is_dragging());
        let scene = ui.scene.borrow();
        let children = scene.children(ui.root());
        assert_eq!(children.len(), original + 1);
        let bounds = scene.bounds(children[original]);
        let expected = match cursor_offset {
            None => (80., 60.),
            Some(_) => (112., 102.),
        };
        assert_eq!((bounds.x, bounds.y), expected);
        drop(scene);
        up(&ui, 100., 90.);
        view.unmount();
    }
}

#[test]
fn overlays_receive_drops_over_their_children() {
    let mut ui = Ui::new(400., 300.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let dropped = log.clone();
    let view = ui.mount(
        column()
            .child(div().id("source").size(40., 40.).on_drag(|| Card(3)))
            .child(
                overlay()
                    .id("area")
                    .size(200., 200.)
                    .on_drop(move |card: &Card, _| dropped.borrow_mut().push(card.0))
                    .child(div().id("child").size(100., 100.)),
            ),
    );
    ui.prepare_frame();
    let bounds = |id: &str| ui.scene.borrow().bounds(view.find(id).unwrap());
    let (source, child) = (bounds("source"), bounds("child"));
    down(&ui, source.x + 5., source.y + 5.);
    moved(&ui, source.x + 15., source.y + 15.);
    moved(&ui, child.x + 20., child.y + 20.);
    up(&ui, child.x + 20., child.y + 20.);
    assert_eq!(*log.borrow(), vec![3]);
}

#[test]
fn a_source_the_drop_removes_still_hears_the_drag_end() {
    let mut ui = Ui::new(400., 200.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let here = ui.signal(true);
    let (ended, gone) = (log.clone(), here.clone());
    let view = ui.mount(
        row()
            .child(switch(
                move || here.get(),
                move |here, _| {
                    if !here {
                        return div();
                    }
                    let ended = ended.clone();
                    div()
                        .id("source")
                        .size(100., 80.)
                        .on_drag(|| Card(3))
                        .on_drag_end(move |_: &Card, accepted| {
                            ended.borrow_mut().push(format!("end:{accepted}"))
                        })
                },
            ))
            .child(
                div()
                    .id("destination")
                    .size(100., 80.)
                    // Taking the card moves it here: the source goes.
                    .on_drop(move |_: &Card, _| {
                        gone.set(false);
                    }),
            ),
    );
    ui.prepare_frame();
    assert!(view.find("source").is_some());
    down(&ui, 20., 20.);
    moved(&ui, 60., 20.);
    moved(&ui, 150., 20.);
    assert!(ui.input.is_dragging());
    up(&ui, 150., 20.);
    assert!(view.find("source").is_none());
    assert!(!ui.input.is_dragging());
    assert_eq!(&*log.borrow(), &["end:true"]);
}
#[test]
fn drops_land_under_a_pointer_transparent_layer() {
    // A layer over the destination that lets the pointer through, inside a
    // parent with listeners of its own: the drop still reaches the
    // destination, as a press would.
    let mut ui = Ui::new(400., 200.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let dropped = log.clone();
    ui.mount(
        row()
            .gap(20.)
            .child(div().id("source").size(100., 80.).on_drag(|| Card(7)))
            .child(
                overlay()
                    .size(100., 80.)
                    .on_click(|| {})
                    .child(div().id("destination").size(100., 80.).on_drop(
                        move |card: &Card, _| dropped.borrow_mut().push(format!("drop:{}", card.0)),
                    ))
                    .child(div().id("layer").size(100., 80.).pointer_events(false)),
            ),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 35., 20.);
    assert!(ui.input.is_dragging());
    moved(&ui, 150., 20.);
    up(&ui, 150., 20.);
    assert_eq!(&*log.borrow(), &["drop:7"]);
}

/// What a drag carries to other apps (`drag_export`) is the host's to read
/// as it starts; a drag another app took ends accepted at its source
/// (`end_drag_elsewhere`), and its host learns how it ended.
#[test]
fn drags_carry_their_export_and_end_where_the_host_says() {
    use zgx::input::DragItem;
    let mut ui = Ui::new(300., 200.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let ended = log.clone();
    ui.mount(
        div()
            .id("source")
            .size(100., 40.)
            .on_drag(|| Card(5))
            .drag_export(|card: &Card| vec![DragItem::Text(format!("card {}", card.0))])
            .on_drag_end(move |card: &Card, accepted| {
                ended
                    .borrow_mut()
                    .push(format!("end:{}:{accepted}", card.0))
            }),
    );
    ui.prepare_frame();
    down(&ui, 10., 10.);
    moved(&ui, 30., 10.);
    let drag = ui.input.drag_state().expect("dragging");
    assert_eq!(
        drag.export.as_deref(),
        Some(&[DragItem::Text("card 5".into())][..])
    );
    // Out of the window, another app took it.
    moved(&ui, 400., 10.);
    ui.input.end_drag_elsewhere(&ui.scene, true, false);
    assert!(!ui.input.is_dragging());
    assert_eq!(*log.borrow(), ["end:5:true"]);
    let ended = ui.input.take_drag_ended().expect("ended");
    assert!(ended.accepted);
    assert!(ui.input.take_drag_ended().is_none());
    // The release the platform's session took: nothing more happens.
    up(&ui, 400., 10.);
    assert_eq!(*log.borrow(), ["end:5:true"]);
}

/// A drag from one window over another (`drag_over`), dropped there
/// (`drag_drop`), and the source told how it ended (`end_drag_elsewhere`),
/// as a host with several windows carries it; then one torn off (released
/// outside every window), which its source takes (`on_drag_outside`).
#[test]
fn drags_cross_windows_and_tear_off() {
    let mut source = Ui::new(300., 200.);
    let mut target = Ui::new(300., 200.);
    let log = Rc::new(RefCell::new(Vec::new()));
    let (ended, outside, hovered, dropped) = (log.clone(), log.clone(), log.clone(), log.clone());
    source.mount(
        div()
            .id("source")
            .size(100., 40.)
            .on_drag(|| Card(3))
            .on_drag_end(move |card: &Card, accepted| {
                ended
                    .borrow_mut()
                    .push(format!("end:{}:{accepted}", card.0))
            })
            .on_drag_outside(move |card: &Card, event| {
                outside
                    .borrow_mut()
                    .push(format!("outside:{}:{}", card.0, event.outside))
            }),
    );
    target.mount(
        div()
            .id("destination")
            .size(300., 200.)
            .on_event(move |cx| {
                if let InputEvent::Drag(event) = &cx.event
                    && event.phase == zgx::input::DragPhase::Over
                    && cx.phase == zgx::input::EventPhase::Target
                {
                    hovered
                        .borrow_mut()
                        .push(format!("over:{}", event.visiting));
                }
            })
            .on_drop(move |card: &Card, _| dropped.borrow_mut().push(format!("drop:{}", card.0))),
    );
    source.prepare_frame();
    target.prepare_frame();
    down(&source, 10., 10.);
    moved(&source, 30., 10.);
    let drag = source.input.drag_state().expect("dragging");
    assert_eq!(drag.payload.downcast_ref::<Card>(), Some(&Card(3)));
    // Over the other window: the source's destinations hear nothing.
    source.input.set_drag_away(&source.scene, true);
    assert!(source.input.drag_state().unwrap().away);
    assert!(
        target
            .input
            .drag_over(&target.scene, &drag.payload, 50., 50.)
    );
    assert!(
        target
            .input
            .drag_drop(&target.scene, &drag.payload, 50., 50.)
    );
    source.input.end_drag_elsewhere(&source.scene, true, false);
    assert!(!source.input.is_dragging());
    assert_eq!(
        *log.borrow(),
        ["over:true", "over:true", "drop:3", "end:3:true"]
    );
    let ended = source.input.take_drag_ended().unwrap();
    assert!(ended.accepted && !ended.outside);
    // Torn off: released over none of the app's windows, and taken by the
    // source, so its preview does not slide back.
    log.borrow_mut().clear();
    up(&source, 30., 10.);
    down(&source, 10., 10.);
    moved(&source, 30., 10.);
    source.input.set_drag_away(&source.scene, true);
    target.input.drag_out(&target.scene);
    source.input.end_drag_elsewhere(&source.scene, false, true);
    assert_eq!(*log.borrow(), ["end:3:false", "outside:3:true"]);
    assert!(source.input.take_drag_ended().unwrap().outside);
}

/// A source whose drag moves its window says so as it starts
/// (`drag_moves_window`), for the value it carries.
#[test]
fn a_drag_may_move_its_window() {
    let mut ui = Ui::new(300., 200.);
    ui.mount(
        div()
            .id("source")
            .size(100., 40.)
            .on_drag(|| Card(1))
            .drag_moves_window(|card: &Card| card.0 == 1),
    );
    ui.prepare_frame();
    down(&ui, 10., 10.);
    moved(&ui, 30., 10.);
    assert!(ui.input.drag_state().expect("dragging").moves_window);
    up(&ui, 30., 10.);
    assert!(!ui.input.is_dragging());
}
#[test]
fn a_drag_resting_on_a_destination_springs_it_open_once() {
    use std::time::{Duration, Instant};
    let mut ui = Ui::new(400., 200.);
    let sprang = Rc::new(RefCell::new(0));
    let count = sprang.clone();
    let view = ui.mount(
        row()
            .gap(20.)
            .child(div().size(80., 80.).on_drag(|| Card(1)))
            .child(
                div()
                    .size(100., 80.)
                    .on_drop(|_: &Card, _| {})
                    .on_drag_spring(move |_: &Card| {
                        *count.borrow_mut() += 1;
                        true
                    }),
            )
            .child(div().size(100., 80.).on_drop(|_: &Card, _| {})),
    );
    ui.prepare_frame();
    let later = |ms| Instant::now() + Duration::from_millis(ms);
    down(&ui, 20., 20.);
    moved(&ui, 110., 20.);
    // Resting on it: it springs once the delay passes, not before.
    let due = ui.input.next_spring_deadline().expect("waiting to spring");
    assert!(
        due <= later(500) && due > later(400),
        "the platform's delay"
    );
    ui.advance_interactions(later(300)).unwrap();
    assert_eq!(*sprang.borrow(), 0);
    // Moving within it does not start the delay again.
    moved(&ui, 150., 40.);
    assert_eq!(ui.input.next_spring_deadline(), Some(due));
    ui.advance_interactions(later(520)).unwrap();
    assert_eq!(*sprang.borrow(), 1);
    ui.advance_interactions(later(2000)).unwrap();
    assert_eq!(*sprang.borrow(), 1, "once per stay");
    assert!(ui.input.next_spring_deadline().is_none());
    // Over another destination and back: it waits again, and Space springs
    // it at once.
    moved(&ui, 260., 20.);
    moved(&ui, 150., 20.);
    assert!(ui.input.next_spring_deadline().is_some());
    pointer(
        &ui,
        InputEvent::KeyDown {
            key: Key::Space,
            modifiers: Modifiers::default(),
            repeat: false,
        },
    );
    assert_eq!(*sprang.borrow(), 2);
    assert!(ui.input.is_dragging(), "Space does not end the drag");
    up(&ui, 150., 20.);
    assert!(
        ui.input.next_spring_deadline().is_none(),
        "nothing waits after the drag"
    );
    // Over another window a while and back: the delay starts again.
    down(&ui, 20., 20.);
    moved(&ui, 110., 20.);
    ui.input.set_drag_away(&ui.scene, true);
    ui.input.set_drag_away(&ui.scene, false);
    let back = Instant::now();
    moved(&ui, 110., 20.);
    let due = ui.input.next_spring_deadline().expect("waiting to spring");
    assert!(due >= back + zgx::input::SPRING_DELAY, "from the return");
    up(&ui, 110., 20.);
    assert_eq!(*sprang.borrow(), 2);
    // Turned off by the platform: never.
    ui.input.set_spring_delay(None);
    down(&ui, 20., 20.);
    moved(&ui, 110., 20.);
    assert!(ui.input.next_spring_deadline().is_none());
    ui.advance_interactions(later(5000)).unwrap();
    assert_eq!(*sprang.borrow(), 2);
    up(&ui, 110., 20.);
    view.unmount();
}
#[test]
fn an_anchored_preview_hangs_below_the_pointer_centred() {
    let mut ui = Ui::new(400., 300.);
    let view = ui.mount(
        div()
            .size(100., 80.)
            .on_drag(|| Card(1))
            .drag_preview_anchored((0.5, 0.), (0., 16.), |_: &Card| div().size(60., 40.)),
    );
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 150., 100.);
    let preview = ui.input.drag_state().and_then(|d| d.preview).unwrap();
    let at = ui.scene.borrow().bounds(preview);
    assert_eq!((at.x, at.y, at.width), (150. - 30., 100. + 16., 60.));
    up(&ui, 150., 100.);
    view.unmount();
}

/// A preview built from a component that reads a provider (an animated one
/// asks for the window's frame clock) finds it as its source did.
#[test]
fn a_drag_preview_keeps_the_providers_its_source_was_built_under() {
    let mut ui = Ui::new(400., 300.);
    let found = Rc::new(RefCell::new(None));
    let seen = found.clone();
    let view = ui.mount(zgx::compose::provide(
        zgx::frame::FrameClock::new(),
        div()
            .size(100., 80.)
            .on_drag(|| Card(1))
            .drag_preview(move |_: &Card| {
                let seen = seen.clone();
                zgx::compose::component(move |cx| {
                    *seen.borrow_mut() = Some(cx.try_service::<zgx::frame::FrameClock>().is_some());
                    let _ = cx.frames();
                    div().size(60., 40.)
                })
            }),
    ));
    ui.prepare_frame();
    down(&ui, 20., 20.);
    moved(&ui, 150., 100.);
    assert_eq!(*found.borrow(), Some(true));
    up(&ui, 150., 100.);
    view.unmount();
}
