--!strict
-- Videos as zgx interfaces. A composition is an ordinary view; what it shows at
-- a frame comes from the frame number, or from any of zgx's animations, which
-- all keep the video's time while it renders (zgx.clock). `require("@video")`
-- under `zgx video`.
--
--     local video = require("@video")
--     return video.compositions {
--         video.composition {
--             id = "Hello", w = 1280, h = 720, fps = 30, duration = 90,
--             props = { name = "you" },
--             render = function(props)
--                 local f = video.frame()
--                 return ui.column {
--                     items = "center", justify = "center", grow = 1, bg = "#101018",
--                     ui.text { `Hello {props.name}`, text_size = 72, opacity = f:map({ 0, 20 }, { 0, 1 }) },
--                 }
--             end,
--         },
--     }
local video = {}

-- What the host tells: which composition to build, with which props.
local host = ui.shared("zgx.video.host", {} :: any)
-- The frame the video stands at.
local frame_state = ui.shared("zgx.video.frame", 0)

local function tell(name: string, arg: any)
	pcall(function()
		ui.invoke(name, arg)
	end)
end

------------------------------------------------------------------------------
-- Easing
------------------------------------------------------------------------------

local pi = math.pi
local function out_bounce(x: number): number
	local n, d = 7.5625, 2.75
	if x < 1 / d then
		return n * x * x
	elseif x < 2 / d then
		x -= 1.5 / d
		return n * x * x + 0.75
	elseif x < 2.5 / d then
		x -= 2.25 / d
		return n * x * x + 0.9375
	end
	x -= 2.625 / d
	return n * x * x + 0.984375
end
local function in_out(f: (number) -> number): (number) -> number
	return function(x)
		if x < 0.5 then
			return f(2 * x) / 2
		end
		return 1 - f(2 - 2 * x) / 2
	end
end
local function flip(f: (number) -> number): (number) -> number
	return function(x)
		return 1 - f(1 - x)
	end
end
local bases: { [string]: (number) -> number } = {
	Quad = function(x)
		return x * x
	end,
	Cubic = function(x)
		return x * x * x
	end,
	Quart = function(x)
		return x ^ 4
	end,
	Quint = function(x)
		return x ^ 5
	end,
	Sine = function(x)
		return 1 - math.cos(x * pi / 2)
	end,
	Expo = function(x)
		return if x <= 0 then 0 else 2 ^ (10 * x - 10)
	end,
	Circ = function(x)
		return 1 - math.sqrt(1 - x * x)
	end,
	Back = function(x)
		return 2.70158 * x ^ 3 - 1.70158 * x * x
	end,
	Elastic = function(x)
		if x <= 0 or x >= 1 then
			return x
		end
		return -(2 ^ (10 * x - 10)) * math.sin((x * 10 - 10.75) * 2 * pi / 3)
	end,
	Bounce = function(x)
		return 1 - out_bounce(1 - x)
	end,
}
local eases: { [string]: (number) -> number } = {
	linear = function(x)
		return x
	end,
}
for name, base in bases do
	eases["in" .. name] = base
	eases["out" .. name] = flip(base)
	eases["inOut" .. name] = in_out(base)
end
video.eases = eases

-- A CSS cubic-bezier, as an easing function.
function video.bezier(x1: number, y1: number, x2: number, y2: number): (number) -> number
	local function at(a: number, b: number, t: number): number
		local u = 1 - t
		return 3 * u * u * t * a + 3 * u * t * t * b + t * t * t
	end
	return function(x)
		if x <= 0 or x >= 1 then
			return x
		end
		local lo, hi = 0, 1
		for _ = 1, 24 do
			local mid = (lo + hi) / 2
			if at(x1, x2, mid) < x then
				lo = mid
			else
				hi = mid
			end
		end
		return at(y1, y2, (lo + hi) / 2)
	end
end

