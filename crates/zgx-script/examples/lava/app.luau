--!strict
-- A calm dark list over a floor of lava: pick up a row, carry it, drop it in.
-- Everything here is public API: passes, shaders and states. Nothing below
-- runs per frame in script; the row's motion is springs, the fire is a
-- simulation in passes, and the light reads where the cards are from the engine.

local HORIZON = 563 -- where the floor meets the air, in points
local ROW, GAP, TOP = 66, 12, 140

local TASKS = {
	{ id = 1, title = "Ship the release notes", sub = "Launch · due today", tag = "Eng", tint = "#7ea6ff" },
	{ id = 2, title = "Review PR #842: layout cache", sub = "Core · 3 comments", tag = "Eng", tint = "#7ea6ff" },
	{ id = 3, title = "Reply to the legal thread", sub = "Inbox · since Tuesday", tag = "Ops", tint = "#f2c066" },
	{ id = 4, title = "Redesign the empty state", sub = "Design · in progress", tag = "Design", tint = "#c79cf2" },
	{
		id = 5,
		title = "Book flights for the offsite",
		sub = "Personal · this week",
		tag = "Personal",
		tint = "#9fe0a4",
	},
}

-- Shared noise, for every shader below.
local NOISE = [[
fn hash21(p: vec2<f32>) -> f32 {
	let q = fract(p * vec2<f32>(123.34, 456.21));
	return fract((q.x + 45.32) * (q.y + 45.32) * (q.x + q.y + 45.32) * 1.0 + dot(q, q + 45.32));
}
fn hash22(p: vec2<f32>) -> vec2<f32> {
	let a = hash21(p);
	return vec2<f32>(a, hash21(p + a + 17.1));
}
fn vnoise(p: vec2<f32>) -> f32 {
	let i = floor(p);
	let f = fract(p);
	let w = f * f * (3.0 - 2.0 * f);
	return mix(mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), w.x),
		mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), w.x), w.y);
}
fn fbm(p0: vec2<f32>) -> f32 {
	var p = p0;
	var a = 0.5;
	var s = 0.0;
	for (var i = 0; i < 5; i++) {
		s += a * vnoise(p);
		p = mat2x2<f32>(1.6, 1.2, -1.2, 1.6) * p + vec2<f32>(3.1, 1.7);
		a *= 0.5;
	}
	return s;
}
]]

-- The ground, drawn once: r how far to a plate's edge, g the plate's height,
-- b a plate's own random, a how wide a crack is here. In perspective: the
-- texture's rows are distances from the line, so everything is laid on the
-- plane and shrinks toward the horizon as it should.
local PLANE = [[
fn plane(uv: vec2<f32>, aspect: f32) -> vec2<f32> {
	let v = max(uv.y, 0.0005);
	let depth = 0.5 / (v + 0.2);
	return vec2<f32>((uv.x - 0.5) * aspect * depth * 2.0, depth * 3.0);
}
]]
local GROUND = NOISE
	.. PLANE
	.. [[
fn plates(q: vec2<f32>) -> vec3<f32> {
	let cell = floor(q);
	var first = 9.0;
	var nearest = vec2<f32>(0.0);
	var id = 0.0;
	for (var y = -1; y <= 1; y++) {
		for (var x = -1; x <= 1; x++) {
			let c = cell + vec2<f32>(f32(x), f32(y));
			let r = c + hash22(c) * 0.8 + 0.1 - q;
			let d = dot(r, r);
			if (d < first) {
				first = d;
				nearest = r;
				id = hash21(c + 5.0);
			}
		}
	}
	var edge = 9.0;
	for (var y = -2; y <= 2; y++) {
		for (var x = -2; x <= 2; x++) {
			let c = cell + vec2<f32>(f32(x), f32(y));
			let r = c + hash22(c) * 0.8 + 0.1 - q;
			let m = r - nearest;
			if (dot(m, m) > 0.0001) {
				edge = min(edge, dot(0.5 * (r + nearest), normalize(m)));
			}
		}
	}
	return vec3<f32>(edge, id, length(nearest));
}
fn run(input: PassInput) -> vec4<f32> {
	let g = plane(input.uv, input.size.x / input.size.y);
	// Cracks wander at every scale: a broad bend, then a jagged one.
	let broad = vec2<f32>(fbm(g * 1.3 + 3.0), fbm(g * 1.3 + 11.0)) - 0.5;
	let jag = vec2<f32>(fbm(g * 11.0 + 21.0), fbm(g * 11.0 + 31.0)) - 0.5;
	let q = (g + broad * 0.55 + jag * 0.05) * 2.1;
	let big = plates(q);
	// Lesser cracks across some plates.
	let small = plates(q * 2.6 + 40.0 + jag * 0.4);
	let broken = smoothstep(0.5, 0.66, fbm(g * 2.4 + 50.0));
	let edge = min(big.x, mix(9.0, small.x + 0.02, broken));
	// Domed plates, smooth, with a little relief.
	let dome = sqrt(smoothstep(0.0, 0.32, big.x)) * 0.75 + (fbm(g * 3.0 + big.y * 30.0) - 0.5) * 0.35;
	let height = dome + (fbm(g * 16.0) - 0.5) * 0.05 - (1.0 - smoothstep(0.0, 0.05, small.x)) * broken * 0.2;
	// Wider cracks where it runs hot.
	let width = 0.008 + 0.016 * smoothstep(0.55, 0.8, fbm(g * 0.9 + 70.0));
	return vec4<f32>(edge, height, big.y, width);
}
]]

-- The rock, lit, drawn once from the ground: rgb the rock, a how much of a
-- crack's glow lies here.
local ROCK = [[
fn run(input: PassInput) -> vec4<f32> {
	let e = 1.0 / input.size;
	let c = zgx_ground(input.uv);
	let dx = zgx_ground(input.uv + vec2<f32>(e.x, 0.0)).g - zgx_ground(input.uv - vec2<f32>(e.x, 0.0)).g;
	let dy = zgx_ground(input.uv + vec2<f32>(0.0, e.y)).g - zgx_ground(input.uv - vec2<f32>(0.0, e.y)).g;
	// Slopes per pixel steepen toward the line, as the plane recedes.
	let k = 2.2 + 10.0 * (1.0 - input.uv.y);
	let n = normalize(vec3<f32>(-dx * k * 14.0, 1.0, dy * k * 14.0));
	// Lit from the glow ahead and a little above: rims facing the line shine.
	let to_glow = normalize(vec3<f32>(0.0, 0.55, 1.0));
	let key = max(dot(n, to_glow), 0.0);
	let shine = pow(key, 12.0);
	let fill = max(n.y, 0.0);
	// A crack's width, never thinner than a pixel can show (then dimmer).
	let fw = fwidth(c.r);
	let w = max(c.a, fw * 1.2);
	let vein = (1.0 - smoothstep(w * 0.35, w, c.r)) * min(c.a / w, 1.0);
	let deep = smoothstep(0.0, 0.07, c.r);
	let base = mix(vec3<f32>(0.05, 0.03, 0.022), vec3<f32>(0.15, 0.092, 0.062), clamp(c.g, 0.0, 1.0));
	var rock = base * (0.25 + 0.55 * fill + 0.9 * key) * mix(0.25, 1.0, deep) * mix(0.85, 1.12, c.b);
	rock += vec3<f32>(0.62, 0.26, 0.1) * shine * 0.5 * deep;
	// The crack's glow on its banks.
	let spill = exp(-max(c.r - w, 0.0) / (w * 2.2 + 0.006));
	rock += vec3<f32>(0.5, 0.14, 0.02) * spill * 0.45;
	return vec4<f32>(rock, vein);
}
]]

