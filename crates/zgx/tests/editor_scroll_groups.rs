//! A text area's scroll groups (`LineInset::scroll_group`) and unwrapped
//! lines (`LineInset::nowrap`): a code block whose lines never wrap scrolls
//! sideways alone, as the web's `white-space: pre` with `overflow-x: auto`.
//! Its lines show in a layer of their own, moved by the group's offset: a
//! sideways scroll shapes and places no line again, and the renderer copies
//! what it drew.
use std::{cell::Cell, cell::RefCell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::InputEvent,
    scene::{ClipInset, Color, NodeId, NodeKind},
    text_layout::{FallbackTextLayout, FontStyle, TextLayout},
    widgets::{EditorHandle, GroupFrame, LineInset, LineMark, Ui},
};

const GROUP: u32 = 3;
/// The block's lines, down the window (each line 20 points).
const BLOCK_Y: f32 = 30.;

/// A note 400 points wide: a prose line, a two-line code block (lines 1 and
/// 2, 30 points in, the first far wider than the note), and a long prose
/// line that wraps.
fn text() -> String {
    format!(
        "intro\nlet wide = \"{}\";\nshort\n{}",
        "x".repeat(200),
        "word ".repeat(60)
    )
}

fn line_range(i: usize) -> std::ops::Range<usize> {
    let text = text();
    let start: usize = text.split('\n').take(i).map(|l| l.len() + 1).sum();
    start..start + text.split('\n').nth(i).unwrap().len()
}

/// The note, its text shaped by a shaper that counts what it shapes, the
/// block's lines grouped (or, `grouped` false, only indented).
fn note(
    grouped: bool,
    bar: bool,
    marks: impl Fn(usize) -> Vec<LineMark> + 'static,
) -> (Ui, EditorHandle, Rc<Cell<usize>>) {
    let mut ui = Ui::new(600., 400.);
    let shaped = Rc::new(Cell::new(0));
    let counted = shaped.clone();
    ui.scene.borrow_mut().set_font_text_shaper(
        move |text: &str, size, width, font: &FontStyle| -> Box<dyn TextLayout> {
            counted.set(counted.get() + usize::from(!text.is_empty()));
            Box::new(FallbackTextLayout::with_line_height(
                text,
                size,
                width,
                font.line_height,
            ))
        },
    );
    let value = ui.signal(text());
    let mut area = text_area("Note", value)
        .w(400.)
        .h(300.)
        .p(0.)
        .text_size(10.)
        .line_height(20.)
        .text_wrap(true);
    if bar {
        area = area.group_scrollbar();
    }
    let view = ui.mount(area);
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_insets(Some(Rc::new(move |i, _, _| match i {
        1 | 2 if grouped => LineInset::new(30., 0., 0.).in_group(GROUP).unwrapped(),
        1 | 2 => LineInset::new(30., 0., 0.),
        _ => LineInset::default(),
    })));
    editor.set_line_marks(Some(Rc::new(move |i, _, _| marks(i))));
    ui.input.focus(&ui.scene, Some(view.node()));
    ui.prepare_frame();
    ui.scene.borrow_mut().flush();
    (ui, editor, shaped)
}

/// The text node showing a line starting `start` under the editor.
fn line_node(ui: &Ui, editor: &EditorHandle, start: &str) -> NodeId {
    let scene = ui.scene.borrow();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if matches!(scene.kind(id), NodeKind::Text { text, .. } if text.starts_with(start)) {
            return id;
        }
        pending.extend_from_slice(scene.children(id));
    }
    panic!("no line {start:?}");
}

/// Where the quads of marks drawn with border width `width` show across,
/// in the window, and their clips.
fn marks(ui: &Ui, editor: &EditorHandle, width: f32) -> Vec<(f32, Option<ClipInset>)> {
    let scene = ui.scene.borrow();
    let mut found = Vec::new();
    let mut pending = vec![editor.node];
    while let Some(id) = pending.pop() {
        if matches!(scene.kind(id), NodeKind::Quad(q) if q.border_width == width) {
            found.push((scene.transformed_bounds(id).x, scene.clip_inset(id)));
        }
        pending.extend_from_slice(scene.children(id));
    }
    found
}

