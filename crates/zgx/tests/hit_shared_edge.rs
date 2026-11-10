//! A point just past a row's bottom, inside its child as the hit walk adds
//! the scroll's translation, hits the child: a row whose child ends where it
//! does (in its own frame) can sit an ulp past it once translated, and a
//! skip proved in the row's frame must not take that ulp away.
use zgx::{compose::prelude::*, scene::NodeId, widgets::Ui};

/// `id`'s box as the hit walk has it: its layout box moved by the
/// translations of the nodes from the root down to it, summed in that order.
fn walked(ui: &Ui, id: NodeId) -> (f32, f32, f32, f32) {
    let scene = ui.scene.borrow();
    let mut chain: Vec<NodeId> = std::iter::once(id).chain(scene.ancestors(id)).collect();
    chain.reverse();
    let (mut x, mut y) = (0_f32, 0_f32);
    for node in chain {
        let t = scene.transform(node);
        x += t.x;
        y += t.y;
    }
    let b = scene.layout_bounds(id);
    let (left, top) = (b.x + x, b.y + y);
    (left, top, left + b.width, top + b.height)
}

/// Whether the child ends where the row does in the row's own frame (what
/// a proof of the row's subtree sums): layout box and size, no translation.
fn ends_with_row(ui: &Ui, row: NodeId, child: NodeId) -> bool {
    let scene = ui.scene.borrow();
    let (r, c) = (scene.layout_bounds(row), scene.layout_bounds(child));
    r.y + r.height == c.y + c.height
}

/// The rows showing in the window whose child the walk puts past their
/// bottom, and a point between: past the row, inside the child.
fn slivers(ui: &Ui, rows: &[(NodeId, NodeId)], window: f32) -> Vec<(NodeId, NodeId, f32)> {
    rows.iter()
        .filter(|(row, child)| ends_with_row(ui, *row, *child))
        .filter_map(|&(row, child)| {
            let (.., row_bottom) = walked(ui, row);
            let (_, child_top, _, child_bottom) = walked(ui, child);
            (row_bottom > 0. && row_bottom < window && child_bottom > row_bottom).then_some((
                row,
                child,
                row_bottom.max(child_top),
            ))
        })
        .collect()
}

fn hits(ui: &Ui, x: f32, y: f32) -> Vec<NodeId> {
    ui.scene.borrow().hit_test_all(x, y)
}

#[test]
fn a_point_past_a_scrolled_rows_bottom_inside_its_child_hits_the_child() {
    // Laid out at 8864.8, 30.15 tall, its child 1.9 down and 28.25 tall:
    // both end at the same f32 in the row's frame. Scrolled up 8613.2168,
    // the walk puts the row's bottom at 281.73300 and the child's at
    // 281.73340.
    let mut ui = Ui::new(300., 1000.);
    let offset = ui.signal(0_f32);
    let view = ui.mount(
        scroll(offset.clone()).w(300.).h(1000.).child(
            column()
                .w(300.)
                .child(div().w(300.).h(8864.8))
                .child(
                    div()
                        .id("row")
                        .w(300.)
                        .h(30.15)
                        .pt(1.9)
                        .on_click(|| {})
                        .child(div().id("child").w(200.).h(28.25).on_click(|| {})),
                )
                .child(div().w(300.).h(2000.)),
        ),
    );
    ui.prepare_frame();
    // Scrolled once laid out, as a wheel does.
    offset.set(8_613.217);
    ui.prepare_frame();
    let (row, child) = (view.find("row").unwrap(), view.find("child").unwrap());
    assert!(ends_with_row(&ui, row, child), "laid out as above");
    let found = slivers(&ui, &[(row, child)], 1000.);
    let [(_, _, y)] = found[..] else {
        panic!(
            "the child walks past the row: {:?} {:?}",
            walked(&ui, row),
            walked(&ui, child)
        );
    };
    assert!(
        hits(&ui, 50., y).contains(&child),
        "at {y}: the child, as the walk places it, is hit"
    );
}

#[test]
fn every_scrolled_row_whose_child_walks_past_it_hits_the_child_there() {
    // Rows of fractional heights, each child a fractional way down and
    // filling the rest, deep in a long list: scrolled to offsets from 8,000
    // to 40,000 points.
    let mut ui = Ui::new(300., 1000.);
    let offset = ui.signal(0_f32);
    let count = 2000;
    let view = ui.mount(
        scroll(offset.clone())
            .w(300.)
            .h(1000.)
            .child(column().w(300.).children((0..count).map(|i| {
                let height = 18. + (i * 37 % 101) as f32 * 0.0731;
                let pad = 0.3 + (i * 13 % 17) as f32 * 0.137;
                div()
                    .id(format!("row-{i}"))
                    .w(300.)
                    .h(height)
                    .pt(pad)
                    .on_click(|| {})
                    .child(
                        div()
                            .id(format!("child-{i}"))
                            .w(200.)
                            .h(height - pad)
                            .on_click(|| {}),
                    )
            }))),
    );
    ui.prepare_frame();
    let rows: Vec<(NodeId, NodeId)> = (0..count)
        .map(|i| {
            (
                view.find(&format!("row-{i}")).unwrap(),
                view.find(&format!("child-{i}")).unwrap(),
            )
        })
        .collect();
    let mut checked = 0;
    for step in 0..64 {
        offset.set(8000. + step as f32 * 499.37);
        ui.prepare_frame();
        for (row, child, y) in slivers(&ui, &rows, 1000.) {
            assert!(
                hits(&ui, 50., y).contains(&child),
                "offset {}: at {y} past {row:?}, its child {child:?} is hit",
                offset.get()
            );
            checked += 1;
        }
    }
    // Offsets deep enough that some child walks past its row.
    assert!(checked > 0, "no shared edge walked apart");
}
