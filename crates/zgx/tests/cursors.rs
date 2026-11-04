use zgx::{compose::prelude::*, widgets::Ui};
#[test]
fn cursor_cascade_changes_without_layout_or_paint_and_disabled_regions_keep_explicit_style() {
    let mut ui = Ui::new(150., 100.);
    let busy = ui.signal(false);
    let view = ui.mount(
        row()
            .cursor(Cursor::Move)
            .child(
                div()
                    .id("child")
                    .size(50., 50.)
                    .disabled(true)
                    .reactive_style({
                        let busy = busy.clone();
                        move || {
                            Styles::new().cursor(if busy.get() {
                                Cursor::NotAllowed
                            } else {
                                Cursor::Grab
                            })
                        }
                    }),
            )
            .child(div().size(50., 50.)),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let node = view.find("child").unwrap();
    assert_eq!(ui.scene.borrow().cursor_for(node), Some(Cursor::Grab));
    assert_eq!(ui.scene.borrow().cursor_at(20., 20.), Some(Cursor::Grab));
    assert_eq!(ui.scene.borrow().cursor_at(70., 20.), Some(Cursor::Move));
    busy.set(true);
    ui.prepare_frame();
    let report = ui.scene.borrow_mut().flush();
    assert_eq!(report.layout_nodes, 0);
    assert!(report.damage.is_empty());
    assert_eq!(
        ui.scene.borrow().cursor_at(20., 20.),
        Some(Cursor::NotAllowed)
    );
    view.unmount();
    ui.prepare_frame();
    assert_eq!(ui.scene.borrow().cursor_at(20., 20.), None);
}

/// The cursor comes from what a press would reach: past a layer the pointer
/// passes through and a region around one, unless the layer says its own.
#[test]
fn the_cursor_is_what_is_under_what_the_pointer_passes_through() {
    let layer = || {
        div()
            .absolute()
            .left(0.)
            .top(0.)
            .size(100., 100.)
            .pointer_events(false)
            .child(div().size(100., 100.))
    };
    let under = || div().size(100., 100.).cursor(Cursor::Grab);
    let at = |view: View| {
        let mut ui = Ui::new(100., 100.);
        let _mounted = ui.mount(view);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        let scene = ui.scene.borrow();
        ui.input.cursor_at(&scene, 50., 50.)
    };
    let stack = || div().relative().size(100., 100.).child(under());
    assert_eq!(at(stack().child(layer())), Some(Cursor::Grab));
    assert_eq!(
        at(stack().child(switch(|| true, move |_, _| layer()).contents())),
        Some(Cursor::Grab),
        "a region has no box to be under"
    );
    assert_eq!(
        at(stack().child(layer().cursor(Cursor::Move))),
        Some(Cursor::Move),
        "the layer says its own"
    );
    assert_eq!(
        at(stack().child(div().absolute().left(0.).top(0.).size(100., 100.))),
        None,
        "a box that lets nothing through hides it"
    );
}
