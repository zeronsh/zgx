-- A click from Luau: three, left, middle and right, a fifth of a second
-- apart, timed on the audio thread to the frame.
local audio = require("@audio")

for i, pan in { -1, 0, 1 } do
	audio.play("click.wav", { pan = pan, volume = 0.8, delay = (i - 1) * 0.2 })
end

return ui.div {}
