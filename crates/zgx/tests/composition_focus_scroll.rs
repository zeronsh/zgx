use zgx::{
    compose::prelude::*,
    input::{InputEvent, Key, Modifiers},
    widgets::Ui,
};
fn tab(ui: &mut Ui, shift: bool) {
    ui.dispatch(InputEvent::KeyDown {
        key: Key::Tab,
        modifiers: Modifiers {
            shift,
            ..Default::default()
        },
        repeat: false,
    });
}
#[test]
fn tab_reveals_padded_viewport_and_manual_scroll_stays_put() {
    let mut ui = Ui::new(500., 300.);
    let offset = ui.signal(0.);
    let view =
        ui.mount(scroll(offset.clone()).w(200.).h(120.).p(10.).children(
            (0..5).map(|n| button().h(40.).id(n.to_string()).child(text(n.to_string()))),
        ));
    for _ in 0..5 {
        tab(&mut ui, false);
    }
    assert_eq!(ui.input.focused(), view.find("4"));
    assert_eq!(offset.get(), 100.);
    for _ in 0..4 {
        tab(&mut ui, true);
    }
    assert_eq!(offset.get(), 0.);
    ui.dispatch(InputEvent::Scroll {
        x: 20.,
        y: 20.,
        delta_x: 0.,
        delta_y: 70.,
    });
    assert_eq!(offset.get(), 70.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 70.);
    assert_eq!(ui.input.focused(), view.find("0"));
}
#[test]
fn nested_focus_reveal_uses_immediate_inner_translation() {
    let mut ui = Ui::new(500., 400.);
    let outer = ui.signal(0.);
    let inner = ui.signal(0.);
    let view = ui.mount(
        scroll(outer.clone())
            .w(250.)
            .h(120.)
            .p(10.)
            .child(div().h(150.))
            .child(
                scroll(inner.clone())
                    .w(200.)
                    .h(100.)
                    .p(10.)
                    .child(div().h(200.))
                    .child(button().h(40.).id("target").child(text("Target"))),
            ),
    );
    let target = view.find("target").unwrap();
    // No explicit prepare_frame: direct focus must settle allocations itself.
    assert!(ui.input.focus(&ui.scene, Some(target)));
    assert_eq!(inner.get(), 160.);
    assert_eq!(outer.get(), 140.);
    let bounds = ui.scene.borrow().bounds(target);
    assert_eq!((bounds.y, bounds.height), (70., 40.));
    ui.input.focus(&ui.scene, None);
    inner.set(0.);
    outer.set(0.);
    ui.runtime.batch(|| {
        ui.input.focus(&ui.scene, Some(target));
    });
    assert_eq!(inner.get(), 160.);
    assert_eq!(outer.get(), 140.);
}
#[test]
fn oversized_focus_target_uses_nearest_edge_without_alternating() {
    let mut ui = Ui::new(400., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .child(button().h(30.).id("first"))
            .child(div().h(120.))
            .child(button().h(200.).id("large"))
            .child(div().h(200.)),
    );
    let large = view.find("large").unwrap();
    assert!(ui.input.focus(&ui.scene, Some(large)));
    assert_eq!(offset.get(), 150.);
    ui.input.focus(&ui.scene, None);
    assert!(ui.input.focus(&ui.scene, Some(large)));
    assert_eq!(offset.get(), 150.);
    offset.set(200.);
    ui.input.focus(&ui.scene, None);
    ui.input.focus(&ui.scene, Some(large));
    assert_eq!(offset.get(), 200.);
    offset.set(450.);
    ui.input.focus(&ui.scene, None);
    ui.input.focus(&ui.scene, Some(large));
    assert_eq!(offset.get(), 250.);
}
#[test]
fn disabled_focus_rejected_and_unmounted_reveal_handler_released() {
    let mut ui = Ui::new(400., 300.);
    let offset = ui.signal(0.);
    let disabled = ui.signal(true);
    let read = disabled.clone();
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .disabled_when(move || read.get())
            .child(div().h(200.))
            .child(button().h(40.).id("target")),
    );
    let target = view.find("target").unwrap();
    assert!(!ui.input.focus(&ui.scene, Some(target)));
    assert_eq!(offset.get(), 0.);
    disabled.set(false);
    assert!(ui.input.focus(&ui.scene, Some(target)));
    assert_eq!(offset.get(), 140.);
    view.unmount();
    assert!(!ui.input.has_listeners(view.node()));
    offset.set(25.);
    ui.prepare_frame();
    assert_eq!(offset.get(), 25.);
}

