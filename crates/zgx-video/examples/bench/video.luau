-- What the instruction gate's `video` cell plays (`video_bench`): every kind
-- of time a video keeps, in one composition. Kinetic type, springs and
-- interpolations of the frame, anime.js animations started as sequences
-- mount, tweens and springs of state, timers, a shader on the frame clock,
-- a series with transitions, and a canvas drawn from the frame.
local video = require("@video")

local SHADER = ui.shader([[
fn fill(input: FillInput) -> vec4<f32> {
    let u = input.p / input.size;
    return vec4(0.5 + 0.5 * sin(input.time + u.x * 6.0), 0.3, 0.6 + 0.4 * u.y, 1.0);
}]])

local function title(t, text)
	local row = { direction = "row", gap = 2 }
	for i = 1, #text do
		table.insert(
			row,
			ui.text {
				text:sub(i, i),
				text_size = 48,
				weight = 800,
				color = "#ffffff",
				y = t:spring { from = 40, to = 0, delay = i * 2, stiffness = 160, damping = 14 },
				opacity = t:map({ i * 2, i * 2 + 8 }, { 0, 1 }),
			}
		)
	end
	return ui.div(row)
end

local Ticker = ui.component(function()
	local count = ui.use_state(0)
	local wide = ui.use_state(false)
	ui.every(100, function()
		count:add(1)
		wide:set(count:peek() % 2 == 0)
	end)
	local box = ui.ref()
	ui.on_mount(function()
		anime.animate(box:get(), { x = { 0, 200 }, duration = 800, loop = true, alternate = true, ease = "inOutSine" })
	end)
	return ui.column {
		gap = 6,
		ui.text {
			function()
				return `ticks {count:get()}`
			end,
			text_size = 18,
			color = "#ccccff",
		},
		ui.div { ref = box, w = 40, h = 12, rounded = 6, bg = "#ffb703" },
		ui.div { h = 10, rounded = 5, bg = "#06d6a0", w = ui.tween(wide, { from = 40, to = 260, duration = 300 }) },
		ui.div { h = 10, rounded = 5, bg = "#4cc9f0", w = ui.spring(wide, { from = 40, to = 260 }) },
	}
end)

local function scene(name, color)
	return video.scene {
		name = name,
		duration = 300,
		function(t)
			return ui.column {
				w_full = true,
				h_full = true,
				p = 32,
				gap = 16,
				bg = color,
				title(t, name),
				Ticker {},
				ui.div { w = 300, h = 60, rounded = 12, bg_shader = SHADER, shader_clock = true },
				ui.canvas {
					w = 300,
					h = 60,
					draw = function(c, w, h)
						local f = t:get()
						for i = 0, 11 do
							c:circle(20 + i * 24, h / 2 + math.sin(f * 0.1 + i) * 18, 6, "#ff5a5f")
						end
					end,
				},
			}
		end,
	}
end

local film = {
	scene("Frame one", "#1a1147"),
	video.fade { duration = 20 },
	scene("Frame two", "#0a3d3a"),
	video.slide { duration = 20 },
	scene("Frame three", "#3a0c2a"),
}

return video.compositions {
	video.composition {
		id = "Bench",
		w = 640,
		h = 360,
		fps = 30,
		duration = video.series_length(film),
		render = function()
			return video.series(film)
		end,
	},
}