/// A mark 10 points wide at `x`, told apart by its border's `width`.
fn mark(x: f32, width: f32) -> LineMark {
    let mut mark = LineMark::new(x, 0., 10., 20., Color(1, 2, 3, 255));
    mark.border_width = width;
    mark
}
/// A block's surface: to the area's edge, pinned.
fn surface() -> LineMark {
    let mut mark = LineMark::new(
        0.,
        0.,
        f32::INFINITY,
        f32::INFINITY,
        Color(240, 240, 240, 255),
    );
    mark.border_width = 1.;
    mark.pinned = true;
    mark
}
fn wheel(ui: &mut Ui, (x, y): (f32, f32), (delta_x, delta_y): (f32, f32), shift: Option<bool>) {
    let event = InputEvent::Scroll {
        x,
        y,
        delta_x,
        delta_y,
    };
    match shift {
        Some(shift) => {
            zgx::input::mouse_wheel(shift, || ui.dispatch(event));
        }
        None => {
            ui.dispatch(event);
        }
    }
    ui.prepare_frame();
}

/// A code block's wide line shows in one row at its full width, its text
/// moving with the group's offset; the prose around it wraps and stays.
#[test]
fn a_code_blocks_line_does_not_wrap_and_scrolls_with_its_group() {
    let (ui, editor, _) = note(true, false, |_| Vec::new());
    let wide = editor.range_boxes(line_range(1));
    assert_eq!(wide.len(), 1, "one row, not wrapped: {wide:?}");
    assert!(
        editor.range_boxes(line_range(3)).len() >= 2,
        "the prose wraps"
    );
    let (view, content) = editor.group_extent(GROUP).expect("a group");
    assert_eq!(view, 370.);
    assert!(content > view, "it scrolls: {view} of {content}");
    let first = line_range(1).start;
    let at = first + 30..first + 31;
    let before = editor.range_boxes(at.clone())[0];
    editor.set_group_scroll(GROUP, 100.);
    ui.prepare_frame();
    assert_eq!(editor.group_scroll(GROUP), 100.);
    let after = editor.range_boxes(at)[0];
    assert!(
        (before.x - after.x - 100.).abs() < 0.01,
        "its text moved by the offset: {before:?} {after:?}"
    );
    let prose = editor.range_boxes(0..1)[0];
    assert!(prose.x.abs() < 0.5, "the prose stayed: {prose:?}");
}

/// A sideways scroll moves the group's layer alone: its lines' nodes keep
/// their places, no line is shaped again, and the renderer is told to copy
/// what the frame shows.
#[test]
fn a_sideways_scroll_moves_the_groups_layer_and_shapes_nothing() {
    let (mut ui, editor, shaped) = note(true, false, |_| Vec::new());
    let line = line_node(&ui, &editor, "let wide");
    let (placed, content) = {
        let scene = ui.scene.borrow();
        let content = scene.parent(line).unwrap();
        assert!(scene.is_scroll_content(content), "in a layer of its own");
        let clip = scene.parent(content).unwrap();
        assert_eq!(scene.clips(clip), (true, true), "cut to its frame");
        (scene.transform(line), content)
    };
    let shapes = shaped.get();
    wheel(&mut ui, (100., BLOCK_Y), (24., 0.), None);
    let report = ui.scene.borrow_mut().flush();
    assert_eq!(editor.group_scroll(GROUP), 24.);
    let scene = ui.scene.borrow();
    assert_eq!(scene.transform(line), placed, "the line node stays");
    assert_eq!(scene.transform(content).x, -24., "its layer moves");
    let moves = scene.scroll_moves().to_vec();
    assert_eq!(moves.len(), 1, "{moves:?}");
    assert_eq!(moves[0].dx, -24.);
    assert_eq!(moves[0].dy, 0.);
    // What it painted: the strip that came into view, the caret's, not the
    // block (two rows, 370 points wide).
    let painted: f32 = report.damage.iter().map(|r| r.width * r.height).sum();
    assert!(
        painted < 370. * 40. / 2.,
        "painted {painted}: {:?}",
        report.damage
    );
    drop(scene);
    assert_eq!(shaped.get(), shapes, "nothing shaped again");
    for to in [60., 10., 200.] {
        editor.set_group_scroll(GROUP, to);
        ui.prepare_frame();
    }
    assert_eq!(shaped.get(), shapes, "nothing shaped again");
}

