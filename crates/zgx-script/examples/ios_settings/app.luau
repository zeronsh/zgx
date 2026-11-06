-- A Settings-like screen in zgx alone, measured against UIKit's: rows of 60
-- points, cards inset by 20, text at x = 81, a floating glass search bar.
-- Colours and text sizes are the system's (`$text.primary`, `$ios.grouped`).
local safe = ui.window.safe_area

local function tile(symbol, tint)
	return ui.div {
		w = 29,
		h = 29,
		rounded = 7,
		bg = tint,
		items = "center",
		justify = "center",
		shrink = 0,
		ui.symbol { name = symbol, size = 16, color = "#ffffff" },
	}
end

local function switch(state)
	local at = ui.state("switch_at", state:get() and 22 or 0)
	local x = ui.spring(at)
	return ui.button {
		w = 63,
		h = 28,
		p = 0,
		rounded = 14,
		bg = function()
			return state:get() and "$ios.green" or "$ios.fill"
		end,
		hover = {},
		active = {},
		role = "switch",
		on_click = function()
			state:set(not state:get())
			at:set(state:get() and 22 or 0)
		end,
		ui.div {
			absolute = true,
			left = 2,
			top = 2,
			w = 39,
			h = 24,
			rounded = 12,
			bg = "#ffffff",
			x = x,
			shadow = { y = 2, blur = 4, color = "#00000026" },
		},
	}
end

local function row(item, last)
	return ui.button {
		min_h = 60,
		p = 0,
		py = 8,
		pl = 16,
		pr = 20,
		rounded = 0,
		bg = "#00000000",
		direction = "row",
		items = "center",
		justify = "start",
		hover = {},
		active = { bg = "$pressed" },
		tile(item.icon, item.tint),
		ui.text { item.title, ml = 16, grow = 1, text_size = "$font.size.md", color = "$text.primary" },
		item.detail and ui.text { item.detail, text_size = "$font.size.md", color = "$text.secondary", mr = 8 } or nil,
		item.toggle and switch(item.toggle)
			or ui.symbol { name = "chevron.forward", size = 14, weight = 600, color = "$text.tertiary" },
		not last and ui.div {
			absolute = true,
			left = 61,
			right = 16,
			bottom = 0,
			h = 0.34,
			bg = "$border",
		} or nil,
	}
end

local function section(items: { any })
	local rows: { any } = {}
	for index, item in items do
		rows[index] = row(item, index == #items)
	end
	return ui.column {
		mx = 20,
		mb = 35,
		rounded = 26,
		bg = "$ios.cell",
		overflow = "hidden",
		rows,
	}
end

local airplane = ui.state("airplane", false)

-- More of the same below the fold, to flick against a native list.
local more: { any } = {}
for index = 1, 30 do
	more[index] = section {
		{ icon = "bell.badge.fill", tint = "$ios.red", title = "Notifications" },
		{ icon = "speaker.wave.3.fill", tint = "$ios.pink", title = "Sounds & Haptics" },
		{ icon = "moon.fill", tint = "$ios.indigo", title = "Focus" },
		{ icon = "gear", tint = "$ios.gray", title = "General" },
	}
end

return ui.div {
	w = "100%",
	h = "100%",
	bg = "$ios.grouped",
	ui.scroll {
		w = "100%",
		h = "100%",
		ui.column {
			pt = function()
				return safe:get().top + 54
			end,
			pb = 120,
			ui.text { "Settings", ml = 16, mb = 4, text_size = "$font.size.xxl", weight = 700, color = "$text.primary" },
			section {
				{ icon = "airplane", tint = "$ios.orange", title = "Airplane Mode", toggle = airplane },
				{ icon = "wifi", tint = "$ios.blue", title = "Wi-Fi", detail = "Home" },
				{ icon = "wave.3.right", tint = "$ios.blue", title = "Bluetooth", detail = "On" },
				{ icon = "antenna.radiowaves.left.and.right", tint = "$ios.green", title = "Cellular" },
			},
			section {
				{ icon = "bell.badge.fill", tint = "$ios.red", title = "Notifications" },
				{ icon = "speaker.wave.3.fill", tint = "$ios.pink", title = "Sounds & Haptics" },
				{ icon = "moon.fill", tint = "$ios.indigo", title = "Focus" },
				{ icon = "hourglass", tint = "$ios.indigo", title = "Screen Time" },
			},
			section {
				{ icon = "gear", tint = "$ios.gray", title = "General" },
				{ icon = "accessibility", tint = "$ios.blue", title = "Accessibility" },
			},
			more,
		},
	},
	ui.glass {
		absolute = true,
		left = 28,
		right = 28,
		bottom = function()
			return math.max(safe:get().bottom - 6, 16)
		end,
		h = 48,
		radius = 24,
		interactive = true,
		direction = "row",
		items = "center",
		pl = 18,
		ui.symbol { name = "magnifyingglass", size = 17, weight = 500, color = "$text.primary" },
		ui.text { "Search", ml = 8, text_size = "$font.size.md", color = "$text.secondary" },
	},
}