local function ease_of(ease: any): (number) -> number
	if ease == nil then
		return eases.linear
	end
	if type(ease) == "function" then
		return ease
	end
	local found = eases[ease]
	if not found then
		error(`video: no easing "{ease}"`, 3)
	end
	return found
end

------------------------------------------------------------------------------
-- Interpolate
------------------------------------------------------------------------------

local function rgba(color: string): (number, number, number, number)
	local hex = color:gsub("^#", "")
	if #hex == 3 or #hex == 4 then
		hex = hex:gsub(".", "%0%0")
	end
	if #hex ~= 6 and #hex ~= 8 then
		error(`video.interpolate: colours are "#rrggbb" or "#rrggbbaa", not "{color}"`, 3)
	end
	return tonumber(hex:sub(1, 2), 16) :: number,
		tonumber(hex:sub(3, 4), 16) :: number,
		tonumber(hex:sub(5, 6), 16) :: number,
		if #hex == 8 then tonumber(hex:sub(7, 8), 16) :: number else 255
end
local function mix(a: any, b: any, t: number): any
	if type(a) == "string" then
		local r1, g1, b1, a1 = rgba(a)
		local r2, g2, b2, a2 = rgba(b)
		local function c(x: number, y: number): number
			return math.clamp(math.round(x + (y - x) * t), 0, 255)
		end
		return string.format("#%02x%02x%02x%02x", c(r1, r2), c(g1, g2), c(b1, b2), c(a1, a2))
	end
	return a + (b - a) * t
end

-- `x` mapped from `input` (numbers rising) to `output` (numbers or colours),
-- a piece at a time. It holds at the ends unless `clamp = false`, or `left`
-- and `right` say ("clamp", "extend"). `ease` eases each piece.
function video.interpolate(x: number, input: { number }, output: { any }, opts: any?): any
	local o = opts or {}
	if #input ~= #output or #input < 2 then
		error("video.interpolate: input and output are lists of the same length, two or more", 2)
	end
	local default = if o.clamp == false then "extend" else "clamp"
	local n = #input
	if x <= input[1] and (o.left or default) == "clamp" then
		return output[1]
	end
	if x >= input[n] and (o.right or default) == "clamp" then
		return output[n]
	end
	local i = 1
	while i < n - 1 and x >= input[i + 1] do
		i += 1
	end
	local t = (x - input[i]) / (input[i + 1] - input[i])
	if t >= 0 and t <= 1 then
		t = ease_of(o.ease)(t)
	end
	return mix(output[i], output[i + 1], t)
end

------------------------------------------------------------------------------
-- Springs
------------------------------------------------------------------------------

-- A damped spring's position at `t` seconds, from 0 going to 1, in closed form:
-- any frame on its own, and the same whichever came before.
local function spring_at(t: number, stiffness: number, damping: number, mass: number, velocity: number): number
	if t <= 0 then
		return 0
	end
	local w = math.sqrt(stiffness / mass)
	local z = damping / (2 * math.sqrt(stiffness * mass))
	local v = -velocity
	if z < 1 then
		local d = w * math.sqrt(1 - z * z)
		local e = math.exp(-z * w * t)
		return 1 - e * (math.cos(d * t) + ((z * w - v) / d) * math.sin(d * t))
	elseif z == 1 then
		return 1 - math.exp(-w * t) * (1 + (w - v) * t)
	end
	local d = w * math.sqrt(z * z - 1)
	local r1, r2 = -z * w + d, -z * w - d
	local a = (v - r2) / (r1 - r2)
	return 1 - (a * math.exp(r1 * t) + (1 - a) * math.exp(r2 * t))
end

-- How long a spring takes to stay within 0.5% of its end, in seconds.
local function spring_settles(stiffness: number, damping: number, mass: number, velocity: number): number
	local t = 20
	local step = 1 / 120
	while t > 0 and math.abs(spring_at(t, stiffness, damping, mass, velocity) - 1) < 0.005 do
		t -= step
	end
	return t + step
end

