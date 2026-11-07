-- `ui.ios`: a small set of iOS's standard pieces, drawn by zgx from the system's
-- colours, text sizes, glass and symbols, to UIKit's measurements. What must be
-- exact is the system's own (`ui.native_tabs`, `ui.native_switch`, `ui.alert`).
-- An app can take any of these apart: they use nothing a script can't.
local ui, IS_IOS = ...
local ios = {}

local function clamp(value, low, high)
	return math.max(low, math.min(high, value))
end

-- A navigation controller's decay, measured beside one: remaining distance
-- falls about 4.3 times in each 83 ms, 99% there at about 0.35 s.
local STIFFNESS = 600
-- A navigation controller's, measured beside a real one: a push or pop by a button,
-- which starts a moment after the call; and what a released swipe settles on.
local PUSH_SPRING = { stiffness = 650, damping = 56, delay = 35 }
local POP_SPRING = { stiffness = 650, damping = 56, delay = 30 }
local SWIPE_SPRING = { stiffness = 440, damping = 35.7 }
-- A flick this fast (points a second) leaves past the edge and comes back.
local FLICK = 650
local SCREEN_CORNER = 62
-- How far the page under the top one lies back, of the width.
local PARALLAX = 0.3
local SETTLE_MOST = 1200
-- A release this fast decides, wherever the finger is (points a second).
local FLING = 300
-- A refresh that never says it's done ends after this long (milliseconds).
local REFRESH_MOST = 15000

-- Runs `state` to `target` as a critically damped spring from where it is, in
-- Rust, then `done`. What it returns stops it and says how fast it was going.
local function settle(state, target, done, speed, spring)
	-- Reduce Motion: there at once.
	if ui.prefers.reduced_motion:get() then
		state:set(target)
		if done then
			done()
		end
		return function() end
	end
	spring = spring or {}
	return ui.spring_to(state, target, {
		stiffness = spring.stiffness or STIFFNESS,
		damping = spring.damping or 2 * math.sqrt(STIFFNESS),
		delay = spring.delay,
		from = spring.from,
		velocity = spring.velocity or speed or 0,
		most = SETTLE_MOST,
		done = function()
			ui.gesture_log("settled at " .. target)
			if done then
				done()
			end
		end,
	})
end
ios.settle = settle

-- A soft fade where content passes under an edge. On a window's edge on iOS,
-- the system's own; elsewhere drawn. `edge`, `style`, `size`, `shown`, `drawn`.
function ios.scroll_edge(props)
	local edge = props.edge or "top"
	local hard = props.style == "hard"
	local across = edge == "top" or edge == "bottom"
	local size = props.size or 50
	local hidden = props.shown
			and function()
				local shown = props.shown
				if type(shown) == "function" then
					shown = shown()
				elseif type(shown) ~= "boolean" then
					shown = shown:get()
				end
				return not shown
			end
		or nil
	local box = {
		absolute = true,
		left = if edge ~= "right" then 0 else nil,
		right = if edge ~= "left" then 0 else nil,
		top = if edge ~= "bottom" then 0 else nil,
		bottom = if edge ~= "top" then 0 else nil,
		w = if across then nil else size,
		h = if across then size else nil,
		hidden = hidden,
		pointer_events = false,
		aria_hidden = true,
	}
	if IS_IOS and not props.drawn then
		box.on_mount = function(element)
			ui.native_edge(element, edge, hard)
		end
		return ui.div(box)
	end
	-- Drawn: the blur fades out across the box, under a veil.
	local to_end = { top = 180, bottom = 0, left = 90, right = 270 }
	if not hard then
		box.backdrop_blur = 4
		box.mask_gradient = { angle = to_end[edge], stops = { "#000000", "#000000", "#00000000" } }
	end
	if hard then
		box.bg = "$surface.base"
	else
		box.gradient = { angle = to_end[edge], stops = { "#00000030", "#00000000" } }
	end
	return ui.div(box)
