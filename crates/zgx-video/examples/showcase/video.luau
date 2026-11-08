-- "Every frame": zgx-video's showcase, a film written as a zgx interface. Its
-- cuts fall on a 120 BPM grid, a beat being 30 frames, and every act shows
-- something a browser-free GPU renderer does for it. `make_score.py` writes its
-- music. Render it:
--     python3 make_score.py
--     zgx video render . film.mp4 --motion-blur 4
local video = require("@video")
local fx = require("@fx")

local INK = "#07051a"
local CREAM = "#fff4e0"
local VIOLET = "#7c4dff"
local CORAL = "#ff5a5f"
local SAFFRON = "#ffb703"
local TEAL = "#06d6a0"
local SKY = "#4cc9f0"
local BEAT = 30

---------------------------------------------------------------- shared pieces

local FILL = { absolute = true, top = 0, left = 0, right = 0, bottom = 0 }
local function fill(props)
	local out = table.clone(FILL)
	for key, value in props or {} do
		out[key] = value
	end
	return out
end

-- A shader fill behind everything.
local function backdrop(effect, props)
	return ui.div(fx.with(effect, fill(props)))
end

-- Letters landing one after another: a line of type that moves as type does.
local function kinetic(t, text, o)
	local row = { items = "end", justify = o.justify or "center", gap = o.gap or 0 }
	local index = 0
	local count = #text
	for at = 1, count do
		local char = text:sub(at, at)
		if char == " " then
			table.insert(row, ui.div { w = o.size * (o.space or 0.3), shrink = 0 })
		else
			index += 1
			local delay = (o.delay or 0) + (index - 1) * (o.stagger or 2)
			local spring = { stiffness = o.stiffness or 170, damping = o.damping or 15, delay = delay }
			local function rise(from, to)
				local s = table.clone(spring)
				s.from, s.to = from, to
				return t:spring(s)
			end
			table.insert(
				row,
				ui.text {
					char,
					text_size = o.size,
					weight = o.weight or 800,
					color = o.color or CREAM,
					shrink = 0,
					y = rise(o.from_y or o.size * 0.8, 0),
					rotate = rise(o.turn or 10, 0),
					scale = rise(o.from_scale or 0.8, 1),
					opacity = t:map({ delay, delay + 7 }, { 0, 1 }),
				}
			)
		end
	end
	return ui.row(row)
end

-- A colour running along what's drawn: type made of light.
local IRIDESCENT = ui.layer_shader([[
fn layer(input: LayerInput) -> vec4<f32> {
    let u = input.p / input.size;
    let phase = u.x * 1.6 + u.y * 0.7 + input.time * 0.32;
    let a = vec3(1.0, 0.35, 0.37);
    let b = vec3(0.49, 0.30, 1.0);
    let c = vec3(1.0, 0.72, 0.01);
    let d = vec3(0.30, 0.79, 0.94);
    let f = fract(phase) * 4.0;
    var color = mix(a, b, clamp(f, 0.0, 1.0));
    color = mix(color, c, clamp(f - 1.0, 0.0, 1.0));
    color = mix(color, d, clamp(f - 2.0, 0.0, 1.0));
    color = mix(color, a, clamp(f - 3.0, 0.0, 1.0));
    let sheen = pow(0.5 + 0.5 * sin(u.x * 9.0 - input.time * 2.4 + u.y * 3.0), 18.0);
    return vec4(color + vec3(sheen * 0.7), input.color.a);
}]])

-- The film's cuts: a diagonal of light sweeps across, and the new scene is
-- behind it. Made of the new scene's own pixels: no read of the old.
local FLARE = [[
fn layer(input: LayerInput) -> vec4<f32> {
    let p = input.params[0].x;
    let u = input.p / input.size;
    // Along the sweep, leaning 20 degrees, with a slow ripple in its edge.
    let along = u.x * 0.86 + u.y * 0.34 + sin(u.y * 9.0 + p * 6.0) * 0.012;
    let edge = mix(-0.12, 1.32, p);
    let shown = smoothstep(edge + 0.015, edge - 0.015, along);
    let near = abs(along - edge);
    let core = exp(-near * near / 0.00006);
    let halo = exp(-near * near / 0.0016);
    let hot = mix(vec3(1.0, 0.36, 0.38), vec3(1.0, 0.85, 0.55), core);
    let glow = (core * 1.2 + halo * 0.55) * smoothstep(0.0, 0.08, p) * smoothstep(1.0, 0.9, p);
    let color = input.color.rgb * shown + hot * glow;
    let alpha = clamp(input.color.a * shown + glow, 0.0, 1.0);
    return vec4(color / max(alpha, 0.001), alpha);
}]]
local function burn(duration)
	return video.shader_transition { wgsl = FLARE, duration = duration or BEAT, ease = "inOutCubic" }
