-- A click from Luau: three, left, middle and right, a fifth of a second
-- apart on the audio clock, each starting on its exact sample.
local audio = require("@audio")

local t = audio.now() + 0.05
for i, pan in { -1, 0, 1 } do
	audio.play("click.wav", { pan = pan, volume = 0.8, at = t + (i - 1) * 0.2 })
end

return ui.div {}
