--!nonstrict
-- Ready-made visual effects, as ordinary props: `layer_shader`, `layer_params`,
-- `layer_bleed`, `layer_clock`, `backdrop_blur`, `bg_shader`. Nothing here is
-- more than a script can do. Written by scripts/fx_luau.py; change it there.
--
--     local fx = require("fx")
--     return ui.div(fx.with(fx.glow { preset = "neon" }, { w = 200, h = 80, rounded = 12 }))
--
-- `fx.with(effect, props)` is `props` with the effect's. A one-shot's
-- `effect:fire(x, y)` starts its next, and goes on the node `fx.with` made.
local fx = {}

-- What an effect takes, all optional: `preset`, `seed`, `intensity`, and any of
-- its own parameters by name (`fx.glow { preset = "neon", radius = 12 }`),
-- checked by name as it's made.
export type Options = { [string]: any }
-- An effect made: its spec, options and state, read by `fx.with`, and a
-- one-shot's `fire(x, y)`.
export type Effect = { [string]: any }

local function channel(text, at)
	return (tonumber(string.sub(text, at, at + 1), 16) or 0) / 255
end
-- Lanes from a colour literal.
local function rgb(color)
	local hex = string.sub(color, 2)
	if #hex == 3 then
		hex = string.gsub(hex, ".", "%0%0")
	end
	return channel(hex, 1), channel(hex, 3), channel(hex, 5)
end

local shaders = {}
local function shader_of(spec)
	local key = spec.name
	local made = shaders[key]
	if not made then
		local options = if spec.fps then { fps = spec.fps } else nil
		made = if spec.fill then ui.shader(spec.source, options) else ui.layer_shader(spec.source, options)
		shaders[key] = made
	end
	return made
end

local reduced = ui.prefers.reduced_motion
local instances = 0

-- `spec`'s options as they go in its uniforms' lanes.
local function lanes(spec, o, seed, intensity)
	local p = { { 0, 0, 0, 0 }, { 0, 0, 0, 0 }, { 0, 0, 0, 0 }, { 0, 0, 0, 0 } }
	for name, field in spec.fields do
		local row, lane = field[1] + 1, field[2] + 1
		if row <= 4 then
			local v = o[name]
			if field[3] == "color" then
				p[row][lane], p[row][lane + 1], p[row][lane + 2] = rgb(v)
			elseif field[3] == "bool" then
				p[row][lane] = if v then 1 else 0
			elseif field[3] == "number" then
				p[row][lane] = v
			else
				p[row][lane] = (table.find(field[3], v) or 1) - 1
			end
		end
	end
	p[4][1] = seed
	p[4][2] = intensity
	return p
end

local function make(spec, opts: Options?): Effect
	local opts: Options = opts or {}
	local o = {}
	for name, field in spec.fields do
		o[name] = field[4]
	end
	if opts.preset then
		local preset = spec.presets[opts.preset]
		if not preset then
			error(`fx.{spec.name} has no preset {opts.preset}`, 3)
		end
		for name, v in preset do
			o[name] = v
		end
	end
	for name, v in opts do
		if name ~= "preset" and name ~= "seed" and name ~= "intensity" then
			if spec.fields[name] == nil then
				error(`fx.{spec.name} has no parameter {name}`, 3)
			end
			o[name] = v
		end
	end
	local seed = opts.seed or 0
	local intensity = opts.intensity or 1
	local effect = { spec = spec, options = o, seed = seed, intensity = intensity }
	effect.bleed = spec.bleed(o)
	effect.reach = spec.reach(o)
	effect.frost = spec.frost(o)
	effect.running = spec.animated(o)
	effect.idle = intensity <= 0 or spec.idle(o)
	if spec.one_shot then
		instances += 1
		local key = `fx:{spec.name}:{instances}`
		effect.on = ui.state(key, false)
		effect.shots = ui.state(key .. ":shots", {})
		effect.life = spec.life(o)
		effect.oldest = 0
		function effect:fire(x, y)
			self.oldest = self.oldest % 3 + 1
			local row = self.oldest
			self.shots:set_key({ row }, { x, y })
			self.on:set(true)
			self.alive = (self.alive or 0) + 1
			if self.element then
				-- Its age, in the engine with no script per step.
				local age = { from = 0, to = 1 }
				anime.animate(self.element, {
					[`layerParam{row * 4 + 2}`] = age,
					duration = self.life * 1000,
					ease = "linear",
					onComplete = function()
						self.alive -= 1
						if self.alive <= 0 then
							self.on:set(false)
						end
					end,
				})
			end
		end
	end
	return effect
end

-- `props` with `effect`'s.
-- (Any element's props, handed back to it: typed by the element they go to.)
function fx.with(effect: Effect, props: any): any
	local spec = effect.spec
	if effect.idle then
		return props
	end
	if spec.fill then
		props.bg_shader = shader_of(spec)
		local still = reduced:peek()
		local p = lanes(spec, effect.options, effect.seed, effect.intensity)
		p[4][3] = if still then 1 else 0
		props.shader_params = p
		props.shader_clock = effect.running
		return props
	end
	props.layer_shader = shader_of(spec)
	props.layer_bleed = effect.bleed
	props.layer_reach = effect.reach
	if effect.frost > 0 then
		props.backdrop_blur = effect.frost
	end
	if effect.shots then
		local own = props.on_mount
		props.on_mount = function(element)
			effect.element = element
			if own then
				own(element)
			end
		end
		props.layer_on = effect.on
		props.layer_params = function()
			local p = lanes(spec, effect.options, effect.seed, effect.intensity)
			for row, shot in effect.shots:get() do
				p[row + 1] = { shot[1], shot[2], 0, effect.intensity }
			end
			return p
		end
	else
		props.layer_clock = effect.running
		props.layer_params = function()
			local p = lanes(spec, effect.options, effect.seed, effect.intensity)
			p[4][3] = if reduced:get() then 1 else 0
			return p
		end
	end
	return props
end

local glow_spec = {
	name = "glow",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Glow: light around the box. params: [color, radius], [second, style],
// [third, inner], [seed, intensity, reduced, breathe].
fn layer(input: LayerInput) -> vec4<f32> {
    let radius = max(input.params[0].a, 0.5);
    let style = input.params[1].a;
    let intensity = input.params[3].y;
    let d = fx_box(input.p, input.size, input.radii);
    let t = select(input.time, 0.0, input.params[3].z > 0.5);
    let breath = 1.0 - input.params[3].w * (0.5 + 0.5 * cos(t * FX_TAU / 2.6));
    let outside = max(d, 0.0);
    let tight = exp(-pow(outside / (0.35 * radius), 2.0));
    let wide = exp(-pow(outside / (1.2 * radius), 2.0));
    // It ends before the bleed does, so no quad edge shows.
    let window = 1.0 - smoothstep(radius * 2.0, radius * 2.5, outside);
    var light = fx_knee((0.9 * tight + 0.45 * wide) * intensity * breath) * window;
    light *= select(0.0, 1.0, d > -1.0);
    var color = input.params[0].rgb;
    if style > 1.5 {
        // Aura: three colours flowing round the box.
        let q = input.p - input.size * 0.5;
        let turn = atan2(q.y, q.x) / FX_TAU + 0.5;
        let s = fract(turn + t * 0.12 + 0.08 * fx_noise(q * 0.02 + vec2(t * 0.1), fx_seed(input.params)));
        let k = s * 3.0;
        let a = mix(input.params[0].rgb, input.params[1].rgb, smoothstep(0.0, 1.0, k));
        let b = mix(input.params[1].rgb, input.params[2].rgb, smoothstep(1.0, 2.0, k));
        color = select(b, a, k < 1.0);
        if k >= 2.0 {
            color = mix(input.params[2].rgb, input.params[0].rgb, smoothstep(2.0, 3.0, k));
        }
    }
    let inner = input.params[2].a;
    var glow = vec4(color, light);
    if inner > 0.0 && d <= 0.0 {
        glow = fx_over(vec4(color, inner * intensity * exp(-pow(-d / (0.5 * radius), 2.0))), glow);
    }
    var out = fx_over(input.color, glow);
    if style > 0.5 && style < 1.5 {
        // Neon: a white-hot hairline along the edge, over the content's rim.
        let hair = exp(-pow(d / max(0.75, radius * 0.06), 2.0)) * intensity * breath;
        out = fx_over(vec4(mix(color, vec3(1.0), 0.8), clamp(hair, 0.0, 1.0)), out);
    }
    return out;
}
]==],
	fields = {
		color = { 0, 0, "color", "#5ea5ff" },
		radius = { 0, 3, "number", 14 },
		second = { 1, 0, "color", "#a99cff" },
		style = { 1, 3, { "soft", "neon", "aura" }, "soft" },
		third = { 2, 0, "color", "#ff7ac6" },
		inner = { 2, 3, "number", 0 },
		breathe = { 3, 3, "number", 0 },
	},
	presets = {
		soft = {},
		neon = { style = "neon", color = "#22d3ee", radius = 12 },
		aura = { style = "aura", radius = 18, inner = 0.12 },
	},
	bleed = function(o)
		return o.radius * 2.5
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.breathe > 0 or o.style == "aura"
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.glow(opts: Options?): Effect
	return make(glow_spec, opts)
end

