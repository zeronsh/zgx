-- `ui.ios`: a small set of iOS's standard pieces, drawn by zgx from the system's
-- colours, text sizes, glass and symbols, to UIKit's measurements. What must be
-- exact is the system's own (`ui.native_tabs`, `ui.native_switch`, `ui.alert`).
-- An app can take any of these apart: they use nothing a script can't.
local ui = ...
local ios = {}

local function clamp(value, low, high)
	return math.max(low, math.min(high, value))
end

-- Runs `state` to `target` as a critically damped spring from where it is, a
-- frame at a time, then `done`. Only while something moves.
local function settle(state, target, done)
	local velocity = 0
	local stop
	local spring = 200
	local damping = 2 * math.sqrt(spring)
	stop = ui.every(16, function()
		local at = state:get()
		local step = 0.016
		local force = spring * (target - at) - damping * velocity
		velocity += force * step
		at += velocity * step
		if math.abs(at - target) < 0.5 and math.abs(velocity) < 5 then
			state:set(target)
			if stop then
				stop()
			end
			if done then
				done()
			end
		else
			state:set(at)
		end
	end)
	return stop
end
ios.settle = settle

-- A list's section: an inset card, with a header and footer in the system's
-- small text. `ui.ios.section { header = "…", footer = "…", ui.ios.row { … } }`.
function ios.section(props)
	local rows = {}
	for index, row in ipairs(props) do
		rows[index] = row
	end
	return ui.column {
		mb = 35,
		props.header and ui.text {
			props.header,
			mx = 36,
			mb = 7,
			text_size = "$font.size.xs",
			color = "$text.secondary",
		} or nil,
		ui.column {
			mx = 20,
			rounded = 26,
			bg = "$ios.cell",
			overflow = "hidden",
			rows,
		},
		props.footer and ui.text {
			props.footer,
			mx = 36,
			mt = 7,
			text_size = "$font.size.xs",
			color = "$text.secondary",
		} or nil,
	}
end

-- The white symbol on a coloured tile that leads a Settings row.
function ios.tile(symbol, tint)
	return ui.div {
		w = 29,
		h = 29,
		rounded = 7,
		bg = tint,
		items = "center",
		justify = "center",
		shrink = 0,
		ui.symbol { name = symbol, size = 16, color = "#ffffff" },
	}
end

-- A row of a section: 60 points at the body text size, growing with it.
-- `icon`, `tint`, `title`, `detail`, `toggle` (a state: the row is a switch),
-- `chevron`, `on_click`, `last` (no separator).
function ios.row(props)
	local toggle = props.toggle
	return ui.button {
		min_h = 60,
		p = 0,
		py = 8,
		pl = 16,
		pr = 20,
		rounded = 0,
		bg = "#00000000",
		direction = "row",
		items = "center",
		justify = "start",
		hover = {},
		active = { bg = "$pressed" },
		role = if toggle then "switch" else "button",
		checked = toggle,
		on_click = if toggle
			then function()
				toggle:set(not toggle:get())
			end
			else props.on_click,
		props.icon and ios.tile(props.icon, props.tint or "$ios.blue") or nil,
		ui.text {
			props.title,
			ml = if props.icon then 16 else 0,
			grow = 1,
			text_size = "$font.size.md",
			color = props.color or "$text.primary",
		},
		props.detail and ui.text { props.detail, text_size = "$font.size.md", color = "$text.secondary", mr = 8 }
			or nil,
		toggle and ui.native_switch { value = toggle, label = props.title } or nil,
		(props.chevron ~= false and not toggle)
				and ui.symbol { name = "chevron.forward", size = 14, weight = 600, color = "$text.tertiary" }
			or nil,
		not props.last and ui.div {
			absolute = true,
			left = if props.icon then 61 else 16,
			right = 16,
			bottom = 0,
			h = 0.34,
			bg = "$border",
		} or nil,
	}
end

-- A search field on glass, the system's keyboard over zgx's editor.
function ios.search(props)
	return ui.glass {
		mx = props.mx or 16,
		h = 48,
		radius = 24,
		interactive = true,
		direction = "row",
		items = "center",
		pl = 16,
		pr = 12,
		ui.symbol { name = "magnifyingglass", size = 17, weight = 500, color = "$text.secondary" },
		ui.input {
			value = props.value,
			label = props.placeholder or "Search",
			placeholder = props.placeholder or "Search",
			inputmode = "search",
			grow = 1,
			ml = 6,
			bg = "#00000000",
			border = 0,
			focus = {},
			text_size = "$font.size.md",
		},
	}
