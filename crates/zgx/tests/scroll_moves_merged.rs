//! Scroll content moved as one: layers in the same view moved the same way
//! (a text area's marks under its lines) are copied as one move, painting
//! only what comes into view. Moves of other distances, in other views over
//! the same pixels, or nested one in another, repaint as ever.
use zgx::scene::{Color, Layout, NodeId, NodeKind, QuadStyle, Rect, Scene, Style, Transform};

fn sized(width: f32, height: f32) -> Style {
    Style {
        width: Some(width),
        height: Some(height),
        ..Style::default()
    }
}
/// A layer of scroll content in `view`: a column of 20-point rows.
fn layer(scene: &mut Scene, view: NodeId, shade: u8) -> NodeId {
    let layer = scene.append(view, NodeKind::Container(Layout::Column), Style::default());
    scene.set_scroll_copy(layer, true);
    for i in 0..30u8 {
        scene.append(
            layer,
            NodeKind::Quad(QuadStyle {
                fill: Color(shade, i, 90, 255),
                ..Default::default()
            }),
            sized(200., 20.),
        );
    }
    layer
}
/// A clipping view 200 by 100 at `(x, y)`.
fn view(scene: &mut Scene, x: f32, y: f32) -> NodeId {
    let root = scene.root();
    let view = scene.append(
        root,
        NodeKind::Container(Layout::Overlay),
        Style {
            clip: true,
            absolute: true,
            ..sized(200., 100.)
        },
    );
    scene.set_transform(view, Transform { x, y });
    view
}
fn scene() -> Scene {
    let mut scene = Scene::new(400., 300.);
    scene.set_pixel_scale(1.);
    let root = scene.root();
    scene.set_kind(root, NodeKind::Container(Layout::Overlay));
    scene
}
fn painted(damage: &[Rect]) -> f32 {
    damage.iter().map(|r| r.width * r.height).sum()
}
fn by(scene: &mut Scene, id: NodeId, y: f32) {
    scene.set_transform(id, Transform { x: 0., y });
}

#[test]
fn two_layers_moved_alike_in_one_view_copy_once() {
    let mut scene = scene();
    let shown = view(&mut scene, 0., 0.);
    let (under, over) = (layer(&mut scene, shown, 10), layer(&mut scene, shown, 200));
    scene.flush();
    by(&mut scene, under, -12.);
    by(&mut scene, over, -12.);
    let report = scene.flush();
    let moves = scene.scroll_moves().to_vec();
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert_eq!((moves[0].dx, moves[0].dy), (0., -12.));
    // The strip that came into view, 200 by 12, not the view.
    assert!(
        painted(&report.damage) <= 200. * 12. + 1.,
        "{:?}",
        report.damage
    );
}

#[test]
fn layers_moved_by_other_distances_repaint() {
    let mut scene = scene();
    let shown = view(&mut scene, 0., 0.);
    let (under, over) = (layer(&mut scene, shown, 10), layer(&mut scene, shown, 200));
    scene.flush();
    by(&mut scene, under, -12.);
    by(&mut scene, over, -20.);
    let report = scene.flush();
    assert!(
        scene.scroll_moves().is_empty(),
        "{:?}",
        scene.scroll_moves()
    );
    assert!(
        painted(&report.damage) >= 200. * 100. - 1.,
        "{:?}",
        report.damage
    );
}

#[test]
fn moves_alike_in_overlapping_views_repaint() {
    let mut scene = scene();
    // Two views over some of the same pixels, their content moved alike.
    let (left, right) = (view(&mut scene, 0., 0.), view(&mut scene, 100., 50.));
    let (a, b) = (layer(&mut scene, left, 10), layer(&mut scene, right, 200));
    scene.flush();
    by(&mut scene, a, -12.);
    by(&mut scene, b, -12.);
    scene.flush();
    assert!(
        scene.scroll_moves().is_empty(),
        "{:?}",
        scene.scroll_moves()
    );
}

#[test]
fn a_layer_in_a_layer_moved_alike_is_not_one_move() {
    let mut scene = scene();
    let shown = view(&mut scene, 0., 0.);
    let outer = layer(&mut scene, shown, 10);
    let inner = layer(&mut scene, outer, 200);
    scene.flush();
    // The inner moves by its own and the outer's: twice in the window.
    by(&mut scene, outer, -12.);
    by(&mut scene, inner, -12.);
    scene.flush();
    assert_eq!(scene.scroll_moves().len(), 0, "{:?}", scene.scroll_moves());
}
