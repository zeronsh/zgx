//! A region laid out as its child (a `switch`'s, a `show`'s) with nothing
//! to style takes no style effect of its own: its children inherit what is
//! around it as if it weren't there, and follow it as it changes. A region
//! with a style, a reactive style or focus keeps its own. The headless
//! core's approximate layout gives a text a cell 0.6 of its size wide per
//! grapheme: "abcd" at size 10 is 24 wide.
use zgx::{compose::prelude::*, widgets::Ui};

fn width(ui: &Ui, view: &zgx::compose::ViewHandle, id: &str) -> f32 {
    ui.scene.borrow().bounds(view.find(id).unwrap()).width
}

#[test]
fn an_unstyled_region_passes_on_the_typography_around_it_as_it_changes() {
    let mut ui = Ui::new(400., 300.);
    let size = ui.signal(10.);
    let read = size.clone();
    let view = ui.mount(
        column()
            .items_start()
            .reactive_style(move || Styles::new().text_size(read.get()))
            .child(switch(|| 0, |_, _| text("abcd").id("switched")))
            .child(switch(|| 0, |_, _| text("abcd").id("contents")).contents())
            .child(text("abcd").id("beside")),
    );
    ui.prepare_frame();
    assert_eq!(width(&ui, &view, "beside"), 24.);
    assert_eq!(width(&ui, &view, "switched"), 24.);
    assert_eq!(width(&ui, &view, "contents"), 24.);
    // The size around them changes: they follow, as the text beside them.
    size.set(20.);
    ui.prepare_frame();
    assert_eq!(width(&ui, &view, "beside"), 48.);
    assert_eq!(width(&ui, &view, "switched"), 48.);
    assert_eq!(width(&ui, &view, "contents"), 48.);
}

#[test]
fn a_region_with_a_reactive_style_keeps_its_effect() {
    let mut ui = Ui::new(400., 300.);
    let size = ui.signal(10.);
    let read = size.clone();
    let view = ui.mount(
        column().items_start().text_size(10.).child(
            switch(|| 0, |_, _| text("abcd").id("styled"))
                .contents()
                .reactive_style(move || Styles::new().text_size(read.get())),
        ),
    );
    ui.prepare_frame();
    assert_eq!(width(&ui, &view, "styled"), 24.);
    size.set(15.);
    ui.prepare_frame();
    assert_eq!(
        width(&ui, &view, "styled"),
        36.,
        "the region's own size, followed"
    );
}

#[test]
fn a_sized_or_focusable_region_keeps_its_box_and_focus() {
    let mut ui = Ui::new(400., 300.);
    let view = ui.mount(
        column()
            .items_start()
            .text_size(10.)
            .child(switch(|| 0, |_, _| text("abcd")).w(100.).id("sized"))
            .child(
                switch(|| 0, |_, _| text("abcd"))
                    .contents()
                    .focusable(true)
                    .id("focusable"),
            ),
    );
    ui.prepare_frame();
    assert_eq!(width(&ui, &view, "sized"), 100., "a box of its own");
    let focusable = view.find("focusable").unwrap();
    assert!(ui.input.focus(&ui.scene, Some(focusable)), "it takes focus");
    assert_eq!(ui.input.focused(), Some(focusable));
}
