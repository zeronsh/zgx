-- Chips in a paragraph: ordinary elements in its lines, atomic as CSS's
-- inline-block. Each has its own state, click and hover; the words wrap around
-- them, and a selection copies each as its text.
local opened = ui.state("opened", "nothing yet")
local count = ui.state("count", 3)

local function chip(name: string, icon: string)
	return ui.row {
		px = 5,
		py = 1,
		gap = 4,
		items = "center",
		rounded = 5,
		bg = "$surface.raised",
		hover = { bg = "$hover" },
		cursor = "pointer",
		on_click = function()
			opened:set(name)
		end,
		ui.svg { code = icon, w = 12, h = 12, tint = "$text.secondary" },
		ui.text { name, literal = true, text_size = 13 },
	}
end

local file =
	[[<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><path d="M2 1h5l3 3v7H2z" fill="currentColor"/></svg>]]

return ui.column {
	w_full = true,
	h_full = true,
	p = 24,
	gap = 16,
	bg = "$surface.base",
	color = "$text.primary",
	selectable = true,
	ui.rich {
		wrap = true,
		max_w = 520,
		text_size = 15,
		"Ask about ",
		chip("src/main.rs", file),
		" and ",
		chip("docs/rich-text.md", file),
		", which wrap with the words around them, never through their middles. A chip that follows state, ",
		ui.row {
			px = 5,
			rounded = 5,
			bg = "$surface.raised",
			on_click = ui.add(count),
			ui.text { "clicked {count} times", text_size = 13 },
		},
		", moves the text after it as it grows.",
	},
	ui.rich {
		wrap = true,
		max_w = 520,
		text_size = 15,
		"A box can sit ",
		ui.inline_box { ui.div { w = 14, h = 14, bg = "$accent", rounded = 3 }, align = "middle", alt = "(middle)" },
		" on the text's middle, ",
		ui.inline_box { ui.div { w = 14, h = 28, bg = "$accent", rounded = 3 }, alt = "(tall)" },
		" on its baseline, growing its line, or ",
		ui.inline_box { ui.div { w = 14, h = 14, bg = "$accent", rounded = 3 }, align = 6, alt = "(raised)" },
		" raised. Select it all and copy: each copies as what it says.",
	},
	ui.text { "Opened: {opened}", color = "$text.secondary" },
}
