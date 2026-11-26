--!strict
-- A streamed reply as a chat app draws one (the gate's md-stream-fade): a
-- measured transcript that follows its end, its last row the reply under
-- way, parsed by a keyed `ui.markdown` and drawn by a `ui.each`, and its
-- newest words fading in as they arrive. Chunks come from the host
-- (`stream.chunk`), a few tokens at a time. Its rows are a chat app's
-- messages (a header, the body in a presence, a tool's row, a footer), and
-- the window around it a chat app's too (a sidebar of chats, tabs, a
-- composer, a panel of files: about 2,200 nodes, as Zeron's window has while
-- a reply streams), with the host's pointer resting over the transcript.
--
-- How they fade is the host's `stream.mode`:
--   bindings  each fading chunk's colour a function of the time and a tick
--             state, the tick animated every frame while any fades (an app
--             doing it in script);
--   native    each fading chunk a span with `fade_in`, drawn in Rust;
--   none      no fade.
-- A chunk arriving while more than two fade speeds the others up, as a
-- reply's fades catch up when the text comes faster (`boost`): the bindings
-- read their chunk's speed as they run; native spans are given a new start
-- and length, keeping how far in they were.

local MODE: string = ui.call_sync("stream.mode", nil)
local BOOST: boolean = ui.call_sync("stream.boost", nil)
local INK = "#202124"
local FADE_MS = 300

local reply = ui.state("reply", "")
local blocks = ui.markdown(reply, { key = "reply" })
-- The chunks still fading, in order: their bytes in the reply.
type Chunk = { from: number, to: number, started: number, ms: number, boost: number }
-- (A table of the script's own, which a chunk's speed is changed in; its
-- version a state, so what reads it is told.)
local fading: { Chunk } = {}
local version = ui.state("fading", 0)
local tick = ui.state("tick", 0)
local ticking: any = nil

local function going(now: number): { Chunk }
	local kept = {}
	for _, c in fading do
		if (now - c.started) * c.boost < c.ms then
			table.insert(kept, c)
		end
	end
	return kept
end

ui.command {
	id = "chunk",
	run = function()
		local text: string = ui.call_sync("stream.chunk", nil)
		local from = #reply:peek() + 1
		reply:append(text)
		if MODE == "none" then
			return
		end
		local now = ui.now()
		local list = going(now)
		-- More than two fading: they all go faster, keeping how far in they are.
		local boost = if BOOST then 1 + 0.3 * math.max(#list - 1, 0) else 1
		for _, c in list do
			if boost > c.boost then
				local p = (now - c.started) * c.boost / c.ms
				c.boost = boost
				c.started = now - p * c.ms / boost
			end
		end
		table.insert(list, { from = from, to = from + #text - 1, started = now, ms = FADE_MS, boost = 1 })
		fading = list
		version:set(version:peek() + 1)
		if MODE == "bindings" then
			-- A clock every frame until the last fade ends.
			if ticking then
				ticking:cancel()
			end
			ticking = anime.animate(tick, { value = tick:peek() + 1000, duration = FADE_MS + 50, ease = "linear" })
		end
	end,
}

-- A colour fading in from clear, as a binding: re-run each frame by the tick.
local function veil(c: Chunk): () -> string
	return function(): string
		tick:get()
		local p = (ui.now() - c.started) * c.boost / c.ms
		if p >= 1 then
			return INK
		end
		local alpha = 1 - (1 - math.max(p, 0)) ^ 1.6
		return string.format("%s%02x", INK, math.floor(alpha * 255 + 0.5))
	end
end

-- A paragraph's text from byte `at` of the reply: what's settled one span,
-- each fading chunk in it a span of its own.
local function paragraph(text: string, at: number): Node
	local props: RichProps = { wrap = true, color = INK }
	local pos = 1
	local last = at + #text - 1
	version:get()
	for _, c in fading do
		local s, e = math.max(c.from, at), math.min(c.to, last)
		if s <= e then
			if s - at + 1 > pos then
				table.insert(props, ui.span { string.sub(text, pos, s - at), literal = true })
			end
			local piece = string.sub(text, s - at + 1, e - at + 1)
			if MODE == "native" then
				table.insert(
					props,
					ui.span {
						piece,
						literal = true,
						fade_in = { start = c.started, ms = c.ms / c.boost, power = 1.6 },
					}
				)
			else
				table.insert(props, ui.span { piece, literal = true, color = veil(c) })
			end
			pos = e - at + 2
		end
	end
	if pos <= #text then
		table.insert(props, ui.span { string.sub(text, pos), literal = true })
	end
	return ui.rich(props)
end

-- Where a block's text starts in the reply, as the chunks' bytes count: its
-- last place (the reply goes round its replies again and again, and only the
-- newest blocks have chunks still fading).
local function at_of(text: string): number
	local whole = reply:peek()
	local at, from = 1, 1
	while true do
		local s = string.find(whole, text, from, true)
		if not s then
			return at
		end
		at, from = s, s + 1
	end
end

local function reply_view(): Node
	return ui.column {
		gap = 8,
		w_full = true,
		ui.each(blocks, function(b: MarkdownBlock)
			if b.t == "p" or b.t == "h" then
				local text = ""
				for _, r in b.s do
					text ..= r.x
				end
				return paragraph(text, at_of(text))
			elseif b.t == "code" then
				return ui.text { b.text, font = "$font.mono", wrap = false }
			end
			return ui.div {}
		end),
	}
end

-- What a chat app puts around a message, as Zeron's transcript does: a
-- header, its body in a presence (which of its forms shows), a tool's row, a
-- footer with a button; and a settled message's body of paragraphs with
-- inline code, bold and a link, and a code block with its own header.
local function header(name: string): Node
	return ui.row {
		gap = 8,
		align = "center",
		ui.div { w = 20, h = 20, rounded = 10, bg = "#d0d4da" },
		ui.text { name, weight = 600 },
		ui.text { "4:45 PM", color = "#80868b", text_size = 12 },
	}
end
local function footer(): Node
	return ui.row {
		gap = 6,
		ui.text { "Nov 25", color = "#80868b", text_size = 12 },
		ui.button { "Copy", on_click = function() end, variant = "ghost" },
	}
end
local function prose(i: number, k: number): Node
	return ui.rich {
		wrap = true,
		color = INK,
		ui.span { `Step {i}.{k}: I read `, literal = true },
		ui.span { "src/invoices.ts", literal = true, font = "mono", bg = "#f1f3f4" },
		ui.span { " and noted ", literal = true },
		ui.span { "what it needs", literal = true, weight = 600 },
		ui.span { " from ", literal = true },
		ui.span { "the ledger", literal = true, href = "https://example.invalid/ledger" },
		ui.span { ", then checked ", literal = true },
		ui.span { "invoiceTotal()", literal = true, font = "mono", bg = "#f1f3f4" },
		ui.span { " against the totals the tests expect.", literal = true },
	}
end
local function settled(i: number): Node
	return ui.column {
		gap = 8,
		w_full = true,
		prose(i, 1),
		prose(i, 2),
		ui.column {
			gap = 4,
			ui.row {
				gap = 6,
				ui.text { "TypeScript", color = "#80868b", text_size = 12, grow = 1 },
				ui.button { "Copy", on_click = function() end, variant = "ghost" },
			},
			ui.text {
				"function summarize(invoice: Invoice): InvoiceSummary {\n  const total = invoiceTotal(invoice);\n  return { id: invoice.id, totalCents: total };\n}",
				font = "$font.mono",
				wrap = false,
			},
		},
		prose(i, 3),
	}
end
local function message(row: any): Node
	local mine = row.id == "reply"
	return ui.column {
		role = "article",
		class = if mine then "message message-assistant" else "message",
		provide = { message = row.id },
		w_full = true,
		gap = 8,
		header(if mine then "Assistant" else "Someone"),
		ui.presence(function(): string
			return "blocks"
		end, function(_: string): Node
			return if mine then reply_view() else settled(row.n)
		end),
		ui.button { "Read 1 file", on_click = function() end, variant = "ghost" },
		footer(),
	}
end

local rows: { { id: string, n: number } } = {}
-- (Enough before the reply that, scrolled to the top, none of it shows.)
for i = 1, 60 do
	table.insert(rows, { id = `m{i}`, n = i })
end
table.insert(rows, { id = "reply", n = 0 })

local transcript = ui.list {
	id = "transcript",
	data = ui.state("rows", rows),
	key = function(row: any): string
		return row.id
	end,
	row = message,
	-- Opened at its end, as a chat opens on its last message; `follow`
	-- keeps it there as the reply grows.
	offset = ui.state("offset", math.huge),
	measured = true,
	follow = true,
	height = 64,
	w = 760,
	h = 700,
}

-- The window around it, as Zeron's is (about 2,200 nodes mounted while a
-- reply streams, most of them outside the transcript): a sidebar of chats,
-- tabs, a composer and a panel of files. What a hit test under the pointer
-- has to rule out.
local function chat_entry(i: number): Node
	return ui.button {
		on_click = function() end,
		variant = "ghost",
		w_full = true,
		ui.row {
			gap = 8,
			align = "center",
			ui.div { w = 16, h = 16, rounded = 4, bg = "#d0d4da" },
			ui.column {
				grow = 1,
				ui.text { `A chat about invoices, {i}`, weight = 500 },
				ui.text { "Read 3 files, edited 1", color = "#80868b", text_size = 12 },
			},
			ui.text { `{i}h`, color = "#80868b", text_size = 12 },
		},
	}
end
local function file_entry(i: number): Node
	return ui.row {
		gap = 6,
		align = "center",
		on_click = function() end,
		ui.div { w = 8 * (i % 4) },
		ui.div { w = 14, h = 14, rounded = 3, bg = "#c8ccd2" },
		ui.text { `file-{i}.ts`, grow = 1 },
		ui.text { if i % 7 == 0 then "M" else "", color = "#b06000", text_size = 12 },
	}
end
local chats: { Node } = {}
for i = 1, 150 do
	table.insert(chats, chat_entry(i))
end
local files: { Node } = {}
for i = 1, 200 do
	table.insert(files, file_entry(i))
end
local tabs: { Node } = {}
for i = 1, 8 do
	table.insert(tabs, ui.button { `Tab {i}`, on_click = function() end, variant = "ghost" })
end

return ui.row {
	w = 1400,
	h = 900,
	ui.column { w = 260, h_full = true, overflow_y = "clip", gap = 2, p = 8, table.unpack(chats) },
	ui.column {
		w = 760,
		h_full = true,
		ui.row { gap = 4, h = 36, table.unpack(tabs) },
		transcript,
		ui.row {
			gap = 6,
			p = 8,
			ui.textarea { value = ui.state("draft", ""), placeholder = "Ask anything", grow = 1 },
			ui.button { "Attach", on_click = function() end, variant = "ghost" },
			ui.button { "Send", on_click = function() end },
		},
	},
	ui.column { w = 380, h_full = true, overflow_y = "clip", gap = 2, p = 8, table.unpack(files) },
}
