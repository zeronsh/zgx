-- A composition playing in an iOS app (`ios_player.rs` exposes it as `film`).
-- `ZGX_APP_ARGS=auto` drives it alone, printing each step: play, pause, a
-- seek, play again, and a pause held. `soak` plays it looping.
local film = ui.shared("film", {} :: any)
local mode = ui.args[1]

ui.effect(function()
	local s = film:get()
	if s.error then
		print(`FILM error {s.error}`)
	end
end)

local function told(what)
	local s = film:get()
	print(`FILM {what} frame={s.frame or "-"} playing={tostring(s.playing)}`)
end

ui.on_mount(function()
	ui.call("film.listen", nil)
	ui.invoke("film.open", { id = "Film", props = {}, frame = 0 })
	if mode == "auto" then
		ui.after(1000, function()
			ui.invoke("film.play", true)
			told("play")
		end)
		ui.after(4000, function()
			ui.invoke("film.play", false)
			told("pause")
		end)
		ui.after(5000, function()
			told("paused")
		end)
		ui.after(6000, function()
			ui.invoke("film.seek", 150)
			told("seek")
		end)
		ui.after(7000, function()
			told("sought")
			ui.invoke("film.play", true)
		end)
		ui.after(9000, function()
			ui.invoke("film.play", false)
			told("pause")
		end)
		ui.after(12000, function()
			told("held")
		end)
	elseif mode == "soak" then
		ui.invoke("film.loop", true)
		ui.invoke("film.play", true)
		ui.every(60000, function()
			told("soak")
		end)
	end
end)

return ui.column {
	grow = 1,
	bg = "#000",
	items = "center",
	justify = "center",
	gap = 12,
	-- The space the controls leave, in either orientation.
	ui.div {
		grow = 1,
		h = 0,
		w_full = true,
		ui.image { src = "host:film", w_full = true, h_full = true, fit = "contain" },
	},
	ui.row {
		gap = 12,
		ui.button {
			"Play",
			on_click = function()
				ui.invoke("film.play", not (film:get().playing == true))
			end,
		},
		ui.button {
			"Start",
			on_click = function()
				ui.invoke("film.seek", 0)
			end,
		},
	},
	ui.text {
		color = "#fff",
		function()
			return `frame {film:get().frame or 0}`
		end,
	},
}
