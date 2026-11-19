//! A script stamping templates in a loop stops at its VM's memory limit.
//! A stamp makes its tables and its node with the limit set aside (counted,
//! not checked: `create_table_relaxed`, `create_userdata_relaxed`), and a
//! loop that keeps each node in a table made beforehand makes no checked
//! allocation of its own: without each stamp's check that the VM is under
//! its limit (`templates::over_limit`), such a loop grows past it unbounded.
//! As it runs with templates or without (the suite runs both ways), it is
//! stopped by the limit, by one node's worth at most.
mod common;
use common::Harness;
use std::time::Duration;
use zgx_script::{Limits, ScriptHost};

/// The VM's limit: room for the table below, not for a node in each slot.
const LIMIT: usize = 6 << 20;
/// Over the limit by at most this: one stamp's tables and node, with room
/// for the error's own.
const OVERSHOOT: usize = 64 << 10;

#[test]
fn a_script_stamping_in_a_loop_stops_at_its_memory_limit() {
    let script = r#"
local made = ui.state("made", 0)
ui.command { id = "fill", run = function()
	-- 3.2 MB, under the limit; its slots take what comes without a
	-- checked allocation.
	local keep = table.create(200000, false)
	local i = 0
	while i < 200000 do
		i += 1
		keep[i] = ui.row { gap = i }
	end
	made:set(i)
end }
return ui.text { "{made}" }
"#;
    let h = Harness::new(|runtime| {
        ScriptHost::inline(runtime, "app", script).limits(Limits {
            memory: LIMIT,
            // Time enough to fill every slot: the limit, not the clock,
            // must stop it.
            call: Duration::from_secs(120),
            load: Duration::from_secs(120),
            ..Limits::default()
        })
    });
    h.host.run_command("fill");
    let error = h.host.error().with_untracked(|e| e.clone());
    let heap = h.host.script_heap();
    let error = error.expect("stopped: 200,000 nodes don't fit in 6 MB");
    assert!(
        error.contains("memory limit") || error.contains("not enough memory"),
        "stopped by the memory limit, not another error: {error}"
    );
    assert!(
        heap <= LIMIT + OVERSHOOT,
        "over the limit by one stamp at most: {heap} bytes held, limit {LIMIT}"
    );
}
