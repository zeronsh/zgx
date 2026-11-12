//! A reactive `outline_color` is a style patch of its own: it keeps the outline width the node
//! set (0 included), and draws 1 point when none was set.
use zgx::{
    compose::prelude::*,
    scene::{Color, NodeKind},
    style::Styles,
    widgets::Ui,
};

const RED: Color = Color(255, 0, 0, 255);

fn outline_of(ui: &Ui, view: &zgx::compose::ViewHandle) -> Option<(f32, Color)> {
    let scene = ui.scene.borrow();
    match scene.kind(view.node()) {
        NodeKind::Panel { quad, .. } => quad.outline.map(|o| (o.width, o.color)),
        other => panic!("{other:?}"),
    }
}

fn mounted(view: View) -> (Ui, zgx::compose::ViewHandle) {
    let mut ui = Ui::new(100., 100.);
    let handle = ui.mount(view.size(20., 20.).bg(Color(16, 16, 16, 255)));
    ui.prepare_frame();
    (ui, handle)
}

#[test]
fn a_reactive_outline_colour_keeps_a_width_of_zero() {
    let (ui, view) = mounted(
        column()
            .outline_width(0.)
            .reactive_style(|| Styles::new().outline_color(RED)),
    );
    assert_eq!(outline_of(&ui, &view), Some((0., RED)));
}

#[test]
fn a_reactive_outline_colour_keeps_the_width_the_node_set() {
    let (ui, view) = mounted(
        column()
            .outline_width(3.)
            .reactive_style(|| Styles::new().outline_color(RED)),
    );
    assert_eq!(outline_of(&ui, &view), Some((3., RED)));
}

#[test]
fn a_reactive_outline_colour_alone_draws_one_point() {
    let (ui, view) = mounted(column().reactive_style(|| Styles::new().outline_color(RED)));
    assert_eq!(outline_of(&ui, &view), Some((1., RED)));
}

#[test]
fn a_static_outline_colour_keeps_a_width_of_zero() {
    let (ui, view) = mounted(column().outline_width(0.).outline_color(RED));
    assert_eq!(outline_of(&ui, &view), Some((0., RED)));
}
