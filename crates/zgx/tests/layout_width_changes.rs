//! A page whose width changes a step at a time (a side panel's edge
//! dragged), and what changes inside it meanwhile, must lay out exactly as a
//! fresh layout of the same tree at that width: probes answered by a box's
//! size alone (its style fixing it), and absolute boxes laid out apart from
//! their parents, place every node where the full layout would.
use std::sync::Arc;
use zgx::layout::{Display, EdgeLengths, LayoutOptions, Length};
use zgx::scene::{Align, Color, Insets, Justify, Layout, NodeId, NodeKind, Scene, Style};

fn text(value: &str) -> NodeKind {
    NodeKind::Text {
        text: value.into(),
        color: Color(255, 255, 255, 255),
        font_size: 14.,
    }
}

fn options(options: LayoutOptions) -> Option<Arc<LayoutOptions>> {
    Some(Arc::new(options))
}

/// A one-cell grid filling its parent (a transition's wrapper).
fn cell() -> Style {
    Style {
        width_percent: Some(1.),
        height_percent: Some(1.),
        layout_options: options(LayoutOptions {
            display: Some(Display::Grid),
            columns: Some(1),
            rows: Some(1),
            ..Default::default()
        }),
        ..Style::default()
    }
}

fn fill() -> Style {
    Style {
        width_percent: Some(1.),
        height_percent: Some(1.),
        ..Style::default()
    }
}

fn inset(top: f32, right: Option<f32>, bottom: f32, left: f32) -> Option<Arc<LayoutOptions>> {
    options(LayoutOptions {
        inset: EdgeLengths {
            top: Some(Length::Px(top)),
            right: right.map(Length::Px),
            bottom: Some(Length::Px(bottom)),
            left: Some(Length::Px(left)),
        },
        ..Default::default()
    })
}

const WORDS: &str = "a width a person drags past breaks these lines again once the page \
is narrower than the column they are kept to";

struct Page {
    all: Vec<NodeId>,
    panel: NodeId,
    /// A paragraph in a box its insets fix (both sides, top and bottom).
    pinned: NodeId,
    /// A paragraph in a box its content sizes, held at the page's foot.
    footer: NodeId,
}