end

-- The light falls away at the edges, and film grain lies over all of it.
local function finish()
	return ui.div {
		fill(),
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		pointer_events = "none",
		ui.div {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			gradient = { angle = 180, stops = { "#00000066", "#00000000", "#00000000", "#00000077" } },
		},
	}
end

local function circle(size, props)
	local node = { w = size, h = size, rounded = size / 2 }
	for key, value in props do
		node[key] = value
	end
	return ui.div(node)
end

---------------------------------------------------------------- act 1: a spark

local function spark(t)
	local rings = {}
	for i = 1, 4 do
		local from = 90 + (i - 1) * 5
		table.insert(
			rings,
			ui.div {
				absolute = true,
				left = 960 - 40,
				top = 540 - 40,
				w = 80,
				h = 80,
				rounded = 40,
				border = 3,
				border_color = ({ CORAL, SAFFRON, VIOLET, SKY })[i],
				scale = t:map({ from, from + 70 }, { 0.2, 18 - i * 2 }, { ease = "outExpo" }),
				opacity = t:map({ from, from + 8, from + 60 }, { 0, 1, 0 }),
			}
		)
	end
	return ui.div {
		fill(),
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		bg = INK,
		overflow = "hidden",
		-- The colour the spark lets in.
		ui.div {
			fill(),
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			opacity = t:map({ 92, 150 }, { 0, 0.85 }),
			scale = t:map({ 92, 270 }, { 1.25, 1 }, { ease = "outCubic" }),
			ui.div(fx.with(fx.mesh_gradient { a = "#100a30", b = VIOLET, c = CORAL, speed = 0.5, warp = 0.6 }, fill())),
		},
		rings,
		-- The spark: gathers, hangs, and goes.
		circle(36, {
			absolute = true,
			left = 960 - 18,
			top = 540 - 18,
			bg = CREAM,
			scale = function()
				local f = t:get()
				if f < 88 then
					return video.spring {
						frame = f,
						from = 0,
						to = 1,
						stiffness = 60,
						damping = 8,
					} * (1 + 0.25 * math.sin(f * 0.22))
				end
				return video.interpolate(f, { 88, 96 }, { 1.2, 0 }, { ease = "inCubic" })
			end,
			shadow = { x = 0, y = 0, blur = 70, spread = 12, color = "#ffb703cc" },
		}),
		-- The words it throws out.
		ui.column {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			items = "center",
			justify = "center",
			gap = 2,
			kinetic(t, "EVERY", { size = 196, weight = 900, delay = 96, stagger = 3, from_y = 170, turn = -16 }),
			kinetic(t, "FRAME", { size = 196, weight = 900, delay = 110, stagger = 3, from_y = 170, turn = 16 }),
		},
	}
end

---------------------------------------------------------------- act 2: a function

local function fn_of_time(t)
	return ui.div {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		bg = "#0a0722",
		overflow = "hidden",
		ui.div(
			fx.with(
				fx.waves { a = "#1a1147", b = "#120b34", c = "#0a0722", amplitude = 0.07, frequency = 1.1, speed = 0.35 },
				fill()
			)
		),
		ui.column {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			items = "center",
			justify = "center",
			gap = 26,
			kinetic(
				t,
				"IS A",
				{ size = 74, weight = 500, delay = 4, stagger = 2, color = "#b7b2d9", from_y = 40, turn = 0 }
			),
			ui.text {
				"FUNCTION",
				text_size = 232,
				weight = 900,
				letter_spacing = -6,
				color = "#fff",
				layer_shader = IRIDESCENT,
				layer_clock = true,
				scale = t:spring { from = 1.5, to = 1, stiffness = 90, damping = 14, delay = BEAT - 12 },
				opacity = t:map({ BEAT - 12, BEAT - 4 }, { 0, 1 }),
				blur = t:map({ BEAT - 12, BEAT + 8 }, { 26, 0 }),
			},
			ui.row {
				items = "center",
				gap = 16,
				kinetic(
					t,
					"OF",
					{ size = 74, weight = 500, delay = BEAT * 2, stagger = 3, color = "#b7b2d9", from_y = 40, turn = 0 }
				),
				ui.text {
					"t",
					font = "monospace",
					text_size = 92,
					weight = 700,
					color = SAFFRON,
					scale = t:spring { from = 0, to = 1, delay = BEAT * 2 + 12, stiffness = 260, damping = 11 },
				},
				-- What t is, as it runs.
				ui.text {
					function()
						return string.format("= %05.2f s", t:get() / 60)
					end,
					font = "monospace",
					text_size = 44,
					color = "#8d86b8",
					ml = 18,
					opacity = t:map({ BEAT * 2 + 24, BEAT * 2 + 40 }, { 0, 1 }),
				},
			},
		},
	}
