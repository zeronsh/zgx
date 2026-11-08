--!strict
-- require("@audio"): sounds for a script, from zgx-audio. The device opens
-- at the first sound played.

local audio = {}

export type PlayOptions = {
	-- Its gain, 1 as recorded.
	volume: number?,
	-- -1 all left, 0 both, 1 all right.
	pan: number?,
	-- Seconds from now it starts, to the frame.
	delay: number?,
	-- The time on the audio clock (audio.now()) it starts at, to the frame:
	-- sounds scheduled on the clock keep their spacing exactly.
	at: number?,
}

-- The audio clock, in seconds: where the next buffer begins. Schedule sounds
-- a little ahead of it (a few milliseconds) with `at`.
function audio.now(): number
	return ui.call_sync("zgx.audio.now", nil) :: number
end

-- Plays the sound in `src` (beside the script) once.
function audio.play(src: string, options: PlayOptions?)
	local o: PlayOptions = options or {}
	ui.invoke("zgx.audio.play", {
		src = src,
		volume = o.volume,
		pan = o.pan,
		delay = o.delay,
		at = o.at,
	})
end

return audio
