//! The work a streamed delta costs: a model published whole on every delta
//! (a transcript as a reply streams in) must cost what changed, not what the
//! model holds. Budgets in tables converted for scripts and rows built.
mod common;

use common::Harness;
use zgx_script::markdown::{Incremental, Options};
use zgx_script::{ScriptHost, Value, tables_converted};

const SCRIPT: &str = r#"local t = ui.shared("t", nil)
local rows = ui.computed(function()
    local v = t:get()
    return if v then v.rows else {}
end)
local builds = ui.state("builds", 0)
return ui.column {
    ui.each(rows, function(row, _, live)
        builds:set(builds:peek() + 1)
        local blocks = ui.computed(function() return live:get().blocks end)
        return ui.column {
            ui.each(blocks, function(b)
                return ui.text { b.t, literal = true }
            end),
        }
    end, { key = function(r) return r.id end }),
    ui.text { id = "builds", "{builds}" },
}"#;

fn row(id: usize, text: &str) -> Value {
    let blocks = zgx_script::markdown::parse(text, &Options::default());
    Value::Map(
        [
            ("blocks".into(), Value::List(blocks.into())),
            ("id".into(), Value::Num(id as f64)),
            ("text".into(), Value::from(text)),
        ]
        .into(),
    )
}

/// Streams a reply after `held` rows; the tables converted per delta, and
/// the rows built while it streamed.
fn per_delta(held: usize) -> (u64, String) {
    let mut h = Harness::new(|runtime| ScriptHost::inline(runtime, "app", SCRIPT));
    let shared = h.host.shared("t", Value::Nil);
    // Each held row: a few paragraphs (sixteen and more values deep, so
    // each is converted and kept on its own).
    let text = "A paragraph of the thread.\n\nAnother, **bold**.\n\n- a list\n- of two\n";
    let publish = |reply: Option<Value>| {
        // Built afresh each time, as a host rebuilding its model does.
        let mut rows: Vec<Value> = (0..held).map(|i| row(i, text)).collect();
        rows.extend(reply);
        shared.set(Value::Map(
            [("rows".into(), Value::List(rows.into()))].into(),
        ))
    };
    publish(None);
    h.settle();
    let mut reply = Incremental::new(Options::default());
    let mut streamed = String::new();
    let reply_row = |reply: &Incremental, text: &str| {
        Value::Map(
            [
                ("blocks".into(), Value::List(reply.blocks().into())),
                ("id".into(), Value::Num(held as f64)),
                ("text".into(), Value::from(text)),
            ]
            .into(),
        )
    };
    // The reply starts, then grows a word at a time.
    streamed.push_str("Once");
    reply.set_text(&streamed);
    publish(Some(reply_row(&reply, &streamed)));
    h.settle();
    let builds = h.text("builds");
    let before = tables_converted();
    let deltas = 40;
    for i in 0..deltas {
        streamed.push_str(if i % 10 == 9 {
            " end.\n\nNext"
        } else {
            " word"
        });
        reply.set_text(&streamed);
        publish(Some(reply_row(&reply, &streamed)));
        h.settle();
    }
    let after = h.text("builds");
    assert_eq!(builds, after, "no row is built again as the reply streams");
    ((tables_converted() - before) / deltas as u64, after)
}

#[test]
fn a_delta_costs_what_changed_not_what_is_held() {
    let (small, _) = per_delta(50);
    let (large, _) = per_delta(500);
    assert_eq!(
        small, large,
        "a delta converts as much with 500 rows held as with 50"
    );
    // The new parts: the model and its row list, the reply's row, its
    // blocks and the growing block's runs, as each reader first reads them.
    assert!(small <= 24, "a delta converts {small} tables");
}

#[test]
fn rows_arriving_later_from_the_host_render() {
    // A new thread: its shared state starts empty, the view mounts, then
    // the host publishes its first rows (a pending message, then the
    // agent's), each as a whole new value.
    let mut h = Harness::new(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r#"local t = ui.shared("t", nil)
            local rows = ui.computed(function()
                local v = t:get()
                return if v then v.rows else {}
            end)
            return ui.column {
                ui.each(rows, function(row, _, live)
                    return ui.presence(function()
                        return live:get()
                    end, function(r)
                        return ui.text { id = "row-" .. r.id, r.text, literal = true }
                    end)
                end, { key = function(r) return r.id end }),
            }"#,
        )
    });
    let shared = h.host.shared("t", Value::Nil);
    h.settle();
    let row = |id: &str, text: &str| {
        Value::Map(
            [
                ("id".into(), Value::from(id)),
                ("text".into(), Value::from(text)),
            ]
            .into(),
        )
    };
    let doc = |rows: Vec<Value>| Value::Map([("rows".into(), Value::List(rows.into()))].into());
    shared.set(doc(vec![row("pending", "hi")]));
    h.settle();
    assert_eq!(h.text("row-pending"), "hi", "the first row renders");
    shared.set(doc(vec![row("pending", "hi"), row("a1", "Hello")]));
    h.settle();
    assert_eq!(h.text("row-a1"), "Hello", "a row added later renders");
    shared.set(doc(vec![row("u1", "hi"), row("a1", "Hello there")]));
    h.settle();
    assert_eq!(h.text("row-u1"), "hi", "a row whose id changed renders");
    assert_eq!(h.text("row-a1"), "Hello there", "a row that grew follows");
}

