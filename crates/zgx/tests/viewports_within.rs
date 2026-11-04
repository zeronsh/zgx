//! A list's, a scroll view's and a text input's viewport is sized from its own
//! box and adds nothing to what that box answers: absolute within it, and laid
//! out alone as it changes. So a window's width change re-measures what it
//! must, once. What each of them is as big as stays as it was: these say so,
//! around the cases where the natural size is what sizes a parent.

use zgx::{
    compose::{View, prelude::*},
    reactive::Signal,
    scene::Rect,
    widgets::Ui,
};

fn bounds(ui: &Ui, mounted: &zgx::compose::ViewHandle, id: &str) -> Rect {
    ui.scene.borrow().bounds(mounted.find(id).expect(id))
}

fn rows(ui: &Ui) -> View {
    measured_rows(
        ui.signal(0.),
        VariableHeights::new(&ui.runtime, 0, 40.),
        2,
        || (0..30).collect::<Vec<usize>>(),
        |_, _| 40.,
        |_, n, _| div().w_full().h(40.).id(format!("row-{n}")),
    )
}

/// Padding still insets what a scroll view, a list and a text area hold.
#[test]
fn what_they_hold_stays_inside_their_padding() {
    let mut ui = Ui::new(800., 600.);
    let offset: Signal<f32> = ui.signal(0.);
    let mounted = ui.mount(
        column()
            .w_full()
            .h_full()
            .child(
                scroll(offset)
                    .id("scroll")
                    .w(300.)
                    .h(100.)
                    .p(20.)
                    .child(div().id("in-scroll").w_full().h(500.)),
            )
            .child(rows(&ui).id("list").w(300.).h(100.).p(10.)),
    );
    ui.prepare_frame();
    ui.prepare_frame();
    let (scroller, inside) = (
        bounds(&ui, &mounted, "scroll"),
        bounds(&ui, &mounted, "in-scroll"),
    );
    assert_eq!((inside.x - scroller.x, inside.y - scroller.y), (20., 20.));
    assert_eq!(inside.width, 260.);
    let (list, row) = (
        bounds(&ui, &mounted, "list"),
        bounds(&ui, &mounted, "row-0"),
    );
    assert_eq!((row.x - list.x, row.y - list.y), (10., 10.));
    assert_eq!(row.width, 280.);
}

/// With nothing else sizing it, a list narrows with the row it's in: it has
/// no minimum of the size it was last.
#[test]
fn a_list_keeps_no_size_from_its_last_layout() {
    let mut ui = Ui::new(1000., 400.);
    let mounted = ui.mount(
        row()
            .w_full()
            .h_full()
            .child(rows(&ui).id("list").grow().h_full())
            .child(div().id("beside").w(100.).h_full()),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "list").width, 900.);
    ui.scene.borrow_mut().resize(500., 400.);
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "list").width, 400.);
    assert_eq!(bounds(&ui, &mounted, "beside").x, 400.);
    ui.scene.borrow_mut().resize(700., 400.);
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "list").width, 600.);
}

/// A scroll view the same.
#[test]
fn a_scroll_view_keeps_no_size_from_its_last_layout() {
    let mut ui = Ui::new(1000., 400.);
    let offset: Signal<f32> = ui.signal(0.);
    let mounted = ui.mount(
        row()
            .w_full()
            .h_full()
            .child(
                scroll(offset)
                    .id("scroll")
                    .grow()
                    .h_full()
                    .child(div().w_full().h(2000.)),
            )
            .child(div().id("beside").w(100.).h_full()),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "scroll").width, 900.);
    ui.scene.borrow_mut().resize(500., 400.);
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "scroll").width, 400.);
    assert_eq!(bounds(&ui, &mounted, "beside").x, 400.);
}

/// A scroll view across, with no height of its own, is as tall as what it
/// holds and as wide as it's given, in its flow as it was.
#[test]
fn a_fitted_scroll_across_is_as_tall_as_what_it_holds() {
    let mut ui = Ui::new(600., 400.);
    let offset: Signal<f32> = ui.signal(0.);
    let mounted = ui.mount(
        column()
            .w_full()
            .h_full()
            .child(
                scroll_x(offset)
                    .id("across")
                    .w_full()
                    .child(div().id("wide").w(2000.).h(70.)),
            )
            .child(div().id("below").w_full().h(10.)),
    );
    ui.prepare_frame();
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "across").height, 70.);
    assert_eq!(bounds(&ui, &mounted, "below").y, 70.);
    ui.scene.borrow_mut().resize(300., 400.);
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "across").width, 300.);
    assert_eq!(bounds(&ui, &mounted, "across").height, 70.);
}