local pulse_spec = {
	name = "pulse",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Pulse: rings or a halo from the box's edge. params: [color, reach],
// [count, period, width, style], [lag, _, _, _], [seed, intensity, reduced, _].
fn fx_beat(phase: f32) -> f32 {
    // Lub-dub: two quick swells in each period.
    let a = exp(-pow((phase - 0.12) / 0.05, 2.0));
    let b = 0.7 * exp(-pow((phase - 0.3) / 0.07, 2.0));
    return a + b;
}
fn layer(input: LayerInput) -> vec4<f32> {
    let reach = max(input.params[0].a, 1.0);
    let count = max(input.params[1].x, 1.0);
    let period = max(input.params[1].y, 0.05);
    let width = max(input.params[1].z, 0.25);
    let style = input.params[1].w;
    let intensity = input.params[3].y;
    let reduced = input.params[3].z > 0.5;
    let d = fx_box(input.p, input.size, input.radii);
    let outside = max(d, 0.0);
    let t = input.time / period;
    var light = 0.0;
    if reduced {
        // A halo that holds still.
        light = 0.45 * exp(-pow(outside / (0.45 * reach), 2.0));
    } else if style < 0.5 {
        // Radar: `count` rings travelling out, fading as they go.
        for (var i = 0; i < 6; i += 1) {
            if f32(i) >= count {
                break;
            }
            let phase = fract(t + f32(i) / count);
            let ring = phase * reach;
            let fade = (1.0 - phase) * (1.0 - phase);
            light += fade * exp(-pow((d - ring) / width, 2.0));
        }
    } else if style < 1.5 {
        // Breathe: a halo growing and easing back.
        let breath = 0.5 - 0.5 * cos(t * FX_TAU);
        light = (0.2 + 0.7 * breath) * exp(-pow(outside / (0.2 * reach + 0.3 * reach * breath), 2.0));
    } else {
        // Heartbeat: rings on each beat.
        let phase = fract(t);
        let beat = fx_beat(phase);
        light = beat * exp(-pow(outside / (0.15 * reach + 0.5 * reach * phase), 2.0));
    }
    light *= select(0.0, 1.0, d > -1.0);
    let window = 1.0 - smoothstep(reach * 0.9, reach, outside);
    let glow = vec4(input.params[0].rgb, clamp(fx_knee(light * intensity * 1.6) * window, 0.0, 1.0));
    return fx_over(input.color, glow);
}
]==],
	fields = {
		color = { 0, 0, "color", "#5ea5ff" },
		reach = { 0, 3, "number", 36 },
		count = { 1, 0, "number", 3 },
		period = { 1, 1, "number", 2.4 },
		width = { 1, 2, "number", 1.6 },
		style = { 1, 3, { "radar", "breathe", "heartbeat" }, "radar" },
	},
	presets = {
		radar = {},
		breathe = { style = "breathe", period = 3.2 },
		heartbeat = { style = "heartbeat", period = 1.2, color = "#ff5d73" },
	},
	bleed = function(o)
		return o.reach
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.pulse(opts: Options?): Effect
	return make(pulse_spec, opts)
end

local ripple_spec = {
	name = "ripple",
	fill = false,
	one_shot = true,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Ripple: rings from where it was fired, cut by the box's shape. params:
// [color, reach], then a shot a row: [x, y, age, strength] (age is its life's
// fraction, and below 0 with motion reduced).
fn fx_ring(input: LayerInput, shot: vec4<f32>, reach: f32) -> f32 {
    if shot.w <= 0.0 {
        return 0.0;
    }
    if shot.z < 0.0 {
        // Reduced motion: a soft brightening, no travel.
        return 0.35 * shot.w * sin(clamp(-shot.z, 0.0, 1.0) * 3.14159);
    }
    let life = clamp(shot.z, 0.0, 1.0);
    let eased = 1.0 - (1.0 - life) * (1.0 - life) * (1.0 - life);
    let r = eased * reach;
    let d = length(input.p - shot.xy) - r;
    let width = 3.0 + 18.0 * life;
    let fade = (1.0 - life) * (1.0 - life);
    return shot.w * fade * (exp(-pow(d / width, 2.0)) + 0.3 * smoothstep(0.0, -r - 1.0, d));
}
fn layer(input: LayerInput) -> vec4<f32> {
    let reach = max(input.params[0].a, 1.0);
    var light = 0.0;
    light += fx_ring(input, input.params[1], reach);
    light += fx_ring(input, input.params[2], reach);
    light += fx_ring(input, input.params[3], reach);
    let inside = fx_cover(fx_box(input.p, input.size, input.radii), input.scale);
    return fx_over(vec4(input.params[0].rgb, clamp(light, 0.0, 1.0) * inside), input.color);
}
]==],
	fields = {
		color = { 0, 0, "color", "#ffffff" },
		reach = { 0, 3, "number", 180 },
		life = { 9, 0, "number", 0.75 },
	},
	presets = {},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return o.life
	end,
}
function fx.ripple(opts: Options?): Effect
	return make(ripple_spec, opts)
end

local shimmer_spec = {
	name = "shimmer",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Shimmer: a bright band crossing the content. params: [color, band],
// [period, angle, pause, on_box], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let band = max(input.params[0].a, 1.0);
    let period = max(input.params[1].x, 0.05);
    let angle = input.params[1].y;
    let pause = clamp(input.params[1].z, 0.0, 0.95);
    let intensity = input.params[3].y;
    var strength = 0.0;
    let across = vec2(cos(angle), sin(angle));
    let extent = abs(across.x) * input.size.x + abs(across.y) * input.size.y;
    if input.params[3].z > 0.5 {
        // Reduced motion: a still, faint sheen.
        strength = 0.35;
    } else {
        // The band starts and ends off the box, so it never cuts in.
        let phase = fract(input.time / period);
        let moving = clamp(phase / (1.0 - pause), 0.0, 1.0);
        let at = -band * 2.0 + moving * (extent + band * 4.0);
        let centre = input.size * 0.5;
        let s = dot(input.p - centre, across) + extent * 0.5;
        strength = exp(-pow((s - at) / band, 2.0));
    }
    // Over the box's own shape where asked, otherwise what the content paints.
    let cover = select(input.color.a, fx_cover(fx_box(input.p, input.size, input.radii), input.scale), input.params[1].w > 0.5);
    let sheen = vec4(input.params[0].rgb, clamp(strength * intensity, 0.0, 1.0) * cover);
    return fx_over(sheen, input.color);
}
]==],
	fields = {
		color = { 0, 0, "color", "#ffffff" },
		band = { 0, 3, "number", 28 },
		period = { 1, 0, "number", 1.8 },
		angle = { 1, 1, "number", 0.35 },
		pause = { 1, 2, "number", 0.3 },
		on_box = { 1, 3, "bool", false },
	},
	presets = {
		sheen = {},
		skeleton = { on_box = true, intensity = 0.5, pause = 0.1 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.shimmer(opts: Options?): Effect
	return make(shimmer_spec, opts)
end

local sparkles_spec = {
	name = "sparkles",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Sparkles: four-point stars twinkling in and past the box. params: [color,
// size], [count, life, margin, _], [_], [seed, intensity, reduced, _].
fn fx_star(q: vec2<f32>, size: f32, scale: f32) -> f32 {
    // A concave four-pointed star, its centre white hot.
    let a = abs(q) / max(size, 0.01);
    let d = pow(pow(a.x + 0.0001, 0.6) + pow(a.y + 0.0001, 0.6), 1.0 / 0.6) - 1.0;
    return clamp(0.5 - d * size * scale, 0.0, 1.0);
}
fn layer(input: LayerInput) -> vec4<f32> {
    let count = i32(clamp(input.params[1].x, 0.0, 12.0));
    let life = max(input.params[1].y, 0.1);
    let margin = input.params[1].z;
    let size = input.params[0].a;
    let seed = fx_seed(input.params);
    let reduced = input.params[3].z > 0.5;
    var light = 0.0;
    var hot = 0.0;
    for (var i = 0; i < count; i += 1) {
        let n = u32(i);
        let span = life * (1.0 + fx_rand(n, seed + 7u));
        let offset = fx_rand(n, seed + 11u) * span;
        let t = (input.time + offset) / span;
        let cycle = floor(t);
        var phase = t - cycle;
        if reduced {
            phase = 0.35;
        }
        let key = n * 97u + u32(select(cycle, 0.0, reduced)) * 131u + seed;
        let at = vec2(fx_rand(key, 1u), fx_rand(key, 2u)) * (input.size + vec2(2.0 * margin)) - vec2(margin);
        // Sizes lean small, with a few large.
        let big = fx_rand(key, 3u);
        let radius = size * (0.45 + 0.9 * big * big);
        // Skip what it can't reach: a star costs its neighbours nothing.
        if length(input.p - at) > radius * 1.5 {
            continue;
        }
        // Pops in, overshooting, and shrinks away; rests between.
        let grow = smoothstep(0.0, 0.15, phase) * (1.0 + 0.3 * exp(-pow((phase - 0.15) / 0.1, 2.0)));
        let shrink = 1.0 - smoothstep(0.55, 0.95, phase);
        let twinkle = 0.75 + 0.25 * sin(input.time * (5.0 + 4.0 * big) * FX_TAU * 0.5 + big * 40.0);
        let spin = (phase - 0.5) * 0.8 + big * 3.0;
        let c = cos(spin);
        let s = sin(spin);
        let q = input.p - at;
        let turned = vec2(c * q.x - s * q.y, s * q.x + c * q.y);
        let star = fx_star(turned, radius * grow * shrink + 0.001, input.scale) * twinkle;
        light = max(light, star);
        hot = max(hot, star * exp(-dot(q, q) / max(radius * radius * 0.05, 0.01)));
    }
    let color = mix(input.params[0].rgb, vec3(1.0), clamp(hot, 0.0, 1.0));
    return fx_over(vec4(color, clamp(light * input.params[3].y, 0.0, 1.0)), input.color);
}
]==],
	fields = {
		color = { 0, 0, "color", "#ffe08a" },
		size = { 0, 3, "number", 11 },
		count = { 1, 0, "number", 6 },
		life = { 1, 1, "number", 1.5 },
		margin = { 1, 2, "number", 10 },
	},
	presets = {},
	bleed = function(o)
		return o.margin + o.size * 2
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return o.count <= 0
	end,
	life = function(o)
		return 0
	end,
}
function fx.sparkles(opts: Options?): Effect
	return make(sparkles_spec, opts)
end

local aurora_spec = {
	name = "aurora",
	fill = false,
	fps = 60,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Aurora: curtains of light over the box. params: [color, scale], [second,
// speed], [third, feather], [seed, intensity, reduced, _].
fn fx_curtain(p: vec2<f32>, t: f32, seed: u32, scale: f32, bias: f32) -> f32 {
    let x = p.x / scale;
    // Its foot wanders; rays hang from it, thinning upward.
    let foot = bias + 0.32 * (fx_noise(vec2(x * 0.7 + t * 0.6, t * 0.2), seed) - 0.5);
    let y = p.y / scale;
    let rays = 0.55 + 0.45 * fx_noise(vec2(x * 7.0 + t * 1.2, y * 0.6), seed + 3u);
    let rise = y - foot;
    return rays * exp(-pow(max(rise, 0.0) / 0.42, 2.0)) * smoothstep(-0.14, 0.0, rise);
}
fn layer(input: LayerInput) -> vec4<f32> {
    let scale = max(input.params[0].a, 1.0);
    let t = select(input.time * input.params[1].a, 3.0, input.params[3].z > 0.5);
    let seed = fx_seed(input.params);
    // Upward from the box's bottom.
    let p = vec2(input.p.x, input.size.y - input.p.y);
    let a = fx_curtain(p, t, seed, scale, 0.3);
    let b = fx_curtain(p + vec2(40.0, 0.0), t * 0.8 + 5.0, seed + 19u, scale, 0.55);
    let c = fx_curtain(p + vec2(90.0, 0.0), t * 1.15 + 11.0, seed + 41u, scale, 0.12);
    let glow = input.params[0].rgb * a + input.params[1].rgb * b + input.params[2].rgb * c;
    let light = clamp(a + b + c, 0.0, 1.0);
    // It feathers out inside the box's shape.
    let feather = max(input.params[2].a, 0.5);
    let d = fx_box(input.p, input.size, input.radii);
    let edge = smoothstep(0.0, feather, -d);
    let alpha = fx_knee(light * 1.3) * edge * input.params[3].y;
    let color = glow / max(max(glow.r, max(glow.g, glow.b)), 0.001);
    return fx_over(vec4(color, clamp(alpha, 0.0, 1.0)), input.color);
}
]==],
	fields = {
		color = { 0, 0, "color", "#4ff7c5" },
		scale = { 0, 3, "number", 120 },
		second = { 1, 0, "color", "#7a7bff" },
		speed = { 1, 3, "number", 0.25 },
		third = { 2, 0, "color", "#dc7bff" },
		feather = { 2, 3, "number", 24 },
	},
	presets = {},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.aurora(opts: Options?): Effect
	return make(aurora_spec, opts)
end

local grain_spec = {
	name = "grain",
	fill = false,
	fps = 24,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Grain: fine noise over the content, by device pixel. params: [amount, mono,
// fps, _], [_], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let seed = fx_seed(input.params);
    let fps = input.params[0].z;
    let frame = select(u32(floor(input.time * fps)), 0u, fps <= 0.0 || input.params[3].z > 0.5);
    let pixel = vec2<i32>(floor(input.p * input.scale));
    let key = seed + frame * 1009u;
    let tri = fx_cell(pixel, key) + fx_cell(pixel, key + 5u) - 1.0;
    let amount = input.params[0].x * input.params[3].y;
    var noise = vec3(tri);
    if input.params[0].y < 0.5 {
        noise = vec3(tri, fx_cell(pixel, key + 11u) + fx_cell(pixel, key + 13u) - 1.0, fx_cell(pixel, key + 17u) + fx_cell(pixel, key + 19u) - 1.0);
    }
    let cover = fx_cover(fx_box(input.p, input.size, input.radii), input.scale);
    // Over a clear box it shows as faint speckle of its own.
    let base = fx_over(input.color, vec4(0.5, 0.5, 0.5, amount * cover));
    return vec4(clamp(base.rgb + noise * amount * cover, vec3(0.0), vec3(1.0)), max(base.a, 0.0));
}
]==],
	fields = {
		amount = { 0, 0, "number", 0.05 },
		mono = { 0, 1, "bool", true },
		fps = { 0, 2, "number", 0 },
	},
	presets = {},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.fps > 0
	end,
	idle = function(o)
		return o.amount <= 0
	end,
	life = function(o)
		return 0
	end,
}
function fx.grain(opts: Options?): Effect
	return make(grain_spec, opts)