-- A spring at frame `frame` of a `fps` video: `from` to `to` (0 and 1),
-- `stiffness` (100), `damping` (10), `mass` (1), `velocity` (0, in units a
-- second), `delay` in frames, `duration` in frames stretching it to settle
-- then, `overshoot = false` holding it at its end.
function video.spring(o: any): number
	local fps = o.fps or (video.config().fps :: number)
	local stiffness, damping, mass = o.stiffness or 100, o.damping or 10, o.mass or 1
	local velocity = o.velocity or 0
	local t = ((o.frame :: number) - (o.delay or 0)) / fps
	if o.duration then
		t *= spring_settles(stiffness, damping, mass, velocity) / (o.duration / fps)
	end
	local p = spring_at(t, stiffness, damping, mass, velocity)
	if o.overshoot == false then
		p = math.min(p, 1)
	end
	local from, to = o.from or 0, o.to or 1
	return mix(from, to, p)
end

------------------------------------------------------------------------------
-- Chance
------------------------------------------------------------------------------

-- A number from 0 to 1 that `seed` (a number or string) alone decides.
function video.random(seed: any): number
	local h = 2166136261
	for _, byte in { string.byte(tostring(seed), 1, -1) } do
		h = bit32.bxor(h, byte)
		h = (h * 16777619) % 4294967296
	end
	h = bit32.bxor(h, bit32.rshift(h, 15))
	h = (h * 2246822519) % 4294967296
	h = bit32.bxor(h, bit32.rshift(h, 13))
	return h / 4294967296
end

------------------------------------------------------------------------------
-- Time
------------------------------------------------------------------------------

export type Frame = {
	offset: number,
	duration: number,
	get: (self: Frame) -> number,
	peek: (self: Frame) -> number,
	seconds: (self: Frame) -> number,
	progress: (self: Frame) -> number,
	map: (self: Frame, input: { number }, output: { any }, opts: any?) -> () -> any,
	spring: (self: Frame, opts: any?) -> () -> any,
}

local Frame = {}
Frame.__index = Frame
-- The frame, from this sequence's start: read in a function, it follows.
function Frame.get(self: any): number
	return frame_state:get() - self.offset
end
function Frame.peek(self: any): number
	return frame_state:peek() - self.offset
end
function Frame.seconds(self: any): number
	return (frame_state:get() - self.offset) / self.fps
end
-- From 0 at its start to 1 at its end.
function Frame.progress(self: any): number
	return math.clamp((frame_state:get() - self.offset) / math.max(self.duration - 1, 1), 0, 1)
end
-- A function of the frame to give a property: `opacity = f:map({ 0, 20 }, { 0, 1 })`.
function Frame.map(self: any, input: { number }, output: { any }, opts: any?): () -> any
	return function()
		return video.interpolate(frame_state:get() - self.offset, input, output, opts)
	end
end
-- A spring of the frame to give a property: `y = f:spring { from = 40, to = 0, delay = 5 }`.
function Frame.spring(self: any, opts: any?): () -> any
	local o = table.clone(opts or {})
	o.fps = o.fps or self.fps
	return function()
		o.frame = frame_state:get() - self.offset
		return video.spring(o)
	end
end

local function new_time(offset: number, duration: number, fps: number): any
	return setmetatable({ offset = offset, duration = duration, fps = fps }, Frame)
end

-- What the function building now sees: its sequence's time. Components built
-- later read the same from the context each sequence provides.
local current: any = nil
local composition: any = nil

local function time_here(): any
	if current then
		return current
	end
	local provided: any = nil
	pcall(function()
		provided = ui.use_context("zgx.video.time")
	end)
	return provided
		or new_time(0, composition and composition.duration or math.huge, composition and composition.fps or 30)
end

-- The current frame, from the start of the sequence it's asked in, as state:
-- `f:get()` in a function follows it, and `f:map` and `f:spring` make a property of it.
function video.frame(): Frame
	return time_here()
