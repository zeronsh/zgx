--!strict
local keys = ui.state("keys", {} :: { string })
local offset = ui.state("offset", 0)
local limit = ui.state("limit", 100)

return ui.column {
	relative = true,
	w = 300,
	h = 220,
	ui.div {
		absolute = true,
		w = 8,
		h = 200,
		ui.each(keys, function(name: string)
			return ui.div {
				id = `thumb-{name}`,
				absolute = true,
				w = 4,
				h = 24,
				top = ui.map(offset, {
					from = {
						0,
						function()
							return limit:get()
						end,
					},
					to = { 4, 104 },
					clamp = true,
				}),
			}
		end, {
			key = function(name: string)
				return name
			end,
		}),
	},
}