/// A pinned mark (a block's wash, its line numbers) stays where it is as the
/// group scrolls; the others move with the text and are cut to its frame.
#[test]
fn pinned_marks_stay_as_the_group_scrolls_and_others_move() {
    let (ui, editor, _) = note(true, false, |i| match i {
        1 | 2 => vec![surface(), mark(60., 2.)],
        _ => Vec::new(),
    });
    let shown = |ui: &Ui, width| {
        let mut found = marks(ui, &editor, width);
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found
    };
    assert_eq!(shown(&ui, 1.), [(0., None), (0., None)]);
    let none = Some(ClipInset::px(0., 0., 0., 0.));
    assert_eq!(shown(&ui, 2.), [(60., none), (60., none)]);
    editor.set_group_scroll(GROUP, 20.);
    ui.prepare_frame();
    assert_eq!(shown(&ui, 1.), [(0., None), (0., None)], "pinned: still");
    // Moved 20 left, still inside the frame (from 30).
    assert_eq!(shown(&ui, 2.), [(40., none), (40., none)]);
    // Moved 35 left: its first 5 points left of the frame, cut.
    editor.set_group_scroll(GROUP, 35.);
    ui.prepare_frame();
    let half = Some(ClipInset::px(0., 0., 0., 5.));
    assert_eq!(shown(&ui, 2.), [(25., half), (25., half)]);
}

/// A mouse wheel with Shift held over the block scrolls the block, not the
/// note; so does a trackpad's sideways motion. Over prose, or left of the
/// block's frame, the block doesn't move. Its listener hears each move.
#[test]
fn shift_wheel_and_sideways_motion_scroll_the_group() {
    let (mut ui, editor, _) = note(true, false, |_| Vec::new());
    let heard = Rc::new(RefCell::new(Vec::new()));
    let log = heard.clone();
    editor.on_group_scroll(move |group, x| log.borrow_mut().push((group, x)));
    wheel(&mut ui, (100., BLOCK_Y), (0., 120.), Some(true));
    assert_eq!(editor.group_scroll(GROUP), 120.);
    assert_eq!(editor.scroll_offset(), (0., 0.), "the note didn't move");
    wheel(&mut ui, (100., BLOCK_Y), (40., 0.), None);
    assert_eq!(editor.group_scroll(GROUP), 160.);
    // Over the prose above it, and over its gutter: nothing of it moves.
    wheel(&mut ui, (100., 10.), (40., 0.), None);
    wheel(&mut ui, (10., BLOCK_Y), (40., 0.), None);
    assert_eq!(editor.group_scroll(GROUP), 160.);
    assert_eq!(*heard.borrow(), vec![(GROUP, 120.), (GROUP, 160.)]);
}

/// Scrolled by the app or the wheel, a group stays between 0 and how far its
/// lines reach past its frame; the app's frame changes how far.
#[test]
fn a_groups_scroll_is_clamped_and_its_frame_set() {
    let (mut ui, editor, _) = note(true, false, |_| Vec::new());
    let (view, content) = editor.group_extent(GROUP).unwrap();
    editor.set_group_scroll(GROUP, 100_000.);
    assert_eq!(editor.group_scroll(GROUP), content - view);
    wheel(&mut ui, (100., BLOCK_Y), (200., 0.), None);
    assert_eq!(editor.group_scroll(GROUP), content - view);
    editor.set_group_scroll(GROUP, -5.);
    assert_eq!(editor.group_scroll(GROUP), 0.);
    wheel(&mut ui, (100., BLOCK_Y), (-50., 0.), None);
    assert_eq!(editor.group_scroll(GROUP), 0.);
    // A frame of the app's: how far its lines reach, an offset past it brought back.
    editor.set_group_scroll(GROUP, 300.);
    editor.set_group_frame(GROUP, GroupFrame::new(None, None, Some(500.)));
    assert_eq!(editor.group_extent(GROUP), Some((370., 500.)));
    assert_eq!(editor.group_scroll(GROUP), 130.);
}

