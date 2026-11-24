//! A long text area scrolls its lines as scroll content: placed where they are
//! in the text, one layer moved under the view, so a scroll copies what
//! showed and paints only what comes into view. Its lines, caret, selection,
//! input method area and hit tests stay where its text is.
use zgx::{
    compose::prelude::*,
    input::InputEvent,
    scene::{NodeId, NodeKind, Rect},
    widgets::{EditorHandle, Ui},
};

const LINES: usize = 400;

fn long_editor() -> (Ui, EditorHandle, (f32, f32)) {
    let mut ui = Ui::new(500., 400.);
    ui.scene.borrow_mut().set_pixel_scale(2.);
    let text = (0..LINES)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let value = ui.signal(text);
    let view = ui.mount(
        text_area("Code", value)
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(false),
    );
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    let editor = ui.editor(view.node()).unwrap();
    let r = ui.scene.borrow().transformed_bounds(view.node());
    (ui, editor, (r.x + r.width / 2., r.y + r.height / 2.))
}
fn wheel(ui: &mut Ui, (x, y): (f32, f32), delta_y: f32) {
    ui.dispatch(InputEvent::Scroll {
        x,
        y,
        delta_x: 0.,
        delta_y,
    });
    ui.prepare_frame();
}
/// Each shown line's node by its number, and where it shows.
fn lines(ui: &Ui, editor: &EditorHandle) -> Vec<(usize, NodeId, Rect)> {
    let scene = ui.scene.borrow();
    let mut found = Vec::new();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if let NodeKind::Text { text, .. } = scene.kind(id)
            && let Some(n) = text.strip_prefix("line ").and_then(|n| n.parse().ok())
        {
            found.push((n, id, scene.transformed_bounds(id)));
        }
        pending.extend(scene.children(id).iter().copied());
    }
    found.sort_by_key(|(n, ..)| *n);
    found
}

#[test]
fn a_scrolled_line_shows_where_its_text_is() {
    let (mut ui, editor, at) = long_editor();
    let top = lines(&ui, &editor)[0].2.y;
    let mut offset = 0.;
    // A wheel's whole points and a trackpad's fractions, down and back.
    for delta in [12., 12., 7.3, 0.45, 30., -12., -19.75] {
        wheel(&mut ui, at, delta);
        offset += delta;
        ui.scene.borrow_mut().flush();
        // What it reports is the offset exactly; what shows, on whole pixels.
        let (across, down) = editor.scroll_offset();
        assert_eq!(across, 0.);
        assert!((down - offset).abs() < 1e-4, "{down}, not {offset}");
        let shown = lines(&ui, &editor);
        assert!(shown.len() > 10, "{} lines shown", shown.len());
        for (n, _, place) in &shown {
            // Where its text is, the view moved by whole device pixels.
            let expected = top + *n as f32 * 20. - (offset * 2_f32).round() / 2.;
            assert!(
                (place.y - expected).abs() < 0.01,
                "line {n} at {} after {offset}, not {expected}",
                place.y
            );
        }
    }
}

#[test]
fn the_caret_selection_input_area_and_hits_follow_the_lines() {
    let (mut ui, editor, at) = long_editor();
    let line_start = |n: usize| (0..n).map(|i| format!("line {i}\n").len()).sum::<usize>();
    for delta in [12., 12., 7.3, 31.] {
        wheel(&mut ui, at, delta);
        ui.scene.borrow_mut().flush();
        let shown = lines(&ui, &editor);
        // A line in the middle of the view: the caret at its start.
        let (n, _, place) = shown[shown.len() / 2];
        editor.set_selection(line_start(n), line_start(n) + 3);
        ui.prepare_frame();
        ui.scene.borrow_mut().flush();
        let place = lines(&ui, &editor)
            .into_iter()
            .find(|(m, ..)| *m == n)
            .map_or(place, |(.., p)| p);
        let caret = ui.scene.borrow().transformed_bounds(editor.caret);
        assert!(
            caret.y >= place.y - 0.5 && caret.y + caret.height <= place.y + place.height + 0.5,
            "line {n}: caret {caret:?}, line {place:?}"
        );
        let area = editor.input_method_area();
        assert!(
            (area.y - caret.y).abs() < 0.01,
            "input area {area:?}, caret {caret:?}"
        );
        // A press on the line's middle hits that line.
        let hit = editor
            .offset_at(place.x + 2., place.y + place.height / 2.)
            .expect("a hit");
        assert!(
            (line_start(n)..line_start(n + 1)).contains(&hit),
            "line {n}: hit byte {hit}"
        );
        assert_eq!(editor.selection(), (line_start(n), line_start(n) + 3));
    }
}

#[test]
fn a_scroll_step_copies_the_lines_and_paints_what_comes_into_view() {
    let (mut ui, editor, at) = long_editor();
    wheel(&mut ui, at, 12.);
    ui.scene.borrow_mut().flush();
    let view = lines(&ui, &editor);
    let width = view.iter().map(|(.., r)| r.width).fold(0_f32, f32::max);
    wheel(&mut ui, at, 12.);
    let report = ui.scene.borrow_mut().flush();
    let moves = ui.scene.borrow().scroll_moves().to_vec();
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert_eq!(moves[0].dy, -12., "{moves:?}");
    // What it painted: the strip that came into view (12 points, and a line
    // that came in whole), not the lines it moved.
    let painted: f32 = report.damage.iter().map(|r| r.width * r.height).sum();
    assert!(
        painted <= 400. * (12. + 20.) + 1.,
        "painted {painted} (lines {width} wide): {:?}",
        report.damage
    );
}

#[test]
fn a_page_jump_repaints_the_view_whole_and_its_lines_show_where_their_text_is() {
    let (mut ui, editor, at) = long_editor();
    let top = lines(&ui, &editor)[0].2.y;
    wheel(&mut ui, at, 12.);
    ui.scene.borrow_mut().flush();
    // Two thirds of the 300-point view at once: every line shown changes.
    wheel(&mut ui, at, 200.);
    let report = ui.scene.borrow_mut().flush();
    let view = ui.scene.borrow().transformed_bounds(editor.node);
    // One area, the view: not each line's, joined one by one.
    let whole = |r: &Rect| {
        r.x <= view.x
            && r.y <= view.y
            && r.x + r.width >= view.x + view.width
            && r.y + r.height >= view.y + view.height
    };
    assert!(report.damage.iter().any(whole), "{:?}", report.damage);
    assert_eq!(editor.scroll_offset().1, 212.);
    let shown = lines(&ui, &editor);
    assert!(shown.len() > 10, "{} lines shown", shown.len());
    for (n, _, place) in shown {
        let expected = top + n as f32 * 20. - 212.;
        assert!((place.y - expected).abs() < 0.01, "line {n} at {}", place.y);
    }
}
