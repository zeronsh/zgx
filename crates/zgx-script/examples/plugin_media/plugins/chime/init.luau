-- A plugin with sound and 3D. Its sound is its own file, played into its
-- own bus (`plugin:chime`); without the grants each call is an error.
local chime = audio.sound("chime.wav")
local spins = ui.state("spins", 0)

ui.contribute(
	"media",
	ui.column {
		gap = 8,
		ui.button {
			id = "chime",
			sound = chime,
			on_click = function()
				spins:update(function(n)
					return n + 1
				end)
			end,
			"Chime",
		},
		ui.text { id = "spins", "Chimed {spins} times" },
		ui.scene3d {
			id = "scene",
			w = 200,
			h = 160,
			camera = { position = { 0, 0, 3 }, fov = 40 },
			ui.mesh {
				geometry = "box",
				material = { color = "#4f8cff" },
				rotation = function()
					return { 20, spins:get() * 15, 0 }
				end,
			},
		},
	}
)
