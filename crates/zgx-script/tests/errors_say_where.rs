//! A script's error says where: the line it failed on, or the function
//! that failed and the role zgx called it in, as the terminal, the banner
//! and `zgx inspect` show it (all one message, `ScriptHost::error`). One
//! test for each kind: a function zgx called returning what it can't take
//! (no traceback is left once it returned, so the function names its line),
//! an error a function zgx called raised (its role above Luau's own line
//! and traceback), and a Rust function's error to the script calling it
//! (the script's line, from the traceback, where Luau puts its own).
mod common;
use common::Harness;
use zgx_script::{ScriptHost, testing::TestApp};

/// The error `code` leaves once loaded and drawn.
fn error_of(code: &str) -> String {
    let h = Harness::inline(code);
    h.frames(3);
    h.host
        .error()
        .with_untracked(|e| e.clone())
        .unwrap_or_else(|| panic!("an error for:\n{code}"))
}

#[test]
fn a_row_returning_nothing_names_the_row_function_of_ui_each() {
    let error = error_of(
        r#"
local items = ui.state("items", { 1, 2 })
return ui.column {
	ui.each(items, function(i)
		local _ = i
	end),
}
"#,
    );
    assert!(
        error.contains("app:4: the row function of ui.each returned nothing; return a ui node"),
        "{error}"
    );
}

#[test]
fn a_list_row_returning_a_number_names_the_row_function_of_ui_list() {
    let error = error_of(
        r#"
local rows = ui.state("rows", { "a", "b" })
return ui.list {
	data = rows,
	height = 20,
	row = function(r)
		return 5
	end,
}
"#,
    );
    assert!(
        error.contains("app:6: the row function of ui.list returned a number; return a ui node"),
        "{error}"
    );
}

#[test]
fn a_lazy_branch_returning_nothing_names_its_role_in_ui_show_and_ui_switch() {
    let error = error_of(
        r#"
local on = ui.state("on", true)
return ui.column {
	ui.show(on, function()
	end),
}
"#,
    );
    assert!(
        error.contains("app:4: the content function of ui.show returned nothing"),
        "{error}"
    );
    let error = error_of(
        r#"
local tab = ui.state("tab", "a")
return ui.column {
	ui.switch(tab, {
		a = function()
			return "not a node"
		end,
	}),
}
"#,
    );
    assert!(
        error.contains("app:5: a case function of ui.switch returned a string"),
        "{error}"
    );
}

#[test]
fn a_component_returning_a_number_names_the_components_function() {
    let error = error_of(
        r#"
local Card = ui.component(function(props)
	return 1
end)
return ui.column { Card {} }
"#,
    );
    assert!(
        error.contains("app:2: the component's function returned a number; return a ui node"),
        "{error}"
    );
}

#[test]
fn a_key_function_failing_names_its_role_and_its_line() {
    let error = error_of(
        r#"
local items = ui.state("items", { 1, 2 })
return ui.column {
	ui.each(items, function(i)
		return ui.text { tostring(i) }
	end, {
		key = function(i)
			error("no key for " .. i)
		end,
	}),
}
"#,
    );
    assert!(
        error.contains("in the key function of ui.each (defined at app:7):"),
        "{error}"
    );
    assert!(error.contains("app:8: no key for"), "{error}");
}

#[test]
fn a_handler_failing_names_its_role_and_its_line() {
    let mut h = Harness::inline(
        r#"
return ui.button {
	id = "go",
	on_click = function()
		error("go failed")
	end,
	"Go",
}
"#,
    );
    h.click("go");
    let error = h
        .host
        .error()
        .with_untracked(|e| e.clone())
        .expect("an error");
    assert!(
        error.contains("in an event handler (defined at app:4):"),
        "{error}"
    );
    assert!(error.contains("app:5: go failed"), "{error}");
}

#[test]
fn an_effect_failing_names_ui_effect() {
    let error = error_of(
        r#"
ui.effect(function()
	error("effect failed")
end)
return ui.column {}
"#,
    );
    assert!(
        error.contains("in the body of ui.effect (defined at app:2):"),
        "{error}"
    );
    assert!(error.contains("app:3: effect failed"), "{error}");
}

#[test]
fn a_computed_value_it_cant_keep_names_ui_computed() {
    let error = error_of(
        r#"
local kept = ui.computed(function()
	return function() end
end)
return ui.text { function() return tostring(kept:get()) end }
"#,
    );
    assert!(
        error.contains("in the function of ui.computed (defined at app:2):"),
        "{error}"
    );
}

#[test]
fn a_text_function_returning_a_function_names_the_text_function() {
    let error = error_of(
        r#"
return ui.text { function()
	return function() end
end }
"#,
    );
    assert!(
        error.contains("in the text function of ui.text (defined at app:2):"),
        "{error}"
    );
}

#[test]
fn a_rust_functions_error_names_the_script_line_that_called_it() {
    // The constructor refuses its child: the line is the script's call, as
    // Luau's own errors name theirs.
    let error = TestApp::try_new(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            "return ui.column {\n\tgap = 4,\n\tui.row { true },\n}\n",
        )
    })
    .err()
    .expect("it fails to load");
    assert!(
        error.contains("runtime error: app:3: a child must be a ui node"),
        "{error}"
    );
}
