//! A host going idle collects its scripts' garbage (`zgx::idle`): Luau's
//! collector runs only as scripts allocate, so nodes a script let go of
//! would otherwise wait for the scripts to run a while.
mod common;

use common::Harness;

#[test]
fn going_idle_collects_the_scripts_garbage() {
    let h = Harness::inline(
        "local made = {}\n\
         for i = 1, 2000 do made[i] = ui.text { \"row\" } end\n\
         made = nil\n\
         return ui.text { \"x\" }\n",
    );
    assert_eq!(h.error(), None);
    let before = h.host.script_heap();
    zgx::idle::trim(&h.ui.runtime);
    let after = h.host.script_heap();
    assert!(
        after < before * 3 / 4,
        "going idle freed the dropped nodes: {before} → {after} bytes"
    );
    // Idle stays idle: nothing more to free, nothing more done.
    zgx::idle::trim(&h.ui.runtime);
    assert!(h.host.script_heap() <= after);
}

#[test]
fn a_host_configured_after_it_is_made_still_collects_when_idle() {
    // A builder method makes the host again (`ScriptHost::configure`): its
    // idle trim must follow it.
    let h = Harness::new(|runtime| {
        zgx_script::ScriptHost::inline(
            runtime,
            "app",
            "local made = {}\n\
             for i = 1, 2000 do made[i] = ui.text { \"row\" } end\n\
             made = nil\n\
             return ui.text { \"x\" }\n",
        )
        .app_id("dev.zgx.idle")
        .image_cache(1 << 20)
    });
    assert_eq!(h.error(), None);
    let before = h.host.script_heap();
    zgx::idle::trim(&h.ui.runtime);
    assert!(h.host.script_heap() < before * 3 / 4);
}

#[test]
fn collect_garbage_frees_what_scripts_dropped() {
    let h = Harness::inline(
        "local made = {}\n\
         for i = 1, 2000 do made[i] = ui.text { \"row\" } end\n\
         made = nil\n\
         return ui.text { \"x\" }\n",
    );
    let before = h.host.script_heap();
    h.host.collect_garbage();
    assert!(h.host.script_heap() < before * 3 / 4);
}

/// A long list's rows, read one at a time as a virtual list's are, are no copy
/// of the list in the VM: a row's table goes with what read it, while the list
/// (a chat's transcript, still open) stands. Measured against what a copy costs
/// (the same rows read and held): the heap's own pages, partly used, move what
/// is left by tens of kilobytes from one run to the next, a copy by megabytes.
#[test]
fn a_long_list_s_rows_are_no_copy_of_it_in_the_vm() {
    const APP: &str = "local rows = {}\n\
         for i = 1, 1000 do local parts = {}; for p = 1, 16 do parts[p] = { id = `p{p}`, text = `part {i} {p}` } end; rows[i] = { id = `row{i}`, text = string.rep(\"words \", 40), parts = parts } end\n\
         local list = ui.state(\"list\", rows)\n\
         rows = nil\n\
         local kept = {}\n\
         local function read_all(keep)\n\
           local n = 0\n\
           for i = 1, list:len() do local row = list:at(i); n += #row.text; if keep then kept[i] = row end end\n\
           return n\n\
         end\n\
         return ui.column {\n\
           ui.button { \"read\", id = \"read\", on_click = function() read_all(false) end },\n\
           ui.button { \"keep\", id = \"keep\", on_click = function() read_all(true) end },\n\
         }\n";
    // Clicked, then the heap after the window goes idle, less before.
    let grown = |button: &str| {
        let mut h = Harness::inline(APP);
        assert_eq!(h.error(), None);
        zgx::idle::trim(&h.ui.runtime);
        let before = h.host.script_heap();
        h.click(button);
        assert_eq!(h.error(), None);
        zgx::idle::trim(&h.ui.runtime);
        h.host.script_heap().saturating_sub(before)
    };
    let copy = grown("#keep");
    assert!(
        copy > 1 << 20,
        "holding the rows costs a copy: {copy} bytes"
    );
    let read = grown("#read");
    assert!(
        read < copy / 16,
        "reading them left no copy: {read} bytes stayed, a copy {copy}"
    );
}

/// A list of folded rows reads as the rows they open to, and costs what they
/// are until a script reads them: its length and keys open none, and a row read
/// opens once while a script holds it.
#[test]
fn folded_rows_open_as_they_are_read() {
    use std::{cell::Cell, rc::Rc};
    use zgx_script::{Unfold, Value};
    thread_local! {
        static OPENED: Cell<usize> = const { Cell::new(0) };
    }
    struct Row(usize);
    impl Unfold for Row {
        fn unfold(&self) -> Value {
            OPENED.set(OPENED.get() + 1);
            let parts: Vec<Value> = (0..40).map(|p| Value::Num(p as f64)).collect();
            Value::record([
                ("id", Value::from(format!("r{}", self.0).as_str())),
                ("text", Value::from(format!("row {}", self.0).as_str())),
                ("parts", Value::List(parts.into())),
            ])
        }
    }
    let mut h = Harness::inline(
        "local list = ui.shared(\"rows\", {})\n\
         local kept = ui.state(\"said\", \"\")\n\
         local row\n\
         return ui.column {\n\
           ui.text { id = \"count\", function() return tostring(list:len()) end },\n\
           ui.text { id = \"where\", function() return tostring(list:index_of(\"id\", \"r5\")) end },\n\
           ui.button { \"read\", id = \"read\", on_click = function()\n\
             row = list:at(6)\n\
             kept:set(row.text .. \"/\" .. #row.parts .. \"/\" .. tostring(list:at(6) == row))\n\
           end },\n\
           ui.text { id = \"said\", function() return kept:get() end },\n\
           ui.button { \"drop\", id = \"drop\", on_click = function() row = nil end },\n\
         }\n",
    );
    assert_eq!(h.error(), None);
    let rows: Vec<Value> = (0..100)
        .map(|i| Value::folded(Rc::from(format!("r{i}")), Row(i)))
        .collect();
    h.host
        .shared("rows", Value::Nil)
        .set(Value::List(rows.into()));
    h.frames(2);
    assert_eq!(h.text("#count"), "100");
    assert_eq!(h.text("#where"), "6");
    assert_eq!(OPENED.get(), 0, "a list's length and keys open no row");
    h.click("#read");
    assert_eq!(h.error(), None);
    assert_eq!(
        h.text("#said"),
        "row 5/40/true",
        "read as what it opens to, one table while held"
    );
    assert_eq!(OPENED.get(), 1, "held, it opens once");
    h.click("#drop");
    zgx::idle::trim(&h.ui.runtime);
    h.click("#read");
    assert_eq!(h.text("#said"), "row 5/40/true");
    assert_eq!(OPENED.get(), 2, "let go, it opens again: nothing kept it");
}
