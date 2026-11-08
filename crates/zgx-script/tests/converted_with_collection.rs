//! What a delta costs to convert for scripts doesn't depend on when Luau's
//! collector last ran: the same writes and reads convert the same values
//! whether or not a collection came between them. A cache the collector
//! empties made a list's cost per tick flip between builds as allocations
//! moved its timing (an extra conversion and computed pass per tick).
mod common;

use common::Harness;

const MAIL: &str = r##"
local rows = {}
local boxes = { "inbox", "archive", "sent", "spam" }
for i = 1, 600 do
    rows[i] = { id = i, mailbox = boxes[(i % 4) + 1], unread = i % 3 == 0, subject = "Subject " .. i }
end
local messages = ui.state("messages", rows)
local mailbox = ui.state("mailbox", "inbox")
local shown = ui.computed(function()
    local out = {}
    local box = mailbox:get()
    for _, m in messages:get() do
        if m.mailbox == box then table.insert(out, m) end
    end
    return out
end)
local unread = ui.computed(function()
    local counts = {}
    for _, m in messages:get() do
        if m.unread then counts[m.mailbox] = (counts[m.mailbox] or 0) + 1 end
    end
    return counts
end)
local tick = 0
ui.command { id = "tick", run = function()
    tick += 1
    local i = (tick * 7) % 600 + 1
    messages:set_key({ i, "unread" }, not messages:peek()[i].unread)
end }
return ui.column { w = 800, h = 600,
    ui.text { function() return "Unread: " .. tostring(unread:get().inbox or 0) end },
    ui.list { h = 560, height = 28, count = function() return #shown:get() end,
        row = function(i)
            return ui.row { h = 28,
                ui.text { function() local m = shown:get()[i]; return if m then m.subject else "" end },
            }
        end,
    },
}
"##;

/// Tables and values converted, and tables filed, over 60 ticks: with a
/// collection between each write and the reads it brings on, or not.
fn converted(collect: bool) -> (u64, u64, u64) {
    let h = Harness::inline(MAIL);
    assert_eq!(h.error(), None);
    h.ui.prepare_frame();
    for _ in 0..5 {
        h.host.run_command("tick");
        h.ui.prepare_frame();
    }
    let (tables, values, filed) = (
        zgx_script::tables_converted(),
        zgx_script::values_converted(),
        zgx_script::values_filed(),
    );
    for _ in 0..60 {
        h.host.run_command("tick");
        // Between the write and the reads it brings on: the last list's
        // table, which the new one's is made from, held by nothing else.
        if collect {
            h.host.collect_luau_only();
        }
        h.ui.prepare_frame();
    }
    (
        zgx_script::tables_converted() - tables,
        zgx_script::values_converted() - values,
        zgx_script::values_filed() - filed,
    )
}

#[test]
fn a_collection_between_ticks_converts_and_files_nothing_more() {
    let left_alone = converted(false);
    let collected = converted(true);
    assert_eq!(
        collected, left_alone,
        "(tables converted, values converted, tables filed) over 60 ticks"
    );
}
