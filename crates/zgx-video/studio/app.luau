-- The studio: a composition played live, written with zgx. Its frames come from
-- a thread of the host's (`host:preview`); everything else is here.
local studio: any = ui.shared("studio", {} :: any)

local palette = {
	base = "#0e0f14",
	panel = "#15161d",
	raised = "#1d1f29",
	line = "#2a2c38",
	text = "#e7e8ee",
	dim = "#8d90a3",
	faint = "#5c5f73",
	accent = "#7c6cff",
	accent2 = "#ff6a8b",
	good = "#3ecf8e",
	bad = "#ff5f6d",
}
local TRACK_COLORS = { "#7c6cff", "#ff6a8b", "#35c2d6", "#f5a524", "#3ecf8e", "#c56cf0", "#4f8cff" }

local selected: any = ui.state("selected", nil :: string?)
local edits: any = ui.state("edits", {} :: any)
local playhead: any = ui.computed(function(): any
	return studio:get().frame or 0
end)
local open: any = ui.computed(function(): any
	return studio:get().open
end)
local compositions: any = ui.computed(function(): any
	return studio:get().compositions or {}
end)

local function field(name: string, fallback: any): any
	local value = studio:get()[name]
	return if value == nil then fallback else value
end

local function props_now(): any
	local comp = open:peek()
	local out = {}
	for key, value in if comp then comp.defaults else {} do
		out[key] = value
	end
	for key, value in edits:peek() do
		out[key] = value
	end
	return out
end

local function choose(id: string)
	selected:set(id)
	edits:set {}
	ui.invoke("studio.open", { id = id, props = {}, frame = 0 })
end
local function seek(frame: number)
	local comp = open:peek()
	if comp then
		ui.invoke("studio.seek", math.clamp(math.round(frame), 0, comp.duration - 1))
	end
end
local function nudge(by: number)
	seek(playhead:peek() + by)
end
local function toggle_play()
	ui.invoke("studio.play", not studio:peek().playing)
end
local function set_prop(key: string, value: any)
	edits:set_key(key, value)
	local comp = open:peek()
	if comp then
		ui.invoke("studio.open", { id = comp.id, props = props_now(), frame = playhead:peek() })
	end
end
local renders_asked = ui.state("renders_asked", 0)
-- The renders asked for, the latest first.
local renders: any = ui.computed(function(): any
	local list = table.clone(studio:get().renders or {})
	table.sort(list, function(a: any, b: any)
		return a.number > b.number
	end)
	return list
end)
local function render_now()
	local comp = open:peek()
	if comp then
		-- Each render a file of its own: the queue may hold several of one.
		renders_asked:add(1)
		ui.call("studio.render", {
			id = comp.id,
			props = props_now(),
			output = `{studio:peek().path or "."}/out/{comp.id}-{renders_asked:peek()}.mp4`,
		})
	end
end

local function timecode(frame: number, fps: number): string
	local total = math.floor(frame / fps)
	local rest = math.floor(frame % fps)
	return string.format("%02d:%02d.%02d", math.floor(total / 60), total % 60, rest)
end

ui.command { id = "play", title = "Play or Pause", menu = "Playback", key = "space", run = toggle_play }
ui.command {
	id = "back",
	title = "Back a Frame",
	menu = "Playback",
	key = "arrowleft",
	run = function()
		nudge(-1)
	end,
}
ui.command {
	id = "on",
	title = "On a Frame",
	menu = "Playback",
	key = "arrowright",
	run = function()
		nudge(1)
	end,
}
ui.command {
	id = "start",
	title = "To the Start",
	menu = "Playback",
	key = "home",
	run = function()
		seek(0)
	end,
}
ui.command { id = "render", title = "Render", menu = "File", key = "cmd+r", run = render_now }

local function eyebrow(text: string): any
	return ui.text { text, text_size = 11, weight = 600, letter_spacing = 1.2, color = palette.faint }
end

local function icon_button(label: string, on_click: () -> (), id: string): any
	return ui.button {
		id = id,
		label = id,
		on_click = on_click,
		w = 34,
		h = 34,
		rounded = 8,
		items = "center",
		justify = "center",
		bg = palette.raised,
		color = palette.text,
		text_size = 13,
		hover = { bg = palette.line },
		active = { bg = palette.faint },
		label,
	}
end

---------------------------------------------------------------- compositions

