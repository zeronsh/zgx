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
}

-- Plays the sound in `src` (beside the script) once.
function audio.play(src: string, options: PlayOptions?)
	local o: PlayOptions = options or {}
	ui.invoke("zgx.audio.play", {
		src = src,
		volume = o.volume,
		pan = o.pan,
		delay = o.delay,
	})
end

return audio