-- The floor's picture: rock, cracks breathing, and fire running on the floor
-- near the line, more when it flares.
local FLOOR = NOISE
	.. PLANE
	.. [[
fn ramp(k: f32) -> vec3<f32> {
	return vec3<f32>(0.95, 0.27, 0.03) * smoothstep(0.0, 0.45, k)
		+ vec3<f32>(0.05, 0.36, 0.08) * smoothstep(0.4, 0.9, k)
		+ vec3<f32>(0.0, 0.24, 0.38) * smoothstep(0.9, 1.3, k);
}
fn layer(input: LayerInput) -> vec4<f32> {
	let uv = input.p / input.size;
	let r = zgx_rock(uv);
	let t = input.time;
	let flare = input.params[0].x;
	let g = plane(uv, input.size.x / input.size.y);
	let breath = 0.55 + 0.45 * vnoise(g * 2.0 + vec2<f32>(t * 0.2, -t * 0.12));
	// Hotter toward the line.
	let toward = exp(-uv.y * 2.2);
	var c = r.rgb * (0.95 + flare * 0.5);
	c += ramp(r.a * (0.25 + 0.35 * breath + 0.3 * toward + flare * 0.42)) * r.a * 0.85;
	// Fire lying on the floor: wisps along the rays from the line, drifting.
	let ray = (uv.x - 0.5) / (uv.y + 0.2);
	let along = g.y;
	let wisp = fbm(vec2<f32>(ray * 4.5, along * 2.6 - t * 0.35));
	let shape = fbm(vec2<f32>(ray * 9.0 + 4.0, along * 6.0 - t * 0.9));
	// Brightest out from where something went in.
	let x = uv.x * input.size.x;
	let at = exp(-abs(x - input.params[0].z) / 220.0);
	let lie = smoothstep(0.6, 0.78, wisp * 0.7 + shape * 0.45) * exp(-uv.y * 4.0) * (0.55 + flare * (0.42 + 2.2 * at));
	c += ramp(lie * 1.3) * lie * 1.1;
	// Heat haze swallows the far plates.
	c = mix(c, vec3<f32>(0.34, 0.12, 0.03), (1.0 - smoothstep(0.0, 0.1, uv.y)) * 0.7);
	// Gain: the floor's mean brightness measured as the reference's at rest.
	c *= 1.13;
	return vec4<f32>(c / (1.0 + 0.12 * dot(c, vec3<f32>(0.33))), 1.0);
}
]]

