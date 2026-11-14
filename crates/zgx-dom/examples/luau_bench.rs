//! The table benchmark (`examples/bench.rs`) written as a Luau app would
//! write it: a `ui.state` list of rows, a keyed `ui.each` over them, the
//! same buttons, ids, rows and styles. zgx-script runs it in Luau's VM
//! (built for wasm32 with wasi-sdk, `luau-env.sh`); `bench/build.sh
//! luau-bench` builds it, and `bench/run.mjs` times it beside the others.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;
use zgx::reactive::Runtime;
use zgx_script::ScriptHost;

const APP: &str = r##"
type Row = { id: number, label: string }

local FIRST = { "quiet", "bright", "rapid", "gentle", "bold", "tiny", "vast", "brave", "calm", "eager", "fancy", "jolly" }
local SECOND = { "amber", "teal", "crimson", "ivory", "olive", "violet", "cobalt", "coral", "slate", "gold" }
local THIRD = { "kettle", "lantern", "comet", "harbor", "meadow", "pebble", "rocket", "willow", "anchor", "falcon", "garden" }

local rows = ui.state("rows", {} :: { Row })
local selected = ui.state("selected", 0)
local next_id = 1
math.randomseed(42)

local function build(count: number): { Row }
	local made = table.create(count)
	for i = 1, count do
		made[i] = {
			id = next_id,
			label = `{FIRST[math.random(#FIRST)]} {SECOND[math.random(#SECOND)]} {THIRD[math.random(#THIRD)]}`,
		}
		next_id += 1
	end
	return made
end

local function action(id: string, label: string, run: () -> ())
	return ui.button {
		id = id,
		px = 10,
		py = 6,
		bg = "#285aa0",
		rounded = 4,
		hover = { bg = "#3269b9" },
		on_click = run,
		label,
	}
end

local function row(item: Row, _: number, current: State<Row>)
	return ui.row {
		h = 28,
		items = "center",
		border_sides = { 0, 0, 1, 0 },
		border_color = "#3c3c46",
		bg = function()
			return if selected:get() == item.id then "#782828" else nil
		end,
		ui.text { tostring(item.id), w = 80 },
		ui.text {
			function()
				return current:get().label
			end,
			grow = 1,
			on_click = ui.set(selected, item.id),
		},
		ui.text {
			"x",
			w = 32,
			on_click = function()
				local at = rows:index_of("id", item.id)
				if at then
					rows:remove(at)
				end
			end,
		},
	}
end

return ui.column {
	p = 12,
	gap = 12,
	ui.row {
		gap = 6,
		action("run", "Create 1,000 rows", function()
			selected:set(0)
			rows:set(build(1000))
		end),
		action("runlots", "Create 10,000 rows", function()
			selected:set(0)
			rows:set(build(10000))
		end),
		action("add", "Append 1,000 rows", function()
			for _, made in build(1000) do
				rows:push(made)
			end
		end),
		action("update", "Update every 10th row", function()
			for i = 1, rows:len(), 10 do
				rows:append({ i, "label" }, " !!!")
			end
		end),
		action("clear", "Clear", function()
			selected:set(0)
			rows:set({})
		end),
		action("swaprows", "Swap rows", function()
			if rows:len() > 998 then
				local a, b = rows:at(2), rows:at(999)
				rows:set_key({ 2 }, b)
				rows:set_key({ 999 }, a)
			end
		end),
	},
	ui.each(rows, row, {
		id = "rows",
		key = function(item: Row)
			return item.id
		end,
	}),
}
"##;

#[wasm_bindgen(start)]
pub fn start() {
    let runtime = Runtime::new();
    let script = ScriptHost::inline(&runtime, "table.luau", APP);
    if let Err(error) = script.reload() {
        web_sys::console::error_1(&error.into());
    }
    std::mem::forget(zgx_dom::mount_in_body(&runtime, script.view()));
    std::mem::forget(script);
}
