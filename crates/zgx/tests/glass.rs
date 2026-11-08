//! A glass box: the system's where the host has it, its material drawn where not.
use zgx::compose::prelude::*;
use zgx::glass::{GlassStyle, GlassVariant, glass};
use zgx::native_view::{NativeContent, NativeViews};
use zgx::widgets::Ui;

fn pane() -> View {
    glass(GlassStyle::default)
        .w(100.)
        .h(100.)
        .child(text("Over"))
}

#[test]
fn with_no_system_glass_its_material_is_drawn() {
    let natives = NativeViews::new();
    natives.set_supported(true);
    let mut ui = Ui::new(400., 300.);
    ui.mount(provide(natives.clone(), pane()));
    ui.prepare_frame();
    assert!(natives.is_empty());
}

#[test]
fn with_system_glass_a_native_view_stands_in_following_its_style() {
    let natives = NativeViews::new();
    natives.set_supported(true);
    natives.set_glass_supported(true);
    let mut ui = Ui::new(400., 300.);
    let variant = ui.signal(GlassVariant::Regular);
    let read = variant.clone();
    ui.mount(provide(
        natives.clone(),
        glass(move || GlassStyle {
            variant: read.get(),
            ..Default::default()
        })
        .w(100.)
        .h(100.),
    ));
    ui.prepare_frame();
    let [(node, content)] = &natives.views()[..] else {
        panic!("one native view")
    };
    assert!(ui.scene.borrow().is_platform_painted(*node));
    let NativeContent::Glass(pane) = content else {
        panic!("glass")
    };
    assert_eq!(pane.style().variant, GlassVariant::Regular);
    variant.set(GlassVariant::Clear);
    ui.prepare_frame();
    assert_eq!(pane.style().variant, GlassVariant::Clear);
}

#[test]
fn drawn_glass_rounds_each_corner_by_its_own_radius() {
    // A sheet's: round on top, its bottom as a screen's corners less its inset.
    let corners = zgx::decoration::Corners {
        top_left: 56.,
        top_right: 56.,
        bottom_right: 38.,
        bottom_left: 38.,
    };
    let natives = NativeViews::new();
    natives.set_supported(true);
    let mut ui = Ui::new(400., 300.);
    ui.mount(provide(
        natives.clone(),
        glass(move || GlassStyle {
            corners: Some(corners),
            ..Default::default()
        })
        .w(200.)
        .h(200.),
    ));
    ui.prepare_frame();
    let scene = ui.scene.borrow();
    let mut todo = vec![scene.root()];
    let mut found = None;
    while let Some(id) = todo.pop() {
        todo.extend_from_slice(scene.children(id));
        if let zgx::scene::NodeKind::Quad(quad) | zgx::scene::NodeKind::Panel { quad, .. } =
            scene.kind(id)
            && let Some(c) = quad.decoration.as_deref().and_then(|d| d.corners)
        {
            found = Some(c);
        }
    }
    assert_eq!(found, Some(corners));
}
