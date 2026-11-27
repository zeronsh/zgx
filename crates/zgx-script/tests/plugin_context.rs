//! A node of a script beside the root reading what the root provides above
//! where it lands (`provide` and `ui.use_context`): plain values converted
//! into the script's VM, a state as the same state (it follows it, and its
//! writes reach the root), the nearest provider first, and nil for a name
//! nobody provides.
mod common;
use common::Harness;
use zgx_script::{Script, ScriptHost};

fn with(test: &str, app: &'static str, code: &str) -> Harness {
    let dir = common::temp_dir(&format!("context-{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("init.luau");
    std::fs::write(&entry, code).unwrap();
    Harness::new(move |runtime| {
        let host = ScriptHost::inline(runtime, "app", app);
        host.set_scripts(vec![Script::file("reader", entry)]);
        host
    })
}

const APP: &str = r##"
local count = ui.state("count", 1)
ui.command { id = "bump", run = function() count:set(count:get() + 1) end }
return ui.column {
    provide = { user = { name = "Ada", tags = { "admin" } }, count = count, tone = "far" },
    ui.row {
        provide = { tone = "near" },
        ui.button { id = "save", w = 60, h = 24, "Save" },
    },
}
"##;

const READER: &str = r##"
local Reader = ui.component(function()
    local user = ui.use_context("user")
    local count = ui.use_context("count")
    local tone = ui.use_context("tone")
    local missing = ui.use_context("missing")
    return ui.column {
        ui.text { id = "user", `{user.name} {user.tags[1]}` },
        ui.text { id = "tone", tone },
        ui.text { id = "missing", if missing == nil then "nil" else "set" },
        ui.text { id = "count", function() return `count {count:get()}` end },
    }
end)
ui.contribute { target = "#save", after = Reader {} }
"##;

#[test]
fn a_scripts_node_reads_what_the_root_provides_above_it() {
    let mut h = with("above", APP, READER);
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("user"), "Ada admin", "a plain table, converted");
    assert_eq!(h.text("tone"), "near", "the nearest provider first");
    assert_eq!(h.text("missing"), "nil", "a name nobody provides");
    assert_eq!(h.text("count"), "count 1");
    assert!(h.host.run_command("bump"));
    h.frames(1);
    assert_eq!(
        h.text("count"),
        "count 2",
        "a state crosses as the same state"
    );
}

#[test]
fn a_state_crossing_provide_is_written_from_the_script() {
    let mut h = with(
        "write",
        r##"
local count = ui.state("count", 1)
return ui.column {
    provide = { count = count },
    ui.text { id = "root-count", function() return `root {count:get()}` end },
    ui.button { id = "save", w = 60, h = 24, "Save" },
}
"##,
        r##"
local Writer = ui.component(function()
    local count = ui.use_context("count")
    return ui.button { id = "bump", w = 60, h = 24, on_click = function() count:set(count:get() + 10) end, "Bump" }
end)
ui.contribute { target = "#save", after = Writer {} }
"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("root-count"), "root 1");
    h.click("bump");
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("root-count"), "root 11", "the root sees the write");
}

#[test]
fn a_provided_function_is_an_error_not_a_crash() {
    let h = with(
        "function",
        r##"return ui.column {
            provide = { open = function() end },
            ui.button { id = "save", w = 60, h = 24, "Save" },
        }"##,
        r##"
local Reader = ui.component(function()
    local open = ui.use_context("open")
    return ui.text { id = "open", type(open) }
end)
ui.contribute { target = "#save", after = Reader {} }
"##,
    );
    h.frames(1);
    let error = h.error().unwrap_or_default();
    assert!(error.contains("can't cross into this one"), "{error}");
}

#[test]
fn a_script_reads_what_it_provides_itself() {
    let mut h = with(
        "own",
        r##"return ui.column { ui.button { id = "save", w = 60, h = 24, "Save" } }"##,
        r##"
local count = ui.state("mine", 3)
local Reader = ui.component(function()
    local mine = ui.use_context("mine")
    return ui.text { id = "mine", function() return `mine {mine:get()}` end }
end)
ui.contribute { target = "#save", after = ui.column { provide = { mine = count }, Reader {} } }
"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("mine"), "mine 3");
}

const ROWS: &str = r##"
local names = ui.state("names", { "a", "b" })
return ui.column {
    ui.each(names, function(name)
        return ui.row { id = "row", ui.text { id = `name-{name}`, name } }
    end),
}
"##;

/// Beside a row of `ui.each`, what a script adds is drawn, a row each.
#[test]
fn before_or_after_a_rows_root_is_drawn_in_each_row() {
    let mut h = with(
        "row",
        ROWS,
        r##"ui.contribute { target = "#row", after = ui.text { id = "extra", "Extra" } }"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(
        h.texts().iter().filter(|t| t.as_str() == "Extra").count(),
        2
    );
}

/// Inside a row, an element takes what scripts add, as anywhere.
#[test]
fn inside_a_row_an_element_takes_what_scripts_add() {
    let mut h = with(
        "inside",
        ROWS,
        r##"ui.contribute { target = "#name-a", after = ui.text { id = "extra", "Extra" } }"##,
    );
    h.frames(1);
    assert_eq!(h.error(), None);
    assert_eq!(h.text("extra"), "Extra");
}