/// The shell of an app's chat page: a sidebar, then the page beside a
/// panel, the page's layer inset under a title bar, a few one-cell grids
/// and wrappers, and a centred column of at most 320 points of wrapping
/// paragraphs; over it, a box pinned by its insets, and a footer sized by
/// what it holds, at the foot.
fn build(width: f32, panel_width: f32) -> (Scene, Page) {
    let mut scene = Scene::new(width, 500.);
    let mut all = Vec::new();
    let mut add = |scene: &mut Scene, parent, kind, style| {
        let id = scene.append(parent, kind, style);
        all.push(id);
        id
    };
    let root = scene.root();
    let shell = add(
        &mut scene,
        root,
        NodeKind::Container(Layout::Column),
        Style {
            relative: true,
            ..fill()
        },
    );
    add(
        &mut scene,
        shell,
        NodeKind::Container(Layout::Column),
        Style {
            absolute: true,
            height: Some(38.),
            layout_options: inset(0., Some(0.), 0., 0.).map(|o| {
                let mut o = *o;
                o.inset.bottom = None;
                Arc::new(o)
            }),
            ..Style::default()
        },
    );
    let row = add(
        &mut scene,
        shell,
        NodeKind::Container(Layout::Row),
        Style {
            width_percent: Some(1.),
            flex_grow: 1.,
            min_height: Some(0.),
            ..Style::default()
        },
    );
    let sidebar = add(
        &mut scene,
        row,
        NodeKind::Container(Layout::Column),
        Style {
            width: Some(120.),
            height_percent: Some(1.),
            flex_shrink: 0.,
            ..Style::default()
        },
    );
    add(&mut scene, sidebar, text("Sessions"), Style::default());
    let page = add(
        &mut scene,
        row,
        NodeKind::Container(Layout::Column),
        Style {
            relative: true,
            clip: true,
            height_percent: Some(1.),
            flex_grow: 1.,
            min_width: Some(0.),
            ..Style::default()
        },
    );
    let panel = add(
        &mut scene,
        row,
        NodeKind::Container(Layout::Column),
        Style {
            width: Some(panel_width),
            height_percent: Some(1.),
            flex_shrink: 0.,
            padding: 4.,
            ..Style::default()
        },
    );
    add(
        &mut scene,
        panel,
        text("crates/zgx/src/scene.rs"),
        Style::default(),
    );
    let layer = add(
        &mut scene,
        page,
        NodeKind::Container(Layout::Column),
        Style {
            absolute: true,
            layout_options: inset(38., Some(0.), 0., 0.),
            ..Style::default()
        },
    );
    let mut parent = layer;
    for _ in 0..3 {
        let grid = add(
            &mut scene,
            parent,
            NodeKind::Container(Layout::Column),
            cell(),
        );
        parent = add(
            &mut scene,
            grid,
            NodeKind::Container(Layout::Column),
            fill(),
        );
    }
    let content = add(
        &mut scene,
        parent,
        NodeKind::Container(Layout::Column),
        Style {
            relative: true,
            ..fill()
        },
    );
    for message in 0..3 {
        let line = add(
            &mut scene,
            content,
            NodeKind::Container(Layout::Row),
            Style {
                width_percent: Some(1.),
                justify: Justify::Center,
                padding_edges: Some(Insets {
                    top: 8.,
                    right: 12.,
                    bottom: 0.,
                    left: 12.,
                }),
                ..Style::default()
            },
        );
        let column = add(
            &mut scene,
            line,
            NodeKind::Container(Layout::Column),
            Style {
                width_percent: Some(1.),
                max_width: Some(320.),
                min_width: Some(0.),
                gap: 4.,
                align: Align::Stretch,
                ..Style::default()
            },
        );
        for paragraph in 0..2 {
            add(
                &mut scene,
                column,
                text(&format!("{message}.{paragraph} {WORDS}")),
                Style {
                    text_wrap: true,
                    ..Style::default()
                },
            );
        }
    }
    let pin = add(
        &mut scene,
        content,
        NodeKind::Container(Layout::Column),
        Style {
            absolute: true,
            layout_options: inset(10., Some(10.), 300., 200.),
            ..Style::default()
        },
    );
    let pinned = add(
        &mut scene,
        pin,
        text(WORDS),
        Style {
            text_wrap: true,
            ..Style::default()
        },
    );
    let foot = add(
        &mut scene,
        content,
        NodeKind::Container(Layout::Column),
        Style {
            absolute: true,
            layout_options: options(LayoutOptions {
                inset: EdgeLengths {
                    left: Some(Length::Px(12.)),
                    right: Some(Length::Px(12.)),
                    bottom: Some(Length::Px(8.)),
                    top: None,
                },
                ..Default::default()
            }),
            padding: 6.,
            ..Style::default()
        },
    );
    let footer = add(
        &mut scene,
        foot,
        text("Ask for follow-up changes"),
        Style {
            text_wrap: true,
            ..Style::default()
        },
    );
    (
        scene,
        Page {
            all,
            panel,
            pinned,
            footer,
        },
    )
}

fn assert_same(live: &Scene, page: &Page, fresh: &Scene, fresh_page: &Page, what: &str) {
    assert_eq!(page.all.len(), fresh_page.all.len());
    for (index, (a, b)) in page.all.iter().zip(&fresh_page.all).enumerate() {
        assert_eq!(
            live.bounds(*a),
            fresh.bounds(*b),
            "node {index} diverged {what}"
        );
    }
}

fn set_width(scene: &mut Scene, id: NodeId, width: f32) {
    let mut style = scene.style(id);
    style.width = Some(width);
    scene.set_style(id, style);
}

#[test]
fn a_page_laid_out_at_each_width_a_drag_passes_matches_a_fresh_layout() {
    let (mut live, page) = build(900., 200.);
    live.flush();
    // Wider panels narrow the page past its column's 320 points and back.
    let widths = (1..=24).map(|i| 200. + 15. * i as f32);
    for (step, panel) in widths.clone().chain(widths.rev()).enumerate() {
        set_width(&mut live, page.panel, panel);
        live.flush();
        let (mut fresh, fresh_page) = build(900., panel);
        fresh.flush();
        assert_same(
            &live,
            &page,
            &fresh,
            &fresh_page,
            &format!("at step {step}"),
        );
    }
}