-- The fire: stable fluids in passes, after the reference's own fire
-- simulation, its constants and defaults (height 54 pt, scale 20 pt, speed
-- 1.5, smoke 0.5, warp 1, swirl 0), stepped by the frame's time. The grid is
-- in points (CELL a cell), its bottom row on the line: fluid.xy is velocity in
-- points a second, z temperature, w smoke. Each frame: curl, forces,
-- divergence, pressure by Jacobi, projection, then carried along itself.
-- params[0]: how much burns (0 to 1, the measured envelope), _, the burning
-- card's centre and half width (points). params[3]: the line, the fire's
-- height, a cell in points, the flames' height (pt).
local FIRE_COMMON = NOISE
	.. [[
fn cell() -> f32 { return zgx_pass.params[3].z; }
fn step_dt(input: PassInput) -> f32 { return clamp(input.dt, 0.0, 1.0 / 30.0); }
fn clampc(c: vec2<i32>, n: vec2<f32>) -> vec2<i32> { return clamp(c, vec2<i32>(0), vec2<i32>(n) - 1); }
fn above_of(input: PassInput) -> f32 { return (input.size.y - input.pixel.y) * cell(); }
fn sn(p: vec2<f32>) -> f32 { return vnoise(p) * 2.0 - 1.0; }
const SPEED: f32 = 1.5;
const SCALE: f32 = 34.0;
// Rise speed and cooling from the flames' height, as the reference sets them.
fn rise() -> f32 { return clamp(zgx_pass.params[3].w * 2.2, 20.0, 140.0) * sqrt(SPEED); }
// Fuel along the card's top: tongues of uneven strength with dark gaps that
// wander and pulse, a fast flicker so tongues pinch off, thinner toward the
// rounded ends and gone before the sides.
fn emission(x: f32, t: f32) -> f32 {
	let p = zgx_pass.params[0];
	let sp = max(SCALE * 0.32, 3.5);
	let slow = sn(vec2<f32>(x / sp, t * 0.55 * SPEED)) * 0.7 + sn(vec2<f32>(x / sp * 2.3 + 5.1, t * 1.1 * SPEED)) * 0.3;
	let fast = sn(vec2<f32>(x / sp * 1.7 + 11.0, t * 6.0 * SPEED));
	let r = 14.0;
	let ends = 1.0 - smoothstep(p.w - r * 1.6, p.w - r * 0.35, abs(x - p.z));
	// How much burns thins the tongues out (fewer, as the reference's flame
	// area falls at about the same height), not their heat.
	let thin = (1.0 - clamp(p.x, 0.0, 1.0)) * 0.28 - 0.05;
	return smoothstep(-0.1 + thin, 0.4 + thin, slow + 0.25 * fast) * ends * step(0.001, p.x);
}
// The band fuel burns in: just over the line.
fn fuel_band(above: f32) -> f32 {
	let h = cell();
	let depth = 2.0;
	return smoothstep(-depth - 1.5, -depth + 0.5, above) * (1.0 - smoothstep(0.0, h + 1.5, above));
}
]]
local CURL = FIRE_COMMON
	.. [[
fn at(c: vec2<i32>) -> vec4<f32> { return zgx_fluid_texel(clampc(c, zgx_fluid_size())); }
fn run(input: PassInput) -> vec4<f32> {
	let c = vec2<i32>(input.pixel);
	let w = (at(c + vec2<i32>(1, 0)).y - at(c - vec2<i32>(1, 0)).y - at(c + vec2<i32>(0, 1)).x + at(c - vec2<i32>(0, 1)).x) / (2.0 * cell());
	return vec4<f32>(w, 0.0, 0.0, 1.0);
}
]]
local FORCES = FIRE_COMMON
	.. [[
fn w_at(c: vec2<i32>) -> f32 { return zgx_curl_texel(clampc(c, zgx_curl_size())).x; }
fn run(input: PassInput) -> vec4<f32> {
	let c = vec2<i32>(input.pixel);
	let h = cell();
	let t = input.time;
	let dt = step_dt(input);
	var s = zgx_fluid_texel(c);
	var v = s.xy;
	var T = s.z;
	var sm = s.w;
	let x = input.pixel.x * h;
	let above = above_of(input);
	let rise = rise();
	let height = zgx_pass.params[3].w;
	// Buoyancy: hot gas rises, smoke is a little heavy.
	v.y -= dt * (rise * 3.0 * T - rise * 3.0 * 0.12 * sm);
	// Vorticity confinement, smooth near the base and more broken higher up.
	let g0 = vec2<f32>(abs(w_at(c + vec2<i32>(1, 0))) - abs(w_at(c - vec2<i32>(1, 0))), abs(w_at(c + vec2<i32>(0, 1))) - abs(w_at(c - vec2<i32>(0, 1)))) / (2.0 * h);
	let gl = length(g0);
	if (gl > 1e-4) {
		let g = g0 / gl;
		let lift = clamp(above / max(rise * 0.4, 8.0), 0.15, 1.0);
		v += dt * (0.25 + 0.6 * 1.0) * lift * h * vec2<f32>(g.y, -g.x) * w_at(c);
	}
	// Slow sideways sway, stronger higher up.
	let sway = sn(vec2<f32>(x / 90.0, t * 0.35 * SPEED)) + 0.5 * sn(vec2<f32>(x / 30.0 + 3.0, -above / 40.0 - t * 0.9 * SPEED));
	v.x += dt * rise * 0.3 * sway * clamp(above / 40.0, 0.0, 1.0);
	let band = fuel_band(above);
	if (band > 0.0) {
		let e = emission(x, t) * band;
		T = max(T, e * (0.95 + 0.25 * sn(vec2<f32>(x / 3.0, t * 9.0))));
		v.y = mix(v.y, -rise * (0.6 + 0.6 * e), e * 0.6);
	}
	// Cooling, and the smoke a cooled flame leaves.
	let cooling = 1.15 * rise / max(height, 4.0);
	let lost = T * (1.0 - exp(-dt * cooling));
	T -= lost;
	sm += lost * 0.5 * 0.6 * (1.0 - smoothstep(0.25, 0.55, T));
	sm *= exp(-dt * 1.4);
	// Hot gas goes up far faster than sideways: tongues stay upright.
	v.x *= exp(-dt * 7.0 * smoothstep(0.1, 0.6, T));
	// Everything leaves cleanly through the top and sides.
	let n = input.size;
	let edge = min(min(input.pixel.x, n.x - 1.0 - input.pixel.x), input.pixel.y);
	let fade = smoothstep(0.0, 4.0, edge);
	return vec4<f32>(v, T * fade, sm * fade);
}
]]
local DIVERGENCE = FIRE_COMMON
	.. [[
fn at(c: vec2<i32>) -> vec4<f32> { return zgx_forced_texel(clampc(c, zgx_forced_size())); }
fn run(input: PassInput) -> vec4<f32> {
	let c = vec2<i32>(input.pixel);
	let h = cell();
	let d = (at(c + vec2<i32>(1, 0)).x - at(c - vec2<i32>(1, 0)).x + at(c + vec2<i32>(0, 1)).y - at(c - vec2<i32>(0, 1)).y) / (2.0 * h);
	// Burning gas expands: the band is a source.
	let src = fuel_band(above_of(input)) * emission(input.pixel.x * h, input.time) * rise() * 0.2 / h;
	return vec4<f32>(d - src, 0.0, 0.0, 1.0);
}
]]
local PRESSURE = FIRE_COMMON
	.. [[
// Open above (p = 0), closed at the line and the sides.
fn pn(c: vec2<i32>, centre: f32) -> f32 {
	let n = vec2<i32>(zgx_pressure_size());
	if (c.y < 0) { return 0.0; }
	if (any(c < vec2<i32>(0)) || any(c >= n)) { return centre; }
	return zgx_pressure_texel(c).x;
}
fn run(input: PassInput) -> vec4<f32> {
	let c = vec2<i32>(input.pixel);
	let h = cell();
	let pc = zgx_pressure_texel(c).x;
	let s = pn(c - vec2<i32>(1, 0), pc) + pn(c + vec2<i32>(1, 0), pc) + pn(c - vec2<i32>(0, 1), pc) + pn(c + vec2<i32>(0, 1), pc);
	return vec4<f32>((s - h * h * zgx_divergence_texel(c).x) * 0.25, 0.0, 0.0, 1.0);
}
]]
local PROJECTED = FIRE_COMMON
	.. [[
fn pn(c: vec2<i32>, centre: f32) -> f32 {
	let n = vec2<i32>(zgx_pressure_size());
	if (c.y < 0) { return 0.0; }
	if (any(c < vec2<i32>(0)) || any(c >= n)) { return centre; }
	return zgx_pressure_texel(c).x;
}
fn run(input: PassInput) -> vec4<f32> {
	let c = vec2<i32>(input.pixel);
	let h = cell();
	let s = zgx_forced_texel(c);
	let pc = zgx_pressure_texel(c).x;
	let g = vec2<f32>(pn(c + vec2<i32>(1, 0), pc) - pn(c - vec2<i32>(1, 0), pc), pn(c + vec2<i32>(0, 1), pc) - pn(c - vec2<i32>(0, 1), pc)) / (2.0 * h);
	var v = s.xy - g;
	// A hitch never blows it up.
	let most = 2.0 * h / max(step_dt(input), 1.0 / 240.0);
	let l = length(v);
	if (l > most) { v *= most / l; }
	return vec4<f32>(v, s.zw);
}
]]
local FLUID = FIRE_COMMON
	.. [[
fn tap(c: vec2<i32>) -> vec4<f32> { return zgx_projected_texel(clampc(c, zgx_projected_size())); }
fn cr(t: f32) -> vec4<f32> {
	let t2 = t * t;
	let t3 = t2 * t;
	return vec4<f32>(-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2);
}
// Monotone cubic: far less smearing than bilinear, so tongues stay sharp as
// they rise, and no overshoot. `g` in cells.
fn cubic(g0: vec2<f32>) -> vec4<f32> {
	let g = g0 - 0.5;
	let i0 = vec2<i32>(floor(g));
	let f = g - floor(g);
	let wx = cr(f.x);
	let wy = cr(f.y);
	var acc = vec4<f32>(0.0);
	for (var y = 0; y < 4; y++) {
		var row = vec4<f32>(0.0);
		for (var x = 0; x < 4; x++) {
			row += wx[x] * tap(i0 + vec2<i32>(x - 1, y - 1));
		}
		acc += wy[y] * row;
	}
	let q0 = tap(i0);
	let q1 = tap(i0 + vec2<i32>(1, 0));
	let q2 = tap(i0 + vec2<i32>(0, 1));
	let q3 = tap(i0 + vec2<i32>(1, 1));
	return clamp(acc, min(min(q0, q1), min(q2, q3)), max(max(q0, q1), max(q2, q3)));
}
fn run(input: PassInput) -> vec4<f32> {
	let dt = step_dt(input);
	let h = cell();
	let p = input.pixel;
	let v = zgx_projected_texel(vec2<i32>(p)).xy;
	// Back along the flow by the midpoint, so curls don't spiral out.
	let vm = cubic(p - 0.5 * dt * v / h).xy;
	let r = cubic(p - dt * vm / h);
	return vec4<f32>(r.xy, max(r.z, 0.0), max(r.w, 0.0));
}
]]

