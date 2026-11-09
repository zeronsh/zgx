-- The sidebar: what it's attached to, a filter, the sort, the processes, and
-- their total.
local theme = require("../theme")
local icons = require("../icons")
local state = require("../state")
local data = require("../data")
local keys = require("../keys")
local actions = require("../actions")
local controls = require("./controls")
local Row = require("./process_row")
local format = require("@lib/format")

local query, sort = state.query, state.sort

local M = {}

-- What the list covers, chosen from a menu: every process, or one app and
-- everything it started.
local function attacher(): Node
	local open = state.attach_open
	local function item(label: string, name: string, shown: string?): Node
		return ui.menu_item {
			label = label,
			radio = true,
			checked = function()
				return state.attached:get() == name
			end,
			shortcut = shown,
			on_click = function()
				actions.attach(name)
			end,
		}
	end
	return ui.div {
		mx = 10,
		mb = 8,
		ui.menu {
			open = open,
			label = "Attach to",
			match_anchor_width = true,
			p = 5,
			gap = 0,
			rounded = 12,
			items = "stretch",
			anchor = ui.button {
				id = "attach",
				w_full = true,
				h = 32,
				pl = 10,
				pr = 8,
				gap = 7,
				items = "center",
				rounded = 9,
				bg = theme.wash,
				border = 1,
				border_color = theme.line,
				hover = { bg = theme.hover },
				focus = { border = 1 },
				on_click = ui.toggle(open),
				ui.svg { code = icons.target, w = 13, h = 13, tint = theme.tertiary, shrink = 0 },
				ui.text {
					function()
						local name = state.attached:get()
						return if name == "" then "All processes" else name
					end,
					grow = 1,
					shrink = 1,
					min_w = 0,
					truncate = true,
					text_size = 12.5,
					weight = 500,
					color = theme.text,
				},
				ui.svg { code = icons.chevron, w = 13, h = 13, tint = theme.tertiary, shrink = 0 },
			},
			-- Built as it opens, from the processes running then.
			build = function()
				local items: ContainerProps = { items = "stretch" }
				table.insert(items, item("All processes", ""))
				table.insert(items, ui.separator {})
				for _, app in data.apps() do
					table.insert(items, item(app.name, app.name, tostring(app.count)))
				end
				return ui.column(items)
			end,
		},
	}
end

local function search(): Node
	local focused = ui.state("search_focused", false)
	return ui.row {
		mx = 10,
		h = 32,
		pl = 10,
		pr = 4,
		gap = 7,
		items = "center",
		rounded = 9,
		bg = theme.wash,
		border = 1,
		border_color = function()
			return if focused:get() then "#ffffff29" else theme.line
		end,
		ui.svg { code = icons.search, w = 13, h = 13, tint = theme.tertiary, shrink = 0 },
		ui.input {
			value = query,
			label = "Filter processes",
			placeholder = "Filter processes",
			placeholder_color = theme.tertiary,
			-- Typing filters at once; ↑ and ↓ browse what it shows.
			ref = state.search,
			grow = 1,
			shrink = 1,
			min_w = 0,
			h = 22,
			px = 0,
			py = 0,
			bg = theme.clear,
			color = theme.text,
			text_size = 12.5,
			focus = { border = 0 },
			on_key = keys.on_key,
			on_focus = function()
				focused:set(true)
			end,
			on_blur = function()
				focused:set(false)
			end,
		},
		ui.row {
			shrink = 0,
			opacity = ui.tween(
				ui.computed(function()
					return query:get() ~= ""
				end),
				{ duration = 120 }
			),
			controls.tool(icons.close, function()
				query:set("")
			end, 22, 12),
		},
	}
end