end

local electric_spec = {
	name = "electric",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Electric: lightning crawling along the box's edge. params: [color,
// amplitude], [frequency, speed, core, glow], [strands, flicker, _, _], [seed,
// intensity, reduced, _].
fn fx_strand(s: f32, strike: f32, seed: u32, freq: f32) -> f32 {
    return (fx_noise(vec2(s * freq, strike * 1.7), seed) - 0.5) * 1.3
        + (fx_noise(vec2(s * freq * 2.3, strike * 2.9), seed + 5u) - 0.5) * 0.55
        + (fx_noise(vec2(s * freq * 5.1, strike * 7.3), seed + 9u) - 0.5) * 0.25;
}
fn layer(input: LayerInput) -> vec4<f32> {
    let amplitude = input.params[0].a;
    let freq = max(input.params[1].x, 0.01);
    let speed = input.params[1].y;
    let core = max(input.params[1].z, 0.2);
    let halo = max(input.params[1].w, 0.5);
    let strands = i32(clamp(input.params[2].x, 1.0, 3.0));
    let flicker = input.params[2].y;
    let seed = fx_seed(input.params);
    let reduced = input.params[3].z > 0.5;
    let d = fx_box(input.p, input.size, input.radii);
    // A coordinate running round the box's edge, in points.
    let q = input.p - input.size * 0.5;
    let perimeter = (input.size.x + input.size.y) * 2.0;
    let s = fract(atan2(q.y, q.x) / FX_TAU + 0.5) * perimeter;
    let t = select(input.time * speed, 2.0, reduced);
    var light = 0.0;
    var hot = 0.0;
    for (var i = 0; i < strands; i += 1) {
        // Each strikes, holds, and strikes again near the same channel.
        let strike = floor(t + f32(i) * 0.37);
        let hold = fract(t + f32(i) * 0.37);
        let n = fx_strand(s, strike + t * 0.15, seed + u32(i) * 23u, freq);
        let off = abs(d - n * amplitude);
        let fade = select(1.0 - flicker * smoothstep(0.1, 1.0, hold), 0.6, reduced);
        let thin = max(core * (1.0 - 0.3 * f32(i)), 0.2);
        hot = max(hot, fade * exp(-pow(off / thin, 2.0)));
        light = max(light, fade * (0.55 * exp(-pow(off / halo, 2.0)) + 0.25 * exp(-off / (halo * 3.0))));
    }
    // It seams where the coordinate wraps: closed there.
    let seam = smoothstep(0.0, perimeter * 0.015, min(s, perimeter - s));
    let intensity = input.params[3].y;
    let glow = vec4(input.params[0].rgb, clamp(light * intensity, 0.0, 1.0));
    let bolt = vec4(mix(input.params[0].rgb, vec3(1.0), 0.85), clamp(hot * intensity, 0.0, 1.0) * mix(0.4, 1.0, seam));
    return fx_over(bolt, fx_over(input.color, glow));
}
]==],
	fields = {
		color = { 0, 0, "color", "#8fd0ff" },
		amplitude = { 0, 3, "number", 5 },
		frequency = { 1, 0, "number", 0.05 },
		speed = { 1, 1, "number", 3.2 },
		core = { 1, 2, "number", 0.7 },
		glow = { 1, 3, "number", 5 },
		strands = { 2, 0, "number", 2 },
		flicker = { 2, 1, "number", 0.45 },
	},
	presets = {
		storm = {},
		charged = { amplitude = 2, speed = 1.2, flicker = 0.15, strands = 1, glow = 3 },
	},
	bleed = function(o)
		return o.amplitude * 1.2 + o.glow * 4
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.electric(opts: Options?): Effect
	return make(electric_spec, opts)
end

local fire_spec = {
	name = "fire",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fire: flames rising from the box's top edge, with embers. params: [color
// (the tips' tint), height], [scale, speed, embers, warp], [_], [seed,
// intensity, reduced, _]. Temperature falls from white through amber and red.
fn fx_flame(heat: f32, tint: vec3<f32>) -> vec3<f32> {
    let white = vec3(1.0, 0.95, 0.72);
    let amber = vec3(1.0, 0.62, 0.14);
    let a = mix(amber, white, smoothstep(0.62, 0.98, heat));
    return mix(tint, a, smoothstep(0.18, 0.6, heat));
}
fn layer(input: LayerInput) -> vec4<f32> {
    let height = max(input.params[0].a, 1.0);
    let scale = max(input.params[1].x, 1.0);
    let speed = input.params[1].y;
    let warp = input.params[1].w;
    let seed = fx_seed(input.params);
    let reduced = input.params[3].z > 0.5;
    let t = select(input.time * speed, 7.0, reduced);
    // The flame's own height above the top edge, 0 to 1, sides pulled in.
    let up = (-input.p.y) / height;
    let x = input.p.x;
    let edge = min(x, input.size.x - x) / max(min(input.size.x * 0.5, height), 1.0);
    let sides = smoothstep(-0.1, 0.5, edge);
    let q = vec2(x / scale, up * height / scale - t);
    let bent = q + warp * (0.25 + up) * vec2(fx_noise(q * 0.9 + vec2(0.0, t * 0.2), seed) - 0.5, 0.0);
    let n = fx_fbm(bent, seed + 3u);
    let flicker = 0.92 + 0.08 * fx_noise(vec2(x / (scale * 3.0), t * 4.0), seed + 7u);
    let body = n * flicker * (1.0 - clamp(up, 0.0, 1.5) * 0.92) * sides * 1.75 - clamp(up, 0.0, 1.5) * 0.28;
    var heat = clamp(body, 0.0, 1.0);
    heat *= select(0.0, 1.0, input.p.y < 0.5);
    let flame = vec4(fx_flame(heat, input.params[0].rgb), smoothstep(0.08, 0.34, heat));
    // Embers flung up, slowing and cooling.
    var ember = 0.0;
    if !reduced {
        let count = i32(clamp(input.params[1].z, 0.0, 12.0));
        for (var i = 0; i < count; i += 1) {
            let life = 1.1 + 0.8 * fx_rand(u32(i), seed + 31u);
            let u = (input.time * speed * 0.4 + fx_rand(u32(i), seed + 37u) * life) / life;
            let cycle = floor(u);
            let age = u - cycle;
            let key = u32(i) * 53u + u32(cycle) * 17u + seed;
            let start = fx_rand(key, 1u) * input.size.x;
            let rise = (1.0 - exp(-age * 2.2)) * height * 1.2;
            let sway = sin(age * 9.0 + fx_rand(key, 2u) * 6.0) * 5.0;
            let at = vec2(start + sway, -rise);
            let r = 0.8 + 0.8 * fx_rand(key, 3u);
            let d = length(input.p - at);
            ember = max(ember, (1.0 - age) * (1.0 - age) * exp(-pow(d / r, 2.0)));
        }
    }
    let spark = vec4(vec3(1.0, 0.68, 0.28), clamp(ember, 0.0, 1.0));
    let intensity = input.params[3].y;
    var out = fx_over(vec4(spark.rgb, spark.a * intensity), vec4(flame.rgb, flame.a * intensity));
    // Under the content: flames rise from behind it.
    return fx_over(input.color, out);
}
]==],
	fields = {
		color = { 0, 0, "color", "#c92a0e" },
		height = { 0, 3, "number", 54 },
		scale = { 1, 0, "number", 20 },
		speed = { 1, 1, "number", 1.5 },
		embers = { 1, 2, "number", 8 },
		warp = { 1, 3, "number", 1 },
	},
	presets = {
		flames = {},
		plasma = { color = "#2b5cff", height = 26, scale = 12, embers = 0 },
	},
	bleed = function(o)
		return o.height * 1.4
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.fire(opts: Options?): Effect
	return make(fire_spec, opts)