const MARKDOWN: &str = r###"local reply = ui.state("reply", "")
local n = 0
local blocks = ui.computed(function()
    return ui.markdown(reply:get(), { key = "reply" })
end)
ui.command { id = "section", run = function()
    n += 1
    reply:append("## Section " .. n .. "\n\nWords of a reply, **bold** and `code`, as it streams.\n\n")
end }
ui.command { id = "word", run = function()
    reply:append("more ")
end }
return ui.column {
    ui.each(blocks, function(b)
        return ui.text { b.t, literal = true }
    end),
}"###;

/// Tables and values a chunk converts for the script, the reply holding
/// `sections` sections when it lands.
fn per_chunk(sections: usize) -> (u64, u64) {
    let h = Harness::inline(MARKDOWN);
    for _ in 0..sections {
        assert!(h.host.run_command("section"));
    }
    h.settle();
    let (tables, values) = (tables_converted(), zgx_script::values_converted());
    let chunks = 20;
    for _ in 0..chunks {
        assert!(h.host.run_command("word"));
        h.settle();
        // However the collector runs between chunks, the next derives from
        // this one (`value::hold`): a full collection each time.
        h.host.collect_garbage();
    }
    (
        (tables_converted() - tables) / chunks,
        (zgx_script::values_converted() - values) / chunks,
    )
}

/// A keyed `ui.markdown` stream's answer, read by a script, costs what is
/// new in it: its last block and the list's few changed entries, not every
/// block it holds. The list's table is derived from the last answer's.
#[test]
fn a_streamed_markdown_chunk_converts_what_changed_not_what_it_holds() {
    let (short, long) = (per_chunk(40), per_chunk(240));
    assert_eq!(
        short, long,
        "a chunk converts as much with 240 sections held as with 40"
    );
}

