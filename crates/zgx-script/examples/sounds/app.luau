-- Sound: clicks on controls, a metronome on the mixer's clock, and a pad
-- whose volume a slider eases, and whose filter a checkbox sweeps, with no
-- script per frame. Run it with
-- `cargo run -p zgx-run --features audio -- crates/zgx-script/examples/sounds`.
-- Its sounds are synthesized for it (assets/).

-- Loaded once, at the top: decoded off the main thread.
local click = audio.sound("assets/click.wav")
local tick = audio.sound("assets/tick.wav")

local volume = ui.state("volume", 0.6)
local muted = ui.state("muted", false)
local tempo = ui.state("tempo", 120)
local muffled = ui.state("muffled", false)

-- The slider moves the music bus's gain through a tween: the mixer reads
-- it each block, so neither the slider nor the tween sends a message.
local music = audio.bus("music")
if music then
	music:gain(ui.tween(volume, { duration = 120 }))
	music:mute(muted)
	-- A low-pass the checkbox sweeps down to 400 Hz and back: the tween
	-- eases the cutoff in Rust, a message a frame while it moves.
	local cutoff = ui.computed(function()
		return if muffled:get() then 400 else 20000
	end)
	music:add_effect(audio.low_pass { cutoff = ui.tween(cutoff, { duration = 600 }), slope = 24 })
end
-- The ticks ring in a small room.
local world = audio.bus("world")
if world then
	world:add_effect(audio.reverb { size = 0.6, decay = 0.8, wet = 0.2 })
end

local pad: Voice? = nil
local padding = ui.state("padding", false)

local function toggle_pad()
	if pad then
		pad:stop(1.5)
		pad = nil
		padding:set(false)
	else
		pad = audio.music("assets/pad.wav", { loop = true, fade_in = { seconds = 2, curve = "equal_power" } })
		padding:set(true)
	end
end

-- Four ticks, each on its own frame of the mixer's clock: a rhythm the
-- timer's tick would smear.
local function bar()
	local beat = 60 / tempo:peek()
	local start = audio.now() + 0.05
	for i = 0, 3 do
		audio.play(tick, { at = start + i * beat, gain = if i == 0 then 1 else 0.6 })
	end
end

return ui.column {
	p = 24,
	gap = 14,
	ui.text { "Sound", text_size = 20, weight = 600 },
	ui.text {
		color = "$text.secondary",
		function()
			return if audio.ui_sounds_enabled:get()
				then "Interface sounds are on."
				else "Interface sounds are off in the system's settings: the clicks are silent."
		end,
	},
	ui.row {
		gap = 8,
		ui.button { id = "save", sound = click, "Save" },
		ui.button { id = "soft", sound = { sound = click, gain = 0.4, pitch = 1.25 }, "Softer, higher" },
		ui.checkbox { id = "mute", sound = click, label = "Mute music", value = muted },
	},
	ui.row {
		gap = 8,
		ui.button {
			id = "pad",
			sound = click,
			on_click = toggle_pad,
			function()
				return if padding:get() then "Stop the pad" else "Play the pad"
			end,
		},
		ui.slider { id = "volume", label = "Music volume", value = volume, step = 0.05, w = 200 },
		ui.checkbox { id = "muffle", sound = click, label = "Muffle", value = muffled },
	},
	ui.row {
		gap = 8,
		ui.button { id = "bar", on_click = bar, "Play a bar" },
		ui.slider { id = "tempo", label = "Tempo", value = tempo, min = 60, max = 200, step = 1, w = 200 },
		ui.text { "{tempo} bpm" },
	},
}