end

local motion_blur_spec = {
	name = "motion_blur",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Motion blur: the content smeared along its velocity. params: [dx, dy,
// samples, _] in points over the shutter, [_], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let by = input.params[0].xy * input.params[3].y;
    let samples = i32(clamp(input.params[0].z, 2.0, 32.0));
    var sum = vec4(0.0);
    var total = 0.0;
    for (var i = 0; i < samples; i += 1) {
        let u = (f32(i) + 0.5) / f32(samples);
        // A raised-cosine shutter: its ends soft.
        let w = 0.5 - 0.5 * cos(u * FX_TAU);
        let c = zgx_sample(input.p - by * (u - 0.5));
        sum += vec4(c.rgb * c.a, c.a) * w;
        total += w;
    }
    let m = sum / max(total, 1e-5);
    return select(vec4(0.0), vec4(m.rgb / m.a, m.a), m.a > 0.0);
}
]==],
	fields = {
		dx = { 0, 0, "number", 0 },
		dy = { 0, 1, "number", 0 },
		samples = { 0, 2, "number", 16 },
	},
	presets = {},
	bleed = function(o)
		return math.max(math.abs(o.dx), math.abs(o.dy)) * 0.5 + 1
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return false
	end,
	idle = function(o)
		return o.dx == 0 and o.dy == 0
	end,
	life = function(o)
		return 0
	end,
}
function fx.motion_blur(opts: Options?): Effect
	return make(motion_blur_spec, opts)
end

local heat_haze_spec = {
	name = "heat_haze",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Heat haze: what's behind the box wavers, rising. params: [amount, scale,
// speed, _], [_], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let amount = input.params[0].x * input.params[3].y;
    let scale = max(input.params[0].y, 1.0);
    let t = select(input.time * input.params[0].z, 4.0, input.params[3].z > 0.5);
    let seed = fx_seed(input.params);
    let q = input.p / scale + vec2(0.0, t);
    let wave = vec2(fx_fbm(q, seed) - 0.5, fx_fbm(q + vec2(5.2, 1.3), seed + 3u) - 0.5);
    let d = fx_box(input.p, input.size, input.radii);
    let edge = smoothstep(0.0, 18.0, -d);
    let behind = zgx_backdrop(input.p + wave * amount * 2.0 * edge);
    return fx_over(input.color, behind);
}
]==],
	fields = {
		amount = { 0, 0, "number", 7 },
		scale = { 0, 1, "number", 42 },
		speed = { 0, 2, "number", 1.2 },
	},
	presets = {},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return math.abs(o.amount) * 1.5
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return o.amount == 0
	end,
	life = function(o)
		return 0
	end,
}
function fx.heat_haze(opts: Options?): Effect
	return make(heat_haze_spec, opts)
end

local shockwave_spec = {
	name = "shockwave",
	fill = false,
	one_shot = true,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Shockwave: a ring bending what's behind the box as it travels from where
// it was fired. params: [amount, reach, width, _], then a shot a row: [x, y,
// age, strength] (age < 0 with motion reduced).
fn fx_wave(input: LayerInput, shot: vec4<f32>, amount: f32, reach: f32, width: f32) -> vec3<f32> {
    if shot.w <= 0.0 || shot.z < 0.0 {
        return vec3(0.0);
    }
    let life = clamp(shot.z, 0.0, 1.0);
    let r = (1.0 - (1.0 - life) * (1.0 - life)) * reach;
    let q = input.p - shot.xy;
    let l = max(length(q), 0.001);
    let s = (l - r) / max(width * (0.5 + life), 0.5);
    // Its profile: out, then in, smooth at both ends.
    let bend = s * exp(-s * s) * (1.0 - life) * (1.0 - life) * amount * shot.w * 2.3;
    return vec3(q / l * bend, exp(-s * s) * (1.0 - life) * shot.w);
}
fn layer(input: LayerInput) -> vec4<f32> {
    let amount = input.params[0].x;
    let reach = max(input.params[0].y, 1.0);
    let width = input.params[0].z;
    var by = vec2(0.0);
    var lit = 0.0;
    var soft = 0.0;
    for (var i = 1; i < 4; i += 1) {
        let wave = fx_wave(input, input.params[i], amount, reach, width);
        by += wave.xy;
        lit += wave.z;
        if input.params[i].z < 0.0 {
            // Reduced motion: a soft brightening, no travel.
            soft += 0.2 * input.params[i].w * sin(clamp(-input.params[i].z, 0.0, 1.0) * 3.14159);
        }
    }
    // Bending to nothing at the box's edge, so no seam shows there.
    let settle = smoothstep(0.0, 24.0, -fx_box(input.p, input.size, input.radii));
    by *= settle;
    lit *= settle;
    let behind = zgx_backdrop(input.p + by);
    let lifted = vec4(behind.rgb + vec3(0.08 * lit + soft), behind.a);
    return fx_over(input.color, clamp(lifted, vec4(0.0), vec4(1.0)));
}
]==],
	fields = {
		amount = { 0, 0, "number", 18 },
		reach = { 0, 1, "number", 260 },
		width = { 0, 2, "number", 28 },
		life = { 9, 0, "number", 0.9 },
	},
	presets = {},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return math.abs(o.amount) * 1.5
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return o.life
	end,
}
function fx.shockwave(opts: Options?): Effect
	return make(shockwave_spec, opts)
end

local lens_spec = {
	name = "lens",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// A lens of glass: what's behind it bent at its rim, its colours split there,
// frosted if asked, with a highlight and a shadow. params: [bevel, depth,
// dispersion, magnification], [frost, saturation, lift, tint alpha], [tint,
// rim], [seed, intensity, reduced, shadow].
fn fx_read(p: vec2<f32>, dir: vec2<f32>, bend: f32, spread: f32) -> vec3<f32> {
    let r = zgx_backdrop(p + dir * bend * (1.0 - spread)).r;
    let g = zgx_backdrop(p + dir * bend).g;
    let b = zgx_backdrop(p + dir * bend * (1.0 + spread)).b;
    return vec3(r, g, b);
}
fn layer(input: LayerInput) -> vec4<f32> {
    let bevel = max(input.params[0].x, 0.5);
    let depth = input.params[0].y;
    let spread = input.params[0].z;
    let magnify = input.params[0].w;
    let saturation = input.params[1].y;
    let lift = input.params[1].z;
    let presence = input.params[3].y;
    let d = fx_box(input.p, input.size, input.radii);
    if d > 0.0 {
        // Its shadow past the box.
        let shadow = input.params[3].w * presence * exp(-pow(d / 12.0, 2.0));
        return fx_over(input.color, vec4(0.0, 0.0, 0.0, clamp(shadow, 0.0, 1.0)));
    }
    let centre = input.size * 0.5;
    let inside = clamp(-d / bevel, 0.0, 1.0);
    // A rounded bevel: flat over the middle, steepening toward the rim.
    let slope = pow(1.0 - inside, 3.0);
    // Inward, across the bevel, from the box's shape.
    let eps = 0.75;
    let gradient = vec2(
        fx_box(input.p + vec2(eps, 0.0), input.size, input.radii) - fx_box(input.p - vec2(eps, 0.0), input.size, input.radii),
        fx_box(input.p + vec2(0.0, eps), input.size, input.radii) - fx_box(input.p - vec2(0.0, eps), input.size, input.radii),
    );
    let dir = -gradient / max(length(gradient), 0.0001);
    let bend = slope * depth * presence;
    let at = centre + (input.p - centre) / max(mix(1.0, magnify, presence), 0.01);
    var color = fx_read(at, dir, bend, spread * slope);
    // Saturated and lifted as glass over a window is: more in the dark than
    // in the light. (What's frosted is blurred already: a backdrop blur under
    // this, `Fx::frost`.)
    color = fx_saturate(color, mix(1.0, saturation, presence));
    color = 1.0 - (1.0 - color) * (1.0 - lift * presence);
    color = mix(color, input.params[2].rgb, input.params[1].w * presence);
    // A highlight along the upper lit rim, a counter-line under it.
    let lit = dot(dir, vec2(-0.5, 0.86));
    let rim = input.params[2].a * presence * exp(-pow(-d / 1.1, 2.0));
    color += rim * (0.6 * max(lit, 0.0) + 0.25 * max(-lit, 0.0));
    let glass = vec4(clamp(color, vec3(0.0), vec3(1.0)), fx_cover(d, input.scale));
    return fx_over(input.color, glass);
}
]==],
	fields = {
		bevel = { 0, 0, "number", 14 },
		depth = { 0, 1, "number", 16 },
		dispersion = { 0, 2, "number", 0.14 },
		magnification = { 0, 3, "number", 1.04 },
		frost = { 1, 0, "number", 0 },
		saturation = { 1, 1, "number", 1 },
		lift = { 1, 2, "number", 0 },
		tint_amount = { 1, 3, "number", 0 },
		tint = { 2, 0, "color", "#ffffff" },
		rim = { 2, 3, "number", 0.7 },
		shadow = { 3, 3, "number", 0.3 },
	},
	presets = {
		clear = {},
		frosted = { frost = 16, saturation = 1.5, lift = 0.05, tint_amount = 0.12, depth = 8 },
	},
	bleed = function(o)
		return (if o.shadow > 0 then 28 else 0)
	end,
	reach = function(o)
		return math.abs(o.depth) * 1.2
	end,
	frost = function(o)
		return o.frost
	end,
	animated = function(o)
		return false
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.lens(opts: Options?): Effect
	return make(lens_spec, opts)
