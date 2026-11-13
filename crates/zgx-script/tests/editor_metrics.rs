mod common;
use common::Harness;
use zgx_script::Value;

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
    let h = Harness::inline(
        r#"
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
    "#,
    );
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