end

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
			wrap = true,
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
			wrap = true,
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
		hidden = props.hidden,
		-- `menu`: a function giving what a long press shows (`ui.system_menu`).
		on_mount = props.menu and function(element)
			ui.system_menu(element, props.menu)
		end or nil,
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
		-- The detail beside the title, or under it where the text is too large
		-- for both, as a Settings row does.
		ui.row {
			ml = if props.icon then 16 else 0,
			grow = 1,
			shrink = 1,
			min_w = 0,
			flex_wrap = true,
			items = "center",
			justify = "between",
			ui.text {
				props.title,
				wrap = true,
				min_w = 0,
				text_size = "$font.size.md",
				color = props.color or "$text.primary",
			},
			props.detail and ui.text {
				props.detail,
				wrap = true,
				min_w = 0,
				text_size = "$font.size.md",
				color = "$text.secondary",
				mr = 8,
			} or nil,
		},
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

-- A bar that goes with the keyboard: a chat's composer. It sits where the app
-- puts it and rises with the keyboard's top edge as the keyboard reaches it, on
-- Core Animation's clock on iOS. Give it layout props only, and its look through
-- its children: what it paints itself stays below, with the page.
function ios.keyboard_bar(props)
	local up = ui.use_state(0)
	local box = {}
	for key, value in props do
		box[key] = value
	end
	box.y = function()
		return -up:get()
	end
	if IS_IOS then
		table.insert(
			box,
			1,
			ui.div {
				absolute = true,
				left = 0,
				right = 0,
				top = 0,
				bottom = 0,
				pointer_events = false,
				aria_hidden = true,
				on_mount = function(element)
					ui.native_keyboard_bar(element, function(lift)
						up:set(lift)
					end)
				end,
			}
		)
	end
	return ui.div(box)
end

-- A chat's composer on a keyboard bar: a small pill at rest that opens into a wider
-- rounded box as it takes focus and closes again, on springs that carry their speed
-- if it changes its mind: width first, height a beat after, corners last, the
-- content settling inside them. It grows with its lines, to `max_lines`, then
-- scrolls inside. `value` (a state), `placeholder`, `on_send` (the text; a send
-- button comes when there is any), `on_plus`, `bottom`.
function ios.composer(props)
	local value = props.value
	local focused = ui.use_state(false)
	local pressed = ui.use_state(false)
	local still = ui.prefers.reduced_motion:get()
	local max_lines = props.max_lines or 6
	-- Opening has some life in it; closing is quicker and doesn't bounce.
	local OPEN = { response = 420, bounce = 0.25 }
	local OPEN_TALL = { response = 420, bounce = 0.2 }
	local CLOSE = { response = 280, bounce = 0 }
	local function delayed(spring, delay)
		return { response = spring.response, bounce = spring.bounce, delay = delay }
	end
	-- A number that is `rest` closed and `wide` open, on `opening`'s spring. Reduced
	-- motion: at once, and the crossfade below shows the change.
	local function channel(read, rest, wide, opening, closing)
		local goal = ui.computed(function()
			return if read:get() then wide else rest
		end)
		if still then
			return ui.tween(goal, { duration = 1 })
		end
		local closes = closing or CLOSE
		if wide > rest then
			return ui.spring(goal, { up = opening, down = closes })
		end
		return ui.spring(goal, { down = opening, up = closes })
	end
	local has_text = ui.computed(function()
		return value:get() ~= ""
	end)
	-- The send button pops in, and is gone quicker than it came.
	local POP = { response = 300, bounce = 0.35 }
	local GONE = { response = 160, bounce = 0 }
	-- One line's height and the lines' now, measured from the text under the editor.
	local line = ui.use_state(22)
	local measured = ui.use_state(22)
	local height = ui.computed(function()
		return math.min(measured:get(), line:get() * max_lines)
	end)
	local fade = ui.use_state(1)
	return ios.keyboard_bar {
		absolute = true,
		left = channel(focused, 36, 8, OPEN),
		right = channel(focused, 36, 8, OPEN),
		bottom = props.bottom or 100,
		ui.glass {
			radius = channel(focused, 22, 18, delayed({ response = 480, bounce = 0.15 }, 70)),
			interactive = true,
			direction = "row",
			items = "end",
			min_h = channel(focused, 44, 56, delayed(OPEN_TALL, 35)),
			px = 6,
			py = channel(focused, 0, 6, delayed(OPEN_TALL, 35)),
			opacity = if still then ui.tween(fade, { duration = 120 }) else nil,
			-- Alive at first contact, before focus lands.
			scale = channel(pressed, 1, 0.975, { response = 150, bounce = 0 }, {
				response = 320,
				bounce = 0.2,
			}),
			on_press = function()
				pressed:set(true)
			end,
			on_release = function()
				pressed:set(false)
			end,
			ui.button {
				w = 32,
				h = 44,
				p = 0,
				bg = "#00000000",
				hover = {},
				active = {},
				label = "Add",
				items = "center",
				justify = "center",
				on_click = props.on_plus,
				x = channel(focused, 0, 2, delayed(OPEN, 40)),
				scale = channel(focused, 0.88, 1, delayed({ response = 360, bounce = 0.3 }, 40)),
				ui.symbol { name = "plus", size = 18, weight = 500, color = "$text.secondary" },
			},
			-- The lines' height comes from their text, laid out unseen beneath the
			-- editor, which fills what that makes and scrolls past the limit.
			ui.div {
				grow = 1,
				min_w = 0,
				ml = channel(focused, 2, 6, delayed(OPEN, 70)),
				mr = 4,
				my = 11,
				h = if still
					then ui.tween(height, { duration = 1 })
					else ui.spring(height, { response = 320, bounce = 0.08 }),
				overflow = "hidden",
				ui.text {
					function()
						local text = value:get()
						return if text == "" then " " else text .. " "
					end,
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					wrap = true,
					opacity = 0,
					aria_hidden = true,
					pointer_events = false,
					on_resize = function(size)
						if value:get() == "" then
							line:set(size.h)
						end
						measured:set(size.h)
					end,
				},
				ui.textarea {
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					h = "100%",
					value = value,
					label = props.placeholder or "Message",
					placeholder = props.placeholder or "Message",
					wrap = true,
					p = 0,
					bg = "#00000000",
					border = 0,
					focus = {},
					on_focus = function()
						focused:set(true)
						pressed:set(false)
						if still then
							fade:set(0.6)
							ui.after(120, function()
								fade:set(1)
							end)
						end
					end,
					on_blur = function()
						focused:set(false)
						if still then
							fade:set(0.6)
							ui.after(120, function()
								fade:set(1)
							end)
						end
					end,
				},
			},
			ui.button {
				w = channel(has_text, 0, 36, POP, GONE),
				h = 44,
				p = 0,
				bg = "#00000000",
				hover = {},
				active = {},
				label = "Send",
				items = "center",
				justify = "center",
				overflow = "hidden",
				opacity = channel(has_text, 0, 1, { response = 180, bounce = 0 }, GONE),
				scale = channel(has_text, 0.6, 1, POP, GONE),
				pointer_events = function()
					return has_text:get()
				end,
				aria_hidden = function()
					return not has_text:get()
				end,
				on_click = function()
					if props.on_send then
						props.on_send(value:get())
					end
					value:set("")
				end,
				ui.symbol { name = "arrow.up.circle.fill", size = 30, color = "$ios.blue" },
			},
		},
	}
