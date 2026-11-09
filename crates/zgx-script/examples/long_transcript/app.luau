-- A long conversation to scroll: a virtualized list of replies as tall as
-- their Markdown, with tables and code. What scrolling over rows that cost
-- something to build is measured on (`zgx inspect replay '#transcript'`).
local WORDS = {
	"the",
	"frame",
	"layout",
	"scroll",
	"reply",
	"measured",
	"row",
	"builds",
	"ahead",
	"of",
	"view",
	"and",
	"keeps",
	"what",
	"it",
	"made",
}

-- The same words each run, whatever else runs: a small generator of its own.
local function text_of(seed: number, words: number): string
	local out = table.create(words)
	for i = 1, words do
		seed = (seed * 1103515245 + 12345) % 2147483648
		out[i] = WORDS[seed % #WORDS + 1]
	end
	return table.concat(out, " ")
end

local function table_of(i: number): string
	local lines = { "| Case | Before | After | Notes |", "| --- | --- | --- | --- |" }
	for row = 1, 4 + i % 8 do
		table.insert(lines, `| {text_of(i + row, 2)} | {row * 3} ms | {row} ms | {text_of(i * row, 6)} |`)
	end
	return table.concat(lines, "\n")
end

local function code_of(i: number): string
	local lines = { "```rust" }
	for line = 1, 6 + i % 12 do
		table.insert(lines, `let {text_of(i + line, 1)}_{line} = {line} * {i}; // {text_of(line, 4)}`)
	end
	table.insert(lines, "```")
	return table.concat(lines, "\n")
end

local function reply_of(i: number): string
	local parts = {
		`## Reply {i}`,
		text_of(i, 40 + i % 80),
	}
	if i % 2 == 0 then
		table.insert(parts, table_of(i))
	end
	if i % 3 == 0 then
		table.insert(parts, code_of(i))
	end
	table.insert(parts, `- {text_of(i + 1, 12)}\n- {text_of(i + 2, 12)}\n- {text_of(i + 3, 12)}`)
	table.insert(parts, text_of(i + 4, 30 + i % 50))
	return table.concat(parts, "\n\n")
end

type Reply = { id: number, text: string }

local replies: { Reply } = {}
for i = 1, 400 do
	table.insert(replies, { id = i, text = reply_of(i) })
end
local rows = ui.state("rows", replies)
-- From the middle, room both ways for a gesture up and down.
local offset = ui.state("offset", 200000)

return ui.list {
	id = "transcript",
	data = rows,
	key = "id",
	measured = true,
	overscan = 3,
	height = 600,
	offset = offset,
	w_full = true,
	h_full = true,
	row = function(reply: Reply)
		return ui.column {
			w_full = true,
			px = 24,
			py = 12,
			selectable = true,
			ui.markdown_view { reply.text, w_full = true },
		}
	end,
}
