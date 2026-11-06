-- Text fields on iOS: the system's keyboard, autocorrect and marked text over
-- zgx's editors, and the window's safe area around them.
local name = ui.state("name", "")
local search = ui.state("search", "")
local submitted = ui.state("submitted", 0)
local safe = ui.window.safe_area

return ui.column {
	w = "100%",
	h = "100%",
	bg = "$ios.grouped",
	px = 20,
	gap = 12,
	pt = function()
		return safe:get().top + 20
	end,
	ui.text { "Keyboard", heading = 1, text_size = "$font.size.xxl", weight = 700, color = "$text.primary" },
	ui.input {
		value = name,
		label = "Name",
		placeholder = "Name",
		on_key = function(event)
			if event.key == "Enter" then
				submitted:set(submitted:get() + 1)
			end
		end,
	},
	ui.input { value = search, label = "Search", placeholder = "Search", inputmode = "search" },
	ui.text { "Name: {name}", color = "$text.secondary" },
	ui.text { "Submitted {submitted} times", color = "$text.secondary" },
	ui.text {
		function()
			return "Keyboard covers " .. ui.window.keyboard:get() .. " points"
		end,
		color = "$text.secondary",
	},
}