end

local crt_spec = {
	name = "crt",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// A cathode-ray tube's look over what's behind the box: bulged, scanned and
// vignetted. params: [curve, scanlines, mask, split], [_], [_], [seed,
// intensity, reduced, fps-less flicker].
fn layer(input: LayerInput) -> vec4<f32> {
    let curve = input.params[0].x * input.params[3].y;
    let lines = input.params[0].y * input.params[3].y;
    let mask = input.params[0].z * input.params[3].y;
    let split = input.params[0].w * input.params[3].y;
    let uv = input.p / max(input.size, vec2(1.0)) * 2.0 - 1.0;
    let r2 = dot(uv, uv);
    let bent = (uv * (1.0 + curve * (r2 - 1.0)) + 1.0) * 0.5 * input.size;
    let r = zgx_backdrop(bent + vec2(split, 0.0)).r;
    let g = zgx_backdrop(bent).g;
    let b = zgx_backdrop(bent - vec2(split, 0.0)).b;
    var color = vec3(r, g, b);
    // Scan lines, a phosphor mask, a vignette; a line per 2 points at 2x.
    let line = 0.5 + 0.5 * cos(bent.y * FX_TAU / 3.0);
    color *= 1.0 - lines * (1.0 - line) * 0.55;
    let phosphor = fract(bent.x / 3.0);
    let tint = vec3(step(phosphor, 0.33), step(0.33, phosphor) * step(phosphor, 0.66), step(0.66, phosphor));
    color *= mix(vec3(1.0), 0.72 + tint * 0.5, mask);
    color *= 1.0 - 0.35 * input.params[3].y * smoothstep(0.5, 2.0, r2);
    // Past the curved glass: dark.
    let off = any(bent < vec2(0.0)) || any(bent > input.size);
    let d = fx_box(input.p, input.size, input.radii);
    let screen = vec4(select(clamp(color, vec3(0.0), vec3(1.0)), vec3(0.02), off), fx_cover(d, input.scale));
    return fx_over(input.color, screen);
}
]==],
	fields = {
		curve = { 0, 0, "number", 0.08 },
		scanlines = { 0, 1, "number", 0.5 },
		mask = { 0, 2, "number", 0.3 },
		split = { 0, 3, "number", 0.8 },
	},
	presets = {},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return math.abs(o.split) + 2
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return false
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.crt(opts: Options?): Effect
	return make(crt_spec, opts)
end

local chroma_split_spec = {
	name = "chroma_split",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// A chromatic split: what's behind the box with its channels pulled apart.
// params: [dx, dy, radial, _], [_], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let intensity = input.params[3].y;
    let radial = input.params[0].z;
    let q = input.p - input.size * 0.5;
    let reach = max(length(input.size * 0.5), 1.0);
    let by = (input.params[0].xy + q / reach * radial) * intensity;
    let r = zgx_backdrop(input.p + by).r;
    let c = zgx_backdrop(input.p);
    let b = zgx_backdrop(input.p - by).b;
    let cover = fx_cover(fx_box(input.p, input.size, input.radii), input.scale);
    return fx_over(input.color, vec4(r, c.g, b, c.a * cover));
}
]==],
	fields = {
		dx = { 0, 0, "number", 3 },
		dy = { 0, 1, "number", 0 },
		radial = { 0, 2, "number", 0 },
	},
	presets = {},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return math.max(math.abs(o.dx), math.abs(o.dy)) + math.abs(o.radial) + 1
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return false
	end,
	idle = function(o)
		return o.dx == 0 and o.dy == 0 and o.radial == 0
	end,
	life = function(o)
		return 0
	end,
}
function fx.chroma_split(opts: Options?): Effect
	return make(chroma_split_spec, opts)
end

