//! A page whose width changes a step at a time (a side panel's edge
//! dragged), and what changes inside it meanwhile, must lay out exactly as a
//! fresh layout of the same tree at that width: probes answered by a box's
//! size alone (its style fixing it) place every node where the full layout
//! would.
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
