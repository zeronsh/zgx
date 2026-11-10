-- Fixed data, shared by the native replay and headless instruction gates.
local scene = ui.args[1] or "plain"
local horizontal_offset = ui.state("horizontal_offset", 90000)
local offset = ui.state("offset", 90000)
local translucent = ui.state("translucent", false)
-- Minimal idle reference: only this tiny marker changes each refresh. A
-- completely static window presents nothing and cannot measure system pacing.
if scene == "idle" then
	local beat = ui.shared("reference_beat", false)
	return ui.column {
		id = "transcript",
		w_full = true,
		h_full = true,
		bg = function()
			return if translucent:get() then "#30405080" else "#182028"
		end,
		ui.div {
			w = 8,
			h = 8,
			bg = function()
				return if beat:get() then "#ffffff" else "#000000"
			end,
		},
	}
end

-- Room for the complete recorded fling in either direction, with actual
-- visible marks throughout the nested horizontal surface.
local horizontal_cells: { Node } = {}
if scene == "nested" then
	for i = 1, 1024 do
		horizontal_cells[i] = ui.div {
			w = 160,
			h = 72,
			shrink = 0,
			bg = if i % 2 == 0 then "#486078" else "#304050",
			ui.text { `Horizontal cell {i}`, p = 8 },
		}
	end
end

local function row(i: number): Node
	if scene == "grid" then
		local cells = {}
		for j = 1, 6 do
			cells[j] = ui.div {
				h = 68,
				p = 8,
				bg = if (i + j) % 2 == 0 then "#486078" else "#304050",
				ui.text { `Cell {i}:{j}` },
			}
		end
		return ui.grid { cols = 6, gap = 4, h = 72, w_full = true, cells }
	elseif scene == "nested" then
		local cells = {}
		for j = 1, 24 do
			cells[j] = ui.div { w = 160, h = 64, shrink = 0, p = 8, bg = "#304050", ui.text { `Nested {i}:{j}` } }
		end
		return ui.scroll_x { id = `horizontal-{i}`, h = 72, w_full = true, cells }
	end
	return ui.text { `Deterministic plain row {i}: retained text and scrolling`, h = 28, literal = true }
end
return ui.column {
	w_full = true,
	h_full = true,
	bg = "#182028",
	color = "#eeeeee",
	if scene == "nested"
		then ui.scroll_x {
			id = "horizontal",
			h = 72,
			w_full = true,
			ui.scroll_x {
				id = "horizontal-content",
				w = 3000,
				h = 72,
				shrink = 0,
				offset = horizontal_offset,
				ui.row { w = 163840, h = 72, horizontal_cells },
			},
		}
		else nil,
	ui.list {
		id = "transcript",
		count = 10000,
		height = if scene == "plain" then 28 else 72,
		offset = offset,
		row = row,
		w_full = true,
		grow = 1,
		bg = function()
			return if translucent:get() then "#30405080" else "#182028"
		end,
	},
}