end

---------------------------------------------------------------- act 3: shaders

local TILES = {
	{ "liquid metal", fx.liquid_metal { preset = "gold" } },
	{ "mesh gradient", fx.mesh_gradient { preset = "orchid", speed = 0.5 } },
	{ "metaballs", fx.metaballs { preset = "lava", speed = 0.7 } },
	{ "voronoi", fx.voronoi { a = "#3a0ca3", b = "#f72585", seam = "#ffe8f4", speed = 0.25, scale = 44 } },
	{ "swirl", fx.swirl { a = "#07051a", b = VIOLET, c = "#ffd6a5", speed = 0.45 } },
	{ "waves", fx.waves { preset = "dunes", speed = 0.25 } },
	{ "plastic", fx.plastic { color = CORAL } },
	{ "mesh gradient", fx.mesh_gradient { preset = "lagoon", speed = 0.5 } },
	{ "metaballs", fx.metaballs { preset = "mint", speed = 0.6 } },
	{ "halftone", fx.halftone { ink = "#1a1147", base = SAFFRON, cell = 11 } },
	{ "liquid metal", fx.liquid_metal { preset = "rose", flow = 0.9 } },
	{ "grain gradient", fx.grain_gradient {} },
}
local function shaders(t)
	local cols, rows = 4, 3
	local gap, w, h = 18, 448, 332
	local field = {
		absolute = true,
		left = (1920 - (cols * w + (cols - 1) * gap)) / 2,
		top = (1080 - (rows * h + (rows - 1) * gap)) / 2,
	}
	field.w, field.h = cols * w + (cols - 1) * gap, rows * h + (rows - 1) * gap
	-- A camera easing back over the whole act.
	field.scale = t:map({ 0, 360 }, { 1.2, 1 }, { ease = "outCubic" })
	field.rotate = t:map({ 0, 360 }, { -2.5, 0 }, { ease = "outCubic" })
	for i, tile in TILES do
		local col, row = (i - 1) % cols, math.floor((i - 1) / cols)
		-- Rippling out from the middle.
		local reach = math.abs(col - 1.5) + math.abs(row - 1)
		local delay = 6 + reach * 9
		table.insert(
			field,
			ui.div {
				absolute = true,
				left = col * (w + gap),
				top = row * (h + gap),
				w = w,
				h = h,
				perspective = 1000,
				rotate_x = t:spring { from = -85, to = 0, delay = delay, stiffness = 120, damping = 13 },
				scale = t:spring { from = 0.6, to = 1, delay = delay, stiffness = 120, damping = 15 },
				opacity = t:map({ delay, delay + 8 }, { 0, 1 }),
				ui.div(fx.with(tile[2], fill { rounded = 22, bg = "#222" })),
				ui.text {
					tile[1],
					absolute = true,
					left = 18,
					bottom = 14,
					text_size = 18,
					weight = 600,
					color = "#ffffffd0",
					font = "monospace",
				},
			}
		)
	end
	return ui.div {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		bg = INK,
		overflow = "hidden",
		ui.div(field),
		-- The word, once the wall has built, cut into it by its blend.
		ui.column {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			items = "center",
			justify = "center",
			opacity = t:map({ BEAT * 4, BEAT * 4 + 10 }, { 0, 1 }),
			bg = t:map({ BEAT * 4, BEAT * 4 + 30 }, { "#07051a00", "#07051a99" }),
			ui.text {
				"SHADERS",
				text_size = 258,
				weight = 900,
				letter_spacing = -6,
				color = CREAM,
				y = t:spring { from = 90, to = 0, delay = BEAT * 4, stiffness = 110, damping = 13 },
				scale = t:map({ BEAT * 4, BEAT * 13 }, { 1, 1.08 }),
			},
			ui.text {
				"WGSL, right in the script. Every pixel, every frame, on the GPU.",
				text_size = 34,
				weight = 500,
				color = "#ffffffcc",
				mt = 8,
				opacity = t:map({ BEAT * 5, BEAT * 5 + 14 }, { 0, 1 }),
				y = t:spring { from = 30, to = 0, delay = BEAT * 5, damping = 16 },
			},
		},
	}
