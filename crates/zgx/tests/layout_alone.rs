//! A change inside a column's child that leaves the one answer its column
//! asked of it as it was (a label in a row of a fixed-width list, the same
//! height) lays out that child alone: its siblings stay where they are. Any
//! change its column would see (a wrap that changes its height) lays out as
//! before, and so does every change in a column sized by its widest child,
//! which asks each child for its natural width too. Every case ends where a
//! fresh scene with the same text puts everything.
use zgx::scene::{Align, Color, Layout, NodeId, NodeKind, Scene, Style};

const ROWS: usize = 40;

/// A stretching column of rows of one text each: 600 wide (`fixed`: each
/// row asked once, at that width), or as wide as its widest row.
fn scene(texts: &[String], fixed: bool) -> (Scene, NodeId, Vec<(NodeId, NodeId)>) {
    let mut scene = Scene::new(640., 4000.);
    let root = scene.root();
    let page = scene.append(
        root,
        NodeKind::Container(Layout::Column),
        Style {
            gap: 4.,
            align: Align::Stretch,
            width: fixed.then_some(600.),
            ..Style::default()
        },
    );
    let rows = texts
        .iter()
        .map(|value| {
            let row = scene.append(
                page,
                NodeKind::Container(Layout::Row),
                Style {
                    padding: 3.,
                    ..Style::default()
                },
            );
            let text = scene.append(
                row,
                NodeKind::Text {
                    text: value.as_str().into(),
                    color: Color(255, 255, 255, 255),
                    font_size: 14.,
                },
                Style {
                    text_wrap: true,
                    ..Style::default()
                },
            );
            (row, text)
        })
        .collect();
    scene.flush();
    (scene, page, rows)
}

fn labels() -> Vec<String> {
    (0..ROWS)
        .map(|i| match i {
            // The widest, by far.
            7 => "a row wider than every other one here".to_owned(),
            _ => format!("Row {i} at 0"),
        })
        .collect()
}

/// `live`, changed to `texts`, is where a fresh scene with them puts it.
fn same_as_fresh(
    live: &Scene,
    page: NodeId,
    rows: &[(NodeId, NodeId)],
    texts: &[String],
    fixed: bool,
) {
    let (fresh, fresh_page, fresh_rows) = scene(texts, fixed);
    assert_eq!(live.bounds(page), fresh.bounds(fresh_page), "the column");
    for (i, ((row, text), (fresh_row, fresh_text))) in rows.iter().zip(&fresh_rows).enumerate() {
        assert_eq!(live.bounds(*row), fresh.bounds(*fresh_row), "row {i}");
        assert_eq!(live.bounds(*text), fresh.bounds(*fresh_text), "text {i}");
    }
}

#[test]
fn a_label_changing_width_lays_out_its_row_alone() {
    let mut texts = labels();
    let (mut live, page, rows) = scene(&texts, true);
    let before: Vec<_> = rows.iter().map(|(row, _)| live.bounds(*row)).collect();
    texts[20] = "Row 20 at 137".into();
    live.set_text(rows[20].1, texts[20].as_str());
    assert!(
        !live.layout_pending(page),
        "the column is laid out as it was"
    );
    let report = live.flush();
    assert!(
        report.layout_nodes <= 3,
        "its row alone: {} nodes laid out",
        report.layout_nodes
    );
    for (i, (row, _)) in rows.iter().enumerate() {
        assert_eq!(live.bounds(*row), before[i], "row {i} where it was");
    }
    same_as_fresh(&live, page, &rows, &texts, true);
}

#[test]
fn a_label_that_wraps_moves_the_rows_after_it() {
    let mut texts = labels();
    let (mut live, page, rows) = scene(&texts, true);
    let below = live.bounds(rows[31].0).y;
    texts[30] =
        "a label long enough to wrap onto a second line in a column as wide as this one is, and on"
            .into();
    live.set_text(rows[30].1, texts[30].as_str());
    live.flush();
    assert!(live.bounds(rows[31].0).y > below, "the next row moved down");
    same_as_fresh(&live, page, &rows, &texts, true);
}

