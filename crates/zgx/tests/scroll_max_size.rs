//! A scroll view given only a cap is as large as its content up to it, as CSS's overflow box is,
//! not as large as its default size (320 by 240).
use zgx::scene::Align;
use zgx::{compose::prelude::*, scene::Rect, widgets::Ui};

fn bounds(ui: &Ui, id: &str, view: &zgx::compose::ViewHandle) -> Rect {
    ui.scene.borrow().bounds(view.find(id).unwrap())
}

fn mounted(content_h: f32, content_w: f32) -> (Ui, zgx::compose::ViewHandle) {
    let mut ui = Ui::new(800., 800.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        column().child(
            scroll(offset)
                .id("list")
                .max_h(192.)
                .child(div().w(content_w).h(content_h)),
        ),
    );
    ui.prepare_frame();
    (ui, view)
}

#[test]
fn a_scroll_with_only_a_cap_is_as_tall_as_its_shorter_content() {
    let (ui, view) = mounted(136., 100.);
    assert_eq!(bounds(&ui, "list", &view).height, 136.);
}

#[test]
fn a_scroll_with_a_cap_stops_at_it_for_longer_content() {
    let (ui, view) = mounted(400., 100.);
    assert_eq!(bounds(&ui, "list", &view).height, 192.);
}

#[test]
fn a_scroll_grows_with_its_content_up_to_its_cap() {
    let mut ui = Ui::new(800., 800.);
    let offset = ui.signal(0.);
    let tall = ui.signal(46.);
    let read = tall.clone();
    let view = ui.mount(
        column().child(
            scroll(offset).id("list").max_h(256.).child(
                div()
                    .w(100.)
                    .reactive_style(move || zgx::style::Styles::new().h(read.get())),
            ),
        ),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, "list", &view).height, 46.);
    for (content, shown) in [(120., 120.), (256., 256.), (300., 256.), (60., 60.)] {
        tall.set(content);
        ui.prepare_frame();
        assert_eq!(
            bounds(&ui, "list", &view).height,
            shown,
            "content {content}"
        );
    }
}

#[test]
fn a_scroll_with_no_cap_keeps_its_default_size() {
    let mut ui = Ui::new(800., 800.);
    let offset = ui.signal(0.);
    let view = ui.mount(column().child(scroll(offset).id("list").child(div().w(100.).h(50.))));
    ui.prepare_frame();
    assert_eq!(bounds(&ui, "list", &view).height, 240.);
}

#[test]
fn a_scroll_with_a_width_cap_is_as_wide_as_its_content_up_to_it() {
    for (content, shown) in [(120., 120.), (400., 200.)] {
        let mut ui = Ui::new(800., 800.);
        let offset = ui.signal(0.);
        let view = ui.mount(
            column().child(
                scroll(offset)
                    .id("list")
                    .max_w(200.)
                    .child(div().w(content).h(50.)),
            ),
        );
        ui.prepare_frame();
        assert_eq!(bounds(&ui, "list", &view).width, shown, "content {content}");
    }
}

#[test]
fn an_explicit_height_still_holds_with_a_cap() {
    let mut ui = Ui::new(800., 800.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        column().child(
            scroll(offset)
                .id("list")
                .h(100.)
                .max_h(192.)
                .child(div().w(100.).h(400.)),
        ),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, "list", &view).height, 100.);
}

#[test]
fn a_capped_scroll_still_scrolls_what_passes_its_cap() {
    let mut ui = Ui::new(800., 800.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        column().child(
            scroll(offset.clone())
                .id("list")
                .max_h(192.)
                .child(div().w(100.).h(400.)),
        ),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, "list", &view).height, 192.);
    ui.dispatch(zgx::input::InputEvent::Scroll {
        x: 10.,
        y: 10.,
        delta_x: 0.,
        delta_y: 1000.,
    });
    ui.prepare_frame();
    assert_eq!(offset.get(), 208., "to the end of 400 in a 192 view");
}

#[test]
fn a_percent_width_in_a_capped_scroll_resolves_against_the_viewport() {
    let mut ui = Ui::new(800., 800.);
    let offset = ui.signal(0.);
    let view = ui.mount(
        column().w(500.).child(
            scroll(offset).id("list").max_h(192.).child(
                div()
                    .id("row")
                    .align_self(Align::Start)
                    .min_w_percent(100.)
                    .h(40.)
                    .child(div().w(100.).h(40.)),
            ),
        ),
    );
    ui.prepare_frame();
    assert_eq!(bounds(&ui, "list", &view).width, 500.);
    assert_eq!(bounds(&ui, "row", &view).width, 500., "fills the viewport");
}