/// The caret moved to the end of the wide line: the group moves to show it.
/// Scrolled out of the frame, the caret hides.
#[test]
fn a_caret_moved_past_the_frame_reveals_it_and_one_scrolled_out_hides() {
    let (mut ui, editor, _) = note(true, false, |_| Vec::new());
    let end = line_range(1).end;
    ui.dispatch(InputEvent::SetTextSelection {
        anchor: end,
        focus: end,
    });
    ui.prepare_frame();
    let (view, content) = editor.group_extent(GROUP).unwrap();
    let x = editor.group_scroll(GROUP);
    assert!(x > 0. && x <= content - view, "revealed: {x}");
    let caret = ui.scene.borrow().transformed_bounds(editor.caret);
    assert!(caret.x <= 400. && caret.x >= 30., "{caret:?}");
    let opacity = |ui: &Ui| ui.scene.borrow().effects(editor.caret).opacity;
    assert_eq!(opacity(&ui), 1.);
    // Scrolled back by the wheel: the caret stays where it is in the text,
    // out of the frame, hidden.
    wheel(&mut ui, (100., BLOCK_Y), (-x, 0.), None);
    assert_eq!(editor.group_scroll(GROUP), 0.);
    assert_eq!(opacity(&ui), 0.);
}

/// With a group scrollbar (`View::group_scrollbar`): none until a group
/// moves, then one at the block that moved, from its frame's left.
#[test]
fn a_group_scrollbar_shows_at_the_block_that_moved() {
    let (ui, editor, _) = note(true, true, |_| Vec::new());
    let holder = |ui: &Ui| {
        let scene = ui.scene.borrow();
        let line = line_node(ui, &editor, "intro");
        let viewport = scene.parent(scene.parent(line).unwrap()).unwrap();
        scene
            .children(viewport)
            .iter()
            .map(|id| scene.transform(*id))
            .any(|t| (t.x, t.y) == (30., 20.))
    };
    assert!(!holder(&ui), "no group moved: no bar placed");
    editor.set_group_scroll(GROUP, 100.);
    ui.prepare_frame();
    assert!(holder(&ui), "at the block: its left, its first line's top");
}

/// Without a group, nothing of this is there: lines in the lines' layer,
/// no mark cut, no extent.
#[test]
fn without_a_group_nothing_is_cut() {
    let (ui, editor, _) = note(false, false, |i| match i {
        1 => vec![mark(60., 2.)],
        _ => Vec::new(),
    });
    assert_eq!(editor.group_extent(GROUP), None);
    let scene = ui.scene.borrow();
    let wide = line_node(&ui, &editor, "let wide");
    let intro = line_node(&ui, &editor, "intro");
    assert_eq!(scene.parent(wide), scene.parent(intro), "one layer");
    assert!(
        marks(&ui, &editor, 2.)
            .iter()
            .all(|(_, clip)| clip.is_none())
    );
}

/// "```" typed above lines puts them in a code block (their highlighter
/// state, which their room depends on): they join its scroll group in that
/// frame, and leave it as the fence goes.
#[test]
fn a_fence_typed_above_lines_groups_them() {
    let mut ui = Ui::new(600., 400.);
    let wide = "y".repeat(200);
    let value = ui.signal(format!("intro\n\n{wide}\nshort"));
    let view = ui.mount(
        text_area("Note", value)
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true)
            .span_highlighter(|line: &str, state| {
                (Vec::new(), if line == "```" { 1 - state } else { state })
            }),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_line_insets(Some(Rc::new(|_, line: &str, state| {
        let mut inset = LineInset::default();
        if state == 1 && line != "```" {
            inset.scroll_group = Some(GROUP);
            inset.nowrap = true;
        }
        inset
    })));
    ui.input.focus(&ui.scene, Some(view.node()));
    ui.prepare_frame();
    assert_eq!(editor.group_extent(GROUP), None);
    let at = "intro\n".len();
    editor.set_selection(at, at);
    for _ in 0..3 {
        ui.dispatch(InputEvent::Text("`".into()));
        ui.prepare_frame();
    }
    let (view_width, content) = editor
        .group_extent(GROUP)
        .expect("the lines below are in it");
    assert!(content > view_width, "{content} in {view_width}");
    editor.set_selection(at, at + 3);
    ui.dispatch(InputEvent::Text("x".into()));
    ui.prepare_frame();
    assert_eq!(editor.group_extent(GROUP), None, "the fence gone");
}