#[test]
fn a_column_sized_by_its_widest_follows_every_change() {
    let mut texts = labels();
    let (mut live, page, rows) = scene(&texts, false);
    let width = live.bounds(page).width;
    texts[20] = "Row 20 at 137".into();
    live.set_text(rows[20].1, texts[20].as_str());
    live.flush();
    same_as_fresh(&live, page, &rows, &texts, false);
    texts[7] = "a row wider than every other one here, and wider still".into();
    live.set_text(rows[7].1, texts[7].as_str());
    live.flush();
    assert!(live.bounds(page).width > width, "the column widened");
    same_as_fresh(&live, page, &rows, &texts, false);
    texts[7] = "short".into();
    live.set_text(rows[7].1, texts[7].as_str());
    live.flush();
    assert!(live.bounds(page).width < width, "and narrowed");
    same_as_fresh(&live, page, &rows, &texts, false);
}

#[test]
fn a_change_its_column_sees_in_a_frame_already_laid_out_from_the_top() {
    // A status beside the column changes too: the scene is laid out from
    // the top first (the column, unchanged as it looks, answers as it
    // did), and only then is the row's change found to be one its column
    // sees (a wrap). Nothing that first layout made of the old label
    // serves.
    let status = |scene: &mut Scene, value: &str| {
        let root = scene.root();
        scene.append(
            root,
            NodeKind::Text {
                text: value.into(),
                color: Color(255, 255, 255, 255),
                font_size: 14.,
            },
            Style::default(),
        )
    };
    let mut texts = labels();
    let (mut live, page, rows) = scene(&texts, true);
    let shown = status(&mut live, "ready");
    live.flush();
    texts[20] =
        "a label long enough to wrap onto a second line in a column as wide as this one is, and on"
            .into();
    live.set_text(rows[20].1, texts[20].as_str());
    live.set_text(shown, "working on it");
    live.flush();
    let (mut fresh, fresh_page, fresh_rows) = scene(&texts, true);
    let fresh_shown = status(&mut fresh, "working on it");
    fresh.flush();
    assert_eq!(live.bounds(shown), fresh.bounds(fresh_shown), "the status");
    assert_eq!(live.bounds(page), fresh.bounds(fresh_page), "the column");
    for (i, (row, fresh_row)) in rows.iter().zip(&fresh_rows).enumerate() {
        assert_eq!(live.bounds(row.0), fresh.bounds(fresh_row.0), "row {i}");
        assert_eq!(live.bounds(row.1), fresh.bounds(fresh_row.1), "text {i}");
    }
}

#[test]
fn a_paragraph_given_new_runs_lays_out_its_row_alone() {
    // A document's block as a reply streams into it (`set_leaf_kind`): a
    // rich paragraph in each row of a fixed-width column.
    use zgx::rich_text::{RichText, TextRun};
    let rich = |value: &str| NodeKind::RichText {
        text: std::sync::Arc::new(
            RichText::new(
                value,
                vec![TextRun {
                    range: 0..value.len(),
                    ..Default::default()
                }],
            )
            .unwrap(),
        ),
    };
    let build = |texts: &[String]| {
        let mut scene = Scene::new(640., 4000.);
        let root = scene.root();
        let page = scene.append(
            root,
            NodeKind::Container(Layout::Column),
            Style {
                gap: 4.,
                align: Align::Stretch,
                width: Some(600.),
                ..Style::default()
            },
        );
        let rows: Vec<_> = texts
            .iter()
            .map(|value| {
                let row = scene.append(
                    page,
                    NodeKind::Container(Layout::Row),
                    Style {
                        padding: 3.,
                        ..Style::default()
                    },
                );
                let text = scene.append(
                    row,
                    rich(value),
                    Style {
                        text_wrap: true,
                        ..Style::default()
                    },
                );
                (row, text)
            })
            .collect();
        scene.flush();
        (scene, rows)
    };
    let mut texts = labels();
    let (mut live, rows) = build(&texts);
    let before: Vec<_> = rows.iter().map(|(row, _)| live.bounds(*row)).collect();
    texts[12] = "Row 12 at 4096".into();
    live.set_leaf_kind(rows[12].1, rich(&texts[12]));
    let report = live.flush();
    assert!(
        report.layout_nodes <= 3,
        "its row alone: {} nodes laid out",
        report.layout_nodes
    );
    for (i, (row, _)) in rows.iter().enumerate() {
        assert_eq!(live.bounds(*row), before[i], "row {i} where it was");
    }
    let (fresh, fresh_rows) = build(&texts);
    for (i, ((row, text), (fresh_row, fresh_text))) in rows.iter().zip(&fresh_rows).enumerate() {
        assert_eq!(live.bounds(*row), fresh.bounds(*fresh_row), "row {i}");
        assert_eq!(live.bounds(*text), fresh.bounds(*fresh_text), "text {i}");
    }
}