local function composition_list(): any
	return ui.column {
		w = 220,
		shrink = 0,
		bg = palette.panel,
		border_sides = { 0, 1, 0, 0 },
		border = 1,
		border_color = palette.line,
		p = 14,
		gap = 8,
		eyebrow("COMPOSITIONS"),
		ui.each(compositions, function(comp: any)
			return ui.button {
				on_click = function()
					choose(comp.id)
				end,
				label = comp.id,
				rounded = 8,
				px = 10,
				py = 8,
				direction = "column",
				items = "start",
				gap = 3,
				w_full = true,
				bg = function()
					local now = open:get()
					return if now and now.id == comp.id then palette.raised else "#00000000"
				end,
				hover = { bg = palette.raised },
				ui.text { comp.id, text_size = 13, weight = 600, color = palette.text },
				ui.text {
					`{math.floor(comp.w)}×{math.floor(comp.h)} · {math.floor(comp.fps)} fps · {string.format(
						"%.1f",
						comp.duration / comp.fps
					)} s`,
					text_size = 11,
					color = palette.dim,
				},
			}
		end, {
			key = function(comp: any)
				return comp.id
			end,
		}),
	}
end

---------------------------------------------------------------- preview

local function preview(): any
	return ui.column {
		relative = true,
		grow = 1,
		min_h = 0,
		items = "center",
		justify = "center",
		p = 20,
		bg = palette.base,
		ui.show(
			open,
			ui.div {
				id = "preview",
				aria_hidden = true,
				absolute = true,
				top = 20,
				left = 20,
				right = 20,
				bottom = 20,
				bg_image = "host:preview",
				bg_fit = "contain",
			},
			ui.column {
				items = "center",
				gap = 8,
				ui.text { "Loading the composition…", color = palette.dim },
			}
		),
	}
end

---------------------------------------------------------------- props

local function prop_row(key: string, value: any, schema: any): any
	local kind = type(value)
	local control: any
	if kind == "number" then
		local state = ui.use_state(value)
		local spec = schema or {}
		control = ui.column {
			gap = 4,
			ui.row {
				justify = "between",
				ui.text { key, text_size = 12, color = palette.dim },
				ui.text {
					function()
						return string.format("%.3g", state:get())
					end,
					text_size = 12,
					color = palette.text,
				},
			},
			ui.slider {
				label = key,
				value = state,
				min = spec.min or 0,
				max = spec.max or math.max(math.abs(value :: number) * 2, 1),
				on_change = function(next: number)
					set_prop(key, next)
				end,
			},
		}
	elseif kind == "boolean" then
		local state = ui.use_state(value)
		control = ui.checkbox {
			label = key,
			text_size = 13,
			value = state,
			on_change = function(next: boolean)
				set_prop(key, next)
			end,
		}
	else
		local state = ui.use_state(tostring(value))
		control = ui.column {
			gap = 4,
			ui.text { key, text_size = 12, color = palette.dim },
			ui.input {
				label = key,
				value = state,
				w_full = true,
				text_size = 13,
				on_commit = function(text: string)
					set_prop(key, text)
				end,
			},
		}
	end
	return control
end