end

-- A page of a stack: a scrolling body under a bar whose large title gives way to
-- a small one as it scrolls, as a navigation bar's does. `title`, `trailing` (a
-- node), `on_refresh` (pull to refresh: a function given `done`), and the
-- body's children. `nav` comes from the stack, for the back button.
function ios.page(props)
	local safe = ui.window.safe_area
	local offset = ui.use_state(0)
	local stretch = ui.window.overscroll
	local pulled = false
	local refreshing = ui.use_state(false)
	local children = {}
	for index, child in ipairs(props) do
		children[index] = child
	end
	local TITLE_AT = 44
	local collapsed = function()
		return clamp((offset:get() - TITLE_AT) / 10, 0, 1)
	end
	return ui.div {
		w = "100%",
		h = "100%",
		bg = props.bg or "$ios.grouped",
		ui.scroll {
			w = "100%",
			h = "100%",
			offset = offset,
			ui.column {
				pt = function()
					return safe:get().top + 44
				end,
				pb = function()
					return safe:get().bottom + 90
				end,
				pl = function()
					return safe:get().left
				end,
				pr = function()
					return safe:get().right
				end,
				ui.text {
					props.title,
					heading = 1,
					ml = 16,
					mb = 4,
					text_size = "$font.size.xxl",
					weight = 700,
					color = "$text.primary",
				},
				children,
			},
		},
		-- The spinner a pull shows, under the bar.
		props.on_refresh and ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = function()
				return safe:get().top + 8
			end,
			h = 30,
			items = "center",
			justify = "center",
			opacity = function()
				if refreshing:get() then
					return 1
				end
				local amount = -stretch:get()
				-- As UIRefreshControl: a pull far enough refreshes, at once.
				if amount >= 70 and not pulled and not refreshing:get() then
					pulled = true
					refreshing:set(true)
					ui.haptic("medium")
					props.on_refresh(function()
						refreshing:set(false)
					end)
				elseif amount <= 0 then
					pulled = false
				end
				return clamp(amount / 70, 0, 1)
			end,
			pointer_events = false,
			ui.symbol { name = "arrow.triangle.2.circlepath", size = 18, weight = 600, color = "$text.secondary" },
		} or nil,
		-- The bar: a soft edge as content passes under it, its small title, and
		-- its buttons on glass.
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			h = function()
				return safe:get().top + 50
			end,
			pointer_events = false,
			bg = props.bg or "$ios.grouped",
			opacity = collapsed,
			ui.row {
				absolute = true,
				left = 0,
				right = 0,
				bottom = 0,
				h = 44,
				items = "center",
				justify = "center",
				ui.text {
					props.title,
					text_size = "$font.size.md",
					weight = 600,
					color = "$text.primary",
					aria_hidden = true,
				},
			},
		},
		props.nav and props.nav.depth() > 1 and ui.glass {
			absolute = true,
			left = 16,
			top = function()
				return safe:get().top + 3
			end,
			w = 44,
			h = 44,
			radius = 22,
			interactive = true,
			ui.button {
				w = 44,
				h = 44,
				p = 0,
				rounded = 22,
				bg = "#00000000",
				hover = {},
				active = {},
				label = "Back",
				items = "center",
				justify = "center",
				on_click = function()
					props.nav.pop()
				end,
				ui.symbol { name = "chevron.backward", size = 18, weight = 600, color = "$text.primary" },
			},
		} or nil,
		props.trailing and ui.div {
			absolute = true,
			right = 16,
			top = function()
				return safe:get().top + 3
			end,
			props.trailing,
		} or nil,
	}
end

-- Where a page's own state lives.
local Page = ui.component(function(props)
	return props.build(props.nav)
end)

