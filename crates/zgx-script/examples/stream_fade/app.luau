--!strict
-- A streamed reply as a chat app draws one (the gate's md-stream-fade): a
-- measured transcript that follows its end, its last row the reply under
-- way, parsed by a keyed `ui.markdown` and drawn by a `ui.each`, and its
-- newest words fading in as they arrive. Chunks come from the host
-- (`stream.chunk`), a few tokens at a time.
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

ui.command { id = "chunk", run = function()
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
end }

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
				table.insert(props, ui.span {
					piece,
					literal = true,
					fade_in = { start = c.started, ms = c.ms / c.boost, power = 1.6 },
				})
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

local rows: { { id: string, text: string? } } = {}
-- (Enough before the reply that, scrolled to the top, none of it shows.)
for i = 1, 60 do
	table.insert(rows, { id = `m{i}`, text = `An earlier message, {i}: a line or two of what was said before the reply.` })
end
table.insert(rows, { id = "reply" })

return ui.list {
	id = "transcript",
	data = ui.state("rows", rows),
	key = function(row: any): string
		return row.id
	end,
	row = function(row: any): Node
		if row.id == "reply" then
			return reply_view()
		end
		return ui.text { row.text or "", wrap = true }
	end,
	-- Opened at its end, as a chat opens on its last message; `follow`
	-- keeps it there as the reply grows.
	offset = ui.state("offset", math.huge),
	measured = true,
	follow = true,
	height = 64,
	w = 760,
	h = 700,
}
