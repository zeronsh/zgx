//! `View::when_container`: styles that follow the size of the container
//! above, as CSS's `@container` rules, in the frame it changes.
use std::{cell::Cell, rc::Rc};

use zgx::{compose::prelude::*, container_query::Query, scene::Rect, style::Styles, widgets::Ui};

fn bounds(ui: &Ui, view: &zgx::compose::ViewHandle, id: &str) -> Rect {
    ui.scene.borrow().bounds(view.find(id).unwrap())
}

#[test]
fn a_query_takes_its_styles_in_the_frame_its_container_crosses() {
    let mut ui = Ui::new(1200., 400.);
    let wide = ui.signal(1000_f32);
    let read = wide.clone();
    let view = ui.mount(
        row()
            .container()
            .p(10.)
            .reactive_style(move || Styles::new().w(read.get()))
            .child(
                div()
                    .id("side")
                    .h(20.)
                    .w(0.)
                    .when_container(Query::new().min_w(600.), |s| s.w(200.)),
            ),
    );
    ui.prepare_frame();
    assert_eq!(
        bounds(&ui, &view, "side").width,
        200.,
        "980 of content: wide"
    );
    wide.set(500.);
    ui.prepare_frame();
    assert_eq!(
        bounds(&ui, &view, "side").width,
        0.,
        "480 of content: narrow"
    );
    // Its content box: 620 less 20 of padding is exactly the bound.
    wide.set(620.);
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &view, "side").width, 200.);
}

#[test]
fn a_resize_that_crosses_no_bound_restyles_nothing() {
    let mut ui = Ui::new(1200., 400.);
    let wide = ui.signal(1000_f32);
    let read = wide.clone();
    let runs = Rc::new(Cell::new(0));
    let counted = runs.clone();
    ui.mount(
        row()
            .container()
            .reactive_style(move || Styles::new().w(read.get()))
            .child(
                div()
                    .h(20.)
                    .reactive_style(move || {
                        counted.set(counted.get() + 1);
                        Styles::new()
                    })
                    .when_container(Query::new().min_w(600.), |s| s.w(200.)),
            ),
    );
    ui.prepare_frame();
    let settled = runs.get();
    for width in [900., 800., 700.] {
        wide.set(width);
        ui.prepare_frame();
    }
    assert_eq!(runs.get(), settled, "still past 600: no style ran");
    wide.set(550.);
    ui.prepare_frame();
    assert_eq!(runs.get(), settled + 1, "across it: once");
}

#[test]
fn the_nearest_container_above_answers() {
    let mut ui = Ui::new(1200., 400.);
    let view = ui.mount(
        row().container().w(1000.).child(
            column().container().w(300.).child(
                div()
                    .id("item")
                    .h(10.)
                    .w(10.)
                    .when_container(Query::new().max_w(400.), |s| s.w(50.)),
            ),
        ),
    );
    ui.prepare_frame();
    assert_eq!(
        bounds(&ui, &view, "item").width,
        50.,
        "its column, 300 wide"
    );
}

#[test]
fn without_a_container_a_query_never_holds() {
    let mut ui = Ui::new(1200., 400.);
    let view = ui.mount(
        row().w(1000.).child(
            div()
                .id("item")
                .h(10.)
                .w(10.)
                .when_container(Query::new(), |s| s.w(50.)),
        ),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &view, "item").width, 10.);
}

#[test]
fn a_containers_content_doesnt_size_its_width() {
    let mut ui = Ui::new(1200., 400.);
    let heard = Rc::new(std::cell::RefCell::new(Vec::new()));
    let told = heard.clone();
    let _listener = zgx::issues::on_issue(&ui.runtime, move |message| {
        told.borrow_mut().push(message.to_string())
    });
    let view = ui.mount(
        row().w(1000.).items_start().child(
            column()
                .id("bare")
                .container()
                .px(4.)
                .child(div().w(300.).h(30.)),
        ),
    );
    ui.prepare_frame();
    let bare = bounds(&ui, &view, "bare");
    assert_eq!(
        (bare.width, bare.height),
        (8., 30.),
        "its padding wide, its content tall"
    );
    if cfg!(debug_assertions) {
        assert!(
            heard.borrow().iter().any(|m| m.contains("0 wide")),
            "a debug build says so: {:?}",
            heard.borrow()
        );
    }
}

#[test]
fn a_stretched_or_growing_container_takes_the_room_it_is_given() {
    let mut ui = Ui::new(1200., 400.);
    let view = ui.mount(
        column()
            .w(600.)
            .child(
                column()
                    .id("stretched")
                    .container()
                    .child(div().w(10.).h(10.)),
            )
            .child(
                row().w(600.).child(div().w(100.).h(10.)).child(
                    row()
                        .id("grown")
                        .container()
                        .grow()
                        .child(div().w(10.).h(10.)),
                ),
            ),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, &view, "stretched").width, 600.);
    assert_eq!(bounds(&ui, &view, "grown").width, 500.);
}