#[test]
fn a_window_resized_lays_the_page_out_as_a_fresh_layout_at_its_size() {
    let (mut live, page) = build(900., 200.);
    live.flush();
    for width in [880., 700., 640.5, 1024., 512.25, 900.] {
        live.resize(width, 500.);
        live.flush();
        let (mut fresh, fresh_page) = build(width, 200.);
        fresh.flush();
        assert_same(&live, &page, &fresh, &fresh_page, &format!("at {width}"));
    }
}

#[test]
fn what_changes_in_an_absolute_box_lays_out_as_a_fresh_layout_does() {
    let (mut live, page) = build(900., 200.);
    live.flush();
    // In the box its insets fix: lines break again within it.
    live.set_kind(page.pinned, text(&format!("{WORDS} {WORDS} {WORDS}")));
    live.flush();
    // In the footer its content sizes: it grows, and its foot stays where
    // its inset holds it (its top moves up).
    live.set_kind(page.footer, text(&format!("{WORDS} {WORDS}")));
    live.flush();
    let (mut fresh, fresh_page) = build(900., 200.);
    fresh.set_kind(fresh_page.pinned, text(&format!("{WORDS} {WORDS} {WORDS}")));
    fresh.set_kind(fresh_page.footer, text(&format!("{WORDS} {WORDS}")));
    fresh.flush();
    assert_same(&live, &page, &fresh, &fresh_page, "after the edits");
    assert!(
        live.bounds(page.footer).height > 20.,
        "the footer's text wraps onto more lines"
    );
    // Then the page narrows: both lay out at its new width.
    set_width(&mut live, page.panel, 420.);
    live.flush();
    set_width(&mut fresh, fresh_page.panel, 420.);
    fresh.flush();
    assert_same(&live, &page, &fresh, &fresh_page, "after narrowing");
}

/// A tree a builder makes, laid out, edited by `edit` and laid out again,
/// against the same tree built, edited and laid out once: every node where
/// the fresh layout puts it. The builder returns every node it made, and
/// the ones `edit` changes.
fn live_matches_fresh(
    what: &str,
    build: impl Fn(&mut Scene) -> (Vec<NodeId>, Vec<NodeId>),
    edit: impl Fn(&mut Scene, &[NodeId]),
) {
    let mut live = Scene::new(600., 400.);
    let (all, targets) = build(&mut live);
    live.flush();
    edit(&mut live, &targets);
    live.flush();
    let mut fresh = Scene::new(600., 400.);
    let (fresh_all, fresh_targets) = build(&mut fresh);
    edit(&mut fresh, &fresh_targets);
    fresh.flush();
    for (index, (a, b)) in all.iter().zip(&fresh_all).enumerate() {
        assert_eq!(live.bounds(*a), fresh.bounds(*b), "{what}: node {index}");
    }
}

fn grow(scene: &mut Scene, targets: &[NodeId]) {
    for id in targets {
        scene.set_kind(*id, text(&format!("{WORDS} {WORDS} {WORDS}")));
    }
}

/// Appends, recording each node.
fn node(
    scene: &mut Scene,
    all: &mut Vec<NodeId>,
    parent: NodeId,
    kind: NodeKind,
    style: Style,
) -> NodeId {
    let id = scene.append(parent, kind, style);
    all.push(id);
    id
}

fn wrapping() -> Style {
    Style {
        text_wrap: true,
        ..Style::default()
    }
}

