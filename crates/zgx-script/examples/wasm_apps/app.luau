--!strict
-- Two WebAssembly apps in this process: a lit 3D scene drawn with WebGPU on
-- the window's own GPU device, and one drawing its pixels on the CPU. Each
-- runs sandboxed on a thread of its own; the window never waits for them.
-- Build them first: crates/zgx-wasm/guest/build.sh.
--
-- The cube: drag to orbit, right-click to look about with the pointer
-- locked (Escape lets go), wheel or arrows to zoom, Space pauses, R
-- reverses, C changes colours, 1-4 set the ring, and what you type builds
-- a wall.

local GUESTS = "../../../zgx-wasm/guest/built/"

local cube = ui.app("cube", { wasm = GUESTS .. "cube.wasm" })
local pixels = ui.app("pixels", { wasm = GUESTS .. "pixels.wasm" })

local function Status(app: App)
	return ui.text {
		size = 12,
		color = "#9aa3b2",
		function()
			if app.status == "failed" then
				return `{app.id}: {app.error}`
			end
			return `{app.id} · {app.status}`
		end,
	}
end

return ui.column {
	w = "100%",
	h = "100%",
	p = 16,
	gap = 12,
	bg = "#0c0e12",
	ui.row {
		gap = 12,
		items = "center",
		ui.text { "WebAssembly apps", size = 18, weight = 600, color = "#e8eaf0" },
		Status(cube),
		Status(pixels),
	},
	ui.row {
		grow = 1,
		gap = 12,
		w = "100%",
		ui.column {
			grow = 1,
			h = "100%",
			relative = true,
			ui.surface { id = "cube", app = cube, w = "100%", h = "100%", rounded = 16, overflow = "hidden" },
			-- UI over it, as over any element.
			ui.text {
				absolute = true,
				left = 14,
				top = 12,
				size = 12,
				color = "#ffffffcc",
				function()
					return cube.title
				end,
			},
		},
		ui.column {
			w = 300,
			gap = 8,
			ui.surface { id = "pixels", app = pixels, w = 300, h = 220, rounded = 12, overflow = "hidden" },
			ui.text {
				size = 11,
				color = "#9aa3b2",
				wrap = true,
				function()
					return pixels.title
				end,
			},
		},
	},
}
