#!/usr/bin/env python3
"""Writes crates/zgx-fx/luau/fx.luau from the effects' Rust table and WGSL.

    python3 scripts/fx_luau.py

The Luau module holds each effect's shader, parameters and presets, and builds
ordinary props from them: it knows nothing a script doesn't. crates/zgx-fx/tests/
luau.rs checks the file is what this writes, and its numbers are the Rust side's.
"""
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
FX = ROOT / 'crates/zgx-fx'
SOURCE = (FX / 'src/effects.rs').read_text()

CHOICES = {}
for m in re.finditer(r'choice!\(\s*///[^\n]*\n\s*(\w+) \{(.*?)\n\s*\}\n\);', SOURCE, re.S):
    CHOICES[m.group(1)] = re.findall(r'(\w+) = "(\w+)"', m.group(2))


def expression(text):
    """A closure's body as Luau: they're small sums of fields."""
    t = text.strip()
    if t.startswith('if '):
        m = re.match(r'if (.*?) \{ (.*?) \} else \{ (.*?) \}$', t)
        return f'(if {expression(m[1])} then {expression(m[2])} else {expression(m[3])})'
    t = re.sub(r'e\.(\w+)\.abs\(\)\.max\(e\.(\w+)\.abs\(\)\)', r'math.max(math.abs(o.\1), math.abs(o.\2))', t)
    t = re.sub(r'e\.(\w+)\.abs\(\)', r'math.abs(o.\1)', t)
    t = re.sub(r'\be\.', 'o.', t)
    t = re.sub(r'(\d)\.(?!\d)', r'\1', t)
    t = t.replace(' && ', ' and ').replace('||', 'or').replace('!=', '~=')
    t = t.replace('GlowStyle::Aura', '"aura"')
    return t


def value(rust, kind):
    r = rust.strip()
    m = re.match(r'rgb\(0x([0-9a-f]{6})\)', r)
    if m:
        return f'"#{m[1]}"'
    m = re.match(r'gray\((\d+)\)', r)
    if m:
        return '"#' + f'{int(m[1]):02x}' * 3 + '"'
    m = re.match(r'(\w+)::(\w+)$', r)
    if m:
        return '"' + dict((v, t) for v, t in CHOICES[m[1]])[m[2]] + '"'
    if r in ('true', 'false'):
        return r
    return re.sub(r'\.$', '', r.rstrip())


def split_fields(text):
    return [part for part in re.split(r',\s*(?=\w+:)', text.strip()) if part.strip()]


effects = []
for m in re.finditer(
    r'effect!\(\n(?:\s*///[^\n]*\n)+\s*(\w+): (layer|fill) "(\w+)" "([^"]+)" \{(.*?)\n    \}\n(?:    presets \{(.*?)\n    \}\n)?'
    r'    fps: (.*?),\n    bleed: \|\w+: &\w+\| (.*?),\n    reach: \|\w+: &\w+\| (.*?),\n    frost: \|\w+: &\w+\| (.*?),\n'
    r'    animated: \|\w+: &\w+\| (.*?),\n    idle: \|\w+: &\w+\| (.*?),\n    life: \|\w+: &\w+\| (.*?),\n    one_shot: (\w+),',
    SOURCE,
    re.S,
):
    name, kind, luau, wgsl, body, presets, fps, bleed, reach, frost, animated, idle, life, one_shot = m.groups()
    fields = re.findall(r'(\w+): (\w+) = (.*?) => (\d) (\d),', body)
    preset_table = {}
    if presets:
        for p in re.finditer(r'(\w+) \{([^\n]*)\}', presets):
            preset_table[p[1]] = [
                (f.split(':', 1)[0].strip(), f.split(':', 1)[1]) for f in split_fields(p[2])
            ]
    types = {f[0]: f[1] for f in fields}
    effects.append((luau, kind, wgsl, fields, preset_table, fps, bleed, reach, frost, animated, idle, life, one_shot, types))

common = (FX / 'src/common.wgsl').read_text()
fills = (FX / 'src/fills.wgsl').read_text()

out = ['''--!nonstrict
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
''']


def luau_type(t):
    if t == 'f32':
        return 'number'
    if t == 'bool':
        return 'bool'
    if t == 'Color':
        return 'color'
    return '{ ' + ', '.join(f'"{text}"' for _, text in CHOICES[t]) + ' }'


for luau, kind, wgsl, fields, presets, fps, bleed, reach, frost, animated, idle, life, one_shot, types in effects:
    source = (FX / 'src' / wgsl).read_text()
    full = common + '\n' + (fills + '\n' if kind == 'fill' else '') + source
    out.append(f'\nlocal {luau}_spec = {{\n\tname = "{luau}",\n\tfill = {str(kind == "fill").lower()},')
    m = re.match(r'Some\(([\d.]+)\)', fps)
    if m:
        out.append(f'\tfps = {m[1].rstrip(".")},')
    out.append(f'\tone_shot = {one_shot},')
    out.append(f'\tsource = [==[\n{full}]==],')
    out.append('\tfields = {')
    for name, ty, default, row, lane in fields:
        out.append(f'\t\t{name} = {{ {row}, {lane}, '.rstrip() + f' "{luau_type(ty)}", {value(default, ty)} }},' if not luau_type(ty).startswith('{') else f'\t\t{name} = {{ {row}, {lane}, {luau_type(ty)}, {value(default, ty)} }},')
    out.append('\t},')
    out.append('\tpresets = {')
    for preset, sets in presets.items():
        body = ', '.join(f'{k} = {value(v, types.get(k, "f32"))}' for k, v in sets)
        out.append(f'\t\t{preset} = {{ {body} }},')
    out.append('\t},')
    for key, text in [('bleed', bleed), ('reach', reach), ('frost', frost), ('animated', animated), ('idle', idle), ('life', life)]:
        out.append(f'\t{key} = function(o)\n\t\treturn {expression(text)}\n\tend,')
    out.append('}')
    out.append(f'function fx.{luau}(opts: Options?): Effect\n\treturn make({luau}_spec, opts)\nend')
out.append('\nreturn fx\n')
(FX / 'luau/fx.luau').write_text('\n'.join(out))
print(len(effects), 'effects')