end

-- The composition being rendered: `id`, `w`, `h`, `fps`, `duration`, `props`.
function video.config(): any
	return composition or { fps = 30, w = 0, h = 0, duration = 0, props = {} }
end

local function building(time: any, build: any): any
	local previous = current
	current = time
	local ok, result = pcall(build)
	current = previous
	if not ok then
		error(result, 0)
	end
	return result
end

local function children_of(opts: any, time: any): { any }
	local out = {}
	for _, child in ipairs(opts) do
		if type(child) == "function" then
			table.insert(
				out,
				building(time, function()
					return (child :: any)(time)
				end)
			)
		else
			table.insert(out, child)
		end
	end
	return out
end

local RESERVED = { from = true, duration = true, name = true, layout = true }

-- Shows its children only from frame `from` for `duration` frames, their time
-- starting at `from`: `f:get()` is 0 as it appears, and whatever starts as it
-- mounts (`anime.animate`, `ui.spring`) starts then. A function child is built
-- as it appears, given that time. It fills its parent unless `layout = "none"`;
-- other properties go on its box.
function video.sequence(opts: any): any
	local parent = time_here()
	local from: number = opts.from or 0
	local duration: number = opts.duration or math.huge
	local offset = parent.offset + from
	local time = new_time(offset, duration, parent.fps)
	if opts.name or opts.track ~= false then
		tell("zgx.video.sequence", {
			name = opts.name or "Sequence",
			from = offset,
			duration = if duration == math.huge then -1 else duration,
		})
	end
	local box: any = { provide = { ["zgx.video.time"] = time } }
	if opts.layout ~= "none" then
		box.absolute, box.top, box.left, box.right, box.bottom = true, 0, 0, 0, 0
	end
	for key, value in opts do
		if type(key) == "string" and not RESERVED[key] and key ~= "track" then
			box[key] = value
		end
	end
	local Body = ui.component(function()
		for i, child in children_of(opts, time) do
			box[i] = child
		end
		return ui.div(box)
	end)
	return ui.show(function()
		local n = frame_state:get() - offset
		return n >= 0 and n < duration
	end, Body {})
end

------------------------------------------------------------------------------
-- Series and transitions
------------------------------------------------------------------------------

-- A scene of a `video.series`: `duration` frames, `name`, and what it shows, as
-- for `video.sequence`.
function video.scene(opts: any): any
	local scene = table.clone(opts)
	scene.is_scene = true
	return scene
end

-- What a transition does to its two scenes, given how far it has come: `enter`
-- makes the properties of the scene arriving, `exit` of the one leaving.
function video.transition(opts: any): any
	local t = table.clone(opts)
	t.is_transition = true
	return t
end
function video.fade(opts: any?): any
	local o = opts or {} :: any
	return video.transition {
		duration = o.duration or 20,
		ease = o.ease or "inOutSine",
		enter = function(p)
			return { opacity = p }
		end,
	}
end
-- The new scene pushes the old out, from `from` ("right", "left", "top", "bottom").
function video.slide(opts: any?): any
	local o = opts or {} :: any
	local dir = o.from or "right"
	local axis = if dir == "left" or dir == "right" then "x" else "y"
	local sign = if dir == "right" or dir == "bottom" then 1 else -1
	local size = function()
		local c = video.config()
		return if axis == "x" then c.w else c.h
	end
	return video.transition {
		duration = o.duration or 24,
		ease = o.ease or "inOutCubic",
		enter = function(p)
			return { [axis] = sign * size() * (1 - p) }
		end,
		exit = function(p)
			return { [axis] = -sign * size() * p }
		end,
	}
end
-- The old scene falls back as the new one comes up from a little large.
function video.zoom(opts: any?): any
	local o = opts or {} :: any
	return video.transition {
		duration = o.duration or 24,
		ease = o.ease or "inOutCubic",
		enter = function(p)
			return { scale = 1.25 - 0.25 * p, opacity = p }
		end,
		exit = function(p)
			return { scale = 1 - 0.12 * p }
		end,
	}
