mod common;
use common::Harness;
use zgx_script::{Limits, ScriptHost, Value};

fn metric(h: &Harness) -> Value {
    h.host
        .states()
        .into_iter()
        .find(|(k, _)| k == "metrics")
        .unwrap()
        .1
}

#[test]
fn natural_metrics_are_reactive_after_layout_and_independent_of_viewport() {
    let mut h = Harness::inline(
        r#"
        local ref = ui.ref()
        local width = ui.state("width", 80)
        local viewport = ui.state("viewport", 300)
        local text = ui.state("text", "one\ntwo\n")
        local metrics = ui.state("metrics", nil)
        local reads = ui.state("reads", 0)
        ui.effect(function()
            local el = ref:get()
            reads:set(reads:peek() + 1)
            metrics:set(if el then el:text_metrics(width:get()) else nil)
        end)
        return ui.textarea { id = "editor", ref = ref, value = text,
            w = viewport, h = 40, text_size = 10, line_height = 20 }
    "#,
    );
    h.frames(3);
    assert_eq!(h.error(), None);
    let before = metric(&h);
    assert_eq!(
        before.field("wrapped_height").and_then(Value::as_f64),
        Some(60.)
    );
    assert_eq!(before.field("exact"), Some(&Value::Bool(true)));
    h.host.set_state("viewport", Value::Num(120.)).unwrap();
    h.frames(3);
    assert_eq!(metric(&h), before);
    let reads = h
        .host
        .states()
        .into_iter()
        .find(|(k, _)| k == "reads")
        .unwrap()
        .1;
    h.click("editor");
    h.frames(3);
    assert_eq!(metric(&h), before, "focus is not a metric revision");
    assert_eq!(
        h.host
            .states()
            .into_iter()
            .find(|(k, _)| k == "reads")
            .unwrap()
            .1,
        reads,
        "focus/viewport changes must not notify natural metric readers"
    );
    h.host
        .set_state("text", Value::from("words words words words\n"))
        .unwrap();
    h.frames(3);
    let edited = metric(&h);
    assert_ne!(edited.field("revision"), before.field("revision"));
    h.host.set_state("width", Value::Num(30.)).unwrap();
    h.frames(3);
    let narrow = metric(&h);
    assert_eq!(narrow.field("revision"), edited.field("revision"));
    assert!(
        narrow
            .field("wrapped_height")
            .and_then(Value::as_f64)
            .unwrap()
            > edited
                .field("wrapped_height")
                .and_then(Value::as_f64)
                .unwrap()
    );
    h.frames(60);
    assert_eq!(metric(&h), narrow, "no idle notifications or layout cycle");
    assert_eq!(h.error(), None);
}

#[test]
fn a_loop_of_widths_is_stopped_by_the_script_budget() {
    // Each new width lays out every line: on the clock, a loop of them ends
    // as any runaway loop does. Off it, the loop's own instructions are all
    // that count, and its 2,000 layouts (seconds) finish unchecked.
    let code = r#"
        local ref = ui.ref()
        local go = ui.state("go", false)
        local text = string.rep("a line of words that wraps at some width\n", 2000)
        ui.effect(function()
            if not go:get() then return end
            local el = ref:peek()
            for i = 1, 2000 do
                el:text_metrics(100 + i / 1000)
            end
        end)
        return ui.textarea { ref = ref, value = text, w = 300, h = 100 }
    "#;
    // The budget an app runs with (`zgx run`'s).
    let h = Harness::new(|runtime| {
        ScriptHost::inline(runtime, "app", code).limits(Limits::app_budgets())
    });
    h.frames(3);
    h.host.set_state("go", Value::Bool(true)).unwrap();
    h.frames(1);
    let error = h.error().unwrap_or_default();
    assert!(error.contains("time budget"), "{error}");
}

#[test]
fn a_highlighter_may_edit_its_editor_while_it_is_measured() {
    let h = Harness::inline(
        r##"
        local ref = ui.ref()
        local el = nil
        local editing = ui.state("editing", false)
        local metrics = ui.state("metrics", nil)
        ui.effect(function()
            el = ref:get()
            if el and editing:get() then
                metrics:set(el:text_metrics(91))
            end
        end)
        local edits = 0
        return ui.textarea { ref = ref, value = ui.state("text", "abc def\nghi"), w = 300, h = 100,
            highlight = function(line, state)
                -- Once, as the measurement shapes the line (the view's own
                -- layout calling one that edits is another matter).
                if el and editing:peek() and edits < 1 then
                    edits += 1
                    el:replace_selection("x")
                end
                return { { 1, 2, "#ff0000", weight = 700 } }, state
            end }
    "##,
    );
    h.frames(3);
    h.host.set_state("editing", Value::Bool(true)).unwrap();
    h.frames(3);
    assert_eq!(h.error(), None);
    assert!(metric(&h).field("wrapped_height").is_some());
}

/// A highlighter's span with a line pitch of its own makes its line that
/// tall, and the lines below it move down; `size`, `underline` and
/// `font = "mono"` are read too, with no error.
#[test]
fn a_highlight_spans_pitch_and_size_shape_its_line() {
    let h = Harness::inline(
        r#"
        local ref = ui.ref()
        local metrics = ui.state("metrics", nil)
        ui.effect(function()
            local el = ref:get()
            metrics:set(if el then el:text_metrics(300) else nil)
        end)
        return ui.textarea { ref = ref, value = ui.state("text", "Title\nbody\n"),
            w = 300, h = 100, text_size = 10, line_height = 20,
            highlight = function(line, state)
                if line == "Title" then
                    return { { 1, 5, size = 16, line_height = 44, underline = true } }, state
                end
                return { { 1, 2, font = "mono" } }, state
            end }
    "#,
    );
    h.frames(4);
    assert_eq!(h.error(), None);
    let height = metric(&h)
        .field("wrapped_height")
        .and_then(Value::as_f64)
        .unwrap();
    assert!(
        height >= 44. + 20. + 20.,
        "the title's pitch pushes the rest: {height}"
    );
}

