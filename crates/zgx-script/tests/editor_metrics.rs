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
