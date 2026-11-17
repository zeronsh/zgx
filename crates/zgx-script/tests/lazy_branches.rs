//! A `ui.show` or `ui.switch` branch given as a function is built only when
//! it shows, each time it does, in a scope of its own (`ui.use_state`
//! starts again), as React's conditional rendering: a row's branches that
//! never show are never built.
mod common;
use common::Harness;
use zgx_script::Value;

fn number(h: &Harness, name: &str) -> f64 {
    h.host
        .states()
        .into_iter()
        .find(|(n, _)| n == name)
        .and_then(|(_, v)| v.as_f64())
        .unwrap()
}

#[test]
fn a_function_branch_is_built_only_when_it_shows() {
    let mut h = Harness::inline(
        r#"local open = ui.state("open", false)
        local built = ui.state("built", 0)
        local otherwise = ui.state("otherwise", 0)
        return ui.column {
            ui.show(open, function()
                built:set(built:peek() + 1)
                return ui.text { id = "shown", "shown" }
            end, function()
                otherwise:set(otherwise:peek() + 1)
                return ui.text { id = "hidden", "hidden" }
            end),
        }"#,
    );
    h.frames(2);
    assert_eq!(number(&h, "built"), 0., "never shown, never built");
    assert_eq!(number(&h, "otherwise"), 1.);
    assert_eq!(h.text("hidden"), "hidden");
    h.host.set_state("open", Value::Bool(true)).unwrap();
    h.frames(2);
    assert_eq!(number(&h, "built"), 1.);
    assert_eq!(h.text("shown"), "shown");
    assert_eq!(h.host.error().get(), None);
}

#[test]
fn a_switch_builds_only_its_selected_cases_function() {
    let mut h = Harness::inline(
        r#"local kind = ui.state("kind", "a")
        local counts = ui.state("counts", "")
        local function case(name)
            return function()
                counts:set(counts:peek() .. name)
                return ui.text { id = "case", name }
            end
        end
        return ui.switch(kind, { a = case("a"), b = case("b"), c = case("c") }, case("d"))"#,
    );
    h.frames(2);
    assert_eq!(h.text("case"), "a");
    h.host.set_state("kind", Value::from("c")).unwrap();
    h.frames(2);
    assert_eq!(h.text("case"), "c");
    h.host.set_state("kind", Value::from("zz")).unwrap();
    h.frames(2);
    assert_eq!(h.text("case"), "d", "the default, as a function too");
    let counts = h
        .host
        .states()
        .into_iter()
        .find(|(n, _)| n == "counts")
        .map(|(_, v)| v.to_string());
    assert_eq!(counts.as_deref(), Some("acd"), "each built as it showed");
}

#[test]
fn a_function_branchs_state_starts_again_each_time_it_shows() {
    let mut h = Harness::inline(
        r#"local open = ui.state("open", true)
        return ui.column {
            ui.show(open, function()
                local n = ui.use_state(0)
                return ui.button { id = "b", on_click = function() n:set(n:peek() + 1) end,
                    ui.text { id = "count", function() return tostring(n:get()) end } }
            end),
        }"#,
    );
    h.frames(2);
    assert_eq!(h.host.error().get(), None);
    h.click("b");
    h.frames(1);
    assert_eq!(h.text("count"), "1");
    h.host.set_state("open", Value::Bool(false)).unwrap();
    h.frames(2);
    h.host.set_state("open", Value::Bool(true)).unwrap();
    h.frames(2);
    assert_eq!(h.text("count"), "0", "built again, its state new");
    assert_eq!(h.host.error().get(), None);
}

#[test]
fn a_function_branch_returning_no_node_says_so() {
    let h = Harness::inline(r#"return ui.column { ui.show(true, function() return 42 end) }"#);
    h.frames(2);
    let error = h.host.error().get().unwrap_or_default();
    assert!(!error.is_empty(), "an error is shown");
}

/// As every builder in zgx, a function branch is built once and follows
/// state through its bindings: what it shows updates in place while it
/// shows, the branch not built again (a direct `:get()` reads once, as
/// `zgx lint` says).
#[test]
fn a_function_branchs_bindings_follow_state_while_it_shows() {
    let mut h = Harness::inline(
        r#"local label = ui.state("label", "a")
        local built = ui.state("built", 0)
        return ui.column {
            ui.show(true, function()
                built:set(built:peek() + 1)
                return ui.text { id = "t", function() return "v" .. label:get() end }
            end),
        }"#,
    );
    h.frames(2);
    assert_eq!(h.text("t"), "va");
    h.host.set_state("label", Value::from("b")).unwrap();
    h.frames(2);
    assert_eq!(h.text("t"), "vb", "its binding followed");
    assert_eq!(number(&h, "built"), 1., "built once");
    assert_eq!(h.host.error().get(), None);
}
