-- Every effect of zgx-fx, each over stripes to bend and split, and a lava melt
-- written here with zgx's public props alone. Press a one-shot's box to fire it.
-- Run: ZGX_SCRIPT_DIR=crates/zgx-fx/examples/gallery cargo run -p zgx-script --example hot_reload
local fx = require("../../luau/fx")

-- Lava: what's painted slumps down in hot drips and comes back, looping. Made of
-- `layer_shader`, `layer_bleed`, `layer_clock` and `zgx_sample`, and nothing else:
-- that an effect takes no more is what this is here to show.
local melt = ui.layer_shader([[
fn hash(x: f32) -> f32 {
    return fract(sin(x * 91.17) * 43758.5453);
}
fn drip(x: f32) -> f32 {
    let i = floor(x);
    let f = x - i;
    let u = f * f * (3.0 - 2.0 * f);
    return mix(hash(i), hash(i + 1.0), u);
}
fn layer(input: LayerInput) -> vec4<f32> {
    // 0 solid to 1 melted and back, over four seconds.
    let phase = fract(input.time / 4.0);
    let melted = smoothstep(0.1, 0.6, 1.0 - abs(phase * 2.0 - 1.0));
    let x = input.p.x / 14.0;
    let long = drip(x) * 0.7 + drip(x * 2.7 + 3.0) * 0.3;
    let fall = melted * long * input.params[0].x;
    let c = zgx_sample(vec2(input.p.x, input.p.y - fall * (0.3 + input.p.y / input.size.y)));
    // Its melted part glows: white hot to orange to dark crust.
    let heat = clamp(fall / input.params[0].x * 1.6, 0.0, 1.0) * c.a;
    let lava = mix(vec3(0.9, 0.2, 0.05), vec3(1.0, 0.85, 0.4), heat);
    return vec4(mix(c.rgb, lava, melted * smoothstep(0.1, 0.5, heat)), c.a);
}]])
local effects = {
	{ name = "glow soft", effect = fx.glow { preset = "soft" } },
	{ name = "glow neon", effect = fx.glow { preset = "neon" } },
	{ name = "glow aura", effect = fx.glow { preset = "aura" } },
	{ name = "pulse radar", effect = fx.pulse { preset = "radar" } },
	{ name = "pulse breathe", effect = fx.pulse { preset = "breathe" } },
	{ name = "pulse heartbeat", effect = fx.pulse { preset = "heartbeat" } },
	{ name = "shimmer sheen", effect = fx.shimmer { preset = "sheen" } },
	{ name = "shimmer skeleton", effect = fx.shimmer { preset = "skeleton" } },
	{ name = "sparkles", effect = fx.sparkles() },
	{ name = "aurora", effect = fx.aurora() },
	{ name = "grain", effect = fx.grain() },
	{ name = "electric storm", effect = fx.electric { preset = "storm" } },
	{ name = "electric charged", effect = fx.electric { preset = "charged" } },
	{ name = "fire flames", effect = fx.fire { preset = "flames" } },
	{ name = "fire plasma", effect = fx.fire { preset = "plasma" } },
	{ name = "motion_blur", effect = fx.motion_blur { dx = 30, dy = 8 } },
	{ name = "heat_haze", effect = fx.heat_haze() },
	{ name = "lens clear", effect = fx.lens { preset = "clear" } },
	{ name = "lens frosted", effect = fx.lens { preset = "frosted" } },
	{ name = "crt", effect = fx.crt() },
	{ name = "chroma_split", effect = fx.chroma_split() },
	{ name = "fluted_glass reeded", effect = fx.fluted_glass { preset = "reeded" } },
	{ name = "fluted_glass fine", effect = fx.fluted_glass { preset = "fine" } },
	{ name = "fluted_glass frosted", effect = fx.fluted_glass { preset = "frosted" } },
	{ name = "fluted_glass wide", effect = fx.fluted_glass { preset = "wide" } },
	{ name = "liquid_metal chrome", effect = fx.liquid_metal { preset = "chrome" } },
	{ name = "liquid_metal gold", effect = fx.liquid_metal { preset = "gold" } },
	{ name = "liquid_metal rose", effect = fx.liquid_metal { preset = "rose" } },
	{ name = "liquid_metal gunmetal", effect = fx.liquid_metal { preset = "gunmetal" } },
	{ name = "mesh_gradient dusk", effect = fx.mesh_gradient { preset = "dusk" } },
	{ name = "mesh_gradient lagoon", effect = fx.mesh_gradient { preset = "lagoon" } },
	{ name = "mesh_gradient orchid", effect = fx.mesh_gradient { preset = "orchid" } },
	{ name = "mesh_gradient graphite", effect = fx.mesh_gradient { preset = "graphite" } },
	{ name = "dither bayer", effect = fx.dither { preset = "bayer" } },
	{ name = "dither grain", effect = fx.dither { preset = "grain" } },
	{ name = "grain_gradient ember", effect = fx.grain_gradient { preset = "ember" } },
	{ name = "grain_gradient halo", effect = fx.grain_gradient { preset = "halo" } },
	{ name = "grain_gradient sweep", effect = fx.grain_gradient { preset = "sweep" } },
	{ name = "metaballs lava", effect = fx.metaballs { preset = "lava" } },
	{ name = "metaballs mint", effect = fx.metaballs { preset = "mint" } },
	{ name = "swirl ink", effect = fx.swirl { preset = "ink" } },
	{ name = "swirl marble", effect = fx.swirl { preset = "marble" } },
	{ name = "voronoi cells", effect = fx.voronoi { preset = "cells" } },
	{ name = "voronoi stained", effect = fx.voronoi { preset = "stained" } },
	{ name = "waves sea", effect = fx.waves { preset = "sea" } },
	{ name = "waves dunes", effect = fx.waves { preset = "dunes" } },
	{ name = "halftone print", effect = fx.halftone { preset = "print" } },
	{ name = "rim_lens clear", effect = fx.rim_lens { preset = "clear" } },
	{ name = "rim_lens thick", effect = fx.rim_lens { preset = "thick" } },
	{ name = "plastic glossy", effect = fx.plastic { preset = "glossy" } },
	{ name = "plastic matte", effect = fx.plastic { preset = "matte" } },
	{ name = "plastic dished", effect = fx.plastic { preset = "dished" } },
}