end

---------------------------------------------------------------- act 4: glass

local function glass(t)
	-- Colour for the glass to bend: orbs on their own slow paths.
	local orbs = {}
	for i, color in { CORAL, VIOLET, SAFFRON, TEAL, SKY, "#f72585" } do
		local size = 340 + video.random(`size{i}`) * 360
		local ox, oy = video.random(`x{i}`) * 1920, video.random(`y{i}`) * 1080
		local sx, sy = 0.5 + video.random(`sx{i}`), 0.5 + video.random(`sy{i}`)
		table.insert(
			orbs,
			circle(size, {
				absolute = true,
				left = ox - size / 2,
				top = oy - size / 2,
				blur = 40,
				gradient = { angle = 135, stops = { color, color .. "55" } },
				x = function()
					return math.sin(t:get() * 0.021 * sx + i) * 240
				end,
				y = function()
					return math.cos(t:get() * 0.017 * sy + i * 2) * 170
				end,
			})
		)
	end
	local function card(i, title, body, left, top, delay, tint)
		return ui.column {
			absolute = true,
			left = left,
			top = top,
			w = 420,
			h = 250,
			rounded = 30,
			p = 30,
			gap = 10,
			justify = "end",
			bg = tint,
			backdrop_blur = 34,
			border = 1,
			border_color = "#ffffff55",
			shadow = { x = 0, y = 30, blur = 80, color = "#00000055" },
			y = t:spring { from = 640, to = 0, delay = delay, stiffness = 90, damping = 13 },
			rotate = t:spring {
				from = (i % 2 == 0) and 12 or -12,
				to = (i % 2 == 0) and 2 or -2,
				delay = delay,
				stiffness = 90,
				damping = 13,
			},
			ui.text { title, text_size = 42, weight = 800, color = "#fff" },
			ui.text { body, text_size = 21, weight = 500, color = "#ffffffd8", wrap = true },
		}
	end
	return ui.div {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		bg = "#120a2c",
		overflow = "hidden",
		orbs,
		ui.text {
			"GLASS",
			absolute = true,
			left = 0,
			right = 0,
			top = 88,
			align = "center",
			text_size = 340,
			weight = 900,
			letter_spacing = -8,
			color = CREAM,
			x = t:map({ 0, 270 }, { -50, 50 }),
		},
		card(1, "Blur", "What's behind it, frosted, a real pass on the GPU.", 170, 610, 26, "#ffffff22"),
		card(2, "Tint", "Any colour, over anything moving under it.", 750, 660, 38, "#ff5a5f33"),
		card(3, "Depth", "Shadows and hairlines, the way a desk has them.", 1330, 610, 50, "#4cc9f02e"),
		-- A lens rolling over the word: it reads what's behind and bends it.
		ui.div(fx.with(fx.lens { preset = "clear", bevel = 44, depth = 80, dispersion = 0.5, magnification = 1.25 }, {
			absolute = true,
			top = 92,
			w = 420,
			h = 420,
			rounded = 210,
			left = t:map({ 18, 250 }, { -480, 1980 }, { ease = "inOutSine" }),
		})),
	}
end

---------------------------------------------------------------- act 5: particles