local fluted_glass_spec = {
	name = "fluted_glass",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fluted glass: ribs each a little cylinder lens over what's behind the box.
// params: [rib, angle, refraction, dispersion], [frost, highlight, _, _], [_],
// [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let rib = max(input.params[0].x, 1.0);
    let angle = input.params[0].y;
    let bend = input.params[0].z * input.params[3].y;
    let spread = input.params[0].w;
    let highlight = input.params[1].y * input.params[3].y;
    let across = vec2(cos(angle), sin(angle));
    let s = dot(input.p, across) / rib;
    let f = fract(s) - 0.5;
    // The strip under it comes back squeezed and flipped.
    let off = -2.0 * f * bend;
    let r = zgx_backdrop(input.p + across * off * (1.0 + spread)).r;
    let g = zgx_backdrop(input.p + across * off).g;
    let b = zgx_backdrop(input.p + across * off * (1.0 - spread)).b;
    var color = vec3(r, g, b);
    // A sheen on each rib, a shaded groove between.
    color += highlight * (0.14 * exp(-pow((f + 0.18) / 0.14, 2.0)) - 0.18 * exp(-pow((abs(f) - 0.5) / 0.06, 2.0)));
    let cover = fx_cover(fx_box(input.p, input.size, input.radii), input.scale);
    return fx_over(input.color, vec4(clamp(color, vec3(0.0), vec3(1.0)), cover));
}
]==],
	fields = {
		rib = { 0, 0, "number", 14 },
		angle = { 0, 1, "number", 0 },
		refraction = { 0, 2, "number", 9 },
		dispersion = { 0, 3, "number", 0.15 },
		frost = { 1, 0, "number", 0 },
		highlight = { 1, 1, "number", 0.5 },
	},
	presets = {
		reeded = {},
		fine = { rib = 6, refraction = 4 },
		frosted = { frost = 6 },
		wide = { rib = 30, refraction = 18 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return math.abs(o.refraction) * 1.2
	end,
	frost = function(o)
		return o.frost
	end,
	animated = function(o)
		return false
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.fluted_glass(opts: Options?): Effect
	return make(fluted_glass_spec, opts)
end

local liquid_metal_spec = {
	name = "liquid_metal",
	fill = true,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// Liquid metal: a polished surface, rippling, reflecting a studio. Rows:
// [tint, flow], [_, scale], [_, speed], [seed, intensity, reduced, bevel].
fn fill(input: FillInput) -> vec4<f32> {
    let flow = input.params[0].a;
    let scale = max(input.params[1].a, 1.0);
    let t = fx_fill_time(input) * input.params[2].a;
    let seed = fx_seed(input.params);
    let q = input.p / scale;
    var ripple = vec2(0.0);
    if flow > 0.0 {
        let warp = vec2(fx_fbm(q + vec2(t, -t * 0.6), seed), fx_fbm(q + vec2(4.1 - t * 0.7, 2.3 + t * 0.4), seed + 3u));
        let e = 0.05;
        let at = q + warp * 1.2;
        let h = fx_fbm(at + vec2(t * 0.4, 0.0), seed + 7u);
        let hx = fx_fbm(at + vec2(e + t * 0.4, 0.0), seed + 7u);
        let hy = fx_fbm(at + vec2(t * 0.4, e), seed + 7u);
        ripple = vec2(hx - h, hy - h) / e * flow * 0.22;
    }
    let n = fx_dome(input, 0.55, input.params[3].w, ripple);
    let light = fx_studio(n);
    // A conductor's reflection: the tint at normal incidence, white at grazing.
    let fresnel = pow(1.0 - n.z, 5.0);
    let tint = input.params[0].rgb;
    let body = mix(tint * 0.2, mix(tint, vec3(1.0), 0.15), clamp(light, 0.0, 1.0));
    let spec = clamp(light - 0.9, 0.0, 1.0) * 2.4;
    let color = mix(body, vec3(1.0), clamp(fresnel + spec, 0.0, 1.0));
    return vec4(color, input.params[3].y);
}
]==],
	fields = {
		tint = { 0, 0, "color", "#d2d2d2" },
		flow = { 0, 3, "number", 0.6 },
		scale = { 1, 3, "number", 100 },
		speed = { 2, 3, "number", 0.35 },
		bevel = { 3, 3, "number", 14 },
	},
	presets = {
		chrome = {},
		gold = { tint = "#f0c040" },
		rose = { tint = "#f0a090" },
		gunmetal = { tint = "#70757d", flow = 0.4 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.flow > 0
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.liquid_metal(opts: Options?): Effect
	return make(liquid_metal_spec, opts)
end

local mesh_gradient_spec = {
	name = "mesh_gradient",
	fill = true,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// A mesh gradient: three colours' points drifting, blended by distance.
// Rows: [a, drift], [b, sharpness], [c, speed], [seed, intensity, reduced, warp].
fn fill(input: FillInput) -> vec4<f32> {
    let drift = input.params[0].a;
    let sharp = max(input.params[1].a, 0.1);
    let t = fx_fill_time(input) * input.params[2].a;
    let seed = fx_seed(input.params);
    let uv = input.p / max(input.size, vec2(1.0));
    let warp = input.params[3].w;
    let w = vec2(fx_noise(uv * 2.2 + vec2(t * 0.5, 3.0), seed), fx_noise(uv * 2.2 + vec2(1.7, t * 0.5), seed + 5u)) - 0.5;
    let at = uv + w * warp;
    var sum = vec3(0.0);
    var total = 0.0;
    for (var i = 0; i < 5; i += 1) {
        let n = u32(i);
        let phase = fx_rand(n, seed) * FX_TAU;
        let home = vec2(fx_rand(n, seed + 11u), fx_rand(n, seed + 13u));
        let point = home + drift * vec2(cos(t + phase), sin(t * 0.8 + phase * 1.3));
        let d = max(length(at - point), 0.16);
        let weight = pow(1.0 / d, sharp * 2.0);
        let color = select(select(input.params[2].rgb, input.params[1].rgb, i % 3 == 1), input.params[0].rgb, i % 3 == 0);
        sum += color * weight;
        total += weight;
    }
    // Fine dither against banding on a long ramp.
    let grain = (fx_cell(vec2<i32>(input.p * input.scale), seed) - 0.5) / 255.0;
    return vec4(sum / total + vec3(grain), input.params[3].y);
}
]==],
	fields = {
		a = { 0, 0, "color", "#1b1f3b" },
		drift = { 0, 3, "number", 0.12 },
		b = { 1, 0, "color", "#ff6f91" },
		sharpness = { 1, 3, "number", 1.6 },
		c = { 2, 0, "color", "#ffc75f" },
		speed = { 2, 3, "number", 0.25 },
		warp = { 3, 3, "number", 0.35 },
	},
	presets = {
		dusk = {},
		lagoon = { a = "#042f4b", b = "#00a8a8", c = "#a3f7bf" },
		orchid = { a = "#2a0a4a", b = "#b5179e", c = "#f7aef8" },
		graphite = { a = "#14161a", b = "#3a3f47", c = "#8d939c", drift = 0.08 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.drift > 0 and o.speed ~= 0
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.mesh_gradient(opts: Options?): Effect
	return make(mesh_gradient_spec, opts)
end

local dither_spec = {
	name = "dither",
	fill = true,
	fps = 30,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// Dither: a tone quantized between two colours in square pixels by a Bayer
// or white-noise pattern. Rows: [a, pixel], [b, angle], [_, pattern], [seed,
// intensity, reduced, speed].
fn fx_bayer(c: vec2<i32>) -> f32 {
    // Recursive 8x8 Bayer, by bits.
    let x = u32(c.x) & 7u;
    let y = u32(c.y) & 7u;
    let v = x ^ y;
    let bits = ((v & 1u) << 5u) | ((y & 1u) << 4u) | ((v & 2u) << 2u) | ((y & 2u) << 1u) | ((v & 4u) >> 1u) | ((y & 4u) >> 2u);
    return (f32(bits) + 0.5) / 64.0;
}
fn fill(input: FillInput) -> vec4<f32> {
    let pixel = max(input.params[0].a, 1.0);
    let angle = input.params[1].a;
    let seed = fx_seed(input.params);
    let t = fx_fill_time(input) * input.params[3].w;
    let cell = floor(input.p / pixel);
    let centre = (cell + 0.5) * pixel;
    let across = vec2(cos(angle), sin(angle));
    let extent = abs(across.x) * input.size.x + abs(across.y) * input.size.y;
    var tone = dot(centre - input.size * 0.5, across) / max(extent, 1.0) + 0.5;
    tone += 0.25 * (fx_noise(centre / 60.0 + vec2(t, -t * 0.7), seed) - 0.5);
    let c = vec2<i32>(cell);
    let threshold = select(fx_cell(c, seed), fx_bayer(c), input.params[2].a < 0.5);
    let color = select(input.params[0].rgb, input.params[1].rgb, tone > threshold);
    return vec4(color, input.params[3].y);
}
]==],
	fields = {
		a = { 0, 0, "color", "#12121a" },
		pixel = { 0, 3, "number", 2 },
		b = { 1, 0, "color", "#e8e6df" },
		angle = { 1, 3, "number", 1.2 },
		pattern = { 2, 3, { "bayer", "noise" }, "bayer" },
		speed = { 3, 3, "number", 0 },
	},
	presets = {
		bayer = {},
		grain = { pattern = "noise", pixel = 3 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.speed ~= 0
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.dither(opts: Options?): Effect
	return make(dither_spec, opts)
end

local grain_gradient_spec = {
	name = "grain_gradient",
	fill = true,
	fps = 24,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// A grainy gradient: noise scatters the ramp, so its colours stipple into
// each other. Rows: [a, amount], [b, kind], [c, angle], [seed, intensity, reduced, fps].
fn fill(input: FillInput) -> vec4<f32> {
    let amount = input.params[0].a;
    let kind = input.params[1].a;
    let angle = input.params[2].a;
    let seed = fx_seed(input.params) + u32(floor(input.time * input.params[3].w)) * 1009u;
    let q = input.p - input.size * 0.5;
    var t = 0.0;
    if kind < 0.5 {
        let across = vec2(cos(angle), sin(angle));
        let extent = abs(across.x) * input.size.x + abs(across.y) * input.size.y;
        t = dot(q, across) / max(extent, 1.0) + 0.5;
    } else if kind < 1.5 {
        t = length(q) / max(length(input.size * 0.5), 1.0);
    } else {
        // Conic: its ramp there and back, so it loops.
        let turn = fract(atan2(q.y, q.x) / FX_TAU + angle / FX_TAU + 1.0);
        t = 1.0 - abs(turn * 2.0 - 1.0);
    }
    let pixel = vec2<i32>(input.p * input.scale);
    let tri = fx_cell(pixel, seed) + fx_cell(pixel, seed + 7u) - 1.0;
    let color = fx_ramp(input.params[0].rgb, input.params[1].rgb, input.params[2].rgb, t + tri * amount * 0.4);
    return vec4(color + vec3(tri * amount * 0.05), input.params[3].y);
}
]==],
	fields = {
		a = { 0, 0, "color", "#1a0b2e" },
		amount = { 0, 3, "number", 0.4 },
		b = { 1, 0, "color", "#e0533d" },
		shape = { 1, 3, { "linear", "radial", "conic" }, "linear" },
		c = { 2, 0, "color", "#fbd38d" },
		angle = { 2, 3, "number", 1.2 },
		fps = { 3, 3, "number", 0 },
	},
	presets = {
		ember = {},
		halo = { shape = "radial", a = "#fff1c2", b = "#7a7bff", c = "#0b0b1e" },
		sweep = { shape = "conic", a = "#0b3d91", b = "#22d3ee", c = "#fde68a" },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.fps > 0
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.grain_gradient(opts: Options?): Effect
	return make(grain_gradient_spec, opts)
end

local metaballs_spec = {
	name = "metaballs",
	fill = true,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// Metaballs: blobs merging, an antialiased edge and a glossy rim. Rows: [a,
// size], [b, speed], [back, gloss], [seed, intensity, reduced, count].
fn fill(input: FillInput) -> vec4<f32> {
    let size = input.params[0].a;
    let t = fx_fill_time(input) * input.params[1].a;
    let gloss = input.params[2].a;
    let seed = fx_seed(input.params);
    let count = i32(clamp(input.params[3].w, 1.0, 8.0));
    let unit = min(input.size.x, input.size.y);
    let aspect = input.size / unit;
    let uv = input.p / unit;
    var field = 0.0;
    var mixed = 0.0;
    for (var i = 0; i < count; i += 1) {
        let n = u32(i);
        let phase = vec2(fx_rand(n, seed + 3u), fx_rand(n, seed + 5u)) * FX_TAU;
        let rate = vec2(0.6 + fx_rand(n, seed + 7u), 0.5 + fx_rand(n, seed + 9u));
        let at = (0.5 + 0.42 * vec2(sin(t * rate.x + phase.x), cos(t * rate.y + phase.y))) * aspect;
        let radius = size * (0.7 + 0.6 * fx_rand(n, seed + 11u));
        let d = uv - at;
        let w = radius * radius / max(dot(d, d), 0.00001);
        field += w;
        mixed += w * fx_rand(n, seed + 13u);
    }
    // Its edge a pixel wide, from the slope of the field held under 1: as the
    // field is at the edge, where it crosses 0, and flat where the field
    // climbs without bound at a blob's centre (its own slope there would
    // mix the background into pixels around it).
    let held = 1.0 - 1.0 / max(field, 0.0001);
    let slope = max(length(vec2(dpdx(held), dpdy(held))), 0.0001);
    let edge = clamp(held / slope + 0.5, 0.0, 1.0);
    let body = mix(input.params[0].rgb, input.params[1].rgb, clamp(mixed / max(field, 0.001), 0.0, 1.0));
    let rim = exp(-pow((field - 1.25) / 0.25, 2.0)) * gloss;
    let color = mix(input.params[2].rgb, body + vec3(rim * 0.6), edge);
    return vec4(color, input.params[3].y);
}
]==],
	fields = {
		a = { 0, 0, "color", "#ff5d3a" },
		size = { 0, 3, "number", 0.2 },
		b = { 1, 0, "color", "#ffc24b" },
		speed = { 1, 3, "number", 0.4 },
		back = { 2, 0, "color", "#24060a" },
		gloss = { 2, 3, "number", 0.5 },
		count = { 3, 3, "number", 6 },
	},
	presets = {
		lava = {},
		mint = { a = "#38e8b0", b = "#9bf6ff", back = "#04211f" },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.speed ~= 0
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.metaballs(opts: Options?): Effect
	return make(metaballs_spec, opts)
end

local swirl_spec = {
	name = "swirl",
	fill = true,
	fps = 30,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// A swirl: a twist about the centre over warped noise, through a colour ramp.
// Rows: [a, scale], [b, speed], [c, twist], [seed, intensity, reduced, warp].
fn fill(input: FillInput) -> vec4<f32> {
    let scale = max(input.params[0].a, 1.0);
    let t = fx_fill_time(input) * input.params[1].a;
    let twist = input.params[2].a;
    let warp = input.params[3].w;
    let seed = fx_seed(input.params);
    let q = input.p - input.size * 0.5;
    let r = length(q) / max(length(input.size * 0.5), 1.0);
    let spin = twist * (1.0 - r) * 3.0 + t * 0.3;
    let c = cos(spin);
    let s = sin(spin);
    let at = vec2(c * q.x - s * q.y, s * q.x + c * q.y) / scale;
    let a = vec2(fx_fbm(at + vec2(t * 0.2, 0.0), seed), fx_fbm(at + vec2(5.2, 1.3 + t * 0.2), seed + 3u));
    let n = fx_fbm(at + a * warp * 2.0, seed + 7u);
    return vec4(fx_ramp(input.params[0].rgb, input.params[1].rgb, input.params[2].rgb, smoothstep(0.25, 0.75, n)), input.params[3].y);
}
]==],
	fields = {
		a = { 0, 0, "color", "#0b1020" },
		scale = { 0, 3, "number", 140 },
		b = { 1, 0, "color", "#3b5bdb" },
		speed = { 1, 3, "number", 0.2 },
		c = { 2, 0, "color", "#e9ecef" },
		twist = { 2, 3, "number", 0.6 },
		warp = { 3, 3, "number", 1 },
	},
	presets = {
		ink = {},
		marble = { a = "#ece7df", b = "#a89f91", c = "#3a3631", twist = 0.2, speed = 0.08 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.speed ~= 0
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.swirl(opts: Options?): Effect
	return make(swirl_spec, opts)
end

local voronoi_spec = {
	name = "voronoi",
	fill = true,
	fps = 30,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// Voronoi: orbiting cells with seams exactly between them. Rows: [a, scale],
// [b, speed], [seam, edge], [seed, intensity, reduced, depth].
fn fx_site(c: vec2<i32>, seed: u32, t: f32) -> vec2<f32> {
    let a = fx_cell(c, seed) * FX_TAU;
    let home = vec2(fx_cell(c, seed + 3u), fx_cell(c, seed + 5u));
    return vec2<f32>(c) + 0.5 + (home - 0.5) * 0.6 + 0.2 * vec2(cos(t + a), sin(t + a));
}
fn fill(input: FillInput) -> vec4<f32> {
    let scale = max(input.params[0].a, 1.0);
    let t = fx_fill_time(input) * input.params[1].a * FX_TAU;
    let edge = input.params[2].a;
    let depth = input.params[3].w;
    let seed = fx_seed(input.params);
    let q = input.p / scale;
    let c = vec2<i32>(floor(q));
    var best = 1e9;
    var site = vec2(0.0);
    var key = c;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let at = c + vec2(x, y);
            let s = fx_site(at, seed, t);
            let d = dot(q - s, q - s);
            if d < best {
                best = d;
                site = s;
                key = at;
            }
        }
    }
    // The distance to the nearest seam: along the line to each neighbour.
    var seam = 1e9;
    for (var y = -1; y <= 1; y += 1) {
        for (var x = -1; x <= 1; x += 1) {
            let at = c + vec2(x, y);
            if at.x == key.x && at.y == key.y {
                continue;
            }
            let s = fx_site(at, seed, t);
            let mid = (site + s) * 0.5;
            let dir = normalize(s - site);
            seam = min(seam, dot(mid - q, dir));
        }
    }
    let shade = 1.0 - depth * clamp(sqrt(best), 0.0, 1.0);
    let body = mix(input.params[0].rgb, input.params[1].rgb, fx_cell(key, seed + 9u)) * shade;
    let line = clamp(0.5 + (edge * 0.5 - seam * scale) * input.scale, 0.0, 1.0);
    return vec4(mix(body, input.params[2].rgb, line), input.params[3].y);
}
]==],
	fields = {
		a = { 0, 0, "color", "#18324f" },
		scale = { 0, 3, "number", 36 },
		b = { 1, 0, "color", "#2f7a9b" },
		speed = { 1, 3, "number", 0.08 },
		seam = { 2, 0, "color", "#b7e3f0" },
		edge = { 2, 3, "number", 1.5 },
		depth = { 3, 3, "number", 0.4 },
	},
	presets = {
		cells = {},
		stained = { a = "#b5322a", b = "#1f5fa8", seam = "#14110f", edge = 2.5, speed = 0 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.speed ~= 0
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.voronoi(opts: Options?): Effect
	return make(voronoi_spec, opts)
end

local waves_spec = {
	name = "waves",
	fill = true,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// Waves: layered bands with two-sine crests, each shading the one behind.
// Rows: [a, amplitude], [b, frequency], [c, speed], [seed, intensity, reduced, softness].
fn fill(input: FillInput) -> vec4<f32> {
    let amplitude = input.params[0].a;
    let frequency = input.params[1].a;
    let t = fx_fill_time(input) * input.params[2].a;
    let soft = max(input.params[3].w, 0.0005);
    let seed = fx_seed(input.params);
    let uv = input.p / max(input.size, vec2(1.0));
    var color = fx_ramp(input.params[0].rgb, input.params[1].rgb, input.params[2].rgb, 0.0);
    for (var i = 0; i < 5; i += 1) {
        let k = f32(i) / 4.0;
        let n = u32(i);
        let phase = fx_rand(n, seed) * FX_TAU;
        let line = 0.15 + 0.75 * k
            + amplitude * sin(uv.x * frequency * FX_TAU + t * (1.0 + k) + phase)
            + amplitude * 0.5 * sin(uv.x * frequency * 2.3 * FX_TAU - t * 0.7 + phase * 2.0);
        let band = fx_ramp(input.params[0].rgb, input.params[1].rgb, input.params[2].rgb, k);
        let under = smoothstep(line - soft, line + soft, uv.y);
        // Darker just under each crest.
        let shade = 1.0 - 0.2 * exp(-pow((uv.y - line) / 0.04, 2.0));
        color = mix(color, band * shade, under);
    }
    return vec4(color, input.params[3].y);
}
]==],
	fields = {
		a = { 0, 0, "color", "#8ecae6" },
		amplitude = { 0, 3, "number", 0.05 },
		b = { 1, 0, "color", "#219ebc" },
		frequency = { 1, 3, "number", 1.6 },
		c = { 2, 0, "color", "#023047" },
		speed = { 2, 3, "number", 0.4 },
		softness = { 3, 3, "number", 0.006 },
	},
	presets = {
		sea = {},
		dunes = { a = "#f6d6ad", b = "#d99a6c", c = "#7b4b35", amplitude = 0.08, frequency = 0.8, speed = 0.08 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return o.speed ~= 0
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.waves(opts: Options?): Effect
	return make(waves_spec, opts)
end

local halftone_spec = {
	name = "halftone",
	fill = true,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// Halftone: round dots on a rotated screen, each sized by a tone. Rows:
// [ink, cell], [base, angle], [_, kind], [seed, intensity, reduced, _].
fn fx_tone(input: FillInput, at: vec2<f32>, kind: f32) -> f32 {
    let q = at - input.size * 0.5;
    if kind < 0.5 {
        return length(q) / max(length(input.size * 0.5), 1.0);
    }
    return at.y / max(input.size.y, 1.0);
}
fn fill(input: FillInput) -> vec4<f32> {
    let cell = max(input.params[0].a, 2.0);
    let angle = input.params[1].a;
    let c = cos(angle);
    let s = sin(angle);
    let to = mat2x2(vec2(c, s), vec2(-s, c));
    let from_screen = mat2x2(vec2(c, -s), vec2(s, c));
    let q = from_screen * input.p / cell;
    let i = floor(q);
    var ink = 0.0;
    // The four nearest dots, so grown dots merge.
    for (var y = 0; y < 2; y += 1) {
        for (var x = 0; x < 2; x += 1) {
            let centre = i + vec2(f32(x), f32(y)) + vec2(select(-0.5, 0.5, fract(q.x) > 0.5), select(-0.5, 0.5, fract(q.y) > 0.5));
            let at = to * centre * cell;
            let tone = clamp(1.0 - fx_tone(input, at, input.params[2].a), 0.0, 1.0);
            let radius = sqrt(tone) * 0.72;
            let d = (length(q - centre) - radius) * cell;
            ink = max(ink, clamp(0.5 - d * input.scale, 0.0, 1.0));
        }
    }
    return vec4(mix(input.params[1].rgb, input.params[0].rgb, ink), input.params[3].y);
}
]==],
	fields = {
		ink = { 0, 0, "color", "#111111" },
		cell = { 0, 3, "number", 6 },
		base = { 1, 0, "color", "#f4efe6" },
		angle = { 1, 3, "number", 0.785 },
		tone = { 2, 3, { "radial", "linear" }, "radial" },
	},
	presets = {
		print = {},
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return false
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.halftone(opts: Options?): Effect
	return make(halftone_spec, opts)
end

local plastic_spec = {
	name = "plastic",
	fill = true,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Fills' shared helpers. A fill's params: up to three colours in rows 0 to 2
// (rgb, each row's `a` a number of its own), [seed, intensity, reduced, _].
// Past three colours none is needed: mixing runs through them in turn.
fn fx_ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
    let k = clamp(t, 0.0, 1.0) * 2.0;
    return select(mix(b, c, k - 1.0), mix(a, b, k), k < 1.0);
}
fn fx_fill_time(input: FillInput) -> f32 {
    return select(input.time, 3.0, input.params[3].z > 0.5);
}
// Lit by a softbox overhead and a strip low down, for polished surfaces.
fn fx_studio(n: vec3<f32>) -> f32 {
    let up = n.y * -1.0;
    let sky = smoothstep(-0.1, 0.65, up);
    let floor_light = 0.22 * smoothstep(0.1, -0.7, up);
    let horizon = smoothstep(0.02, -0.03, up);
    let strip = exp(-pow((up + 0.35) / 0.06, 2.0)) * 0.55;
    return sky * 0.95 + floor_light * horizon + strip;
}
// The normal of a domed slab with a rolled rim.
fn fx_dome(input: FillInput, dome: f32, bevel: f32, ripple: vec2<f32>) -> vec3<f32> {
    let half_size = input.size * 0.5;
    let q = (input.p - half_size) / max(half_size, vec2(1.0));
    let edge = min(half_size.x - abs(input.p.x - half_size.x), half_size.y - abs(input.p.y - half_size.y));
    let rim = 1.0 - clamp(edge / max(bevel, 0.5), 0.0, 1.0);
    let roll = rim * rim;
    let outward = normalize(vec2(q.x * half_size.x, q.y * half_size.y) + vec2(0.0001));
    let slope = q * dome + outward * sign(q) * sign(q) * roll * 1.4 + ripple * (1.0 - roll);
    return normalize(vec3(slope, 1.0));
}

// Moulded plastic under one soft light. Rows: [color, roughness], [_, dome],
// [_, bevel], [seed, intensity, reduced, angle].
fn fill(input: FillInput) -> vec4<f32> {
    let color = input.params[0].rgb;
    let roughness = clamp(input.params[0].a, 0.03, 1.0);
    let dome = input.params[1].a;
    let bevel = input.params[2].a;
    let angle = input.params[3].w;
    let n = fx_dome(input, dome, bevel, vec2(0.0));
    let light = normalize(vec3(sin(angle) * 0.6, -cos(angle) * 0.6, 0.8));
    let view = vec3(0.0, 0.0, 1.0);
    let half_way = normalize(light + view);
    let lambert = max(dot(n, light), 0.0);
    let shade = 0.48 + 0.65 * lambert;
    // A microfacet highlight: small and bright when glossy, broad when matte.
    let a2 = roughness * roughness * roughness * roughness;
    let nh = max(dot(n, half_way), 0.0);
    let denom = nh * nh * (a2 - 1.0) + 1.0;
    let spec = min(a2 / (3.14159 * denom * denom) * 0.12, 0.8);
    let fresnel = 0.04 + 0.96 * pow(1.0 - max(n.z, 0.0), 5.0);
    var c = color * min(shade, 1.0);
    if shade > 1.0 {
        c = mix(c, vec3(1.0), (shade - 1.0) * 0.6);
    }
    // They fill what room is left above the base, so it never clips to white.
    c = c + (vec3(1.0) - c) * clamp(spec + fresnel * 0.35, 0.0, 1.0);
    return vec4(c, input.params[3].y);
}
]==],
	fields = {
		color = { 0, 0, "color", "#d7263d" },
		roughness = { 0, 3, "number", 0.22 },
		dome = { 1, 3, "number", 0.35 },
		bevel = { 2, 3, "number", 6 },
		angle = { 3, 3, "number", -0.5 },
	},
	presets = {
		glossy = {},
		matte = { roughness = 0.7 },
		dished = { dome = -0.4, roughness = 0.5 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return false
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.plastic(opts: Options?): Effect
	return make(plastic_spec, opts)
end

local wave_spec = {
	name = "wave",
	fill = false,
	one_shot = true,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// A ripple through what's painted, from where it's fired: a wave travelling at
// a finite speed and dying exponentially, each pixel read along its radius
// (`amplitude sin(frequency t) exp(-decay t)`, t delayed by the distance over
// the speed). params: [amplitude, frequency, decay, speed], then a shot a row:
// [x, y, age (of its 2 second life; below 0 with motion reduced), strength].
const FX_WAVE_LIFE: f32 = 2.0;
fn fx_wave_at(input: LayerInput, shot: vec4<f32>) -> vec3<f32> {
    if shot.w <= 0.0 || shot.z < 0.0 {
        return vec3(0.0);
    }
    let q = input.p - shot.xy;
    let l = max(length(q), 0.001);
    let t = max(shot.z * FX_WAVE_LIFE - l / max(input.params[0].w, 1.0), 0.0);
    let height = sin(input.params[0].y * t) * exp(-input.params[0].z * t) * shot.w;
    return vec3(q / l * height * input.params[0].x, height);
}
fn layer(input: LayerInput) -> vec4<f32> {
    var by = vec2(0.0);
    var height = 0.0;
    var soft = 0.0;
    for (var i = 1; i < 4; i += 1) {
        let wave = fx_wave_at(input, input.params[i]);
        by += wave.xy;
        height += wave.z;
        if input.params[i].z < 0.0 {
            // Reduced motion: a soft brightening, no travel.
            soft += 0.2 * input.params[i].w * sin(clamp(-input.params[i].z, 0.0, 1.0) * 3.14159);
        }
    }
    let c = zgx_sample(input.p + by);
    return vec4(clamp(c.rgb + vec3(0.25 * height + soft) * c.a, vec3(0.0), vec3(1.0)), c.a);
}
]==],
	fields = {
		amplitude = { 0, 0, "number", 12 },
		frequency = { 0, 1, "number", 15 },
		decay = { 0, 2, "number", 8 },
		speed = { 0, 3, "number", 1200 },
	},
	presets = {},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return 0
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return true
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 2
	end,
}
function fx.wave(opts: Options?): Effect
	return make(wave_spec, opts)
end

local rim_lens_spec = {
	name = "rim_lens",
	fill = false,
	one_shot = false,
	source = [==[
// Ready-made effects' shared helpers (`zgx::fx`): shapes, integer hashes (so
// the same time and seed give the same pixels on every GPU), noise, and
// compositing straight-alpha colours. `params[3]` of every effect holds its
// seed, its intensity, whether motion is reduced, and one value of its own.
const FX_TAU: f32 = 6.2831853;

// Signed distance to a box at the origin, each corner its own radius
// (top-left, top-right, bottom-right, bottom-left, as `input.radii` has them).
fn fx_box(p: vec2<f32>, size: vec2<f32>, radii: vec4<f32>) -> f32 {
    let half_size = size * 0.5;
    let q = p - half_size;
    let r = min(select(select(radii.w, radii.z, q.x > 0.0), select(radii.x, radii.y, q.x > 0.0), q.y < 0.0), min(half_size.x, half_size.y));
    let d = abs(q) - half_size + vec2(r);
    return length(max(d, vec2(0.0))) + min(max(d.x, d.y), 0.0) - r;
}
// How much of a pixel a shape with signed distance `d` covers.
fn fx_cover(d: f32, scale: f32) -> f32 {
    return clamp(0.5 - d * scale, 0.0, 1.0);
}
fn fx_hash(v: u32) -> u32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}
fn fx_rand(a: u32, b: u32) -> f32 {
    return f32(fx_hash(a ^ fx_hash(b)) >> 8u) / 16777216.0;
}
fn fx_cell(c: vec2<i32>, seed: u32) -> f32 {
    return fx_rand(u32(c.x + 32768) * 65536u + u32(c.y + 32768), seed);
}
// Value noise, 0 to 1, smooth.
fn fx_noise(p: vec2<f32>, seed: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let c = vec2<i32>(i);
    let a = fx_cell(c, seed);
    let b = fx_cell(c + vec2(1, 0), seed);
    let d = fx_cell(c + vec2(0, 1), seed);
    let e = fx_cell(c + vec2(1, 1), seed);
    return mix(mix(a, b, u.x), mix(d, e, u.x), u.y);
}
// Three octaves of it.
fn fx_fbm(p: vec2<f32>, seed: u32) -> f32 {
    return 0.55 * fx_noise(p, seed) + 0.3 * fx_noise(p * 2.03 + vec2(13.1, 7.7), seed) + 0.15 * fx_noise(p * 4.01 + vec2(3.3, 21.9), seed);
}
// `top` over `bottom`, straight alpha both.
fn fx_over(top: vec4<f32>, bottom: vec4<f32>) -> vec4<f32> {
    let a = top.a + bottom.a * (1.0 - top.a);
    if a <= 0.0 {
        return vec4(0.0);
    }
    return vec4((top.rgb * top.a + bottom.rgb * bottom.a * (1.0 - top.a)) / a, a);
}
// Light keeps its hue as it saturates rather than clipping to white.
fn fx_knee(x: f32) -> f32 {
    return 1.0 - exp(-x);
}
fn fx_seed(params: array<vec4<f32>, 4>) -> u32 {
    return u32(params[3].x);
}
fn fx_saturate(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3(dot(c, vec3(0.2126, 0.7152, 0.0722))), c, amount);
}

// Clear glass: flat in its middle and at its very edge, what's behind it bent
// in the band between, each colour by its own amount. params: [width, strength,
// dispersion, highlight], [tint, tint alpha], [_], [seed, intensity, reduced, _].
fn layer(input: LayerInput) -> vec4<f32> {
    let width = max(input.params[0].x, 0.5);
    let strength = input.params[0].y * input.params[3].y;
    let spread = input.params[0].z;
    let d = fx_box(input.p, input.size, input.radii);
    let t = clamp(-d / width, 0.0, 1.0);
    // 0 at the boundary and in the core, 1 between.
    let bump = 4.0 * t * (1.0 - t);
    let eps = 0.75;
    let gradient = vec2(
        fx_box(input.p + vec2(eps, 0.0), input.size, input.radii) - fx_box(input.p - vec2(eps, 0.0), input.size, input.radii),
        fx_box(input.p + vec2(0.0, eps), input.size, input.radii) - fx_box(input.p - vec2(0.0, eps), input.size, input.radii),
    );
    let dir = -gradient / max(length(gradient), 0.0001);
    let bend = dir * bump * bump * strength;
    let r = zgx_backdrop(input.p + bend * (1.0 - spread));
    let g = zgx_backdrop(input.p + bend);
    let b = zgx_backdrop(input.p + bend * (1.0 + spread));
    var color = vec3(r.r, g.g, b.b);
    color = mix(color, input.params[1].rgb, input.params[1].a * input.params[3].y);
    // A faint lit edge, for the eye to find it.
    let lit = input.params[0].w * input.params[3].y * exp(-pow(d / 0.9, 2.0)) * (0.55 + 0.45 * -dir.y);
    color = clamp(color + vec3(lit), vec3(0.0), vec3(1.0));
    let glass = vec4(color, max(max(r.a, g.a), b.a) * fx_cover(d, input.scale));
    return fx_over(input.color, glass);
}
]==],
	fields = {
		width = { 0, 0, "number", 18 },
		strength = { 0, 1, "number", 14 },
		dispersion = { 0, 2, "number", 0.18 },
		highlight = { 0, 3, "number", 0.25 },
		tint = { 1, 0, "color", "#ffffff" },
		tint_amount = { 1, 3, "number", 0.04 },
	},
	presets = {
		clear = {},
		thick = { width = 30, strength = 26, dispersion = 0.26 },
	},
	bleed = function(o)
		return 0
	end,
	reach = function(o)
		return math.abs(o.strength) * (1 + math.abs(o.dispersion)) + 1
	end,
	frost = function(o)
		return 0
	end,
	animated = function(o)
		return false
	end,
	idle = function(o)
		return false
	end,
	life = function(o)
		return 0
	end,
}
function fx.rim_lens(opts: Options?): Effect
	return make(rim_lens_spec, opts)
end

return fx
