-- Actual Zeron shell, title bar, sidebar, transcript and frosted composer.
-- Copied next to shell_fixture.luau in an isolated UI tree by scroll.py.
local shell = require("./shell_fixture")
local store = require("@app/screens/chat/store")
local route = require("@app/screens/shell/route")
local shared = require("@app/screens/chat/shared")
local material = require("@ds/material")
-- A known fixture model is settled, as in an opened real conversation.
-- The driver can retain the loading case explicitly for animation regressions.
local settle_model_catalog = true
if settle_model_catalog then
	local catalog: any = table.clone(store.catalog:peek())
	catalog.complete = true
	store.catalog:set(catalog)
end
local effective_catalog = ui.state("bench_catalog_complete", false)
ui.effect(function()
	effective_catalog:set((store.catalog:get() :: any).complete == true)
end)
local chats = ui.shared("fixture.chats", {} :: { [string]: any })
local reference = ui.shared("scroll.reference", false):peek()
local beat = ui.shared("scroll.reference.beat", false)
local reset_offset = ui.state("bench_offset", 12000)
local streaming = ui.state("bench_streaming", false)
local effective_frost = ui.state("bench_effective_frost", false)
ui.effect(function()
	effective_frost:set(material.see_through())
end)

local function reply(i: number): string
	local parts: { string } = {
		`## Retained rendering, turn {i}`,
		string.rep(
			"A scroll should keep its measured rows and retained pixels. **Tables**, `code`, images and clipped prompts must remain coherent through reversal and momentum. ",
			3 + i % 4
		),
		"| Stage | Main work | GPU work | Refresh | Copied pixels | Detail |\n| --- | --- | --- | --- | --- | --- |",
	}
	for j = 1, 8 + i % 13 do
		parts[#parts + 1] =
			`| phase {j} | {j}.25 ms | 0.{j} ms | 120 Hz | retained | A wide column that scrolls horizontally under a fixed fade |`
	end
	parts[#parts + 1] = "\n```rust"
	parts[#parts + 1] = string.format("fn retained_frame_%d(frame: &mut Frame) {", i)
	for j = 1, 12 + i % 20 do
		parts[#parts + 1] =
			string.format('    let row_%d = frame.scroll_and_compose(%d, "stable glyphs and masks");', j, i * 100 + j)
	end
	parts[#parts + 1] = "}\n```"
	parts[#parts + 1] =
		"\n> Keep the composer, sidebar and title bar in the same frame.\n\n- Reverse without rebuilding rows.\n- Preserve image and clipping bounds.\n- Leave time before the next refresh."
	return table.concat(parts, "\n")
end

local messages: { any } = {}
for i = 1, 180 do
	messages[#messages + 1] = {
		id = `scroll-user-{i}`,
		role = "user",
		text = if i % 3 == 0
			then string.rep(
				`Long prompt {i}: this content is folded behind the real five-line user-message clip and fade.\n`,
				12
			)
			else `Explain retained scroll step {i}.`,
		here = true,
		attachments = if i % 8 == 0
			then {
				{ kind = "image", name = "fixture.jpg", src = "assets/art/plate-3-sunrise.jpg" },
				{ kind = "file", name = "src/frame.rs", meta = "Rust source" },
			}
			else nil,
		at = "Nov 9, 2026, 12:00 AM",
	}
	messages[#messages + 1] = {
		id = `scroll-reply-{i}`,
		role = "assistant",
		text = reply(i),
		model = "Opus",
		at = "Nov 9, 2026, 12:01 AM",
		tools = { { id = `scroll-tool-{i}`, kind = "read", verb = "Read", target = "src/frame.rs" } },
	}
end
chats:set {
	a1 = {
		session = {
			id = "a1",
			state = "ready",
			status = "idle",
			title = "Long scroll benchmark",
			project = "atlas",
			model = "opus",
			effort = "high",
			access = "full",
			plan = false,
		},
		messages = messages,
		draft = { id = "", text = "" },
	},
}
ui.after(0, function()
	route.open_session("a1")
end)
ui.command {
	id = "bench.reset",
	run = function()
		streaming:set(false)
		shared.offset:set(reset_offset:peek())
	end,
}
ui.command {
	id = "bench.opaque",
	run = function()
		material.frosted:set(false)
	end,
}
ui.command {
	id = "bench.translucent",
	run = function()
		material.frosted:set(true)
	end,
}
ui.command {
	id = "bench.stream",
	run = function()
		streaming:set(true)
	end,
}
ui.command {
	id = "bench.dark",
	run = function()
		ui.theme.follow_system(false)
		ui.theme.current:set("zeron-dark")
	end,
}
local tokens = 0
ui.every(33, function()
	if streaming:peek() then
		tokens += 1
		local n = store.messages:len()
		store.messages:set_key({ n, "text" }, store.messages:peek()[n].text .. " token")
	end
end)
return ui.overlay {
	w_full = true,
	h_full = true,
	shell,
	if reference
		then ui.div {
			absolute = true,
			left = 80,
			top = 8,
			w = 8,
			h = 8,
			bg = function()
				return if beat:get()
					then ui.raw("#ffffff", "display-clock reference marker")
					else ui.raw("#000000", "display-clock reference marker")
			end,
		}
		else nil,
}