/// Text 7 points a character, 18 a line, wrapped at the room it is given.
fn measure(text: &str, _: f32, limit: Option<f32>) -> (f32, f32) {
    let width = text.len() as f32 * 7.;
    let room = limit.unwrap_or(width).max(7.);
    (width.min(room), (width / room).ceil().max(1.) * 18.)
}

fn label(scene: &mut Scene, parent: NodeId, value: &str) -> NodeId {
    scene.append(
        parent,
        NodeKind::Text {
            text: value.into(),
            color: Color(255, 255, 255, 255),
            font_size: 14.,
        },
        Style {
            text_wrap: true,
            ..Style::default()
        },
    )
}

#[test]
fn a_card_its_column_holds_at_its_floor_shrinks_when_its_text_does() {
    // A card of a set height in a column too short for it and a label: the
    // column shrinks it no further than its text, which is taller. Its text
    // cut short, the card answers its own height as it did, but its floor
    // is its text's now, and its column shrinks it to that.
    let build = |words: &str| {
        let mut scene = Scene::new(300., 600.);
        scene.set_text_measurer(measure);
        let root = scene.root();
        let column = scene.append(
            root,
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(110.),
                height: Some(38.),
                gap: 2.,
                ..Style::default()
            },
        );
        let card = scene.append(
            column,
            NodeKind::Container(Layout::Column),
            Style {
                width_percent: Some(0.8),
                height: Some(48.),
                padding: 1.5,
                align: Align::Stretch,
                ..Style::default()
            },
        );
        let text = label(&mut scene, card, words);
        label(&mut scene, column, "t6");
        scene.flush();
        (scene, card, text)
    };
    let long = "a few words that wrap past the card's height";
    let (mut live, card, text) = build(long);
    assert_eq!(live.bounds(card).height, 48., "held at its own height");
    live.set_text(text, "ab");
    live.flush();
    let (fresh, fresh_card, fresh_text) = build("ab");
    assert!(fresh.bounds(fresh_card).height < 48., "shrunk to its text");
    assert_eq!(live.bounds(card), fresh.bounds(fresh_card), "the card");
    assert_eq!(live.bounds(text), fresh.bounds(fresh_text), "its text");
}

#[test]
fn a_box_found_not_to_stand_after_one_laid_out_alone_lays_out_inside_it() {
    // A band laid out alone first (its label changed, the same size): it
    // answers as it did, from what a box of a set size inside it answered
    // before. That box's text, cut short in the same frame, drops the floor
    // its column held it at: laid out from the top after all, the band's
    // answer, given before, mustn't serve, or nothing lays the box out.
    let build = |status: &str, words: &str| {
        let mut scene = Scene::new(300., 600.);
        scene.set_text_measurer(measure);
        let root = scene.root();
        let page = scene.append(
            root,
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(280.),
                ..Style::default()
            },
        );
        let band = scene.append(
            page,
            NodeKind::Container(Layout::Column),
            Style {
                width_percent: Some(0.8),
                height: Some(17.),
                align: Align::Stretch,
                ..Style::default()
            },
        );
        let frame = scene.append(
            band,
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(174.),
                height: Some(81.),
                gap: 6.,
                align: Align::Center,
                ..Style::default()
            },
        );
        let held = scene.append(
            frame,
            NodeKind::Container(Layout::Column),
            Style {
                width: Some(56.),
                height: Some(72.),
                align: Align::Center,
                ..Style::default()
            },
        );
        let text = label(&mut scene, held, words);
        label(&mut scene, frame, "a caption");
        let shown = label(&mut scene, band, status);
        scene.flush();
        (scene, held, text, shown)
    };
    let long = "w ".repeat(48);
    let (mut live, held, text, shown) = build("ab", &long);
    live.set_text(shown, "cd");
    live.set_text(text, "ab");
    live.flush();
    let (fresh, fresh_held, fresh_text, _) = build("cd", "ab");
    assert_eq!(live.bounds(held), fresh.bounds(fresh_held), "the box");
    assert_eq!(live.bounds(text), fresh.bounds(fresh_text), "its text");
}
