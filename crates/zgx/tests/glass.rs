//! A glass box off iOS (headless): it lays out and holds children like a
//! `div`, draws the stand-in look, and leaves the window with no native views,
//! so it stays on one plane.
use std::rc::Rc;
use zgx::{
    compose::prelude::*,
    glass::{GlassStyle, glass, glass_group},
    native_view::NativeViews,
    widgets::Ui,
};

#[test]
fn glass_is_ordinary_paint_without_the_systems_material() {
    let mut ui = Ui::new(400., 300.);
    let natives = NativeViews::new();
    natives.set_supported(true);
    let held = Rc::new(std::cell::Cell::new(None));
    let child = held.clone();
    let view = provide(
        natives.clone(),
        glass_group(40.).w(400.).h(300.).child(
            glass(GlassStyle::default)
                .absolute()
                .left(10.)
                .top(20.)
                .w(120.)
                .h(80.)
                .child(div().w(50.).h(30.).on_mount(move |handle| {
                    child.set(Some(handle.node()));
                })),
        ),
    );
    ui.mount(view);
    ui.prepare_frame();
    assert!(natives.is_empty(), "no native view off iOS");
    let scene = ui.scene.borrow();
    let inside = scene.bounds(held.get().expect("the child mounted"));
    assert_eq!((inside.x, inside.y), (10., 20.), "children sit in its box");
}
