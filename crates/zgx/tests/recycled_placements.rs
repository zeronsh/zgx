//! A container measured for the first time may take the child placements of
//! one freed before it (`Scene::recycled_placements`): it lays out as in a
//! scene that kept none, a container measured more than once in a pass too.
use zgx::{compose::prelude::*, scene::NodeId, widgets::Ui};

/// Every node under `root`, in tree order, `root` included.
fn subtree(ui: &Ui, root: NodeId) -> Vec<NodeId> {
    fn walk(scene: &zgx::scene::Scene, id: NodeId, into: &mut Vec<NodeId>) {
        into.push(id);
        for &child in scene.children(id) {
            walk(scene, child, into);
        }
    }
    let mut ids = Vec::new();
    walk(&ui.scene.borrow(), root, &mut ids);
    ids
}
fn bounds(ui: &Ui, root: NodeId) -> Vec<[f32; 4]> {
    let ids = subtree(ui, root);
    let scene = ui.scene.borrow();
    ids.iter()
        .map(|id| scene.bounds(*id))
        .map(|b| [b.x, b.y, b.width, b.height])
        .collect()
}
/// Containers of one to ten children, whose placements are kept as they go.
fn gone() -> View {
    column().children((0..40).map(|at| {
        row()
            .gap(3.)
            .children((0..at % 10 + 1).map(|w| div().size(7. + w as f32, 11.)))
    }))
}
/// Rows measured more than once a pass: natural, then as they grow, shrink
/// and stretch, nested, over wrapping text and an overlay, and empty boxes.
fn page() -> View {
    column()
        .w_full()
        .items_stretch()
        .children((0..12).map(|at| {
            row()
                .items_stretch()
                .gap(4.)
                .child(
                    column()
                        .grow()
                        .min_w(20.)
                        .child(text(
                            "words that wrap where the column ends, at every width",
                        ))
                        .child(
                            row()
                                .flex_wrap()
                                .children((0..at % 4 + 1).map(|_| div().size(60., 9.))),
                        )
                        .child(row())
                        .child(
                            row()
                                .items_stretch()
                                .child(column().grow().child(div().h(5. + at as f32)))
                                .child(column().w(30.)),
                        ),
                )
                .child(column().w_percent(0.25).child(div().w_full().h(10.)))
        }))
}

#[test]
fn a_row_laid_out_in_a_freed_rows_room_is_where_it_would_be_without() {
    let widths = [400., 250., 130., 400.];
    let mut fresh = Ui::new(400., 600.);
    let mut reused = Ui::new(400., 600.);
    let first = reused.mount(gone());
    reused.prepare_frame();
    reused.remove(first.node());
    for _ in 0..4 {
        reused.prepare_frame();
    }
    let (again, taken) = zgx::scene::containers_remeasured_and_reused();
    for cycle in 0..3 {
        let a = fresh.mount(page());
        let b = reused.mount(page());
        for width in widths {
            for ui in [&mut fresh, &mut reused] {
                ui.scene.borrow_mut().resize(width, 600.);
                ui.prepare_frame();
            }
            let want = bounds(&fresh, a.node());
            assert!(want.len() > 150, "laid out: {}", want.len());
            assert_eq!(want, bounds(&reused, b.node()), "cycle {cycle} at {width}");
        }
        // The page goes, and the next is measured in the room it leaves; the
        // other scene starts anew, with none kept.
        reused.remove(b.node());
        for _ in 0..4 {
            reused.prepare_frame();
        }
        fresh = Ui::new(400., 600.);
    }
    let (remeasured, reused) = zgx::scene::containers_remeasured_and_reused();
    if cfg!(debug_assertions) {
        assert!(remeasured > again, "a container measured twice in a pass");
        assert!(reused > taken, "a freed container's room reused");
    }
}

#[test]
fn the_kept_room_goes_at_rest_and_comes_back() {
    let reused = || zgx::scene::containers_remeasured_and_reused().1;
    let mut ui = Ui::new(400., 600.);
    let free = |ui: &mut Ui, node| {
        ui.remove(node);
        for _ in 0..4 {
            ui.prepare_frame();
        }
    };
    let first = ui.mount(gone());
    ui.prepare_frame();
    free(&mut ui, first.node());
    let before = reused();
    // Kept: a page measured now takes some of it.
    let page_ = ui.mount(page());
    ui.prepare_frame();
    let taken = reused();
    // At rest all of it goes: the next page has none to take.
    free(&mut ui, page_.node());
    ui.scene.borrow_mut().rest();
    let page_ = ui.mount(page());
    ui.prepare_frame();
    let rested = reused();
    // Rows going again leave theirs, as before.
    free(&mut ui, page_.node());
    let _page = ui.mount(page());
    ui.prepare_frame();
    let back = reused();
    if cfg!(debug_assertions) {
        assert!(taken > before, "kept: {before} to {taken}");
        assert_eq!(rested, taken, "none kept at rest");
        assert!(back > rested, "kept again: {rested} to {back}");
    }
}