local GLYPHS = {
	{ "11111", "00001", "00010", "00100", "01000", "10000", "11111" },
	{ "01110", "10001", "10000", "10111", "10001", "10001", "01110" },
	{ "10001", "10001", "01010", "00100", "01010", "10001", "10001" },
}
local CELL, SUB = 34, 4
local PARTICLES = {}
do
	local across = (#GLYPHS * 5 + (#GLYPHS - 1)) * CELL
	local left, top = (1920 - across) / 2, (1080 - 7 * CELL) / 2
	local n = 0
	for g, glyph in GLYPHS do
		for row, line in glyph do
			for col = 1, 5 do
				if line:sub(col, col) == "1" then
					for a = 0, SUB - 1 do
						for b = 0, SUB - 1 do
							n += 1
							local x = left + ((g - 1) * 6 + col - 1) * CELL + (a + 0.5) * CELL / SUB
							local y = top + (row - 1) * CELL + (b + 0.5) * CELL / SUB
							local angle = video.random(`a{n}`) * math.pi * 2
							local far = 700 + video.random(`d{n}`) * 900
							local hue = (x - left) / across
							PARTICLES[n] = {
								x = x,
								y = y,
								ox = 960 + math.cos(angle) * far,
								oy = 540 + math.sin(angle) * far * 0.7,
								spin = (video.random(`s{n}`) - 0.5) * 4,
								delay = 8 + video.random(`w{n}`) * 60,
								r = 2.4 + video.random(`r{n}`) * 2.6,
								color = (
									video.interpolate(hue, { 0, 0.35, 0.7, 1 }, { CORAL, SAFFRON, TEAL, SKY }) :: string
								):sub(1, 7),
								seed = video.random(`e{n}`),
							}
						end
					end
				end
			end
		end
	end
end
local function particles(t)
	local APART = 170
	return ui.div {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		bg = "#05031a",
		overflow = "hidden",
		ui.div(
			fx.with(
				fx.aurora { color = "#2b1f7a", second = "#0c4a6e", third = "#4c1d95", speed = 0.3, scale = 240 },
				fill { opacity = 0.7 }
			)
		),
		ui.canvas {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			draw = function(c, w, h)
				local f = t:get()
				for _, p in PARTICLES do
					local arrive = video.spring { frame = f, delay = p.delay, stiffness = 40, damping = 9, fps = 60 }
					local angle = (1 - math.min(arrive, 1)) * p.spin
					local dx, dy = p.ox - 960, p.oy - 540
					local rx = dx * math.cos(angle) - dy * math.sin(angle) + 960
					local ry = dx * math.sin(angle) + dy * math.cos(angle) + 540
					local x = rx + (p.x - rx) * arrive
					local y = ry + (p.y - ry) * arrive
					local radius = p.r
					local alpha = math.clamp((f - p.delay) / 10, 0, 1)
					-- Breathing while it holds; thrown out at the end.
					if f > APART then
						local out = video.interpolate(
							f - APART - p.seed * 16,
							{ 0, 70 },
							{ 0, 1 },
							{ ease = "inCubic" }
						)
						x += (x - 960) * out * (1.6 + p.seed * 2.4)
						y += (y - 540) * out * (1.6 + p.seed * 2.4)
						alpha *= 1 - out * out
						radius *= 1 + out * 1.4
					else
						local wave = math.sin(f * 0.1 + x * 0.012)
						y += wave * 3 * arrive
					end
					if alpha > 0.01 then
						c:circle(x, y, radius, p.color .. string.format("%02x", math.floor(alpha * 255)))
					end
				end
			end,
		},
	}
end

---------------------------------------------------------------- act 6: an app

local Board = ui.component(function()
	-- An ordinary interface, with ordinary state, timers and animations: nothing
	-- here knows it's in a video.
	local names = { "Aurora", "Kestrel", "Marlin", "Orchid", "Quartz", "Sable" }
	local colors = { CORAL, SAFFRON, TEAL, SKY, VIOLET, "#f72585" }
	local rows = ui.use_state((function()
		local out = {}
		for i, name in names do
			out[i] = { id = name, score = 40 + i * 7, color = colors[i] }
		end
		return out
	end)())
	local total = ui.use_state(1200)
	local open = ui.use_state(false)
	local bars = {}
	for i = 1, 14 do
		bars[i] = ui.use_state(20 + i * 3)
	end
	local tick = 0
	ui.every(500, function()
		tick += 1
		local list = table.clone(rows:peek())
		local bump = (tick * 3) % #list + 1
		local row = table.clone(list[bump])
		row.score += 30 + math.random(0, 25)
		list[bump] = row
		table.sort(list, function(a, b)
			return a.score > b.score
		end)
		rows:set(list)
		total:add(math.random(200, 900))
		for i, bar in bars do
			bar:set(20 + math.random() * 160 * (0.4 + 0.6 * math.sin(i * 0.5 + tick * 0.7) ^ 2))
		end
		open:set(tick % 2 == 1)
	end)
	local max_score = ui.computed(function()
		local most = 1
		for _, row in rows:get() do
			most = math.max(most, row.score)
		end
		return most
	end)
	local shown_total = ui.tween(total, { duration = 420, ease = "outCubic" })
	local chart = { direction = "row", items = "end", gap = 8, h = 200, grow = 1 }
	for i, bar in bars do
		table.insert(
			chart,
			ui.div {
				grow = 1,
				rounded = 7,
				h = ui.spring(bar, { stiffness = 170, damping = 15 }),
				gradient = { angle = 180, stops = { colors[(i - 1) % 6 + 1], colors[(i - 1) % 6 + 1] .. "44" } },
			}
		)
	end
	return ui.row {
		w = 1320,
		h = 700,
		rounded = 26,
		bg = "#141225",
		border = 1,
		border_color = "#ffffff1c",
		overflow = "hidden",
		shadow = { x = 0, y = 50, blur = 120, color = "#00000099" },
		-- Sidebar
		ui.column {
			w = 280,
			bg = "#0f0d1f",
			p = 22,
			gap = 8,
			ui.row {
				gap = 8,
				mb = 18,
				circle(13, { bg = CORAL }),
				circle(13, { bg = SAFFRON }),
				circle(13, { bg = TEAL }),
			},
			ui.text { "RANKING", text_size = 13, weight = 700, letter_spacing = 1.5, color = "#7b7799", mb = 6 },
			ui.each(rows, function(row: any)
				return ui.row {
					h = 52,
					px = 12,
					gap = 12,
					items = "center",
					rounded = 12,
					bg = row.color .. "1c",
					circle(10, { bg = row.color }),
					ui.text { row.id, text_size = 18, weight = 600, color = CREAM, grow = 1 },
					ui.text { tostring(row.score), text_size = 16, font = "monospace", color = "#c9c5e8" },
				}
			end, {
				key = function(row: any)
					return row.id
				end,
				animate = { duration = 340, ease = "outBack" },
				gap = 8,
			}),
		},
		-- Main
		ui.column {
			grow = 1,
			p = 30,
			gap = 22,
			ui.row {
				justify = "between",
				items = "center",
				ui.column {
					gap = 2,
					ui.text { "Live revenue", text_size = 18, color = "#9a96ba" },
					ui.text {
						function()
							return "$" .. tostring(math.floor(total:get()))
						end,
						text_size = 68,
						weight = 800,
						color = CREAM,
					},
				},
				ui.row {
					w = 94,
					h = 48,
					rounded = 24,
					p = 5,
					bg = ui.tween(open, { from = "#2a2740", to = TEAL, duration = 240 }),
					ui.div {
						w = 38,
						h = 38,
						rounded = 19,
						bg = "#fff",
						x = ui.spring(open, { from = 0, to = 46, stiffness = 300, damping = 18 }),
					},
				},
			},
			ui.row(chart),
			ui.row {
				gap = 16,
				grow = 1,
				ui.column {
					grow = 1,
					rounded = 18,
					bg = "#1d1a33",
					p = 20,
					gap = 10,
					ui.text { "Share", text_size = 16, color = "#9a96ba" },
					ui.column {
						gap = 9,
						ui.each(rows, function(row: any)
							return ui.div {
								h = 8,
								rounded = 4,
								bg = "#ffffff12",
								ui.div {
									h = 8,
									rounded = 4,
									bg = row.color,
									w = function()
										return `{row.score / max_score:get() * 100}%`
									end,
								},
							}
						end, {
							key = function(row: any)
								return row.id
							end,
							animate = { duration = 340, ease = "outCubic" },
						}),
					},
				},
				ui.div(fx.with(fx.mesh_gradient { a = "#2a0a4a", b = CORAL, c = SAFFRON, speed = 0.6 }, {
					w = 280,
					rounded = 18,
					bg = "#222",
					items = "center",
					justify = "center",
					ui.text { "Shaders, too", text_size = 26, weight = 800, color = "#fff" },
				})),
			},
		},
	}
end)

local function an_app(t)
	return ui.div {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		overflow = "hidden",
		ui.div(fx.with(fx.grain_gradient {}, fill { opacity = 0 })),
		ui.div(
			fx.with(fx.mesh_gradient { a = "#0c0726", b = "#3a0ca3", c = "#0a9396", speed = 0.25, warp = 0.5 }, fill())
		),
		ui.column {
			absolute = true,
			top = 44,
			left = 0,
			right = 0,
			items = "center",
			gap = 4,
			kinetic(
				t,
				"YOUR APP IS THE FILM",
				{ size = 76, weight = 900, delay = 8, stagger = 1.4, from_y = 60, turn = 6 }
			),
			ui.text {
				"An ordinary zgx app: state, timers, springs, layout animation. Not a recording of one.",
				text_size = 26,
				weight = 500,
				color = "#e4e0ff",
				opacity = t:map({ 34, 50 }, { 0, 1 }),
			},
		},
		ui.div {
			absolute = true,
			left = 300,
			top = 262,
			perspective = 1600,
			origin = { 0.5, 0.5 },
			rotate_x = t:map({ 20, 90, 300, 420 }, { 24, 7, 2, -3 }, { ease = "outCubic" }),
			rotate_y = t:map({ 20, 420 }, { -13, 6 }, { ease = "inOutSine" }),
			scale = t:spring { from = 0.72, to = 1, delay = 16, stiffness = 60, damping = 14 },
			y = t:spring { from = 560, to = 0, delay = 16, stiffness = 60, damping = 14 },
			Board {},
		},
	}
end

---------------------------------------------------------------- act 7: the end

local function finale(t)
	local LAND = 90
	local claims = { "No browser.", "No window.", "Same pixels, every run." }
	local lines = {
		direction = "column",
		items = "center",
		gap = 4,
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		justify = "center",
	}
	for i, claim in claims do
		local from = (i - 1) * 20
		table.insert(
			lines,
			ui.text {
				claim,
				text_size = 76,
				weight = 800,
				color = ({ CREAM, CREAM, SAFFRON })[i],
				opacity = t:map({ from, from + 8, LAND - 12, LAND }, { 0, 1, 1, 0 }),
				y = t:spring { from = 50, to = 0, delay = from, stiffness = 180, damping = 16 },
				scale = t:map({ LAND - 12, LAND }, { 1, 0.9 }, { ease = "inCubic" }),
			}
		)
	end
	local command = "zgx video render . film.mp4"
	-- A ring on every beat behind the name: one pass of a loop, played again.
	local rings = video.sequence {
		from = LAND,
		video.loop {
			duration = BEAT,
			function(beat)
				return ui.div {
					absolute = true,
					top = 0,
					left = 0,
					right = 0,
					bottom = 0,
					items = "center",
					justify = "center",
					ui.div {
						w = 520,
						h = 520,
						rounded = 260,
						border = 3,
						border_color = SAFFRON,
						scale = beat:map({ 0, BEAT - 1 }, { 0.55, 1.7 }, { ease = "outCubic" }),
						opacity = beat:map({ 0, BEAT - 1 }, { 0.45, 0 }),
					},
				}
			end,
		},
	}
	-- The film so far, as stills: each act held at one frame of its own time.
	local strip =
		{ direction = "row", gap = 18, absolute = true, left = 0, right = 0, bottom = 118, justify = "center" }
	for i, held in { { spark, 200 }, { shaders, 300 }, { glass, 150 }, { an_app, 300 } } do
		local delay = LAND + 70 + (i - 1) * 6
		table.insert(
			strip,
			ui.div {
				w = 1920 * 0.11,
				h = 1080 * 0.11,
				rounded = 10,
				overflow = "hidden",
				border = 1,
				border_color = "#ffffff33",
				opacity = t:map({ delay, delay + 10 }, { 0, 1 }),
				y = t:spring { from = 40, to = 0, delay = delay, stiffness = 170, damping = 16 },
				ui.div {
					-- Out of the box's flow (which would shrink it to the box)
					-- and the containing block of the act's own insets.
					absolute = true,
					left = 0,
					top = 0,
					overflow = "hidden",
					w = 1920,
					h = 1080,
					scale = 0.11,
					origin = { 0, 0 },
					video.freeze { frame = held[2], held[1] },
				},
			}
		)
	end
	-- The score, as it plays, in bars along the floor.
	local score = (function()
		local ok, data = pcall(video.audio_data, "score.wav")
		return if ok then data else nil
	end)()
	local floor =
		{ direction = "row", items = "end", gap = 6, absolute = true, left = 120, right = 120, bottom = 0, h = 110 }
	if score then
		local COUNT = 56
		local bars = ui.computed(function()
			return score:bars { count = COUNT, floor_db = -84 }
		end)
		for i = 1, COUNT do
			table.insert(
				floor,
				ui.div {
					grow = 1,
					rounded_tl = 4,
					rounded_tr = 4,
					h = function()
						return 3 + math.sqrt(bars:get()[i]) * 104
					end,
					gradient = { angle = 0, stops = { CORAL .. "ee", SAFFRON .. "66" } },
					opacity = t:map({ LAND + 20, LAND + 50 }, { 0, 1 }),
				}
			)
		end
	end
	return ui.div {
		absolute = true,
		top = 0,
		left = 0,
		right = 0,
		bottom = 0,
		bg = "#07051a",
		overflow = "hidden",
		ui.div {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			opacity = t:map({ LAND - 4, LAND + 40 }, { 0, 1 }),
			ui.div(
				fx.with(fx.swirl { a = "#07051a", b = "#26146e", c = "#a53860", speed = 0.25, scale = 260 }, fill())
			),
		},
		ui.div(lines),
		rings,
		ui.div(floor),
		ui.div(strip),
		-- The name lands.
		ui.column {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			items = "center",
			justify = "center",
			gap = 30,
			ui.text {
				"zgx",
				text_size = 340,
				weight = 900,
				letter_spacing = -12,
				color = "#fff",
				layer_shader = IRIDESCENT,
				layer_clock = true,
				scale = t:spring { from = 3.2, to = 1, delay = LAND, stiffness = 150, damping = 15 },
				opacity = t:map({ LAND, LAND + 5 }, { 0, 1 }),
				blur = t:map({ LAND, LAND + 16 }, { 30, 0 }),
			},
			ui.text {
				function()
					local typed = math.floor(video.interpolate(t:get(), { LAND + 40, LAND + 90 }, { 0, #command }))
					local caret = if t:get() % 30 < 18 then "▌" else " "
					return "$ " .. command:sub(1, typed) .. caret
				end,
				font = "monospace",
				text_size = 40,
				color = "#e4e0ff",
				opacity = t:map({ LAND + 32, LAND + 40 }, { 0, 1 }),
			},
		},
		-- The light goes down.
		ui.div {
			absolute = true,
			top = 0,
			left = 0,
			right = 0,
			bottom = 0,
			bg = "#000",
			pointer_events = "none",
			opacity = t:map({ 268, 299 }, { 0, 1 }),
		},
	}
end

---------------------------------------------------------------- the film

local film = {
	video.scene { name = "A spark", duration = 270, spark },
	burn(),
	video.scene { name = "A function", duration = 270, fn_of_time },
	burn(),
	video.scene { name = "Shaders", duration = 390, shaders },
	video.zoom { duration = BEAT },
	video.scene { name = "Glass", duration = 270, glass },
	burn(),
	video.scene { name = "Particles", duration = 270, particles },
	video.slide { from = "right", duration = BEAT },
	video.scene { name = "An app", duration = 390, an_app },
	burn(),
	video.scene { name = "The end", duration = 300, finale },
}

return video.compositions {
	video.composition {
		id = "EveryFrame",
		w = 1920,
		h = 1080,
		fps = 60,
		duration = video.series_length(film),
		render = function()
			-- The pill under the captions shows while one does.
			local cues = video.captions_of("captions.srt")
			local now = video.frame()
			local speaking = function()
				return if video.caption_at(cues, now:get() / 60) then 1 else 0
			end
			return ui.div {
				w_full = true,
				h_full = true,
				bg = "#000",
				video.series(film),
				video.audio { src = "score.wav", fade_out = 60 },
				-- What it says, a word at a time.
				ui.row {
					absolute = true,
					left = 0,
					right = 0,
					bottom = 54,
					justify = "center",
					ui.div {
						px = 26,
						py = 12,
						opacity = ui.tween(ui.computed(speaking), { duration = 160 }),
						rounded = 18,
						bg = "#07051a99",
						backdrop_blur = 18,
						border = 1,
						border_color = "#ffffff1f",
						video.captions {
							cues = cues,
							words = true,
							text_size = 34,
							weight = 600,
							color = "#ffffffee",
							active_color = SAFFRON,
							upcoming_color = "#ffffff59",
						},
					},
				},
				ui.div(fx.with(fx.grain { amount = 0.035, fps = 24 }, fill { pointer_events = "none" })),
				finish(),
			}
		end,
	},
}