end
-- A soft-edged wipe across the new scene, in the direction of `angle` degrees.
-- Its shader sees the old scene behind it: `shader` takes any WGSL of the same
-- shape, reading `input.params[0].x` as how far it has come.
local WIPE = [[
fn layer(input: LayerInput) -> vec4<f32> {
    let p = input.params[0].x;
    let dir = input.params[0].yz;
    let span = abs(dir.x) * input.size.x + abs(dir.y) * input.size.y;
    let c = input.p - input.size * 0.5;
    let at = (dot(c, dir) + span * 0.5) / span;
    let soft = input.params[0].w;
    let shown = smoothstep(at - soft, at + soft, p * (1.0 + 2.0 * soft) - soft);
    return vec4(input.color.rgb, input.color.a * shown);
}]]
function video.shader_transition(opts: any): any
	local shader = ui.layer_shader(opts.wgsl or opts.shader)
	return video.transition {
		duration = opts.duration or 30,
		ease = opts.ease or "inOutSine",
		shader = shader,
		params = opts.params or function(p)
			return { { p, 0, 0, 0 } }
		end,
	}
end
function video.wipe(opts: any?): any
	local o = opts or {} :: any
	local angle = math.rad(o.angle or 0)
	local dx, dy, soft = math.cos(angle), math.sin(angle), o.soft or 0.06
	return video.shader_transition {
		wgsl = WIPE,
		duration = o.duration or 30,
		ease = o.ease or "inOutCubic",
		params = function(p)
			return { { p, dx, dy, soft } }
		end,
	}
end