local function stripes()
	local rows = {}
	for i = 1, 8 do
		rows[i] = ui.div { w_full = true, h = 18, bg = if i % 2 == 0 then "#d65c48" else "#26305a" }
	end
	return ui.column { absolute = true, left = 0, top = 0, w_full = true, table.unpack(rows) }
end

local function cell(name: string, effect, on_press: ((e: PointerEvent) -> ...unknown)?, id: string?)
	return ui.overlay {
		w = 220,
		h = 170,
		stripes(),
		ui.div(fx.with(effect, {
			absolute = true,
			left = 40,
			top = 28,
			w = 140,
			h = 84,
			rounded = 14,
			id = id,
			on_press = on_press,
			ui.text { "Sample", color = "#ffffff", size = 18, p = 10 },
		})),
		ui.text { name, absolute = true, left = 6, top = 146, size = 12, color = "#ffffff" },
	}
end

local ripple = fx.ripple()
local shockwave = fx.shockwave { reach = 160 }
local wave = fx.wave()
local cells = {
	cell("ripple: press", ripple, function(e)
		ripple:fire(e.local_x, e.local_y)
	end, "ripple"),
	cell("shockwave: press", shockwave, function(e)
		shockwave:fire(e.local_x, e.local_y)
	end, "shockwave"),
	cell("wave: press", wave, function(e)
		wave:fire(e.local_x, e.local_y)
	end, "wave"),
}
table.insert(
	cells,
	ui.overlay {
		w = 220,
		h = 170,
		stripes(),
		ui.glass {
			absolute = true,
			left = 40,
			top = 28,
			w = 140,
			h = 84,
			radius = 14,
			ui.text { "Glass", color = "#ffffff", size = 18, p = 10 },
		},
	}
)
table.insert(
	cells,
	1,
	ui.overlay {
		w = 220,
		h = 170,
		stripes(),
		ui.div {
			id = "melt",
			absolute = true,
			left = 40,
			top = 28,
			w = 140,
			h = 84,
			rounded = 14,
			bg = "#30343c",
			layer_shader = melt,
			layer_clock = true,
			layer_bleed = 76,
			layer_params = { { 50, 0, 0, 0 } },
			ui.text { "Melt", color = "#ffffff", size = 18, p = 10 },
		},
		ui.text { "melt: this file's own", absolute = true, left = 6, top = 146, size = 12, color = "#ffffff" },
	}
)
for _, effect in effects do
	table.insert(cells, cell(effect.name, effect.effect))
end

return ui.scroll {
	w_full = true,
	h_full = true,
	bg = "#0c0e14",
	ui.row { flex_wrap = true, gap = 4, p = 8, table.unpack(cells) },
}