/// `line_insets` gives a line room of its own from its text and the state
/// `highlight` was given for it: a fenced block's lines, inside it by their
/// state, each take room above; a hidden line takes none.
#[test]
fn line_insets_give_lines_room_by_their_text_and_state() {
    let h = Harness::inline(
        r#"
        local ref = ui.ref()
        local metrics = ui.state("metrics", nil)
        ui.effect(function()
            local el = ref:get()
            metrics:set(if el then el:text_metrics(300) else nil)
        end)
        return ui.textarea { ref = ref, value = ui.state("text", "a\n```\nb\n```\nc\nhidden"),
            w = 300, h = 200, text_size = 10, line_height = 20,
            highlight = function(line, state)
                if line == "```" then
                    return {}, if state == "code" then 0 else "code"
                end
                return {}, state
            end,
            line_insets = function(line, state)
                if line == "hidden" then
                    return { hidden = true }
                end
                if state == "code" then
                    return { above = 10 }
                end
                return nil
            end }
    "#,
    );
    h.frames(4);
    assert_eq!(h.error(), None);
    let height = |h: &Harness| {
        metric(h)
            .field("wrapped_height")
            .and_then(Value::as_f64)
            .unwrap()
    };
    // Five lines of 20 (the sixth hidden), and 10 above "b" and the
    // closing fence.
    assert_eq!(height(&h), 120.);
}

/// `line_marks` draws shapes beside lines, given each line's text and the
/// state `highlight` was given for it: a fenced block's lines, inside it by
/// their state, each take a numbered label.
#[test]
fn line_marks_draw_beside_lines_by_their_text_and_state() {
    let mut h = Harness::inline(
        r##"
        return ui.textarea { value = ui.state("text", "a\n```\nb\nc\n```\nd"),
            w = 300, h = 200, text_size = 10, line_height = 20,
            highlight = function(line, state)
                if line == "```" then
                    return {}, if state == "code" then 0 else "code"
                end
                return {}, state
            end,
            line_marks = function(line, state)
                if state == "code" and line ~= "```" then
                    return { { x = 0, w = "fill", h = "fill", fill = "#eeeeee",
                        label = "#" .. line, label_color = "#112233", label_size = 10,
                        mono = true } }
                end
                return nil
            end }
    "##,
    );
    h.frames(4);
    assert_eq!(h.error(), None);
    let texts = h.texts();
    assert!(texts.iter().any(|t| t == "#b"), "b's label: {texts:?}");
    assert!(texts.iter().any(|t| t == "#c"), "c's label: {texts:?}");
    assert!(
        !texts.iter().any(|t| t == "#a" || t == "#d"),
        "outside: {texts:?}"
    );
}

/// A fenced code block's lines, put in one scroll group and never wrapped
/// by `line_insets` (by their state), scroll sideways together under a
/// wheel's sideways motion; its wash, `pinned`, stays where it is, and the
/// prose around it wraps and stays.
#[test]
fn a_code_blocks_lines_scroll_sideways_in_a_group() {
    let wide = "x".repeat(120);
    let mut h = Harness::inline(&format!(
        r##"
        return ui.textarea {{ id = "note", value = ui.state("text", "a\n```\n{wide}\nb\n```\n{prose}"),
            w = 300, h = 200, p = 0, text_size = 10, line_height = 20, wrap = true,
            group_scrollbar = true,
            highlight = function(line, state)
                if line == "```" then
                    return {{}}, if state == "code" then 0 else "code"
                end
                return {{}}, state
            end,
            line_insets = function(line, state)
                if state == "code" and line ~= "```" then
                    return {{ left = 20, group = 7, nowrap = true }}
                end
                return nil
            end,
            line_marks = function(line, state)
                if state == "code" and line ~= "```" then
                    return {{ {{ x = 0, w = "fill", h = "fill", fill = "#eeeeee", pinned = true }},
                        {{ x = 20, w = 4, h = 4, fill = "#ff0000" }} }}
                end
                return nil
            end }}
    "##,
        prose = "word ".repeat(40),
    ));
    h.frames(4);
    assert_eq!(h.error(), None);
    let node = h.find("note").expect("the note");
    let editor = h.ui.editor(node).expect("an editor");
    let (view, content) = editor.group_extent(7).expect("its block's group");
    assert_eq!(view, 280.);
    assert!(content > view, "the wide line reaches past it: {content}");
    let start = 6;
    assert_eq!(
        editor.range_boxes(start..start + 120).len(),
        1,
        "the wide line is one row"
    );
    // "a\n```\n", the wide line, "\nb\n```\n", then the prose.
    let prose = start + 120 + 7;
    let rows = editor.range_boxes(prose..prose + 200).len();
    assert!(rows >= 2, "the prose wraps");
    let before = editor.range_boxes(start + 40..start + 41)[0];
    let at = h.ui.scene.borrow().transformed_bounds(node);
    h.ui.dispatch(zgx::input::InputEvent::Scroll {
        x: at.x + 100.,
        y: at.y + before.y + 5.,
        delta_x: 30.,
        delta_y: 0.,
    });
    h.frames(2);
    assert_eq!(editor.group_scroll(7), 30.);
    let after = editor.range_boxes(start + 40..start + 41)[0];
    assert!(
        (before.x - after.x - 30.).abs() < 0.01,
        "{before:?} {after:?}"
    );
    assert_eq!(editor.scroll_offset(), (0., 0.), "the note didn't move");
    assert_eq!(h.error(), None);
}
