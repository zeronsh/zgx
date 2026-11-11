-- `ui.args[1]` = "native" hosts the system's UITabBar. Otherwise a tab bar drawn
-- by zgx on the system's glass, measured against UITabBar's:
-- a platter 360 by 62 inset 21, tabs 98 by 54, a selection pill behind the
-- chosen one. Each tab is a button in one tree of elements.
local selected = ui.state("selected", 1)
local safe = ui.window.safe_area
local tabs = {
	{ title = "Settings", icon = "house.fill" },
	{ title = "Search", icon = "magnifyingglass" },
	{ title = "Library", icon = "books.vertical.fill" },
	{ title = "Profile", icon = "person.fill" },
}

local function tab(index: number, item: { title: string, icon: string })
	local color = function()
		return selected:get() == index and "$accent" or "$text.primary"
	end
	return ui.button {
		w = 85,
		h = 54,
		p = 0,
		rounded = 27,
		role = "tab",
		selected = function()
			return selected:get() == index
		end,
		bg = function()
			return selected:get() == index and "$ios.fill" or "#00000000"
		end,
		hover = {},
		active = {},
		direction = "column",
		items = "center",
		justify = "center",
		gap = 1,
		on_click = function()
			selected:set(index)
		end,
		ui.symbol { name = item.icon, size = 22, color = color },
		ui.text { item.title, text_size = 10, weight = 500, color = color },
	}
end

local bar: { any } = {}
for index, item in tabs do
	bar[index] = tab(index, item)
end

if ui.args[1] == "native" then
	return ui.div {
		w = "100%",
		h = "100%",
		bg = "$ios.grouped",
		ui.column {
			pt = function()
				return safe:get().top + 54
			end,
			ui.text {
				"Settings",
				heading = 1,
				ml = 16,
				text_size = "$font.size.xxl",
				weight = 700,
				color = "$text.primary",
			},
			ui.text { "Tab {selected}", ml = 16, mt = 8, color = "$text.secondary" },
		},
		ui.native_tabs {
			absolute = true,
			left = 0,
			right = 0,
			bottom = 0,
			h = 83,
			value = selected,
			items = {
				{ title = "Settings", symbol = "house.fill" },
				{ title = "Search", symbol = "magnifyingglass" },
				{ title = "Library", symbol = "books.vertical.fill" },
				{ title = "Profile", symbol = "person.fill" },
			},
		},
	}
end

return ui.div {
	w = "100%",
	h = "100%",
	bg = "$ios.grouped",
	ui.column {
		pt = function()
			return safe:get().top + 54
		end,
		ui.text { "Settings", heading = 1, ml = 16, text_size = "$font.size.xxl", weight = 700, color = "$text.primary" },
	},
	ui.glass {
		absolute = true,
		left = 21,
		right = 21,
		bottom = 21,
		h = 62,
		radius = 31,
		interactive = true,
		direction = "row",
		items = "center",
		justify = "between",
		px = 4,
		bar,
	},
}