-- A navigation stack: `root` builds the first page, and `nav.push(build)` slides
-- another over it. A drag from the left edge takes the top page back, following
-- the finger, as the system's does. Each `build` is given `nav`.
ios.stack = ui.component(function(props)
	-- State holds no functions: its list is of ids into `builders`.
	local builders = { props.root }
	local next_id = 1
	local pages = ui.use_state { 1 }
	local width = 402
	local drag = ui.use_state(0)
	local entering = ui.use_state(0)
	local nav = {}
	function nav.depth()
		return #pages:get()
	end
	function nav.push(build)
		next_id += 1
		builders[next_id] = build
		local list = table.clone(pages:get())
		table.insert(list, next_id)
		entering:set(width)
		pages:set(list)
		ui.haptic("light")
		settle(entering, 0)
	end
	function nav.pop()
		local list = pages:get()
		if #list < 2 then
			return
		end
		settle(drag, width, function()
			local kept = table.clone(pages:get())
			builders[table.remove(kept)] = nil
			pages:set(kept)
			drag:set(0)
		end)
	end
	local start = 0
	local tracking = false
	return ui.div {
		w = "100%",
		h = "100%",
		overflow = "hidden",
		on_resize = function(size)
			width = size.width
		end,
		ui.each(pages, function(id, index)
			local on_top = function()
				return index == #pages:get()
			end
			return ui.div {
				absolute = true,
				left = 0,
				top = 0,
				w = "100%",
				h = "100%",
				-- What lies under the top page moves a third as far.
				x = function()
					local depth = #pages:get()
					local moved = drag:get() + entering:get()
					if index == depth then
						return moved
					elseif index == depth - 1 then
						return (moved - width) / 3
					end
					return -width / 3
				end,
				shadow = { x = -4, blur = 16, color = "#00000020" },
				pointer_events = on_top,
				Page { build = builders[id], nav = nav },
			}
		end, { display = "contents" }),
		-- The edge a swipe back starts from.
		ui.div {
			absolute = true,
			left = 0,
			top = 0,
			bottom = 0,
			w = 20,
			hidden = function()
				return #pages:get() < 2
			end,
			aria_hidden = true,
			on_press = function(event)
				start = event.x
				tracking = true
			end,
			on_pointer_move = function(event)
				if tracking then
					drag:set(math.max(0, event.x - start))
				end
			end,
			on_release = function()
				if not tracking then
					return
				end
				tracking = false
				if drag:get() > width * 0.35 then
					nav.pop()
				else
					settle(drag, 0)
				end
			end,
		},
	}
end)

-- A sheet that rises from the bottom over a dimmed page, on glass where the
-- system's are: `open`, a state; `detents`, its heights as fractions of the
-- window, smallest first; `build`, its content. A drag on its handle moves
-- between them, and past the smallest closes it.
ios.sheet = ui.component(function(props)
	local open = props.open
	local detents = props.detents or { 0.5, 1 }
	local rise = ui.use_state(0)
	local safe = ui.window.safe_area
	local window = ui.window.size
	local detent = 1
	local grabbed = 0
	local start = 0
	local function height()
		return window:get().height
	end
	local function target(index)
		return height() * detents[index] - (if detents[index] >= 1 then safe:get().top + 8 else 0)
	end
	return ui.show(
		function()
			return open:get() or rise:get() > 0
		end,
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			bottom = 0,
			on_mount = function()
				detent = 1
				settle(rise, target(1))
				ui.haptic("light")
			end,
			ui.button {
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				rounded = 0,
				p = 0,
				hover = {},
				active = {},
				label = "Dismiss",
				bg = "#000000",
				opacity = function()
					return 0.25 * clamp(rise:get() / target(1), 0, 1)
				end,
				on_click = function()
					settle(rise, 0, function()
						open:set(false)
					end)
				end,
			},
			ui.glass {
				absolute = true,
				left = 8,
				right = 8,
				bottom = 8,
				radius = 32,
				h = function()
					return math.max(rise:get(), 1)
				end,
				role = "dialog",
				overflow = "hidden",
				ui.div {
					h = 28,
					items = "center",
					justify = "center",
					label = "Sheet handle",
					role = "button",
					on_press = function(event)
						start = event.y
						grabbed = rise:get()
					end,
					on_pointer_move = function(event)
						if start ~= 0 then
							rise:set(clamp(grabbed + start - event.y, 0, height()))
						end
					end,
					on_release = function()
						start = 0
						local at = rise:get()
						if at < target(1) * 0.6 then
							settle(rise, 0, function()
								open:set(false)
							end)
							return
						end
						local nearest, gap = 1, math.huge
						for index in ipairs(detents) do
							local distance = math.abs(target(index) - at)
							if distance < gap then
								nearest, gap = index, distance
							end
						end
						detent = nearest
						ui.haptic("light")
						settle(rise, target(nearest))
					end,
					ui.div { w = 36, h = 5, rounded = 3, bg = "$text.tertiary" },
				},
				props.build(),
			},
		}
	)
end)

return ios
