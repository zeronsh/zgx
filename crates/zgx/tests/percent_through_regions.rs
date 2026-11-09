//! A percentage inside a region: a `switch` lays out as what it shows, in its
//! place, so its view's percentages resolve against the region's parent. A
//! `switch` given styles asked for a box of its own, and is one. `keyed`
//! holds many children, so it is a box, unless marked `contents`.
use std::{cell::Cell, rc::Rc};
use zgx::{compose::prelude::*, widgets::Ui};

/// A window-tall column holding, through `region`, a virtual list as tall as
/// `h_full` makes it (or 300 when not `full`): the rows it builds.
fn rows_built(region: impl FnOnce(Rc<dyn Fn() -> View>) -> View, full: bool) -> usize {
    let mut ui = Ui::new(400., 300.);
    let offset = ui.signal(0_f32);
    let heights = VariableHeights::new(&ui.runtime, 100, 40.);
    let rows = Rc::new(Cell::new(0));
    let count = rows.clone();
    let list: Rc<dyn Fn() -> View> = Rc::new(move || {
        let count = count.clone();
        let list = measured_virtual_list(
            offset.clone(),
            heights.clone(),
            2,
            |i| i,
            move |_, _, _| {
                count.set(count.get() + 1);
                row().h(40.)
            },
        )
        .w_full();
        if full { list.h_full() } else { list.h(300.) }
    });
    let _view = ui.mount(column().w_full().h_full().child(region(list)));
    ui.prepare_frame();
    rows.get()
}

#[test]
fn a_list_as_tall_as_its_parent_builds_its_rows_through_a_switch() {
    for full in [true, false] {
        // Directly under the column, as the measure of the rest.
        let direct = rows_built(|list| list(), full);
        assert!(direct > 0);
        let through = rows_built(|list| switch(|| true, move |_: bool, _| list()), full);
        assert_eq!(through, direct, "full {full}");
        let marked = rows_built(
            |list| switch(|| true, move |_: bool, _| list()).contents(),
            full,
        );
        assert_eq!(marked, direct, "full {full}");
    }
}

#[test]
fn a_styled_switch_is_a_box_of_its_own() {
    // Its height is its own, and what it shows is sized against it.
    let mut ui = Ui::new(400., 300.);
    let view = ui.mount(
        column().w_full().h_full().child(
            switch(|| true, |_: bool, _| div().id("shown").w_full().h_full())
                .id("region")
                .w_full()
                .h(100.),
        ),
    );
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let bounds = |id: &str| scene.bounds(view.find(id).unwrap());
    assert_eq!(bounds("region").height, 100.);
    assert_eq!(bounds("shown").height, 100.);
    assert_eq!(bounds("shown").width, 400.);
}

#[test]
fn keyed_is_a_box_unless_marked_contents() {
    // It holds many children: a percentage inside it has a height of its own
    // to read only when it is given one, or marked to lay out as its last.
    let keyed_list = |list: Rc<dyn Fn() -> View>| keyed(|| vec![1], move |_: i32, _| list());
    assert_eq!(rows_built(keyed_list, true), 0);
    assert!(rows_built(|list| keyed_list(list).contents(), true) > 0);
    assert!(rows_built(|list| keyed_list(list).h_full(), true) > 0);
}
