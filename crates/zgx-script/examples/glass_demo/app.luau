-- The system's glass from Luau: a list scrolling under a bar, a tinted pane,
-- and panes that merge as they near each other. On iOS through
-- `scripts/ios/sim.py run -p zgx-run --example ios_app --resource <this folder>`.
local apart = ui.state("apart", 8)
local tinted = ui.state("tinted", false)
local function pane(x)
	return ui.glass {
		position = "absolute",
		left = x,
		top = 0,
		w = 96,
		h = 96,
		radius = 48,
		align = "center",
		justify = "center",
		ui.text { "Hi", size = 20, color = "#111111" },
	}
end
local rows = {}
for i = 1, 40 do
	rows[i] = ui.row {
		h = 56,
		px = 20,
		align = "center",
		bg = i % 2 == 0 and "#3478f6" or "#f2552c",
		ui.text { "Row " .. i, color = "#ffffff", size = 17 },
	}
end
return ui.div {
	w = "100%",
	h = "100%",
	bg = "#101018",
	ui.scroll { w = "100%", h = "100%", ui.column { pt = 120, pb = 240, rows } },
	ui.glass {
		position = "absolute",
		left = 16,
		right = 16,
		top = 60,
		h = 56,
		radius = 28,
		interactive = true,
		align = "center",
		justify = "center",
		ui.text { "Glass over Luau", size = 17, weight = 600, color = "#111111" },
	},
	ui.glass_group {
		position = "absolute",
		left = 60,
		bottom = 130,
		w = 300,
		h = 96,
		spacing = 40,
		pane(0),
		ui.glass {
			position = "absolute",
			left = function()
				return 96 + apart:get()
			end,
			top = 0,
			w = 96,
			h = 96,
			radius = 48,
		},
	},
	ui.glass {
		position = "absolute",
		left = 16,
		right = 16,
		bottom = 40,
		h = 72,
		radius = 24,
		tint = function()
			return tinted:get() and "#34c759" or "#ffffff00"
		end,
		direction = "row",
		align = "center",
		justify = "center",
		gap = 16,
		ui.button {
			on_click = function()
				apart:set(apart:get() < 40 and 90 or 8)
			end,
			"Merge",
		},
		ui.button {
			on_click = function()
				tinted:set(not tinted:get())
			end,
			"Tint",
		},
	},
}