/// A text input keeps its own default size through a resize of the window.
#[test]
fn a_text_input_keeps_its_own_size() {
    let mut ui = Ui::new(800., 400.);
    let value: Signal<String> = ui.signal(String::new());
    let mounted = ui.mount(row().child(text_input("Name", value).id("input")));
    ui.prepare_frame();
    let was = bounds(&ui, &mounted, "input");
    assert!(was.width > 0. && was.height > 0.);
    ui.scene.borrow_mut().resize(500., 300.);
    ui.prepare_frame();
    ui.scene.borrow_mut().resize(800., 400.);
    ui.prepare_frame();
    let now = bounds(&ui, &mounted, "input");
    assert_eq!((now.width, now.height), (was.width, was.height));
}

/// An absolute box placed from its right moves as its size changes, where
/// one placed from its top left needs nothing around it laid out.
#[test]
fn an_absolute_box_from_its_right_moves_as_it_resizes() {
    let mut ui = Ui::new(800., 400.);
    let width: Signal<f32> = ui.signal(100.);
    let read = width.clone();
    let mounted = ui.mount(
        overlay().w_full().h_full().child(
            div()
                .id("anchored")
                .absolute()
                .right(0.)
                .top(0.)
                .h(50.)
                .reactive_style(move || Styles::new().w(read.get())),
        ),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "anchored").x, 700.);
    width.set(300.);
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &mounted, "anchored").x, 500.);
    assert_eq!(bounds(&ui, &mounted, "anchored").width, 300.);
}

/// A text area a percent wide of a parent sized by what it holds takes its
/// natural size from what it holds, in its flow, and keeps it.
#[test]
fn a_text_area_a_percent_of_a_content_sized_parent_keeps_its_natural_size() {
    let mut ui = Ui::new(1000., 500.);
    let value: Signal<String> = ui.signal(String::from("words"));
    let mounted = ui.mount(
        column()
            .id("column")
            .items_start()
            .child(text_area("Notes", value).id("area").w_full()),
    );
    ui.prepare_frame();
    ui.prepare_frame();
    for (width, height) in [(1000., 500.), (300., 200.), (1000., 500.)] {
        ui.scene.borrow_mut().resize(width, height);
        ui.prepare_frame();
        ui.prepare_frame();
        assert_eq!(bounds(&ui, &mounted, "area").width, 240., "at {width}");
        assert_eq!(bounds(&ui, &mounted, "column").width, 240., "at {width}");
    }
}

/// A box sized by its content in one measure and given a base when placed: a
/// percent of that isn't a size of its own, placed or not.
#[test]
fn a_text_input_a_percent_of_a_height_sized_by_content_keeps_its_natural_height() {
    let mut ui = Ui::new(1000., 500.);
    let value = ui.signal(String::from("words"));
    let mounted = ui.mount(
        row()
            .id("row")
            .items_start()
            .child(text_input("Name", value).id("input").h_full())
            .child(div().w(20.).h(20.)),
    );
    for _ in 0..4 {
        ui.prepare_frame();
    }
    let height = ui
        .scene
        .borrow()
        .bounds(mounted.find("row").unwrap())
        .height;
    assert_eq!(height, 36.);
}

/// A sideways scroll in a parent its content sizes reads as it does in any
/// other: the size it's had stands, and what it holds doesn't size the parent.
#[test]
fn a_sideways_scroll_a_percent_of_a_content_sized_parent_stays_as_wide() {
    let mut ui = Ui::new(1000., 500.);
    let offset = ui.signal(0.);
    let mounted = ui.mount(
        column()
            .id("column")
            .items_start()
            .child(scroll_x(offset).w_full().child(div().w(900.).h(30.))),
    );
    for _ in 0..4 {
        ui.prepare_frame();
    }
    let width = ui
        .scene
        .borrow()
        .bounds(mounted.find("column").unwrap())
        .width;
    assert_eq!(width, 0.);
}

/// An absolute box's text wraps at the width its insets give, not the width
/// of the box it's in.
#[test]
fn text_in_an_absolute_box_wraps_at_the_width_of_its_insets() {
    let words = "words ".repeat(40);
    let mut ui = Ui::new(400., 500.);
    let mounted = ui.mount(
        column().w_full().h_full().child(
            div()
                .id("box")
                .absolute()
                .left(100.)
                .right(100.)
                .top(0.)
                .child(text(words.clone()).w_full()),
        ),
    );
    ui.prepare_frame();
    ui.prepare_frame();
    let narrow = ui.scene.borrow().bounds(mounted.find("box").unwrap());
    let mut ui = Ui::new(400., 500.);
    let mounted = ui.mount(
        column()
            .w_full()
            .h_full()
            .child(div().id("box").w(200.).child(text(words).w_full())),
    );
    ui.prepare_frame();
    ui.prepare_frame();
    let same = ui.scene.borrow().bounds(mounted.find("box").unwrap());
    assert_eq!((narrow.width, narrow.height), (same.width, same.height));
}
