--!strict
-- The pointer-lock probe's app (`scripts/pointer_lock_smoke.py`): a game area
-- on the left locks the pointer on a press and sums its motion; it takes focus
-- on that press (before it locks), so it hears the keys. What it hears is
-- printed.
local game = ui.ref()
local tx, ty = 0, 0

return ui.row {
	w_full = true,
	h_full = true,
	ui.div {
		ref = game,
		id = "game",
		w = 480,
		h_full = true,
		bg = "#223",
		focusable = true,
		on_key = function(e)
			print("KEY " .. e.key)
			return false
		end,
		on_press = function()
			local el = game:get()
			if el then
				print("LOCK asked " .. tostring(el:lock_pointer()))
			end
		end,
		on_pointer_lock = function(locked)
			print("LOCK " .. tostring(locked))
			if not locked then
				-- Escape is no gesture to lock with: this is refused.
				local el = game:get()
				print("RELOCK " .. tostring(el ~= nil and el:lock_pointer()))
			end
		end,
		on_pointer_move = function(e)
			if e.locked then
				tx += e.dx or 0
				ty += e.dy or 0
				print(string.format("MOTION %d %d", math.floor(tx + 0.5), math.floor(ty + 0.5)))
			end
		end,
	},
	ui.div { grow = 1, h_full = true, bg = "#332" },
}
