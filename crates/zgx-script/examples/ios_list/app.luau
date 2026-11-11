-- A long list of rows with a control each, mounted only as they show: what
-- hosting a system control costs against drawing one (scripts/ios/cost.py).
-- `ui.args`: plain, drawn or hosted; `still` to leave it to a finger, or a
-- top speed in points a second (a flick is a few thousand).
local mode = ui.args[1] or "hosted"
local still = ui.args[2] == "still"
local speed = tonumber(ui.args[2]) or 3000
local offset = ui.state("offset", 0)
local time = 0
if not still then
	ui.every(16, function()
		time += 0.016
		offset:set((math.sin(time * 1.2) + 1) * speed / 1.2)
	end)
end

local function Row(props: { index: number })
	local on = ui.use_state(props.index % 3 == 0)
	return ui.row {
		h = 60,
		px = 16,
		items = "center",
		justify = "between",
		bg = props.index % 2 == 0 and "$ios.cell" or "$ios.grouped",
		ui.text { "Row " .. props.index, text_size = "$font.size.md", color = "$text.primary" },
		mode == "hosted" and ui.native_switch { value = on, label = "Row " .. props.index }
			or mode == "drawn" and ui.div {
				w = 63,
				h = 28,
				rounded = 14,
				bg = function()
					return on:get() and "$ios.green" or "$ios.fill"
				end,
			}
			or nil,
	}
end
local RowComponent = ui.component(Row)

return ui.list {
	w = "100%",
	h = "100%",
	count = 2000,
	height = 60,
	offset = offset,
	row = function(index)
		return RowComponent { index = index }
	end,
}