/// A group an edit cut short is brought back, and its listener, told once
/// the draw is done, may set the area's marks and draw it.
#[test]
fn a_group_cut_short_tells_its_listener_after_the_draw() {
    let mut ui = Ui::new(600., 400.);
    let value = ui.signal(format!("a\nc{}\nd", "x".repeat(110)));
    let view = ui.mount(
        text_area("Note", value.clone())
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true)
            .line_insets(|_, line: &str, _| {
                if line.starts_with('c') {
                    LineInset::new(30., 0., 0.).in_group(GROUP).unwrapped()
                } else {
                    LineInset::default()
                }
            }),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    editor.set_group_scroll(GROUP, 200.);
    ui.prepare_frame();
    let heard = Rc::new(RefCell::new(Vec::new()));
    let (log, handle) = (heard.clone(), editor.clone());
    editor.on_group_scroll(move |group, x| {
        log.borrow_mut().push((group, x));
        handle.set_line_marks(Some(Rc::new(|_, _, _| vec![mark(0., 2.)])));
    });
    value.set("a\nc\nd".to_string());
    ui.prepare_frame();
    assert_eq!(*heard.borrow(), [(GROUP, 0.)]);
    assert_eq!(editor.group_scroll(GROUP), 0.);
    assert_eq!(marks(&ui, &editor, 2.).len(), 3);
}

/// A theme change asks the highlighter for the lines shown, not the note:
/// its lines' room, and their group, hold. A new highlighter is asked for
/// them all, and groups what its states say.
#[test]
fn a_theme_change_remeasures_nothing_and_a_new_highlighter_does() {
    use zgx::theme::{Theme, ThemeKind, tok};
    let mut ui = Ui::new(600., 400.);
    for (name, color) in [("a", Color(255, 0, 0, 255)), ("b", Color(0, 255, 0, 255))] {
        let theme = Theme::new(name, ThemeKind::Dark).with(tok::TEXT_PRIMARY, color);
        ui.theme().register(theme);
    }
    ui.theme().set("a").unwrap();
    let below: String = (0..1_000).map(|i| format!("\nline {i}")).collect();
    let value = ui.signal(format!("```{below}"));
    let (asked, room) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (a, r) = (asked.clone(), room.clone());
    let view = ui.mount(
        text_area("Note", value)
            .w(400.)
            .h(300.)
            .p(0.)
            .text_size(10.)
            .line_height(20.)
            .text_wrap(true)
            .span_highlighter(move |line: &str, state| {
                a.set(a.get() + 1);
                (Vec::new(), if line == "```" { 1 - state } else { state })
            })
            .line_insets(move |_, line: &str, state| {
                r.set(r.get() + 1);
                if state == 1 && line != "```" {
                    LineInset::new(0., 0., 0.).in_group(GROUP)
                } else {
                    LineInset::default()
                }
            }),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    assert!(editor.group_extent(GROUP).is_some());
    asked.set(0);
    room.set(0);
    ui.theme().set("b").unwrap();
    ui.prepare_frame();
    assert!(asked.get() > 0, "asked for its colours anew");
    assert!(asked.get() < 40, "{}", asked.get());
    assert!(room.get() < 200, "{}", room.get());
    assert!(editor.group_extent(GROUP).is_some());
    editor.set_span_highlighter(Some(Rc::new(|_: &str, _| (Vec::new(), 0))));
    ui.prepare_frame();
    assert_eq!(editor.group_extent(GROUP), None);
}