-- The air's light, worked out at a quarter of its pixels and blurred: the
-- bloom round the line and the flames. Shared by the bloom and the shader.
local AIR_COMMON = NOISE
	.. [[
fn to_srgb(c: vec3<f32>) -> vec3<f32> { return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)); }
// Blackbody colour, linear, 1000 to 4000 K.
fn blackbody(k: f32) -> vec3<f32> {
	let t = k / 100.0;
	let g = clamp((99.4708 * log(t) - 161.1196) / 255.0, 0.0, 1.0);
	let b = select(clamp((138.5177 * log(max(t - 10.0, 1.0)) - 305.0448) / 255.0, 0.0, 1.0), 0.0, t <= 19.0);
	return pow(vec3<f32>(1.0, g, b), vec3<f32>(2.2));
}
// The light a flame of temperature T gives, linear: dim red tips to a pale
// core, rising steeply, with a shoulder instead of clipping.
fn flame_light(T: f32, gain: f32) -> vec3<f32> {
	let x = max(T, 0.0);
	let e = gain * x * x * x * smoothstep(0.08, 0.3, x);
	return vec3<f32>(1.0) - exp(-blackbody(1000.0 + 1700.0 * min(x, 1.3)) * e);
}
// The sim at uv: a cubic B-spline from four bilinear taps, so no grid shows.
fn sim(uv: vec2<f32>) -> vec4<f32> {
	let dim = zgx_fluid_size();
	let g = uv * dim - 0.5;
	let i = floor(g);
	let f = g - i;
	let w0 = (1.0 - f) * (1.0 - f) * (1.0 - f) / 6.0;
	let w1 = (4.0 - 6.0 * f * f + 3.0 * f * f * f) / 6.0;
	let w3 = f * f * f / 6.0;
	let w2 = 1.0 - w0 - w1 - w3;
	let s0 = w0 + w1;
	let s1 = w2 + w3;
	let o0 = (i - 0.5 + w1 / s0) / dim;
	let o1 = (i + 1.5 + w3 / s1) / dim;
	let a = zgx_fluid(vec2<f32>(o0.x, o0.y));
	let b = zgx_fluid(vec2<f32>(o1.x, o0.y));
	let c = zgx_fluid(vec2<f32>(o0.x, o1.y));
	let d = zgx_fluid(vec2<f32>(o1.x, o1.y));
	return mix(mix(a, b, s1.x), mix(c, d, s1.x), s1.y);
}
// Where in the fire's grid a point of the box is: uv, and whether it's in it.
fn fire_uv(p: vec2<f32>, params: array<vec4<f32>, 4>) -> vec3<f32> {
	let line = params[3].x;
	let fh = params[3].y;
	let size = zgx_fluid_size() * params[3].z;
	let uv = vec2<f32>(p.x / size.x, 1.0 - (line - p.y) / fh);
	let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
	return vec3<f32>(uv, select(0.0, 1.0, inside));
}
// The line's own light, linear.
fn line_light(p: vec2<f32>, t: f32, params: array<vec4<f32>, 4>) -> vec3<f32> {
	let line = params[3].x;
	let waver = (vnoise(vec2<f32>(p.x * 0.006, t * 0.18)) - 0.5) * 7.0 + (vnoise(vec2<f32>(p.x * 0.02, t * 0.9)) - 0.5) * 1.6;
	let d = p.y - (line + waver);
	let pulse = 0.85 + 0.15 * vnoise(vec2<f32>(p.x * 0.012, t * 0.5));
	// A raised lip: a hot rim on top, the swell's face below it lit and
	// falling into shadow.
	let lip = smoothstep(-1.2, -0.1, d) * (1.0 - smoothstep(1.0, 9.0, d));
	let core = exp(-abs(d + 0.4) / 0.7);
	return (vec3<f32>(1.0, 0.42, 0.1) * lip * 0.3 + vec3<f32>(1.0, 0.62, 0.26) * core * 0.45) * pulse * (1.0 + params[0].x * 0.5);
}
]]
local GLOW = AIR_COMMON
	.. [[
// What glows, in the box's quarter pixels: the flames and the line, linear.
fn run(input: PassInput) -> vec4<f32> {
	let p = input.uv * input.area.zw;
	let params = input.params;
	let f = fire_uv(p, params);
	var light = line_light(p, input.time, params);
	if (f.z > 0.5) {
		light += flame_light(sim(f.xy).z, 2.6);
	}
	return vec4<f32>(light, 1.0);
}
]]
local BLUR_X = [[
fn run(input: PassInput) -> vec4<f32> {
	let e = vec2<f32>(1.0 / input.size.x, 0.0);
	var acc = vec4<f32>(0.0);
	var total = 0.0;
	for (var i = -12; i <= 12; i++) {
		let w = exp(-f32(i * i) / 50.0);
		acc += zgx_glow(input.uv + e * f32(i) * 1.5) * w;
		total += w;
	}
	return acc / total;
}
]]
local BLUR_Y = [[
fn run(input: PassInput) -> vec4<f32> {
	let e = vec2<f32>(0.0, 1.0 / input.size.y);
	var acc = vec4<f32>(0.0);
	var total = 0.0;
	for (var i = -12; i <= 12; i++) {
		let w = exp(-f32(i * i) / 50.0);
		acc += zgx_blurx(input.uv + e * f32(i) * 1.5) * w;
		total += w;
	}
	return acc / total;
}
]]