/// `ui.markdown` given the state holding a reply rather than its text
/// answers a state of its blocks: at every append the same blocks as a
/// stream of its text, past where the string becomes a long one
/// (`state:append` keeping its edits, which the stream then parses alone),
/// and kept current as the reply grows (a stale answer would differ). A
/// hanging `**` across chunks included.
#[test]
fn markdown_takes_the_state_holding_its_text() {
    let mut h = Harness::inline(
        r###"local reply = ui.state("reply", "")
local chunks = ui.state("chunks", 0)
local function same(a: any, b: any): boolean
    if type(a) ~= type(b) then return false end
    if type(a) ~= "table" then return a == b end
    for k, v in a do if not same(v, b[k]) then return false end end
    for k in b do if a[k] == nil then return false end end
    return true
end
-- Given the state: a state of its blocks, kept current in Rust.
local by_state = ui.markdown(reply, { key = "state" })
local by_text = ui.computed(function()
    return ui.markdown(reply:get(), { key = "text" })
end)
local words = string.rep("Words of a long paragraph streaming in, ", 20)
ui.command { id = "append", run = function()
    local n = chunks:get() + 1
    chunks:set(n)
    local k = n % 6
    reply:append(if k == 1 then "## Part " .. n .. "\n\n"
        elseif k == 2 then words .. "with **bo"
        elseif k == 3 then "ld** and `code`, " .. words
        elseif k == 4 then "the end.\n\n- one\n- "
        elseif k == 5 then "two\n\n"
        else "```\nlet x = " .. n .. ";\n```\n\n")
end }
return ui.column {
    ui.text { id = "same", function()
        return if same(by_state:get(), by_text:get()) then "same" else "differ"
    end },
    ui.text { id = "blocks", function() return tostring(#by_state:get()) end },
    ui.text { id = "length", function() return tostring(#reply:get()) end },
    -- Read as a list every way: its length, an item, and ui.each's rows.
    ui.text { id = "len", function() return tostring(by_state:len()) end },
    ui.text { id = "first", function() return tostring(by_state:at(1).t) end },
    ui.column {
        ui.each(by_state, function(b: MarkdownBlock, i: number)
            return ui.text { id = "row" .. i, b.t }
        end),
    },
}"###,
    );
    h.settle();
    let mut blocks = 0;
    for chunk in 1..=240 {
        assert!(h.host.run_command("append"));
        h.settle();
        assert_eq!(h.text("same"), "same", "chunk {chunk}");
        let now: usize = h.text("blocks").parse().unwrap();
        assert!(now >= blocks, "chunk {chunk}: the blocks follow the state");
        blocks = now;
        assert_eq!(h.text("len"), now.to_string(), "chunk {chunk}");
        assert_eq!(h.text("first"), "h", "chunk {chunk}");
        assert!(!h.text(&format!("row{now}")).is_empty(), "chunk {chunk}");
    }
    // Long by the end: the stream followed `state:append`'s edits.
    let length: usize = h.text("length").parse().unwrap();
    assert!(length > 64 * 1024, "{length} bytes");
}

/// A reply streamed into `ui.markdown`'s state of blocks, drawn by
/// `ui.each`: a chunk converts as many tables for scripts with a long reply
/// as with a short one (the last block's row, not the list), as no table is
/// made for the answer.
#[test]
fn a_chunk_of_a_streamed_state_converts_what_it_adds() {
    let per_chunk = |sections: usize| {
        let h = Harness::inline(
            r###"local reply = ui.state("reply", "")
local chunks = ui.state("chunks", 0)
local blocks = ui.markdown(reply, { key = "reply" })
ui.command { id = "append", run = function()
    local n = chunks:get() + 1
    chunks:set(n)
    reply:append(if n % 3 == 1 then "## Section " .. n .. "\n\n"
        elseif n % 3 == 2 then "Words with **bold** and `code`, "
        else "the end of it.\n\n")
end }
return ui.column {
    ui.each(blocks, function(b: MarkdownBlock)
        return ui.text { b.t, literal = true }
    end),
}"###,
        );
        h.settle();
        for _ in 0..sections * 3 {
            assert!(h.host.run_command("append"));
            h.settle();
        }
        let before = tables_converted();
        for _ in 0..30 {
            assert!(h.host.run_command("append"));
            h.settle();
        }
        (tables_converted() - before) / 30
    };
    let (short, long) = (per_chunk(10), per_chunk(120));
    assert_eq!(
        short, long,
        "a chunk converts as much at 120 sections as at 10"
    );
    // The last block's row: its block, its runs and theirs.
    assert!(short <= 8, "a chunk converts {short} tables");
}

/// A memory report says what named states hold: the largest one by one, with
/// its rows and its text, and a value two states share counted once.
#[test]
fn a_memory_report_names_what_states_hold() {
    let h = Harness::new(|runtime| {
        ScriptHost::inline(
            runtime,
            "app",
            r#"local messages = ui.state("messages", {})
local again = ui.state("again", {})
local small = ui.state("small", "hi")
return ui.text { "{small}" }"#,
        )
    });
    h.settle();
    let words = "a message in a transcript, as a reply writes it. ".repeat(4);
    let rows: Vec<Value> = (0..1000)
        .map(|id| {
            Value::Map(
                [
                    ("id".into(), Value::Num(id as f64)),
                    ("text".into(), Value::from(format!("{id} {words}").as_str())),
                ]
                .into(),
            )
        })
        .collect();
    let list = Value::List(rows.into());
    h.host.set_state("messages", list.clone()).unwrap();
    h.host.set_state("again", list).unwrap();
    h.settle();
    let mut report = zgx::memory::Report::default();
    h.host.memory(&mut report);
    let line = |name: &str| {
        (report.lines.iter())
            .find(|line| line.area == "state" && line.name == name)
            .cloned()
    };
    let text = 1000 * words.len();
    let all = line("all named states").expect("listed");
    let listed = line("messages")
        .or_else(|| line("again"))
        .expect("the largest listed");
    assert_eq!(listed.entries, Some(1000), "its rows");
    let bytes = listed.bytes.unwrap();
    assert!(bytes > text, "{bytes} bytes for {text} of text");
    assert!(
        all.bytes.unwrap() < bytes + 4096,
        "shared, counted once: {} of all, {bytes} of one",
        all.bytes.unwrap()
    );
    let own_text = line(&format!("{}: text", listed.name)).unwrap();
    assert!(own_text.bytes.unwrap() >= text);
    assert!(line("small").is_none(), "small ones go in all alone");
}
