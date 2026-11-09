-- A long transcript to measure scrolling on: replies of markdown (prose,
-- tables, code blocks) in a measured list, a heading in each row that fades
-- at its edges, and a glass composer floating over the bottom, blurring what
-- scrolls under it. Every run builds the same rows.
--
--   zgx-run --inspect crates/zgx-script/examples/transcript_scroll
--   zgx inspect replay "#transcript" gesture.tape
--
-- Back to the start: `zgx inspect set offset 90000`. A reply streaming in:
-- `zgx inspect set stream_row 240`, the row to grow (0, the default: none).
local WORDS = {
	"retained",
	"compositor",
	"latency",
	"refresh",
	"texture",
	"the",
	"of",
	"a",
	"scroll",
	"frame",
	"budget",
	"damage",
	"glyph",
	"atlas",
	"and",
	"into",
	"without",
	"layout",
	"margin",
	"present",
	"surface",
	"row",
	"markdown",
	"measured",
}

type Message = { id: number, mine: boolean, title: string, text: string }

-- The same numbers every run.
local seed = 12345
local function next_number(below: number): number
	seed = (seed * 1103515245 + 12345) % 2147483648
	return math.floor(seed / 65536) % below
end

local function sentence(words: number): string
	local out = table.create(words)
	for i = 1, words do
		local word = WORDS[next_number(#WORDS) + 1]
		out[i] = if next_number(14) == 0 then `**{word}**` elseif next_number(17) == 0 then `\`{word}\`` else word
	end
	return table.concat(out, " ") .. "."
end

local function paragraph(): string
	local out = {}
	for i = 1, 3 + next_number(5) do
		out[i] = sentence(8 + next_number(14))
	end
	return table.concat(out, " ")
end

local function code(): string
	local out = { "```rust" }
	for i = 1, 6 + next_number(14) do
		out[#out + 1] =
			`{string.rep("    ", next_number(4))}let value_{i} = frame.{WORDS[next_number(#WORDS) + 1]}({next_number(
				1000
			)});`
	end
	out[#out + 1] = "```"
	return table.concat(out, "\n")
end

local function markdown_table(): string
	local out = { "| Stage | Time | Pixels | Note |", "| --- | --- | --- | --- |" }
	for i = 1, 4 + next_number(8) do
		out[#out + 1] =
			`| {WORDS[next_number(#WORDS) + 1]} {i} | {next_number(90)} ms | {next_number(9000)} | {sentence(4)} |`
	end
	return table.concat(out, "\n")
end

local function reply(): string
	local out = {}
	for _ = 1, 3 + next_number(6) do
		local kind = next_number(6)
		out[#out + 1] = if kind == 0
			then code()
			elseif kind == 1 then markdown_table()
			elseif kind == 2 then `## {sentence(4)}`
			elseif kind == 3 then `- {sentence(9)}\n- {sentence(12)}\n- {sentence(6)}`
			else paragraph()
	end
	return table.concat(out, "\n\n")
end

local messages: { Message } = table.create(400)
for i = 1, 400 do
	local mine = i % 2 == 1
	messages[i] = {
		id = i,
		mine = mine,
		title = sentence(30),
		text = if mine then sentence(12 + next_number(24)) else reply(),
	}
end
local thread = ui.state("thread", messages)
-- Well inside the list, so a gesture either way has room.
local offset = ui.state("offset", 90000)

local stream_row = ui.state("stream_row", 0)
local tokens = 0
ui.every(33, function()
	local row = stream_row:peek()
	if row > 0 then
		tokens += 1
		thread:set_key({ row, "text" }, thread:peek()[row].text .. (if tokens % 40 == 0 then "\n\n" else " token"))
	end
end)

local function message(m: Message, _: State<number>, row: State<Message>): Node
	return ui.row {
		id = `message-{m.id}`,
		w_full = true,
		px = 32,
		py = 12,
		justify = if m.mine then "end" else "start",
		ui.column {
			max_w = 860,
			min_w = 0,
			grow = if m.mine then 0 else 1,
			gap = 8,
			px = if m.mine then 14 else 0,
			py = if m.mine then 10 else 0,
			rounded = 14,
			bg = if m.mine then "$surface.raised" else nil,
			selectable = true,
			if m.mine
				then ui.text { m.text, literal = true, wrap = true }
				else ui.column {
					w_full = true,
					gap = 8,
					-- A line wider than the row, fading at its edges.
					ui.row {
						w_full = true,
						overflow = "hidden",
						fade_x = { 0, 28 },
						ui.text { m.title, literal = true, color = "$text.tertiary", shrink = 0 },
					},
					ui.markdown_view {
						function()
							return row:get().text
						end,
						w_full = true,
					},
				},
		},
	}
end

return ui.column {
	w_full = true,
	h_full = true,
	color = "$text.primary",
	bg = "$surface.base",
	ui.list {
		id = "transcript",
		data = thread,
		key = "id",
		row = message,
		offset = offset,
		measured = true,
		height = 420,
		scrollbar = true,
		w_full = true,
		grow = 1,
	},
	ui.div {
		id = "composer",
		absolute = true,
		left = 120,
		right = 120,
		bottom = 20,
		h = 96,
		rounded = 22,
		backdrop_blur = 24,
		bg = "#20202080",
		border = 1,
		border_color = "#ffffff20",
		shadow = { y = 8, blur = 24, color = "#00000040" },
		p = 16,
		ui.text { "Ask anything", color = "$text.tertiary" },
	},
}