-- The air over everything: flames, smoke, embers, the line, its bloom, and
-- warm light on the cards nearest the fire. Output is light: alpha its
-- strongest channel, which over a dark scene reads as added light without a
-- backdrop copy.
local AIR = AIR_COMMON
	.. [[
// Embers: short streaks with a hot core, rising from the line, cooling white
// to orange as they go; many near the fire, few higher up.
fn segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
	let ab = b - a;
	let k = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-4), 0.0, 1.0);
	return length(p - a - ab * k);
}
fn embers(p: vec2<f32>, t: f32, line: f32, size: f32, seed: f32, heat: f32) -> vec3<f32> {
	var out = vec3<f32>(0.0);
	let lane = floor(p.x / size);
	for (var dx = -1; dx <= 1; dx++) {
		let l = lane + f32(dx);
		for (var k = 0; k < 3; k++) {
			let id = vec2<f32>(l, f32(k) + seed);
			let h = hash22(id);
			let life = 2.6 + h.y * 2.4;
			let start = h.x * life;
			let phase = fract((t + start) / life);
			let born = floor((t + start) / life);
			let r = hash22(id + born * 7.31);
			if (r.x > 0.4 * heat + 0.08) { continue; }
			let rise = 40.0 + 90.0 * r.y;
			let y = line - 6.0 - phase * life * rise;
			let drift = (vnoise(vec2<f32>(l * 0.7 + born, phase * 3.0 + t * 0.3)) - 0.5) * 80.0 * phase;
			let x = (l + 0.5 + (r.y - 0.5) * 0.8) * size + drift;
			let vel = vec2<f32>((vnoise(vec2<f32>(l * 0.7 + born, phase * 3.0 + t * 0.3 + 0.05)) - 0.5) * 80.0 * 1.2, -rise);
			let head = vec2<f32>(x, y);
			let tail = head - vel * 0.018;
			let d = segment(p, tail, head);
			let T = (1.0 - phase) * (1.0 - phase) * (0.6 + 0.4 * r.y);
			let w = 0.9 + 1.3 * r.x * r.x;
			let core = exp(-d * d / (w * w));
			let halo = exp(-d * d / (9.0 * w * w)) * 0.12;
			out += flame_light(0.5 + T * 0.75, 6.0) * (core * 1.4 + halo) * T * smoothstep(0.0, 0.08, phase);
		}
	}
	return out;
}
fn layer(input: LayerInput) -> vec4<f32> {
	let params = input.params;
	let line = params[3].x;
	let flare = params[0].x;
	let base = params[0].y;
	let t = input.time;
	let p = input.p;
	var light = vec3<f32>(0.0);
	var smoke_a = 0.0;
	let above = line - p.y;
	// Bloom: the line's and the flames', blurred.
	light += zgx_bloom(p / input.size).rgb * 0.22;
	// Heat over the line, a low haze.
	if (above > 0.0) {
		let haze = exp(-above / 30.0) * 0.045 + exp(-above / 140.0) * 0.012;
		light += vec3<f32>(0.9, 0.3, 0.06) * haze * (1.0 + flare * 4.5 + base * 1.5);
	}
	light += line_light(p, t, params);
	// The fire as the reference draws it: flame light from temperature (gain
	// 6), and smoke that covers what's behind it, lit from below by the flame
	// (occlusion 0.25).
	let f = fire_uv(p, params);
	if (f.z > 0.5) {
		let s = sim(f.xy);
		let ext = zgx_fluid_size() * params[3].z;
		let m = min(min(p.x, ext.x - p.x), ext.y - above);
		let win = smoothstep(0.0, 18.0, m);
		light += flame_light(s.z, 6.0) * win;
		let sm = s.w * win;
		if (sm > 0.002) {
			smoke_a = (1.0 - exp(-sm * 1.2)) * 0.25 * 1.6;
			var lit = 0.0;
			for (var k = 1; k < 5; k++) {
				let tk = sim(f.xy + vec2<f32>(0.0, f32(k) * 7.0 / params[3].y)).z;
				lit += tk * tk / f32(k);
			}
			light += smoke_a * (vec3<f32>(0.11, 0.11, 0.115) + vec3<f32>(1.0, 0.45, 0.18) * min(lit, 1.0) * 0.3);
		}
		smoke_a = min(smoke_a + smoothstep(0.12, 0.35, s.z) * win * 0.25 * 0.5, 1.0);
	}
	// Warm light on the cards the fire can see: more the nearer the line, and
	// on their lower edges. The engine says where they are, each frame.
	let window = zgx_window(p);
	if (above > 0.0) {
		for (var i = 0u; i < zgx_rect_count(); i++) {
			let box = zgx_rect(i);
			let sd = zgx_rect_distance(window, i);
			if (sd < 0.5) {
				let gap = max(line - (box.y + box.w), 0.0);
				let near = exp(-gap / 110.0);
				let strength = near * (0.12 + flare * 0.55 + base * 0.6);
				let lower = smoothstep(box.y, box.y + box.w, window.y);
				let tint = vec3<f32>(0.62, 0.2, 0.05) * strength * (0.03 + 0.22 * lower * lower * lower);
				let rim = smoothstep(-1.6, -0.2, sd) * lower * lower;
				light += (tint + vec3<f32>(1.0, 0.45, 0.14) * rim * strength * 0.35) * smoothstep(0.5, -0.5, sd);
			}
		}
	}
	// Embers, thick near the fire, thin higher up.
	let heat = 0.6 + flare * 0.7 + base * 0.4;
	light += embers(p, t, line, 46.0, 0.0, heat) + embers(p + vec2<f32>(23.0, 0.0), t * 0.8, line, 70.0, 9.0, heat * 0.7);
	let srgb = to_srgb(light);
	let a = clamp(max(max(srgb.r, srgb.g), srgb.b), 0.0, 1.0);
	let total = 1.0 - (1.0 - a) * (1.0 - smoke_a);
	return vec4<f32>(srgb / max(total, 0.001), total);
}
]]