end

-- A search field on glass, the system's keyboard over zgx's editor.
function ios.search(props)
	return ui.glass {
		mx = props.mx or 16,
		min_h = 48,
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
			-- The system's own: its spinner, its threshold, and it holds the page
			-- down until `done`, which a page that forgets still gets in time.
			on_mount = function(element)
				if not props.on_refresh then
					return
				end
				ui.pull_refresh {
					element = element,
					top = function()
						return safe:get().top + 44
					end,
					on_refresh = function(done)
						ui.haptic("medium")
						local finished = false
						local function finish()
							if not finished then
								finished = true
								done()
							end
						end
						ui.after(REFRESH_MOST, finish)
						props.on_refresh(finish)
					end,
				}
			end,
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
		-- The system's edge under the bar as content passes under it, then its
		-- small title and its buttons on glass, above it.
		ios.scroll_edge {
			size = function()
				return safe:get().top + 44
			end,
			shown = function()
				return offset:get() > 0
			end,
		},
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = 0,
			h = function()
				return safe:get().top + 50
			end,
			pointer_events = false,
			-- Eased, as a bar's title fades: a page raised for the keyboard
			-- arrives at its offset all at once.
			opacity = ui.tween(ui.computed(collapsed), { duration = 150 }),
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
	-- A spring each: one taking over from the other would leave that one's page
	-- stopped where it was, with nothing to move it on.
	local running_drag, running_enter
	-- From rest a button's waits as a navigation controller does; a change of mind
	-- keeps the speed it had and goes at once.
	local function from_button(spring, carried)
		return if carried ~= 0
			then { stiffness = spring.stiffness, damping = spring.damping, velocity = carried }
			else spring
	end
	local function run_drag(target, done, spring)
		local carried = if running_drag then running_drag() else 0
		running_drag = settle(drag, target, function()
			running_drag = nil
			if done then
				done()
			end
		end, nil, spring or from_button(POP_SPRING, carried))
	end
	local function run_enter()
		local carried = if running_enter then running_enter() else 0
		running_enter = settle(entering, 0, function()
			running_enter = nil
		end, nil, from_button(PUSH_SPRING, carried))
	end
	-- What a released swipe does, as a navigation controller's: a flick home leaves
	-- from behind the finger and quicker than it, so it passes the edge and comes back;
	-- anything else starts from rest a moment later.
	local function released(target, vx, at)
		if target ~= 0 and vx >= FLICK then
			return {
				stiffness = SWIPE_SPRING.stiffness,
				damping = SWIPE_SPRING.damping,
				from = at - (0.115 * vx - 30),
				velocity = 6 * vx - 1000,
			}
		end
		return { stiffness = SWIPE_SPRING.stiffness, damping = SWIPE_SPRING.damping, delay = 12 }
	end
	-- Nothing may rest part way: whatever got here, a page out of place with no
	-- spring on it goes home, and says so in the record.
	local function recover()
		local at = drag:get()
		if not running_drag and at ~= 0 then
			ui.gesture_log("recovered: drag " .. at .. " with nothing moving it")
			run_drag(0)
		end
		local coming = entering:get()
		if not running_enter and coming ~= 0 then
			ui.gesture_log("recovered: entering " .. coming .. " with nothing moving it")
			run_enter()
		end
	end
	local holding = false
	-- Looked at after each gesture and page change, never while idle.
	local function check_soon()
		ui.after(SETTLE_MOST + 300, function()
			if not holding then
				recover()
			end
		end)
	end
	function nav.push(build)
		next_id += 1
		builders[next_id] = build
		local list = table.clone(pages:get())
		table.insert(list, next_id)
		entering:set(width)
		pages:set(list)
		run_enter()
		check_soon()
	end
	local function finish_pop(spring)
		run_drag(width, function()
			local kept = table.clone(pages:get())
			builders[table.remove(kept)] = nil
			pages:set(kept)
			drag:set(0)
		end, spring)
	end
	function nav.pop()
		if #pages:get() < 2 then
			return
		end
		finish_pop()
		check_soon()
	end
	local start = 0
	-- iOS recognizes the swipe, and tells every ending; the page only follows.
	local function shows()
		return not props.active or props.active()
	end
	ui.native_pan {
		edge = "left",
		can_begin = function()
			return #pages:get() > 1 and shows()
		end,
		on_pan = function(e)
			if e.phase == "began" then
				holding = true
				if running_drag then
					running_drag()
					running_drag = nil
				end
				if running_enter then
					running_enter()
					running_enter = nil
				end
				if entering:get() ~= 0 then
					entering:set(0)
				end
				start = drag:get()
			elseif e.phase == "changed" then
				drag:set(clamp(start + e.x, 0, width))
			elseif e.phase == "ended" then
				holding = false
				check_soon()
				-- Where the finger left it, which its last move may not have drawn.
				local at = clamp(start + e.x, 0, width)
				drag:set(at)
				if e.vx > FLING or (at > width / 2 and e.vx > -FLING) then
					finish_pop(released(width, e.vx, at))
				else
					run_drag(0, nil, released(0, e.vx, at))
				end
			else
				holding = false
				check_soon()
				run_drag(0, nil, released(0, 0, drag:get()))
			end
		end,
	}
	-- VoiceOver's two-finger scrub back. `active`: whether this stack shows.
	ui.on_escape(function()
		if #pages:get() < 2 or (props.active and not props.active()) then
			return false
		end
		nav.pop()
		return true
	end)
	return ui.div {
		w = "100%",
		h = "100%",
		overflow = "hidden",
		on_resize = function(size)
			width = size.w
		end,
		ui.each(pages, function(id, index)
			-- What lies under the top page moves as a navigation controller's does.
			local function page_x(at)
				local depth = #pages:get()
				local moved = drag:get() + entering:get()
				if at == depth then
					return moved
				elseif at == depth - 1 then
					return (moved - width) * PARALLAX
				end
				return -width * PARALLAX
			end
			local on_top = function()
				return index == #pages:get()
			end
			return ui.div {
				absolute = true,
				left = 0,
				top = 0,
				w = "100%",
				h = "100%",
				x = function()
					return page_x(index)
				end,
				-- A page in motion has the screen's rounded corners.
				rounded = function()
					return if index == #pages:get()
							and index > 1
							and drag:get() + entering:get() ~= 0
						then SCREEN_CORNER
						else 0
				end,
				overflow = "hidden",
				-- Quite covered once it settles: it shows nothing, and asks no
				-- plane of its own.
				hidden = function()
					local depth = #pages:get()
					return index < depth - 1 or (index < depth and drag:get() == 0 and entering:get() == 0)
				end,
				pointer_events = on_top,
				Page { build = builders[id], nav = nav },
				-- What a navigation controller lays over the page behind, and the
				-- soft edge of the page coming over it (measured beside one:
				-- black at 10%, gone as the top page goes; a 9 point edge).
				ui.div {
					absolute = true,
					left = 0,
					top = 0,
					right = 0,
					bottom = 0,
					bg = "#000000",
					pointer_events = false,
					aria_hidden = true,
					hidden = function()
						return index ~= #pages:get() - 1
					end,
					opacity = function()
						return 0.1 * (1 - clamp((drag:get() + entering:get()) / width, 0, 1))
					end,
				},
				ui.div {
					absolute = true,
					left = -9,
					top = 0,
					bottom = 0,
					w = 9,
					pointer_events = false,
					aria_hidden = true,
					gradient = { angle = 270, stops = { "#00000018", "#00000000" } },
					hidden = function()
						return index ~= #pages:get() or index == 1
					end,
					opacity = function()
						local entered = entering:get()
						if entered ~= 0 then
							return clamp(entered / width, 0, 1)
						end
						local moved = drag:get()
						return if moved > 0 then 1 - clamp(moved / width, 0, 1) else 0
					end,
				},
			}
		end, { display = "contents" }),
		-- One back button over the pages, as a navigation controller's: it stays
		-- where it is and fades as a first page comes and goes.
		ui.glass {
			absolute = true,
			left = 16,
			top = function()
				return ui.window.safe_area:get().top + 3
			end,
			w = 44,
			h = 44,
			radius = 22,
			interactive = true,
			hidden = function()
				return #pages:get() < 2
			end,
			opacity = function()
				if #pages:get() > 2 then
					return 1
				end
				local moved = (drag:get() + entering:get()) / width
				return clamp((0.88 - moved) / 0.78, 0, 1)
			end,
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
					nav.pop()
				end,
				ui.symbol { name = "chevron.backward", size = 18, weight = 600, color = "$text.primary" },
			},
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
	local function height()
		return window:get().height
	end
	local function target(index)
		return height() * detents[index] - (if detents[index] >= 1 then safe:get().top + 8 else 0)
	end
	local running
	local function run(state, to, done, speed)
		local carried = if running then running() else 0
		running = settle(state, to, done, speed or carried)
	end
	local function dismiss(speed)
		run(rise, 0, function()
			open:set(false)
		end, if type(speed) == "number" then speed else 0)
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
				run(rise, target(1))
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
				on_click = dismiss,
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
				-- iOS recognizes the drag, anywhere on the sheet, and tells every
				-- ending. Up is more of the sheet, so the speeds are negated.
				on_mount = function(element)
					ui.native_pan {
						element = element,
						on_pan = function(e)
							if e.phase == "began" then
								if running then
									running()
									running = nil
								end
								grabbed = rise:get()
							elseif e.phase == "changed" then
								rise:set(clamp(grabbed - e.y, 0, height()))
							else
								local speed = -e.vy
								-- Where it would come to rest, as UIKit picks a detent.
								local aim = rise:get() + speed * 0.2
								if e.phase == "ended" and aim < target(1) * 0.5 then
									dismiss(speed)
									return
								end
								local nearest, gap = 1, math.huge
								for index in ipairs(detents) do
									local distance = math.abs(target(index) - aim)
									if distance < gap then
										nearest, gap = index, distance
									end
								end
								detent = nearest
								run(rise, target(nearest), nil, speed)
							end
						end,
					}
				end,
				ui.div {
					h = 28,
					items = "center",
					justify = "center",
					label = "Sheet handle",
					role = "button",
					ui.div { w = 36, h = 5, rounded = 3, bg = "$text.tertiary" },
				},
				props.build(),
			},
		}
	)
end)

return ios