#[test]
fn redirected_focus_does_not_reveal_stale_event_target() {
    let mut ui = Ui::new(400., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .child(button().h(40.).id("first"))
            .child(div().h(200.))
            .child(button().h(40.).id("redirect")),
    );
    let first = view.find("first").unwrap();
    let redirect = view.find("redirect").unwrap();
    let input = ui.input.clone();
    let scene = ui.scene.clone();
    ui.on_event(redirect, true, move |cx| {
        if matches!(cx.event, InputEvent::Focus) {
            input.focus(&scene, Some(first));
        }
    });
    assert!(ui.input.focus(&ui.scene, Some(redirect)));
    assert_eq!(ui.input.focused(), Some(first));
    assert_eq!(offset.get(), 0.);
}

#[test]
fn focus_keeps_the_scroll_padding_clear() {
    // A composer 30 points tall floats over the view's bottom: a control
    // tabbed to comes in above it, and one above the view below a 20-point
    // bar over its top.
    let mut ui = Ui::new(400., 300.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(100.)
            .scroll_padding(zgx::scene::Insets {
                top: 20.,
                bottom: 30.,
                ..Default::default()
            })
            .children((0..6).map(|n| button().h(40.).id(n.to_string()))),
    );
    // 0..40 and 40..80 show above 70 (100 less 30); 80..120 doesn't.
    tab(&mut ui, false);
    tab(&mut ui, false);
    assert_eq!(offset.get(), 10.);
    tab(&mut ui, false);
    assert_eq!(ui.input.focused(), view.find("2"));
    assert_eq!(offset.get(), 50.);
    // Back to the first: at 20, below the bar, as far as the view goes.
    tab(&mut ui, true);
    tab(&mut ui, true);
    assert_eq!(ui.input.focused(), view.find("0"));
    assert_eq!(offset.get(), 0.);
    // A control the view shows whole, under the bar: down to 20.
    offset.set(30.);
    ui.prepare_frame();
    tab(&mut ui, false);
    assert_eq!(ui.input.focused(), view.find("1"));
    assert_eq!(offset.get(), 20.);
}

/// A modal's first control, in a scroll view sized by its own `w`, focused by
/// the effect that opens it: the view's content is never laid out at a scroll
/// view's default size, which its observers would hear (`320`, then `400`).
#[test]
fn focus_as_a_modal_opens_lays_its_scroll_out_at_its_own_size() {
    use std::{cell::RefCell, rc::Rc};
    let mut ui = Ui::new(1200., 800.);
    let open = ui.signal(false);
    let widths: Rc<RefCell<Vec<f32>>> = Rc::default();
    let listening = Rc::new(RefCell::new(Vec::new()));
    let (shown, said, runtime) = (open.clone(), widths.clone(), ui.runtime.clone());
    ui.mount(modal("m", open.clone()).w(460.).p(0.).child(switch_or_none(
        move || shown.get(),
        move |shown, _| {
            let (said, runtime, keep) = (said.clone(), runtime.clone(), listening.clone());
            shown.then(|| {
                column().w_full().child(
                    scroll(runtime.signal(0.)).w(400.).h(100.).child(
                        column()
                            .w_full()
                            .on_mount(move |handle| {
                                let size = handle.observe_size().expect("size");
                                let said = said.clone();
                                keep.borrow_mut().push(runtime.effect(move || {
                                    said.borrow_mut().push(size.get().0);
                                }));
                            })
                            .child(button().w(10.).h(10.).id("first")),
                    ),
                )
            })
        },
    )));
    ui.prepare_frame();
    open.set(true);
    ui.prepare_frame();
    assert!(ui.input.focused().is_some(), "the control took focus");
    let widths = widths.borrow();
    assert!(!widths.contains(&320.), "{widths:?}");
    assert_eq!(widths.last(), Some(&400.), "{widths:?}");
}

/// The same, its first control below the fold: shown by the frame that opens
/// the modal, in a scroll view within a scroll view.
#[test]
fn focus_as_a_modal_opens_is_revealed_through_nested_scrolls() {
    let mut ui = Ui::new(1200., 800.);
    let (open, outer, inner) = (ui.signal(false), ui.signal(0.), ui.signal(0.));
    let (shown, to_outer, to_inner) = (open.clone(), outer.clone(), inner.clone());
    ui.mount(modal("m", open.clone()).w(460.).p(0.).child(switch_or_none(
        move || shown.get(),
        move |shown, _| {
            shown.then(|| {
                scroll(to_outer.clone())
                    .w(250.)
                    .h(120.)
                    .p(10.)
                    .child(div().h(150.))
                    .child(
                        scroll(to_inner.clone())
                            .w(200.)
                            .h(100.)
                            .p(10.)
                            .child(div().h(200.))
                            .child(button().h(40.).child(text("Target"))),
                    )
            })
        },
    )));
    ui.prepare_frame();
    open.set(true);
    ui.prepare_frame();
    // As focused directly (`nested_focus_reveal_uses_immediate_inner_translation`).
    assert_eq!((inner.get(), outer.get()), (160., 140.));
}