-- A row going under: cut at the line, its edge in glowing cracks.
local BURN = NOISE
	.. [[
// params[0]: the line, how far it has burnt (0 to 1).
fn layer(input: LayerInput) -> vec4<f32> {
	let line = input.params[0].x;
	let burn = input.params[0].y;
	let t = input.time;
	let size = input.size;
	let top = zgx_window(vec2<f32>(0.0)).y;
	let bottom = zgx_window(vec2<f32>(0.0, size.y)).y;
	// How near the lava it is: its light, and the air shimmering at its edge.
	let near = exp(-max(line - bottom, 0.0) / 70.0);
	let low = smoothstep(size.y * 0.35, size.y, input.p.y);
	let shimmer = (vec2<f32>(vnoise(vec2<f32>(input.p.x * 0.09, t * 5.0)), vnoise(vec2<f32>(input.p.x * 0.07 + 3.0, t * 4.0))) - 0.5)
		* vec2<f32>(1.2, 3.0) * near * low * low;
	var c = zgx_sample(input.p + shimmer);
	let w = zgx_window(input.p);
	let waver = (vnoise(vec2<f32>(w.x * 0.006, t * 0.18)) - 0.5) * 7.0 + (vnoise(vec2<f32>(w.x * 0.02, t * 0.9)) - 0.5) * 1.6;
	let below = w.y - (line + waver);
	if (below > -0.5) { return vec4<f32>(0.0); }
	// Lit from below by the lava: its lower half warms, its bottom rim glows.
	let lit = near * (0.25 + 0.75 * low * low);
	var rgb = c.rgb + vec3<f32>(0.55, 0.2, 0.06) * lit * 0.55;
	let edge = smoothstep(size.y - 3.0, size.y - 0.5, input.p.y);
	rgb += vec3<f32>(1.0, 0.5, 0.18) * edge * near * 0.6;
	// Going in: a crust forms from the line up, cracked and glowing, then
	// crumbles.
	let crust = smoothstep(0.0, 0.35, burn);
	let reach = 6.0 + 26.0 * crust;
	let band = smoothstep(-reach, -reach * 0.4, below) * crust;
	let q = w * 0.11;
	let vein = 1.0 - smoothstep(0.0, 0.06, abs(fbm(q + 4.0) - 0.5));
	let vein2 = 1.0 - smoothstep(0.0, 0.05, abs(fbm(q * 2.3 + 9.0) - 0.5));
	let rock = vec3<f32>(0.09, 0.05, 0.035);
	let cracks = vec3<f32>(1.0, 0.55, 0.18) * max(vein, vein2 * 0.6) * (0.7 + 0.6 * smoothstep(-reach, 0.0, below));
	rgb = mix(rgb, rock + cracks, band);
	// Its outline burns where the crust is.
	let outline = max(smoothstep(2.5, 0.0, input.p.x), max(smoothstep(size.x - 2.5, size.x, input.p.x), smoothstep(2.5, 0.0, input.p.y)));
	rgb += vec3<f32>(1.0, 0.6, 0.24) * outline * band * 0.9;
	// The rest of it goes translucent as it sinks, and all of it crumbles.
	var a = c.a * mix(1.0, 0.55, crust * (1.0 - band));
	let gone = smoothstep(0.55, 1.0, burn);
	a *= 1.0 - smoothstep(0.3, 0.7, fbm(q * 0.7 + 2.0) - 0.6 + gone * 1.2);
	return vec4<f32>(rgb, max(a, band * c.a * (1.0 - gone)));
}
]]
local burnShader = ui.layer_shader(BURN)

