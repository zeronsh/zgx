local video = require("@video")
local fx = require("@fx")

return video.compositions {
	video.composition {
		id = "Hello",
		w = 1280,
		h = 720,
		fps = 30,
		duration = 120,
		props = { name = "zgx", accent = "#ff6a3d" },
		render = function(props)
			local f = video.frame()
			return ui.overlay {
				grow = 1,
				w_full = true,
				h_full = true,
				bg = "#0d0a1f",
				ui.div(
					fx.with(
						fx.mesh_gradient {},
						{ absolute = true, top = 0, left = 0, right = 0, bottom = 0, opacity = 0.75 }
					)
				),
				video.sequence {
					name = "Title",
					from = 10,
					function(t)
						return ui.column {
							w_full = true,
							h_full = true,
							items = "center",
							justify = "center",
							gap = 12,
							ui.text {
								`Hello, {props.name}`,
								text_size = 96,
								weight = 800,
								color = "#fff",
								opacity = t:map({ 0, 12 }, { 0, 1 }),
								y = t:spring { from = 60, to = 0, damping = 12 },
							},
							ui.div {
								h = 6,
								rounded = 3,
								bg = props.accent,
								w = t:spring { from = 0, to = 420, delay = 10, damping = 14 },
							},
						}
					end,
				},
				ui.text {
					function()
						return `frame {math.floor(f:get())}`
					end,
					absolute = true,
					right = 24,
					bottom = 16,
					color = "#ffffff80",
				},
			}
		end,
	},
}