local function props_panel(): any
	return ui.column {
		w = 260,
		shrink = 0,
		bg = palette.panel,
		border_sides = { 0, 0, 0, 1 },
		border = 1,
		border_color = palette.line,
		p = 14,
		gap = 12,
		eyebrow("PROPS"),
		ui.presence(function()
			local comp = open:get()
			return if comp then comp.id else nil
		end, function()
			local comp = open:peek()
			local keys = {}
			for key in comp.defaults do
				table.insert(keys, key)
			end
			table.sort(keys)
			if #keys == 0 then
				return ui.text { "This composition takes no props.", text_size = 12, color = palette.faint }
			end
			local Row = ui.component(function(p: any)
				return prop_row(p.key, comp.defaults[p.key], comp.schema[p.key])
			end)
			local rows: any = { gap = 14 }
			for _, key in keys do
				table.insert(rows, Row { key = key })
			end
			return ui.column(rows)
		end),
		ui.div { grow = 1 },
		eyebrow("RENDER"),
		ui.button {
			id = "render",
			label = "Render",
			on_click = render_now,
			h = 38,
			rounded = 9,
			items = "center",
			justify = "center",
			bg = palette.accent,
			hover = { bg = "#8f81ff" },
			color = "#fff",
			weight = 600,
			text_size = 13,
			function()
				local busy = false
				for _, job in field("renders", {}) do
					busy = busy or job.state == "running" or job.state == "queued"
				end
				return if busy then "Add to queue" else "Render MP4"
			end,
		},
		ui.column {
			id = "renders",
			gap = 8,
			ui.each(renders, function(job: any, _i: number, item: any)
				local function now(): any
					return item:get()
				end
				return ui.column {
					gap = 4,
					ui.row {
						gap = 6,
						items = "center",
						ui.text {
							function()
								local j = now()
								return `#{j.number} {j.id or ""}`
							end,
							text_size = 12,
							weight = 600,
							color = palette.text,
							grow = 1,
							truncate = true,
						},
						ui.text {
							function()
								local j = now()
								if j.state == "running" then
									return `{j.done}/{j.total}`
								end
								return j.state
							end,
							text_size = 11,
							color = function()
								local state = now().state
								return if state == "failed"
									then palette.bad
									elseif state == "done" then palette.good
									else palette.dim
							end,
						},
						ui.show(
							function()
								local state = now().state
								return state == "running" or state == "queued"
							end,
							ui.button {
								label = `Stop render {job.number}`,
								on_click = function()
									ui.invoke("studio.cancel", now().number)
								end,
								w = 20,
								h = 20,
								rounded = 5,
								items = "center",
								justify = "center",
								bg = palette.raised,
								hover = { bg = palette.line },
								color = palette.dim,
								text_size = 11,
								"✕",
							}
						),
					},
					ui.div {
						h = 3,
						rounded = 2,
						bg = palette.raised,
						overflow = "hidden",
						ui.div {
							h = 3,
							rounded = 2,
							bg = function()
								return if now().state == "failed" then palette.bad else palette.good
							end,
							w = function()
								local j = now()
								if j.state == "done" then
									return "100%"
								end
								return `{math.floor((j.done or 0) / math.max(j.total or 1, 1) * 100)}%`
							end,
						},
					},
					ui.show(
						function()
							local state = now().state
							return state == "done" or state == "failed"
						end,
						ui.text {
							function()
								local j = now()
								return if j.state == "done" then `{j.said}\n{j.output}` else j.said
							end,
							text_size = 11,
							wrap = true,
							color = palette.dim,
							selectable = true,
						}
					),
				}
			end, {
				key = function(job: any)
					return job.number
				end,
			}),
		},
	}
end

---------------------------------------------------------------- timeline

local RULER = 22
local ROW = 20

