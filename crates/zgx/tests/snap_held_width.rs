//! A `snap` box's edges fall on whole points, each rounded on its own, so the
//! distance between them comes and goes by a point as the box only narrows.
//! While it's changing, what it holds is laid out at its size rounded instead,
//! which goes one way: text in it doesn't wrap and unwrap as a neighbour's edge
//! moves. Held still for a pause, it's laid out at the size between its edges.

use std::time::{Duration, Instant};
use zgx::scene::{Color, Layout, NodeId, NodeKind, SNAP_SETTLE, Scene, Style};

/// Four panes sharing what a sidebar leaves, a gap between them, each holding
/// a box as wide as it.
fn split() -> (Scene, NodeId, Vec<(NodeId, NodeId)>) {
    let mut s = Scene::new(2000., 500.);
    let root = s.root();
    s.set_kind(root, NodeKind::Container(Layout::Row));
    let mut style = s.style(root);
    style.gap = 4.;
    s.set_style(root, style);
    let sidebar = s.append(
        root,
        NodeKind::Rect(Color(0, 0, 0, 255)),
        Style {
            width: Some(260.),
            height: Some(500.),
            flex_shrink: 0.,
            ..Default::default()
        },
    );
    let panes = (0..4)
        .map(|_| {
            let pane = s.append(
                root,
                NodeKind::Container(Layout::Column),
                Style {
                    flex_grow: 1.,
                    width: Some(0.),
                    height: Some(500.),
                    snap: true,
                    ..Default::default()
                },
            );
            let held = s.append(
                pane,
                NodeKind::Rect(Color(255, 255, 255, 255)),
                Style {
                    width_percent: Some(1.),
                    height: Some(20.),
                    ..Default::default()
                },
            );
            (pane, held)
        })
        .collect();
    s.flush();
    (s, sidebar, panes)
}

#[test]
fn what_a_snapped_box_holds_only_narrows_as_its_share_does() {
    let (mut s, sidebar, panes) = split();
    let (mut was, mut edges_moved_back) = (vec![f32::MAX; panes.len()], 0);
    let mut last_pane = vec![f32::MAX; panes.len()];
    for step in 0..400 {
        let mut style = s.style(sidebar);
        style.width = Some(260. + 0.37 * step as f32);
        s.set_style(sidebar, style);
        s.flush();
        for (at, (pane, held)) in panes.iter().enumerate() {
            let (box_, inner) = (s.bounds(*pane), s.bounds(*held));
            assert!(
                inner.width <= was[at],
                "step {step}, pane {at}: {} after {}",
                inner.width,
                was[at]
            );
            was[at] = inner.width;
            assert_eq!(
                inner.x, box_.x,
                "step {step}: it stays where its box starts"
            );
            assert!(
                (inner.width - box_.width).abs() <= 1.,
                "step {step}: {} in {}",
                inner.width,
                box_.width
            );
            edges_moved_back += usize::from(box_.width > last_pane[at]);
            last_pane[at] = box_.width;
        }
    }
    // The boxes themselves do come and go: this is what it was hiding from.
    assert!(edges_moved_back > 0);
}

#[test]
fn snapped_boxes_keep_whole_edges_and_equal_gaps() {
    let (mut s, sidebar, panes) = split();
    for step in 0..400 {
        let mut style = s.style(sidebar);
        style.width = Some(260. + 0.37 * step as f32);
        s.set_style(sidebar, style);
        s.flush();
        let boxes: Vec<_> = panes.iter().map(|(pane, _)| s.bounds(*pane)).collect();
        for b in &boxes {
            assert_eq!(
                (b.x, b.width),
                (b.x.round(), b.width.round()),
                "step {step}"
            );
        }
        for pair in boxes.windows(2) {
            assert_eq!(
                pair[1].x - (pair[0].x + pair[0].width),
                4.,
                "step {step}: {pair:?}"
            );
        }
    }
}

/// Measured again and placed again in its parent's next layout, a box's result
/// is the one it was given, and its parent's size doesn't take the rounding in.
#[test]
fn a_snapped_box_settles_where_a_fresh_layout_puts_it() {
    let (mut live, sidebar, panes) = split();
    let width = 301.37;
    for step in 0..50 {
        let mut style = live.style(sidebar);
        style.width = Some(260. + (width - 260.) * step as f32 / 49.);
        live.set_style(sidebar, style);
        live.flush();
    }
    let (mut fresh, fresh_sidebar, fresh_panes) = split();
    let mut style = fresh.style(fresh_sidebar);
    style.width = Some(width);
    fresh.set_style(fresh_sidebar, style);
    fresh.flush();
    for ((pane, held), (fresh_pane, fresh_held)) in panes.iter().zip(&fresh_panes) {
        assert_eq!(live.bounds(*pane), fresh.bounds(*fresh_pane));
        assert_eq!(live.bounds(*held), fresh.bounds(*fresh_held));
    }
}

