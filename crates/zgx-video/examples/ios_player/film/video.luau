-- The film the iOS player example plays: a phone's portrait clip (`clip.mp4`,
-- turned by its file) beside a frame count and a moving bar.
local video = require("@video")

return video.compositions {
	video.composition {
		id = "Film",
		w = 360,
		h = 640,
		fps = 30,
		duration = 300,
		render = function()
			local f = video.frame()
			return ui.column {
				w_full = true,
				h_full = true,
				bg = "#101018",
				items = "center",
				justify = "center",
				gap = 16,
				video.clip { id = "phone", src = "clip.mp4", w = 180, h = 320, rounded = 16, volume = 0, loop = true },
				ui.text {
					function()
						return `frame {f:get()}`
					end,
					text_size = 28,
					weight = 700,
					color = "#fff",
				},
				ui.div {
					h = 8,
					rounded = 4,
					bg = "#ff6a3d",
					w = function()
						return 20 + (f:get() % 60) * 5
					end,
				},
			}
		end,
	},
}