-- Measured on the reference video (scripts in the PR body): the dropped card
-- is let go 53 pt above the line and falls from rest at g = 1869 pt/s^2 (fit
-- rms 0.13 pt over 15 frames), meets the surface, approaches a float with its
-- top 26.4 pt above the line (tau 155 ms, rms 0.66 pt), sinks at 30 pt/s from
-- 0.63 s after meeting the surface to 15.5 pt above the line, and is gone 1.18 s
-- after meeting it.
local SINK = { g = 1869, float = 26.4, tau = 0.155, sinkAt = 0.63, sinkRate = 30, rest = 15.5, gone = 1.18 }
-- A measured curve, (seconds, value) pairs from when the card meets the
-- surface, as an ease over its span: `linear(…)`, sampled every 0.1 s.
local function curve(points: { { number } }): (number, string)
	local span = points[#points][1]
	local stops = {}
	for i = 0, math.floor(span * 10 + 0.5) do
		local t = i / 10
		local v = points[#points][2]
		for k = 2, #points do
			if t <= points[k][1] then
				local a, b = points[k - 1], points[k]
				v = a[2] + (b[2] - a[2]) * (t - a[1]) / math.max(b[1] - a[1], 1e-6)
				break
			end
		end
		table.insert(stops, string.format("%.3f", v))
	end
	return span, "linear(" .. table.concat(stops, ", ") .. ")"
end
-- How much burns: the reference's flame area, normalised, over time from the
-- card meeting the surface (0.33 s to first flame, peak at 1.06 s, out by 4.06 s).
local fireSpan, fireEase = curve {
	{ 0, 0 },
	{ 0.33, 0 },
	{ 0.46, 0.21 },
	{ 0.66, 0.56 },
	{ 0.86, 0.9 },
	{ 1.06, 1 },
	{ 1.26, 0.92 },
	{ 1.46, 0.87 },
	{ 1.66, 0.84 },
	{ 1.86, 0.65 },
	{ 2.06, 0.52 },
	{ 2.26, 0.51 },
	{ 2.46, 0.54 },
	{ 2.66, 0.52 },
	{ 2.86, 0.49 },
	{ 3.06, 0.48 },
	{ 3.26, 0.42 },
	{ 3.46, 0.31 },
	{ 3.66, 0.2 },
	{ 3.86, 0.07 },
	{ 4.06, 0 },
}
local FIRE = { span = fireSpan, ease = fireEase }
-- The floor's brightening: its mean below the line, normalised (35 to 55.7 of
-- 255 at the peak), over the same time.
local floorSpan, floorEase = curve {
	{ 0, 0 },
	{ 0.26, 0 },
	{ 0.46, 0.18 },
	{ 0.66, 0.36 },
	{ 0.86, 0.62 },
	{ 1.06, 0.89 },
	{ 1.26, 1 },
	{ 1.46, 0.93 },
	{ 1.66, 0.79 },
	{ 1.86, 0.7 },
	{ 2.06, 0.63 },
	{ 2.26, 0.54 },
	{ 2.46, 0.49 },
	{ 2.66, 0.44 },
	{ 2.86, 0.4 },
	{ 3.06, 0.36 },
	{ 3.26, 0.35 },
	{ 3.46, 0.32 },
	{ 3.66, 0.29 },
	{ 3.86, 0.27 },
	{ 4.06, 0.25 },
	{ 4.46, 0.2 },
	{ 6.5, 0 },
}
local FLOOR_FLARE = { span = floorSpan, ease = floorEase }

-- Row placement, as rows of a list: the engine springs them.
local function rowY(slot: number): number
	return TOP + (slot - 1) * (ROW + GAP)
end

-- The list's order and the row in hand.
local order = ui.state("order", { 1, 2, 3, 4, 5 })
local held = ui.state("held", 0)
local burning = ui.state("burning", 0)
local count = ui.state("count", 5)
local fireEl: Element? = nil
local floorEl: Element? = nil

local function slotOf(id: number): number
	for slot, other in order:peek() do
		if other == id then
			return slot
		end
	end
	return 0
end

local function Row(task: any, ys: { [number]: any }, zs: any)
	local y = ys[task.id]
	local lift = ui.state("lift" .. task.id, 1)
	local gone = ui.state("gone" .. task.id, false)
	local element: Element? = nil
	local grab = 0
	local function place()
		for slot, id in order:peek() do
			if id ~= held:peek() then
				ys[id]:set(rowY(slot))
			end
		end
	end
	return ui.div {
		id = "task" .. task.id,
		hidden = gone,
		absolute = true,
		left = 64,
		top = 0,
		w = 772,
		h = ROW,
		y = ui.spring(y, { stiffness = 260, damping = 26 }),
		z = function()
			return if held:get() == task.id or burning:get() == task.id then 1 else 0
		end,
		-- The card itself, inside the slot the list springs: what it does on
		-- its own (lifting, sinking) moves it within that.
		ui.div {
			w = "100%",
			h = "100%",
			scale = ui.spring(lift, { stiffness = 300, damping = 24 }),
			geometry = "cards",
			rounded = 14,
			bg = "#17151aeb",
			border = 1,
			border_color = "#ffffff12",
			shadow = function()
				return if held:get() == task.id then "0 18 42 #000000b0" else "0 3 10 #00000060"
			end,
			layer_shader = burnShader,
			layer_clock = true,
			layer_bleed = 4,
			layer_on = function()
				return held:get() == task.id or burning:get() == task.id
			end,
			layer_params = { { HORIZON, 0, 0, 0 } },
			on_mount = function(el: Element)
				element = el
			end,
			cursor = "grab",
			on_press = function(e: any)
				grab = e.y - y:peek()
				held:set(task.id)
				lift:set(1.025)
			end,
			on_pointer_move = function(e: any)
				if held:peek() ~= task.id then
					return
				end
				local at = e.y - grab
				y:set(at)
				-- Others make room: the row sits at the slot it's over.
				local list = table.clone(order:peek())
				local from = slotOf(task.id)
				local to = math.clamp(math.round((at - TOP) / (ROW + GAP)) + 1, 1, #list)
				if to ~= from then
					table.remove(list, from)
					table.insert(list, to, task.id)
					order:set(list)
					place()
				end
			end,
			on_release = function(e: any)
				if held:peek() ~= task.id then
					return
				end
				-- Let go near the line, it falls in on its own (the reference lets go
				-- 53 pt above it, and the card falls in).
				local dropped = y:peek() + ROW > HORIZON - 60
				held:set(0)
				lift:set(1)
				if not dropped then
					y:set(rowY(slotOf(task.id)))
					return
				end
				-- The list closes at once. The row falls on its own from where it was
				-- let go, meets the surface, floats, sinks a little, and burns away;
				-- fire burns along its width meanwhile. Each curve is measured from
				-- the reference video (see SINK, FIRE and FLOOR_FLARE above).
				burning:set(task.id)
				local list = table.clone(order:peek())
				table.remove(list, slotOf(task.id))
				order:set(list)
				count:set(#list)
				place()
				local top0 = y:peek()
				local fall = math.sqrt(2 * math.max(HORIZON - ROW - top0, 0) / SINK.g)
				local function at(t: number): number
					if t < fall then
						return top0 + 0.5 * SINK.g * t * t
					end
					local s = t - fall
					local float = HORIZON - SINK.float
					local entry = HORIZON - ROW
					local yy = float - (float - entry) * math.exp(-s / SINK.tau)
					if s > SINK.sinkAt then
						yy = math.min(float + SINK.sinkRate * (s - SINK.sinkAt), HORIZON - SINK.rest)
					end
					return yy
				end
				local total = fall + SINK.gone
				local last = at(total)
				local stops = {}
				for i = 0, 60 do
					local v = (at(total * i / 60) - top0) / math.max(last - top0, 1e-3)
					table.insert(stops, string.format("%.4f", v))
				end
				if element then
					anime.animate(element, {
						y = { from = 0, to = last - top0 },
						duration = total * 1000,
						ease = "linear(" .. table.concat(stops, ", ") .. ")",
					})
					anime.animate(
						element,
						{ layerParam1 = 1, delay = fall * 1000, duration = SINK.gone * 1000, ease = "linear" }
					)
				end
				if fireEl then
					anime.animate(fireEl, { layerParam2 = 450, layerParam3 = 386, duration = 1 })
					anime.animate(fireEl, {
						layerParam0 = { from = 0, to = 1 },
						delay = fall * 1000,
						duration = FIRE.span * 1000,
						ease = FIRE.ease,
					})
					-- (A finished animation lands on its `to`: this one ends at nothing.)
					anime.animate(fireEl, { layerParam0 = 0, delay = (fall + FIRE.span) * 1000, duration = 1 })
				end
				if floorEl then
					anime.animate(floorEl, { layerParam2 = 450, duration = 1 })
					anime.animate(floorEl, {
						layerParam0 = { from = 0, to = 1 },
						delay = fall * 1000,
						duration = FLOOR_FLARE.span * 1000,
						ease = FLOOR_FLARE.ease,
					})
					anime.animate(floorEl, { layerParam0 = 0, delay = (fall + FLOOR_FLARE.span) * 1000, duration = 1 })
				end
				ui.after(math.floor(total * 1000), function()
					gone:set(true)
					burning:set(0)
				end)
			end,
			ui.row {
				w = "100%",
				h = "100%",
				items = "center",
				px = 18,
				gap = 14,
				ui.div { w = 22, h = 22, rounded = 11, border = 1.5, border_color = "#ffffff40" },
				ui.column {
					grow = 1,
					gap = 3,
					ui.text { task.title, text_size = 15, weight = 700, color = "#f6f4f2" },
					ui.text { task.sub, text_size = 12.5, color = "#8e8a90" },
				},
				ui.div {
					px = 10,
					py = 4,
					rounded = 12,
					bg = task.tint .. "22",
					ui.text { task.tag, text_size = 12, weight = 600, color = task.tint },
				},
				ui.column {
					gap = 3,
					ui.row {
						gap = 3,
						ui.div { w = 3, h = 3, rounded = 2, bg = "#ffffff33" },
						ui.div { w = 3, h = 3, rounded = 2, bg = "#ffffff33" },
					},
					ui.row {
						gap = 3,
						ui.div { w = 3, h = 3, rounded = 2, bg = "#ffffff33" },
						ui.div { w = 3, h = 3, rounded = 2, bg = "#ffffff33" },
					},
					ui.row {
						gap = 3,
						ui.div { w = 3, h = 3, rounded = 2, bg = "#ffffff33" },
						ui.div { w = 3, h = 3, rounded = 2, bg = "#ffffff33" },
					},
				},
			},
		},
	}
end

local floorShader = ui.layer_shader(FLOOR, {
	passes = {
		{ name = "ground", shader = GROUND, scale = 1, format = "rgba16float", once = true },
		{ name = "rock", shader = ROCK, scale = 1, format = "rgba16float", once = true },
	},
})
local FIRE_H = 300
-- The flames' height: fitted so their tops (95th percentile) measure as the
-- reference's do, about 38 pt over the line at the plateau.
local FLAME_H = 70
-- A cell: the reference sizes it sqrt(area / 16000), within 1 to 3 pt.
local CELL = 3
local GRID = { 300, 100 }
local airShader = ui.layer_shader(AIR, {
	geometry = "cards",
	passes = {
		{ name = "curl", shader = CURL, size = GRID, format = "rgba16float" },
		{ name = "forced", shader = FORCES, size = GRID, format = "rgba16float" },
		{ name = "divergence", shader = DIVERGENCE, size = GRID, format = "rgba16float" },
		{ name = "pressure", shader = PRESSURE, size = GRID, format = "rgba16float", feedback = true, steps = 14 },
		{ name = "projected", shader = PROJECTED, size = GRID, format = "rgba16float" },
		{ name = "fluid", shader = FLUID, size = GRID, format = "rgba16float", feedback = true },
		{ name = "glow", shader = GLOW, scale = 0.25, format = "rgba16float" },
		{ name = "blurx", shader = BLUR_X, scale = 0.25, format = "rgba16float" },
		{ name = "bloom", shader = BLUR_Y, scale = 0.25, format = "rgba16float" },
	},
})

-- Heat rising off the floor: the backdrop shifted by rising gradient noise,
-- clear just over the line, strongest a little up where hot air has risen,
-- gone by its height. params[0]: height, noise scale (pt), speed (scales a
-- second), amount (pt).
local HAZE = [[
fn pcg(v: u32) -> u32 {
	let s = v * 747796405u + 2891336453u;
	let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
	return (w >> 22u) ^ w;
}
fn grad(c: vec2<i32>, seed: u32) -> vec2<f32> {
	let a = f32(pcg(bitcast<u32>(c.x) ^ pcg(bitcast<u32>(c.y) ^ pcg(seed))) >> 8u) * (6.2831853 / 16777216.0);
	return vec2<f32>(cos(a), sin(a));
}
fn gnoise(p: vec2<f32>, seed: u32) -> f32 {
	let i = vec2<i32>(floor(p));
	let f = p - floor(p);
	let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
	let a = dot(grad(i, seed), f);
	let b = dot(grad(i + vec2<i32>(1, 0), seed), f - vec2<f32>(1.0, 0.0));
	let c = dot(grad(i + vec2<i32>(0, 1), seed), f - vec2<f32>(0.0, 1.0));
	let d = dot(grad(i + vec2<i32>(1, 1), seed), f - vec2<f32>(1.0, 1.0));
	return mix(mix(a, b, u.x), mix(c, d, u.x), u.y) * 1.41421;
}
fn fbm3(p0: vec2<f32>, seed: u32) -> f32 {
	var p = p0;
	var a = 0.5;
	var s = 0.0;
	for (var o = 0u; o < 3u; o++) {
		s += a * gnoise(p, seed + o * 131u);
		p = mat2x2<f32>(0.8, 0.6, -0.6, 0.8) * p * 2.02;
		a *= 0.5;
	}
	return s / 0.875;
}
fn layer(input: LayerInput) -> vec4<f32> {
	let k = input.params[0];
	let h = k.x;
	let p = input.p;
	let above = input.size.y - p.y;
	let m = smoothstep(1.0, 1.0 + h * 0.3, above) * (1.0 - smoothstep(h * 0.45, h, above));
	let q = p / max(k.y, 1.0) + vec2<f32>(0.0, input.time * k.z);
	let o = vec2<f32>(fbm3(q, 7u), fbm3(q * vec2<f32>(1.0, 0.7) + vec2<f32>(7.3, 2.1), 12u)) * k.w;
	return zgx_backdrop(p + o * m);
}
]]
local hazeShader = ui.layer_shader(HAZE)
-- The reference's heat haze shape and noise (scale 14 pt); height, speed and
-- amount fitted so this scene's rows near the line measure as the
-- reference's do (optical flow against the median frame, by distance from the
-- line; the haze rises at about 90 pt/s).
local HAZE_H, HAZE_SCALE, HAZE_SPEED, HAZE_AMOUNT = 140, 14, 7.2, 1.15

local ys: { [number]: any } = {}
for i, task in TASKS do
	ys[task.id] = ui.state("y" .. task.id, rowY(i))
end

local rows = {}
for _, task in TASKS do
	table.insert(rows, Row(task, ys, nil))
end

local list: any = { absolute = true, left = 0, top = 0, w = "100%", h = "100%" }
for _, row in rows do
	table.insert(list, row)
end

return ui.div {
	w = "100%",
	h = "100%",
	bg = "#0b0a0d",
	-- The sky: nearly black, warmer toward the line.
	ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = HORIZON,
		bottom = 0,
		pointer_events = "none",
		layer_shader = floorShader,
		layer_clock = true,
		layer_params = { { 0, 0, 0, 0 } },
		on_mount = function(el: Element)
			floorEl = el
		end,
	},
	ui.div {
		absolute = true,
		left = 64,
		top = 56,
		ui.text { "Today", text_size = 28, weight = 700, color = "#f6f4f2" },
		ui.text {
			function()
				return count:get() .. " tasks · the floor is lava"
			end,
			size = 13,
			color = "#8e8a90",
			mt = 4,
		},
	},
	ui.text { "Friday, Nov 6", absolute = true, right = 64, top = 64, text_size = 12.5, color = "#6d6a70" },
	ui.div(list),
	ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = HORIZON - HAZE_H,
		h = HAZE_H,
		pointer_events = "none",
		layer_shader = hazeShader,
		layer_clock = true,
		layer_reach = 8,
		layer_params = { { HAZE_H, HAZE_SCALE, HAZE_SPEED, HAZE_AMOUNT } },
	},
	ui.div {
		absolute = true,
		left = 0,
		top = 0,
		w = "100%",
		h = "100%",
		pointer_events = "none",
		layer_shader = airShader,
		layer_clock = true,
		layer_params = { { 0, 0, 450, 386 }, { 0, 0, 0, 0 }, { 0, 0, 0, 0 }, { HORIZON, FIRE_H, CELL, FLAME_H } },
		on_mount = function(el: Element)
			fireEl = el
		end,
	},
}
