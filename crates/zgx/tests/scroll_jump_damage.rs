//! A scroll that moves its content too far to copy its pixels (or one a
//! layer holds, which it can't copy) repaints what it showed before the
//! move, whatever else changed in it in that flush: a box gone or faded
//! out after the content moved was damaged where it lies at the new
//! offset, out of the view, and the content's walk at the old offset no
//! longer holds it. Zeron left a turn's text on screen this way, its
//! transcript jumping as the turn's rows were rebuilt.
use zgx::{
    compose::prelude::*,
    scene::{NodeId, Rect, Scene},
    widgets::Ui,
};

/// Whether `damage` covers every point of `rect` (on a 1-point grid).
fn covers(damage: &[Rect], rect: Rect) -> bool {
    let inside = |x: f32, y: f32| {
        damage
            .iter()
            .any(|d| x >= d.x && x < d.x + d.width && y >= d.y && y < d.y + d.height)
    };
    let (columns, rows) = (rect.width as i32, rect.height as i32);
    (0..columns)
        .all(|i| (0..rows).all(|j| inside(rect.x + i as f32 + 0.5, rect.y + j as f32 + 0.5)))
}

/// A bar at the top of a view's content (painted at y 0..20), in a box that
/// keeps its size, then a long stretch painting nothing. In one flush the
/// content jumps `by` points and `change` acts on the bar after it moved:
/// the damage that flush reports.
fn jumped(by: f32, change: impl FnOnce(&mut Scene, NodeId)) -> Vec<Rect> {
    let mut ui = Ui::new(200., 200.);
    let offset = ui.signal(0_f32);
    let view = ui.mount(
        scroll(offset.clone())
            .w(200.)
            .h(200.)
            .bg(rgb(0x000000))
            .child(
                column()
                    .w(200.)
                    .child(
                        div()
                            .w(200.)
                            .h(20.)
                            .child(div().id("bar").w(200.).h(20.).bg(rgb(0xffffff))),
                    )
                    .child(div().w(200.).h(1000.)),
            ),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let bar = view.find("bar").expect("mounted");
    offset.set(by);
    ui.prepare_frame();
    change(&mut ui.scene.borrow_mut(), bar);
    let damage = ui.scene.borrow_mut().flush().damage;
    assert!(
        ui.scene.borrow().scroll_moves().is_empty(),
        "the move copies nothing"
    );
    damage
}

/// Where the bar was painted.
const WAS: Rect = Rect {
    x: 0.,
    y: 0.,
    width: 200.,
    height: 20.,
};

#[test]
fn a_box_gone_as_its_scroll_jumps_repaints_where_it_was() {
    let damage = jumped(600., |scene, bar| scene.remove(bar));
    assert!(covers(&damage, WAS), "where the bar was: {damage:?}");
}

#[test]
fn a_box_faded_out_as_its_scroll_jumps_repaints_where_it_was() {
    let damage = jumped(600., |scene, bar| {
        let effects = zgx::scene::Effects {
            opacity: 0.,
            ..scene.effects(bar)
        };
        scene.set_effects(bar, effects);
    });
    assert!(covers(&damage, WAS), "where the bar was: {damage:?}");
}

/// The same jump with nothing else changed repaints where the bar was too
/// (it shows there no more).
#[test]
fn a_jump_alone_repaints_what_the_view_showed() {
    let damage = jumped(600., |_, _| {});
    assert!(covers(&damage, WAS), "where the bar was: {damage:?}");
}
