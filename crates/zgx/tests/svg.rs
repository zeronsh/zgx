use std::sync::Arc;
use zgx::{compose::prelude::*, scene::NodeKind, widgets::Ui};
#[test]
fn svg_component_retains_semantics_size_and_reactive_transform() {
    let mut ui = Ui::new(160., 120.);
    let angle = ui.signal(0.);
    let source = Arc::new(SvgData::new(&b"<svg/>"[..]).unwrap());
    let view = ui.mount(
        svg_signal("Icon", {
            let angle = angle.clone();
            move || Arc::new(source.transformed(Affine::rotation(angle.get())))
        })
        .id("icon")
        .size(40., 20.)
        .svg_tint(rgb(0x55aa88)),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let root = view.find("icon").unwrap();
    let node = ui.scene.borrow().children(root)[0];
    assert!(matches!(ui.scene.borrow().kind(node), NodeKind::Svg(_)));
    assert_eq!(ui.scene.borrow().bounds(root).width, 40.);
    assert_eq!(ui.semantics.borrow().get(root).unwrap().label, "Icon");
    angle.set(0.5);
    ui.prepare_frame();
    let report = ui.scene.borrow_mut().flush();
    assert_eq!(report.layout_nodes, 0);
    assert!(!report.damage.is_empty());
    assert_eq!(ui.scene.borrow().children(root)[0], node);
}

fn wide_artwork() -> Arc<SvgData> {
    Arc::new(
        SvgData::new(&br#"<svg viewBox="0 0 32 16"><rect width="32" height="16"/></svg>"#[..])
            .unwrap(),
    )
}

/// Where layout put `icon`'s artwork, and its content box (inside its
/// padding and border), in window points.
fn artwork_and_content_box(
    ui: &Ui,
    icon: zgx::scene::NodeId,
) -> (zgx::scene::Rect, zgx::scene::Rect) {
    let scene = ui.scene.borrow();
    let artwork = scene.children(icon)[0];
    assert!(matches!(scene.kind(artwork), NodeKind::Svg(_)));
    let (bounds, inset) = (scene.layout_bounds(icon), scene.content_insets(icon));
    let content = zgx::scene::Rect::new(
        bounds.x + inset.left,
        bounds.y + inset.top,
        bounds.width - inset.left - inset.right,
        bounds.height - inset.top - inset.bottom,
    );
    (scene.layout_bounds(artwork), content)
}

/// An icon's case: its name, and the icon (in what holds it, if anything).
type Case = (&'static str, fn() -> View);

#[test]
fn an_icon_draws_in_its_content_box_from_its_first_layout() {
    let icons: [Case; 6] = [
        ("default", || svg("Icon", wide_artwork())),
        ("non-square", || svg("Icon", wide_artwork()).size(40., 20.)),
        ("wide", || svg("Icon", wide_artwork()).w_full().h(24.)),
        ("padded", || {
            svg("Icon", wide_artwork()).size(40., 30.).p(4.)
        }),
        ("bordered", || {
            svg("Icon", wide_artwork()).size(40., 30.).border(2.)
        }),
        ("grown", || {
            row()
                .w(120.)
                .child(svg("Icon", wide_artwork()).grow().h(16.))
        }),
    ];
    for (name, icon) in icons {
        // A relative block above takes its absolute descendants, but not
        // an icon's own artwork.
        for relative in [false, true] {
            let mut ui = Ui::new(300., 200.);
            let icon = column()
                .p(10.)
                .child(div().id("holder").w(200.).child(icon()));
            let page = match relative {
                true => column().relative().p(5.).child(icon),
                false => column().child(icon),
            };
            let view = ui.mount(page);
            ui.prepare_frame();
            let holder = view.find("holder").unwrap();
            let mut node = ui.scene.borrow().children(holder)[0];
            // The grown icon sits in a row of its own.
            if !matches!(ui.semantics.borrow().get(node), Some(s) if s.label == "Icon") {
                node = ui.scene.borrow().children(node)[0];
            }
            let (artwork, content) = artwork_and_content_box(&ui, node);
            assert_eq!(
                artwork, content,
                "{name}, under a relative block: {relative}"
            );
            assert!(content.width > 0. && content.height > 0., "{name}");
        }
    }
}

#[test]
fn an_icon_resized_draws_at_its_new_size() {
    let mut ui = Ui::new(300., 200.);
    let width = ui.signal(40.);
    let read = width.clone();
    let view = ui.mount(
        column().child(
            svg("Icon", wide_artwork())
                .id("icon")
                .h(20.)
                .reactive_style(move || zgx::style::Styles::new().w(read.get())),
        ),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let icon = view.find("icon").unwrap();
    width.set(90.);
    ui.prepare_frame();
    let report = ui.scene.borrow_mut().flush();
    let (artwork, content) = artwork_and_content_box(&ui, icon);
    assert_eq!(artwork, content);
    assert_eq!(artwork.width, 90.);
    let artwork_node = ui.scene.borrow().children(icon)[0];
    let drawn = ui.scene.borrow().layout_bounds(artwork_node);
    assert!(
        report
            .damage
            .iter()
            .any(|d| d.x <= drawn.x && d.x + d.width >= drawn.x + drawn.width),
        "its new box is repainted: {:?}",
        report.damage
    );
}