-- The sort, a segmented control whose selection slides between options.
local function sorting(): Node
	local function option(label: string, value: state.Sort): Node
		local function on(): boolean
			return sort:get() == value
		end
		return ui.button {
			grow = 1,
			basis = 0,
			h = 24,
			p = 0,
			rounded = 7,
			relative = true,
			justify = "center",
			bg = theme.clear,
			hover = { bg = theme.hover },
			focus = { border = 0 },
			on_click = function()
				sort:set(value)
				state.list_offset:set(0)
			end,
			ui.show(
				on,
				ui.div {
					absolute = true,
					left = 0,
					top = 0,
					w_full = true,
					h_full = true,
					rounded = 7,
					bg = "#ffffff17",
					border = 1,
					border_color = "#ffffff12",
					layout_id = "sort",
					layout_duration = 240,
					layout_ease = "outQuint",
				}
			),
			ui.text {
				label,
				text_size = 10.5,
				weight = 600,
				letter_spacing = 0.8,
				color = function()
					return if on() then theme.text else theme.tertiary
				end,
			},
		}
	end
	return ui.row {
		mx = 10,
		mt = 8,
		mb = 6,
		p = 2,
		gap = 2,
		rounded = 9,
		bg = "#0000001f",
		option("CPU", "cpu"),
		option("GPU", "gpu"),
		option("MEM", "memory"),
		option("A–Z", "name"),
	}
end

local function list(): Node
	return ui.list {
		count = data.count,
		height = theme.row,
		overscan = 4,
		offset = state.list_offset,
		w_full = true,
		grow = 1,
		min_h = 0,
		fade = { 0, 18 },
		on_hover = function(inside)
			state.list_hover:set(inside)
		end,
		on_resize = function(size)
			state.list_h:set(size.h)
		end,
		row = Row,
	}
end

local function footer(): Node
	return ui.row {
		w_full = true,
		h = function()
			return if data.counted:get() ~= "" then 50 else 34
		end,
		shrink = 0,
		px = 16,
		gap = 8,
		items = "center",
		border_sides = { 1, 0, 0, 0 },
		border_color = theme.line,
		ui.column {
			grow = 1,
			shrink = 1,
			min_w = 0,
			gap = 3,
			-- What the apps in view count of their own, when they say.
			ui.show(
				function()
					return data.counted:get() ~= ""
				end,
				ui.text {
					function()
						return data.counted:get()
					end,
					truncate = true,
					text_size = 11,
					tabular = true,
					color = theme.secondary,
				}
			),
			ui.text {
				function()
					if state.attached:get() ~= "" then
						local scope = data.scope:get()
						return `{scope.count} processes · {format.percent(scope.cpu)} CPU · {format.percent(scope.gpu)} GPU · {format.memory(
							scope.mem
						)}`
					end
					local s: data.Summary = data.summary:get() or {}
					local count, cpu = s.count or 0, s.cpu or 0
					local text = `{count} processes · {format.percent(cpu)} CPU`
					return if s.gpu then `{text} · {string.format("%.0f", s.gpu)}% GPU` else text
				end,
				truncate = true,
				text_size = 11,
				tabular = true,
				color = theme.tertiary,
			},
		},
	}
end

-- The sidebar, as wide as it shows now: opening and closing, it uncovers
-- or covers what stays laid out at its full width inside.
function M.Sidebar(): Node
	return ui.row {
		w = function()
			return state.sidebar_shown:get()
		end,
		h_full = true,
		shrink = 0,
		overflow = "hidden",
		ui.column {
			w = state.sidebar_w,
			h_full = true,
			shrink = 0,
			items = "stretch",
			attacher(),
			search(),
			sorting(),
			list(),
			footer(),
		},
	}
end

-- Between the sidebar and the panels: dragging it resizes the sidebar.
function M.Resizer(): Node
	local start: { x: number, w: number }? = nil
	return ui.div {
		w = theme.inset,
		h_full = true,
		shrink = 0,
		cursor = function()
			return if state.sidebar_open:get() then "col_resize" else "default"
		end,
		on_press = function(e)
			if state.sidebar_open:peek() then
				start = { x = e.x, w = state.sidebar_w:peek() }
			end
		end,
		on_pointer_move = function(e)
			local s = start
			if s then
				local w = math.clamp(math.floor(s.w + e.x - s.x), 220, 420)
				state.sidebar_w:set(w)
				state.sidebar_shown:set(w)
			end
		end,
		on_release = function()
			start = nil
		end,
	}
end

return M
