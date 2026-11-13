-- Topmost: a layer above everything the app shows. Open the menu or the
-- popover: the dot, drawn by `ui.topmost`, stays over them (and over a drag's
-- preview), in a window of its own on macOS, its root at the window's 0, 0.
-- The pointer passes through it but for the button it holds.
local menu_open = ui.state("menu_open", false)
local popover_open = ui.state("popover_open", false)
local hits = ui.state("hits", 0)
local angle = ui.state("angle", 0)

-- The dot circles the window, over whatever opens beneath it.
ui.every(16, function()
	angle:set(angle:peek() + 0.04)
	return true
end)

local menu_button = ui.button { id = "menu-button", "Menu", on_click = ui.toggle(menu_open) }
local popover_button = ui.button { id = "popover-button", "Popover", on_click = ui.toggle(popover_open) }

return ui.column {
	p = 24,
	gap = 12,
	ui.text { "Open a menu or popover; the dot stays above." },
	ui.row { gap = 8, menu_button, popover_button },
	ui.menu {
		open = menu_open,
		anchor = menu_button,
		ui.menu_item { "One" },
		ui.menu_item { "Two" },
		ui.menu_item { "Three" },
	},
	ui.popover {
		open = popover_open,
		anchor = popover_button,
		ui.text { "A popover" },
	},
	ui.topmost {
		-- Over everything and out of the pointer's way.
		ui.div {
			id = "dot",
			absolute = true,
			w = 24,
			h = 24,
			rounded = 12,
			bg = "$accent",
			pointer_events = "none",
			left = function()
				return 300 + math.cos(angle:get()) * 200
			end,
			top = function()
				return 200 + math.sin(angle:get()) * 120
			end,
		},
		-- The one control it holds: pressing it closes no menu.
		ui.button {
			id = "count",
			absolute = true,
			right = 24,
			bottom = 24,
			function()
				return "Pressed " .. hits:get()
			end,
			on_click = function()
				hits:set(hits:peek() + 1)
			end,
		},
	},
}