-- Scenes one after another, each starting as the last ends; a transition
-- between two overlaps them by its duration. Fills its parent.
function video.series(opts: any): any
	local parent = time_here()
	local items: { any } = {}
	local at: number = 0
	local before: any = nil
	for i, item in ipairs(opts) do
		if item.is_transition then
			before = item
		elseif item.is_scene then
			local from = at
			if before then
				from -= before.duration
			end
			local entry = { scene = item, from = from, enter = before, exit = nil :: any }
			if before and #items > 0 then
				items[#items].exit = before
			end
			before = nil
			at = from + item.duration
			table.insert(items, entry)
		else
			error(`video.series: entry {i} is neither a video.scene nor a transition`, 2)
		end
	end
	local nodes: any = { absolute = true, top = 0, left = 0, right = 0, bottom = 0 }
	for _, entry in items do
		local scene = entry.scene
		local offset = parent.offset + entry.from
		local function how_far(transition: any, first: number): number
			local n = frame_state:get() - first
			return ease_of(transition.ease)(math.clamp((n + 1) / (transition.duration + 1), 0, 1))
		end
		local box: any = {}
		for key, value in scene do
			if type(key) == "string" and key ~= "is_scene" then
				box[key] = value
			end
		end
		for i, child in ipairs(scene) do
			box[i] = child
		end
		box.from = entry.from
		box.duration = scene.duration
		local enter, exit = entry.enter, entry.exit
		local function looks(): any
			local out = {}
			local n = frame_state:get()
			if enter and enter.enter and n < offset + enter.duration then
				for key, value in enter.enter(how_far(enter, offset)) do
					out[key] = value
				end
			end
			if exit and exit.exit and n >= offset + scene.duration - exit.duration then
				for key, value in exit.exit(how_far(exit, offset + scene.duration - exit.duration)) do
					out[key] = value
				end
			end
			return out
		end
		local keys: { [string]: boolean } = {}
		for _, t in { enter and enter.enter or false, exit and exit.exit or false } do
			if t then
				for key in (t :: any)(0.5) do
					keys[key] = true
				end
			end
		end
		for key in keys do
			local rest = if key == "opacity" or key == "scale" then 1 else 0
			box[key] = function()
				local value = looks()[key]
				return if value == nil then rest else value
			end
		end
		if enter and enter.shader then
			box.layer_shader = enter.shader
			box.layer_params = function()
				return enter.params(how_far(enter, offset))
			end
			box.layer_on = function()
				return frame_state:get() < offset + enter.duration
			end
		end
		table.insert(nodes, video.sequence(box))
	end
	return ui.div(nodes)
end

-- How many frames a series takes, its transitions overlapping: for a duration.
function video.series_length(list: { any }): number
	local total = 0
	for _, item in list do
		if item.is_transition then
			total -= item.duration
		elseif item.is_scene then
			total += item.duration
		end
	end
	return total
end

------------------------------------------------------------------------------
-- Audio and waiting
------------------------------------------------------------------------------

-- A sound under the video: `src` (a file beside the script), `from` (its start,
-- in frames, within this sequence), `duration`, `trim` (frames of it to skip),
-- `volume` (1), `fade_in` and `fade_out` (frames), `loop`. Goes anywhere among
-- children; it adds no node.
function video.audio(opts: any): nil
	local time = time_here()
	tell("zgx.video.audio", {
		src = opts.src,
		from = time.offset + (opts.from or 0),
		duration = opts.duration or -1,
		trim = opts.trim or 0,
		volume = opts.volume or 1,
		fade_in = opts.fade_in or 0,
		fade_out = opts.fade_out or 0,
		loop = opts.loop or false,
	})
	return nil
end

-- A video file shown from frame `from` (of this sequence) for `duration`
-- frames, `trim` frames of it skipped: each frame of the video shows the frame
-- of the file its time says, decoded by ffmpeg at the video's rate. `w` and
-- `h` size its box and the pictures decoded; its other properties are an
-- image's (`fit` is "cover" unless given). `volume` (1) mixes its sound in,
-- 0 leaves it out; `loop` plays it again from its start.
local CLIP_KEYS = { src = true, from = true, duration = true, trim = true, volume = true, loop = true }
function video.clip(opts: any): any
	local time = time_here()
	local from: number = opts.from or 0
	local at = time.offset + from
	local id = `zgx.clip:{opts.src}:{at}:{opts.trim or 0}:{opts.w or 0}x{opts.h or 0}`
	tell("zgx.video.clip", {
		id = id,
		src = opts.src,
		from = at,
		duration = opts.duration or -1,
		trim = opts.trim or 0,
		w = opts.w,
		h = opts.h,
		volume = if opts.volume == nil then 1 else opts.volume,
		loop = opts.loop or false,
	})
	local image: any = { src = "host:" .. id, fit = "cover" }
	for key, value in opts do
		if type(key) == "string" and not CLIP_KEYS[key] then
			image[key] = value
		end
	end
	return video.sequence {
		from = from,
		duration = opts.duration,
		layout = "none",
		track = false,
		ui.image(image),
	}
end

-- What a sound file holds, moment by moment, to draw from: `bars`, its
-- spectrum as `count` values from 0 to 1 in bands from `low` to `high` hertz
-- (32, 40 to 16000), spaced as pitch is; `level`, its loudness from 0 to 1;
-- `waveform`, `count` peaks from -1 to 1 over `span` seconds from then.
-- Each takes a frame (this sequence's, `f:get()` by default) and reads the
-- same whichever frame came before; levels below `floor_db` (-70) read 0.
-- `seconds` is the sound's length. Read in a function given to a property,
-- it follows the frame: `h = function() return 4 + song:level() * 200 end`.
export type AudioData = {
	seconds: number,
	bars: (self: AudioData, opts: any?) -> { number },
	level: (self: AudioData, opts: any?) -> number,
	waveform: (self: AudioData, opts: any?) -> { number },
}
local function audio_ask(src: string, what: string, args: any): any
	args.src = src
	args.what = what
	local answer = ui.call_sync("zgx.video.audio_data", args)
	if type(answer) == "table" and answer.error then
		error(`video.audio_data: {answer.error}`, 3)
	end
	return answer
end
function video.audio_data(src: string): AudioData
	local time = time_here()
	local data: any = { seconds = audio_ask(src, "length", {}) }
	local function seconds_of(o: any): number
		local frame = if o and o.frame ~= nil then o.frame else (frame_state:get() - time.offset)
		return (frame - (if o and o.from then o.from else 0)) / time.fps
	end
	function data.bars(_self: any, o: any?): { number }
		local opts = o or {}
		return audio_ask(src, "bars", {
			seconds = seconds_of(opts),
			count = opts.count,
			low = opts.low,
			high = opts.high,
			floor_db = opts.floor_db,
		})
	end
	function data.level(_self: any, o: any?): number
		local opts = o or {}
		return audio_ask(src, "level", { seconds = seconds_of(opts), floor_db = opts.floor_db })
	end
	function data.waveform(_self: any, o: any?): { number }
		local opts = o or {}
		return audio_ask(src, "waveform", { seconds = seconds_of(opts), count = opts.count, span = opts.span })
	end
	return data
end

-- Hold the render until the function this returns is called: for what loads in
-- real time (`ui.fetch`, a remote picture). `label` says what, should it take too long.
function video.hold(label: string?): () -> ()
	local id = tostring(video.random(`{label}{os.clock()}{math.random()}`))
	tell("zgx.video.hold", { id = id, label = label or "video.hold" })
	local released = false
	return function()
		if not released then
			released = true
			tell("zgx.video.release", { id = id })
		end
	end
end

------------------------------------------------------------------------------
-- Compositions
------------------------------------------------------------------------------

-- A video: `id`, `w`, `h`, `fps` (30), `duration` in frames, `props` (what it
-- takes, overridden by the render's), `render(props)` making its view. `w`, `h`,
-- `fps` and `duration` may be functions of the props.
function video.composition(opts: any): any
	if type(opts.id) ~= "string" then
		error("video.composition: give it an `id`", 2)
	end
	if type(opts.render) ~= "function" then
		error(`video.composition "{opts.id}": give it a ` .. "`render`" .. " function", 2)
	end
	return opts
end

local function merged(defaults: any, input: any): any
	local out = {}
	for key, value in defaults or {} do
		out[key] = value
	end
	for key, value in input or {} do
		out[key] = value
	end
	return out
end

local function resolved(comp: any, input: any): any
	local props = merged(comp.props, input)
	local function of(value: any, fallback: any): any
		if type(value) == "function" then
			return (value :: any)(props)
		end
		return if value == nil then fallback else value
	end
	local fps = of(comp.fps, 30)
	local duration = of(comp.duration, nil)
	if comp.seconds and not duration then
		duration = math.round(of(comp.seconds, 1) * fps)
	end
	return {
		id = comp.id,
		w = of(comp.w, 1920),
		h = of(comp.h, 1080),
		fps = fps,
		duration = duration or 1,
		props = props,
		defaults = comp.props or {},
		schema = comp.schema or {},
	}
end

-- The script's result: its compositions, the one the host asks for shown.
function video.compositions(list: { any }): any
	local h: any = host:peek()
	local input = if type(h) == "table" then h.input else nil
	local listed = {}
	for _, comp in list do
		table.insert(listed, resolved(comp, if h.selected == comp.id then input else nil))
	end
	tell("zgx.video.compositions", listed)
	local chosen: any = nil
	for i, comp in list do
		if comp.id == h.selected then
			chosen = comp
			composition = listed[i]
		end
	end
	if not chosen then
		return ui.div {}
	end
	local time = new_time(0, composition.duration, composition.fps)
	local Root = ui.component(function()
		return building(time, function()
			return chosen.render(composition.props)
		end)
	end)
	return ui.div {
		w_full = true,
		h_full = true,
		provide = { ["zgx.video.time"] = time },
		Root {},
	}
end

return video
