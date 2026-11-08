use std::{cell::Cell, rc::Rc};
use zgx::{
    compose::prelude::*,
    input::InputEvent,
    rich_text::RichText,
    scene::Color,
    text_layout::{FallbackTextLayout, FontStyle, LetterSpacing},
    widgets::{EditorHandle, SpanStyle, Ui},
};

fn fixture(text: &str) -> (Ui, EditorHandle, Rc<Cell<usize>>) {
    let mut ui = Ui::new(400., 300.);
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    ui.scene.borrow_mut().set_font_text_shaper(
        move |text: &str, size, width, font: &FontStyle| -> Box<dyn zgx::text_layout::TextLayout> {
            count.set(count.get() + 1);
            Box::new(FallbackTextLayout::with_line_height(
                text,
                size,
                width,
                font.line_height,
            ))
        },
    );
    let view = ui.mount(
        text_area("Draft", ui.signal(text.to_owned()))
            .size(300., 90.)
            .p(10.)
            .text_size(10.)
            .line_height(20.),
    );
    ui.prepare_frame();
    let editor = ui.editor(view.node()).unwrap();
    (ui, editor, calls)
}

#[test]
fn exact_metrics_preserve_empty_unicode_and_final_lines_at_requested_width() {
    let (ui, editor, _) = fixture("");
    let empty = editor.text_metrics(60.).unwrap();
    assert_eq!(empty.unwrapped_width, 0.);
    assert_eq!(empty.wrapped_height, 20.);
    assert_eq!(empty.line_height, 20.);
    for text in ["one\ntwo\n", "你好🙂\n", "abcdefghijabcdefghij\n"] {
        editor.value().set(text.into());
        ui.prepare_frame();
        let natural = editor.text_metrics(60.).unwrap();
        let expected = FallbackTextLayout::with_line_height(
            text,
            10.,
            Some(60.),
            zgx::text_layout::LineHeight::px(20.),
        );
        use zgx::text_layout::TextLayout;
        assert_eq!(natural.wrapped_height, expected.size().1, "{text:?}");
        assert!(natural.revision > empty.revision);
        assert_eq!(natural.width, 60.);
    }
    for width in [0., -1., f32::NAN, f32::INFINITY] {
        assert!(editor.text_metrics(width).is_none());
    }
}

#[test]
fn viewport_focus_selection_and_scroll_reuse_metrics_but_content_and_font_invalidate() {
    let (ui, editor, calls) = fixture(&"words here\n".repeat(2000));
    let before = editor.text_metrics(60.).unwrap();
    let shaped = calls.get();
    for _ in 0..100 {
        assert_eq!(editor.text_metrics(60.), Some(before));
    }
    assert_eq!(calls.get(), shaped, "cached queries must do no shaping");
    ui.input.focus(&ui.scene, Some(editor.node));
    for focus in [0, 5, 100] {
        editor.set_selection(focus, focus);
        ui.prepare_frame();
        assert_eq!(editor.text_metrics(60.), Some(before));
    }
    let narrow = editor.text_metrics(30.).unwrap();
    assert!(narrow.wrapped_height > before.wrapped_height);
    assert_eq!(narrow.revision, before.revision);
    editor.set_typography(Color(123, 45, 67, 255), 10., FontStyle::default());
    ui.prepare_frame();
    assert!(editor.text_metrics(60.).unwrap().revision > before.revision);
}

#[test]
fn span_weights_and_mentions_use_the_displayed_rich_shaper() {
    let (ui, editor, _) = fixture("@mention and bold");
    ui.scene.borrow_mut().set_rich_text_shaper(|rich, width| {
        let runs: Vec<_> = rich
            .runs()
            .iter()
            .cloned()
            .map(|mut run| {
                if run.font.weight >= 600 {
                    run.font.letter_spacing = LetterSpacing::px(3.);
                }
                run
            })
            .collect();
        let rich = RichText::new(rich.text(), runs).unwrap();
        Box::new(FallbackTextLayout::with_rich(&rich, width))
    });
    let plain = editor.text_metrics(80.).unwrap();
    editor.set_span_highlighter(Some(Rc::new(|line, _| {
        (vec![(0..line.len(), SpanStyle::default().weight(600))], 0)
    })));
    ui.prepare_frame();
    let styled = editor.text_metrics(80.).unwrap();
    assert!(styled.unwrapped_width > plain.unwrapped_width);
    assert!(styled.wrapped_height > plain.wrapped_height);
    assert!(styled.revision > plain.revision);
    editor.rehighlight();
    ui.prepare_frame();
    assert!(editor.text_metrics(80.).unwrap().revision > styled.revision);
}

#[test]
fn ime_replaces_selection_without_committing_and_cursor_only_changes_keep_revision() {
    let (mut ui, editor, _) = fixture("abcdef");
    ui.input.focus(&ui.scene, Some(editor.node));
    editor.set_selection(1, 4);
    let original = editor.text_metrics(30.).unwrap();
    ui.dispatch(InputEvent::ImePreedit {
        text: "你好🙂你好".into(),
        cursor: Some((0, 0)),
    });
    ui.prepare_frame();
    let composing = editor.text_metrics(30.).unwrap();
    assert!(composing.revision > original.revision);
    assert!(composing.unwrapped_width > original.unwrapped_width);
    assert_eq!(editor.value().get(), "abcdef");
    ui.dispatch(InputEvent::ImePreedit {
        text: "你好🙂你好".into(),
        cursor: Some((3, 3)),
    });
    ui.prepare_frame();
    assert_eq!(editor.text_metrics(30.), Some(composing));
    ui.dispatch(InputEvent::ImePreedit {
        text: String::new(),
        cursor: None,
    });
    ui.prepare_frame();
    assert_eq!(
        editor.text_metrics(30.).unwrap().unwrapped_width,
        original.unwrapped_width
    );
}

#[test]
fn password_and_unmounted_editors_do_not_expose_metrics() {
    let (mut ui, editor, _) = fixture("secret");
    assert!(editor.text_metrics(60.).is_some());
    editor.set_secure(true);
    assert!(editor.text_metrics(60.).is_none());
    editor.set_secure(false);
    ui.remove(editor.node);
    editor.refresh();
    assert!(editor.text_metrics(60.).is_none());
}

#[test]
#[ignore = "measurement: run with --ignored --nocapture"]
fn natural_metrics_long_draft_cost() {
    let text: String = (0..2000)
        .map(|i| format!("line {i}: a long draft with some words to wrap around\n"))
        .collect();
    let (ui, editor, calls) = fixture(&text);
    let mut fresh = Vec::new();
    let mut cached = Vec::new();
    for i in 0..7 {
        editor.value().set(format!("{text}edit {i}"));
        ui.prepare_frame();
        let start = std::time::Instant::now();
        editor.text_metrics(280.).unwrap();
        fresh.push(start.elapsed().as_micros());
        let shaped = calls.get();
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            std::hint::black_box(editor.text_metrics(280.));
        }
        cached.push(start.elapsed().as_micros());
        assert_eq!(calls.get(), shaped);
    }
    fresh.sort_unstable();
    cached.sort_unstable();
    eprintln!(
        "2001 lines, {} bytes, 7 runs: median fresh={} us; 1000 cached={} us; cached shaping=0",
        text.len(),
        fresh[3],
        cached[3]
    );
}