/// Held still for a pause, what a snapped box holds is as wide as the box;
/// frames alone don't end the pause, or a slow hand's moves, an eighth of a second
/// apart, would each settle and move again.
#[test]
fn at_rest_what_a_snapped_box_holds_is_as_wide_as_it() {
    let (mut s, sidebar, panes) = split();
    let mut unequal = 0;
    for step in 0..60 {
        let mut style = s.style(sidebar);
        style.width = Some(260. + 0.37 * step as f32);
        s.set_style(sidebar, style);
        s.flush();
        // Frames between moves, the pause not over: still going one way.
        for _ in 0..3 {
            s.prepare_layout();
            s.flush();
        }
        assert!(!s.settle_snaps(Instant::now()), "step {step}: still moving");
        unequal += panes
            .iter()
            .filter(|(pane, held)| s.bounds(*pane).width != s.bounds(*held).width)
            .count();
        if step % 10 == 9 {
            assert!(s.settle_snaps(Instant::now() + SNAP_SETTLE));
            s.flush();
            for (pane, held) in &panes {
                assert_eq!(s.bounds(*held).width, s.bounds(*pane).width, "step {step}");
            }
        }
    }
    assert!(unequal > 0, "moving, what it holds kept its own width");
}

/// Once settled it asks for nothing: no deadline, nothing laid out.
#[test]
fn a_snapped_box_at_rest_asks_for_nothing_more() {
    let (mut s, sidebar, _) = split();
    assert_eq!(s.snaps_settle_at(), None, "not moved yet");
    let mut style = s.style(sidebar);
    style.width = Some(261.37);
    s.set_style(sidebar, style);
    let before = Instant::now();
    s.flush();
    let at = s
        .snaps_settle_at()
        .expect("it moved: a time to see it stop");
    assert!(at >= before + SNAP_SETTLE && at <= Instant::now() + SNAP_SETTLE);
    assert!(s.settle_snaps(at + Duration::from_millis(1)));
    s.flush();
    assert_eq!(s.snaps_settle_at(), None);
    assert!(!s.settle_snaps(at + Duration::from_secs(1)));
    assert_eq!(s.flush().layout_nodes, 0);
}

/// One snapped box alone, moved again and again: each time its edges are found
/// afresh, not left from the last time it was placed.
#[test]
fn a_lone_snapped_box_finds_its_edges_each_time_it_moves() {
    let mut s = Scene::new(1000., 500.);
    let root = s.root();
    s.set_kind(root, NodeKind::Container(Layout::Row));
    let sidebar = s.append(
        root,
        NodeKind::Rect(Color(0, 0, 0, 255)),
        Style {
            width: Some(100.),
            height: Some(500.),
            flex_shrink: 0.,
            ..Default::default()
        },
    );
    let pane = s.append(
        root,
        NodeKind::Container(Layout::Column),
        Style {
            flex_grow: 1.,
            width: Some(0.),
            height: Some(500.),
            snap: true,
            ..Default::default()
        },
    );
    s.flush();
    for step in 1..=40 {
        let at = 100. + 0.37 * step as f32;
        let mut style = s.style(sidebar);
        style.width = Some(at);
        s.set_style(sidebar, style);
        s.flush();
        let b = s.bounds(pane);
        assert_eq!(b.x, at.round(), "step {step}");
        assert_eq!(b.x + b.width, 1000., "step {step}");
    }
}

/// A leaf that snaps takes its edges as a box that holds things does.
#[test]
fn a_snapped_leaf_keeps_whole_edges() {
    let mut s = Scene::new(1000., 500.);
    let root = s.root();
    s.set_kind(root, NodeKind::Container(Layout::Row));
    let sidebar = s.append(
        root,
        NodeKind::Rect(Color(0, 0, 0, 255)),
        Style {
            width: Some(100.),
            height: Some(500.),
            flex_shrink: 0.,
            ..Default::default()
        },
    );
    let leaf = s.append(
        root,
        NodeKind::Rect(Color(255, 255, 255, 255)),
        Style {
            width: Some(200.25),
            height: Some(500.),
            flex_shrink: 0.,
            snap: true,
            ..Default::default()
        },
    );
    s.flush();
    for step in 1..=40 {
        let at = 100. + 0.37 * step as f32;
        let mut style = s.style(sidebar);
        style.width = Some(at);
        s.set_style(sidebar, style);
        s.flush();
        let b = s.bounds(leaf);
        assert_eq!(b.x, at.round(), "step {step}");
        assert_eq!(b.x + b.width, (at + 200.25).round(), "step {step}");
    }
}
