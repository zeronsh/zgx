//! A point outside a subtree's boxes skips the subtree's hit walk once it is
//! proved nothing under it reaches past that side: icons included. An image
//! or SVG is hit where it paints, which for one drawn in its box (an icon,
//! its transform the identity) is inside its box, so a sidebar of rows with
//! icons is skipped as plain rows are; one transformed past its box is still
//! hit out there.
use std::sync::Arc;
use zgx::{affine::Affine, compose::prelude::*, input::InputEvent, svg::SvgData, widgets::Ui};

fn icon() -> Arc<SvgData> {
    Arc::new(
        SvgData::new(&br#"<svg viewBox="0 0 16 16"><rect width="16" height="16"/></svg>"#[..])
            .unwrap(),
    )
}

/// Nodes the hit walk visits for each of ten moves over a page beside a
/// sidebar of 500 rows, each with an icon if `icons`.
fn walked_beside_a_sidebar(icons: bool) -> Vec<u64> {
    let mut ui = Ui::new(1200., 800.);
    let icon = icon();
    let sidebar = column().w(200.).shrink_0().children((0..500).map(move |i| {
        let row = row().h(24.).w(200.).hover(|s| s.bg(rgb(0x202830)));
        let row = match icons {
            true => row.child(svg("Folder", icon.clone()).size(16., 16.)),
            false => row,
        };
        row.child(text(format!("Folder {i}")))
    }));
    let page =
        column().children((0..60).map(|_| row().h(30.).w(800.).hover(|s| s.bg(rgb(0x111111)))));
    ui.mount(row().child(sidebar).child(page));
    ui.prepare_frame();
    (0..10)
        .map(|step| {
            let before = ui.scene.borrow().hit_nodes_walked();
            ui.dispatch(InputEvent::PointerMove {
                x: 400. + step as f32 * 7.,
                y: 20. + step as f32 * 9.,
            });
            let walked = ui.scene.borrow().hit_nodes_walked() - before;
            ui.prepare_frame();
            walked
        })
        .collect()
}

#[test]
fn a_sidebar_of_rows_with_icons_is_skipped_as_plain_rows_are() {
    let plain = walked_beside_a_sidebar(false);
    let icons = walked_beside_a_sidebar(true);
    assert!(plain.iter().all(|n| *n < 100), "{plain:?}");
    assert_eq!(icons, plain, "the icons' rows are not walked");
}

#[test]
fn an_svg_transformed_past_its_box_is_still_hit_there() {
    let mut ui = Ui::new(400., 300.);
    // Drawn three times its size about its centre: 16 points past its box
    // on each side.
    let big = Arc::new(icon().transformed(Affine::scale(3., 3.)));
    let view = ui.mount(
        column().p(100.).child(
            div()
                .size(16., 16.)
                .child(svg("Big", big).id("big").size(16., 16.)),
        ),
    );
    ui.prepare_frame();
    let big = view.find("big").unwrap();
    // The SVG drawn in it: what is hit where it paints.
    let drawn = |hits: &[zgx::scene::NodeId]| {
        let scene = ui.scene.borrow();
        hits.iter().any(|hit| scene.parent(*hit) == Some(big))
    };
    // Past its box to the right and below, inside where it paints; asked
    // twice, as the walk proves directions the first time and reuses them.
    for _ in 0..2 {
        for (x, y) in [
            (100. + 16. + 10., 108.),
            (108., 100. + 16. + 10.),
            (108., 108.),
        ] {
            let hits = ui.scene.borrow().hit_test_all(x, y);
            assert!(drawn(&hits), "({x}, {y}): {hits:?}");
        }
        let far = ui.scene.borrow().hit_test_all(100. + 16. + 30., 108.);
        assert!(!drawn(&far), "past where it paints: {far:?}");
    }
}