#[test]
fn an_absolute_box_its_content_sizes_grows_where_its_parent_places_it() {
    // Centred in an overlay, held by its foot, and in a column sized by
    // what it holds: each box's place reads its size, so it grows from
    // where a fresh layout puts it.
    for (name, layout, inset_options, parent_width) in [
        ("centred in an overlay", Layout::Overlay, None, Some(400.)),
        (
            "held by its foot",
            Layout::Column,
            inset(0., None, 12., 20.).map(|o| {
                let mut o = *o;
                o.inset.top = None;
                Arc::new(o)
            }),
            Some(400.),
        ),
        ("in a column its content sizes", Layout::Column, None, None),
    ] {
        live_matches_fresh(
            name,
            |scene| {
                let mut all = Vec::new();
                let row = node(
                    scene,
                    &mut all,
                    scene.root(),
                    NodeKind::Container(Layout::Row),
                    Style::default(),
                );
                let parent = node(
                    scene,
                    &mut all,
                    row,
                    NodeKind::Container(layout),
                    Style {
                        width: parent_width,
                        height: parent_width.map(|_| 300.),
                        relative: true,
                        align: Align::Center,
                        justify: Justify::Center,
                        ..Style::default()
                    },
                );
                node(scene, &mut all, parent, text("beside the box"), wrapping());
                let boxed = node(
                    scene,
                    &mut all,
                    parent,
                    NodeKind::Container(Layout::Column),
                    Style {
                        absolute: true,
                        max_width: Some(200.),
                        layout_options: inset_options.clone(),
                        ..Style::default()
                    },
                );
                let held = node(scene, &mut all, boxed, text("what it holds"), wrapping());
                (all, vec![held])
            },
            grow,
        );
    }
}

#[test]
fn a_box_a_grid_pins_on_every_side_and_one_it_pins_on_two_lay_out_as_fresh() {
    for (name, all_sides) in [
        ("pinned on every side", true),
        ("pinned at its top left", false),
    ] {
        live_matches_fresh(
            name,
            |scene| {
                let mut all = Vec::new();
                let grid = node(
                    scene,
                    &mut all,
                    scene.root(),
                    NodeKind::Container(Layout::Column),
                    {
                        let mut style = cell();
                        style.relative = true;
                        style
                    },
                );
                node(scene, &mut all, grid, text("the grid's cell"), wrapping());
                let options = if all_sides {
                    inset(10., Some(10.), 10., 300.)
                } else {
                    options(LayoutOptions {
                        inset: EdgeLengths {
                            top: Some(Length::Px(10.)),
                            left: Some(Length::Px(300.)),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                };
                let boxed = node(
                    scene,
                    &mut all,
                    grid,
                    NodeKind::Container(Layout::Column),
                    Style {
                        absolute: true,
                        max_width: Some(250.),
                        layout_options: options,
                        ..Style::default()
                    },
                );
                let held = node(scene, &mut all, boxed, text("what it holds"), wrapping());
                (all, vec![held])
            },
            grow,
        );
    }
}

#[test]
fn boxes_placed_by_a_block_further_up_and_nested_lay_out_as_fresh() {
    // An absolute box whose containing block is two levels up, held by
    // its foot and right edge; inside it, another absolute box; and the
    // page's width changing in the same frame as both edits.
    live_matches_fresh(
        "escaped and nested",
        |scene| {
            let mut all = Vec::new();
            let page = node(
                scene,
                &mut all,
                scene.root(),
                NodeKind::Container(Layout::Column),
                Style {
                    relative: true,
                    width: Some(500.),
                    height: Some(350.),
                    ..Style::default()
                },
            );
            let middle = node(
                scene,
                &mut all,
                page,
                NodeKind::Container(Layout::Column),
                fill(),
            );
            node(scene, &mut all, middle, text(WORDS), wrapping());
            let outer = node(
                scene,
                &mut all,
                middle,
                NodeKind::Container(Layout::Column),
                Style {
                    absolute: true,
                    width_percent: Some(0.5),
                    padding: 4.,
                    relative: true,
                    layout_options: options(LayoutOptions {
                        inset: EdgeLengths {
                            right: Some(Length::Px(8.)),
                            bottom: Some(Length::Px(8.)),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    ..Style::default()
                },
            );
            let first = node(
                scene,
                &mut all,
                outer,
                text("the outer box's line"),
                wrapping(),
            );
            let inner = node(
                scene,
                &mut all,
                outer,
                NodeKind::Container(Layout::Column),
                Style {
                    absolute: true,
                    layout_options: inset(0., Some(0.), 0., 40.),
                    ..Style::default()
                },
            );
            let second = node(
                scene,
                &mut all,
                inner,
                text("the inner box's line"),
                wrapping(),
            );
            (all, vec![first, second, page])
        },
        |scene, targets| {
            grow(scene, &targets[..2]);
            let mut style = scene.style(targets[2]);
            style.width = Some(420.);
            scene.set_style(targets[2], style);
        },
    );
}