local function timeline(): any
	local box: any = ui.ref()
	local function frame_at(e: any): number
		local comp = open:peek()
		local el = box:peek()
		if not comp or not el then
			return 0
		end
		return e.local_x / math.max(el.width, 1) * comp.duration
	end
	local dragging = ui.state("dragging", false)
	local tracks: any = ui.computed(function(): any
		-- Whole composition first, then by start: rows don't reorder as new ones appear.
		local list = table.clone(studio:get().tracks or {})
		table.sort(list, function(a: any, b: any)
			if a.from == b.from then
				return a.name < b.name
			end
			return a.from < b.from
		end)
		return list
	end)
	local seconds_marks: any = ui.computed(function(): any
		local comp = open:get()
		local marks = {}
		if comp then
			local seconds = comp.duration / comp.fps
			local step = if seconds > 120 then 30 elseif seconds > 40 then 10 elseif seconds > 12 then 5 else 1
			for s = 0, math.floor(seconds), step do
				table.insert(marks, s)
			end
		end
		return marks
	end)
	local function fraction(frame: number): string
		local comp = open:get()
		return `{(frame / math.max(comp and comp.duration or 1, 1)) * 100}%`
	end
	return ui.column {
		h = 240,
		shrink = 0,
		bg = palette.panel,
		border_sides = { 1, 0, 0, 0 },
		border = 1,
		border_color = palette.line,
		-- Transport
		ui.row {
			h = 50,
			shrink = 0,
			px = 14,
			gap = 8,
			items = "center",
			icon_button("⏮", function()
				seek(0)
			end, "start"),
			icon_button("‹", function()
				nudge(-1)
			end, "back"),
			ui.button {
				id = "play",
				label = "Play",
				on_click = toggle_play,
				w = 46,
				h = 34,
				rounded = 8,
				items = "center",
				justify = "center",
				bg = palette.accent,
				color = "#fff",
				hover = { bg = "#8f81ff" },
				function()
					return if field("playing", false) then "❚❚" else "▶"
				end,
			},
			icon_button("›", function()
				nudge(1)
			end, "on"),
			ui.text {
				function()
					local comp = open:get()
					return if comp then timecode(playhead:get(), comp.fps) else "--:--.--"
				end,
				id = "timecode",
				font = "monospace",
				text_size = 15,
				color = palette.text,
				ml = 10,
			},
			ui.text {
				function()
					local comp = open:get()
					return if comp then `frame {math.floor(playhead:get())} / {comp.duration - 1}` else ""
				end,
				font = "monospace",
				text_size = 12,
				color = palette.dim,
				ml = 8,
			},
			ui.div { grow = 1 },
			ui.checkbox {
				label = "Loop",
				text_size = 13,
				value = ui.state("loop", true),
				on_change = function(on: boolean)
					ui.invoke("studio.loop", on)
				end,
			},
		},
		-- Ruler, tracks and the playhead over them.
		ui.column {
			id = "timeline",
			ref = box,
			grow = 1,
			min_h = 0,
			relative = true,
			mx = 14,
			mb = 10,
			overflow = "hidden",
			rounded = 8,
			bg = palette.base,
			cursor = "pointer",
			on_press = function(e: any)
				dragging:set(true)
				seek(frame_at(e))
			end,
			on_pointer_move = function(e: any)
				if dragging:peek() then
					seek(frame_at(e))
				end
			end,
			on_release = function()
				dragging:set(false)
			end,
			-- Seconds
			ui.div {
				h = RULER,
				shrink = 0,
				relative = true,
				bg = palette.raised,
				ui.each(seconds_marks, function(s: number)
					return ui.div {
						absolute = true,
						top = 0,
						bottom = 0,
						left = function()
							local comp = open:get()
							return fraction(s * (if comp then comp.fps else 30))
						end,
						border_sides = { 0, 0, 0, 1 },
						border = 1,
						border_color = palette.line,
						pl = 5,
						justify = "center",
						pointer_events = "none",
						ui.text { `{s}s`, text_size = 10, color = palette.dim },
					}
				end),
			},
			-- Sequences
			ui.scroll {
				grow = 1,
				min_h = 0,
				ui.column {
					gap = 2,
					py = 3,
					ui.each(tracks, function(track: any, index: number)
						return ui.div {
							h = ROW - 2,
							relative = true,
							ui.div {
								absolute = true,
								top = 0,
								bottom = 0,
								left = fraction(track.from),
								w = function()
									local comp = open:get()
									local lasts = if track.duration >= 0
										then track.duration
										else (comp and comp.duration or 1) - track.from
									return fraction(lasts)
								end,
								rounded = 5,
								bg = TRACK_COLORS[(index - 1) % #TRACK_COLORS + 1] .. "44",
								border = 1,
								border_color = TRACK_COLORS[(index - 1) % #TRACK_COLORS + 1] .. "aa",
								pl = 7,
								justify = "center",
								overflow = "hidden",
								pointer_events = "none",
								ui.text { track.name, text_size = 11, color = palette.text, truncate = true },
							},
						}
					end),
				},
			},
			-- The playhead
			ui.div {
				absolute = true,
				top = 0,
				bottom = 0,
				w = 2,
				bg = palette.accent2,
				pointer_events = "none",
				left = function()
					return fraction(playhead:get())
				end,
			},
		},
	}
end

---------------------------------------------------------------- the app

local function error_strip(): any
	return ui.show(
		function()
			return field("error", nil) ~= nil
		end,
		ui.row {
			bg = "#3a1620",
			px = 14,
			py = 10,
			shrink = 0,
			ui.text {
				function()
					return tostring(field("error", ""))
				end,
				id = "error",
				font = "monospace",
				text_size = 12,
				wrap = true,
				selectable = true,
				color = "#ffb4bc",
			},
		}
	)
end

-- The composition to open as soon as the script's list comes.
ui.effect(function()
	local list = compositions:get()
	if selected:peek() == nil and #list > 0 then
		choose(list[1].id)
	end
end)
ui.on_mount(function()
	ui.call("studio.listen", nil)
	ui.call("studio.watch", nil)
end)

return ui.column {
	w_full = true,
	h_full = true,
	bg = palette.base,
	color = palette.text,
	ui.row {
		grow = 1,
		min_h = 0,
		composition_list(),
		ui.column { grow = 1, min_w = 0, error_strip(), preview() },
		props_panel(),
	},
	timeline(),
}
