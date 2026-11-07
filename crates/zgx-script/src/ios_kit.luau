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
-- A spring said as UIKit says one: damping ratio and response (seconds).
local function uikit_spring(ratio, response)
	local turn = 2 * math.pi / response
	return { stiffness = turn * turn, damping = 2 * ratio * turn }
end
-- A navigation bar's, read from UIKit (iOS 26.5) as a table under it scrolls:
-- 54 points tall, 106 with its large title; the large title's top 57.7 below
-- the safe area at 34 points, the small one's 11.7 at 17. Once the large title
-- has scrolled under the bar, the large title fades out on a 0.2 s
-- ease-in-out (its own CABasicAnimation) and the small one fades in rising 15
-- points on a critically damped spring of 0.447 s (fit to UIKit's frames, rms
-- 0.001: SwiftUI hosts it, so its spring isn't read yet). Both run by time,
-- not with the scroll: the same at two drag speeds.
local BAR, BAR_LARGE = 54, 106
local LARGE_TITLE_TOP, LARGE_TITLE_HEIGHT = 57.7, 40.7
local SMALL_TITLE_TOP, SMALL_TITLE_HEIGHT, SMALL_TITLE_RISE = 11.7, 20.7, 15
local LARGE_TITLE_EASE = "cubicBezier(0.42, 0, 0.58, 1)"
local TITLE_SPRING = uikit_spring(1, 0.447)
-- A navigation controller's own, read from UIKit as it runs: a push or pop by a
-- button runs its progress on this spring from 8.75 a second; a finger's position
-- is followed through the tracking spring, and a release runs on the last.
local NAV_BUTTON = uikit_spring(1, 0.3)
local NAV_BUTTON_SPEED = 8.75
-- When it starts after the call, measured beside UIKit on an iPhone at 120 Hz (ms).
local NAV_PUSH_START = 53
local NAV_POP_START = 40
-- The back button's chevron fades in and out on its own critically damped
-- spring of 0.415 s, read off the glass's content view in UIKit (response
-- 0.414 s on a push, 50 ms after the page starts, rms 0.001; 0.418 s on a pop,
-- with the page); its glass shows at once and goes when the pop is done.
local BACK_FADE = uikit_spring(1, 0.415)
local BACK_FADE_LAG = 50
local NAV_TRACKING = uikit_spring(0.85, 0.08)
local NAV_RELEASE = uikit_spring(0.85, 0.3)
-- A swipe's release, as UIKit's interactive pop (`_UINavigationInteractiveTransitionBase`)
-- decides it, read from its code: each finger sample's speed (in widths a second)
-- and the change in it a second, each averaged over the last three samples; a
-- sample under a millisecond after the last, or under ten with the finger where it
-- was, doesn't count, and a change in acceleration is held within 5 of the last.
-- It goes back when where the page would be after the transition's 0.35 s at that
-- speed and acceleration is past 187.5 points or halfway, whichever is less, and
-- leaves at the finger's speed, 4.25 times it while still a flick: every average
-- since the finger landed at 1.5 widths a second or more.
local NAV_PROJECTION = 0.35
local NAV_BACK_POINTS = 187.5
local NAV_FLICK_AVERAGE = 1.5
local NAV_FLICK_BOOST = 4.25
local NAV_ACCELERATION_STEP = 5
local function swipe_statistics()
	local s = {}
	local function average(ring, now)
		local n = s.n
		local slot = n % 3 + 1
		local value
		if n <= 2 then
			value = (s[ring .. "_average"] * n + now) / (n + 1)
		else
			value = (s[ring .. "_average"] * 3 - s[ring][slot] + now) / 3
		end
		s[ring][slot] = now
		s[ring .. "_average"] = value
	end
	-- `t` in seconds, `along` and `speed` in widths and widths a second.
	function s.first(t, along, speed)
		s.n, s.time, s.skip, s.along = 1, t, t, math.max(along, 0)
		s.speed, s.acceleration = speed, 0
		s.speeds, s.accelerations = { speed, 0, 0 }, { 0, 0, 0 }
		s.speeds_average, s.accelerations_average = speed, 0
		s.flick = true
	end
	function s.sample(t, along, speed)
		if not s.n then
			s.first(t, along, speed)
			return
		end
		along = math.max(along, 0)
		local since = t - s.skip
		s.skip = t
		if since < 0.01 and (since <= 0.001 or along == s.along) then
			return
		end
		local dt = t - s.time
		average("speeds", speed)
		if s.flick and math.abs(s.speeds_average) < NAV_FLICK_AVERAGE then
			s.flick = false
		end
		local acceleration = (speed - s.speed) / dt
		if math.abs(acceleration - s.acceleration) > NAV_ACCELERATION_STEP then
			acceleration = s.acceleration
				+ (if acceleration > s.acceleration then NAV_ACCELERATION_STEP else -NAV_ACCELERATION_STEP)
		end
		average("accelerations", acceleration)
		s.acceleration, s.speed, s.along, s.time = acceleration, speed, along, t
		s.n += 1
	end
	-- Whether a release at `along` (0 to 1) goes back, for a page `width` wide.
	function s.goes_back(along, width)
		local d = NAV_PROJECTION
		local projected = along + s.speeds_average * d + 0.5 * s.accelerations_average * d * d
		return projected > math.min(NAV_BACK_POINTS / width, 0.5)
	end
	-- The release's speed in points a second.
	function s.release_speed(width)
		return s.speed * width * (if s.flick then NAV_FLICK_BOOST else 1)
	end
	return s
end
-- How far the page under the top one lies back, of the width.
local PARALLAX = 0.3
local SETTLE_MOST = 1200
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

-- A chat's composer on a keyboard bar: a thin one-line pill at rest that opens into
-- a taller box as it takes focus, the text above and a row for its buttons below,
-- and closes again, on the keyboard's own spring so it finishes as the keyboard
-- does. It grows with its lines, to `max_lines`, then scrolls inside. `value` (a
-- state), `placeholder`, `on_send` (the text; a send button comes when there is
-- any), `on_plus`, `bottom`, `ref` (its editor's).
function ios.composer(props)
	local value = props.value
	local focused = ui.use_state(false)
	local still = ui.prefers.reduced_motion:get()
	local max_lines = props.max_lines or 6
	-- Everything moves on the keyboard's own spring, read from UIKit as it runs
	-- (mass 1, stiffness 555, damping 47.1, the same up and down), all at once.
	local KEYBOARD = { stiffness = 555.0265, damping = 47.118 }
	local function eased(goal)
		if still then
			return ui.tween(goal, { duration = 1 })
		end
		return ui.spring(goal, KEYBOARD)
	end
	-- A number that is `rest` closed and `open` open. Reduced motion: at once, and
	-- the crossfade below shows the change.
	local function channel(read, rest, open)
		return eased(ui.computed(function()
			return if read:get() then open else rest
		end))
	end
	local has_text = ui.computed(function()
		return value:get() ~= ""
	end)
	-- One line's height and the lines' now, measured from the text under the editor.
	local line = ui.use_state(22)
	local measured = ui.use_state(22)
	-- Open, room for two lines at least.
	local height = ui.computed(function()
		local lines = math.min(measured:get(), line:get() * max_lines)
		return if focused:get() then math.max(lines, line:get() * 2) else lines
	end)
	local fade = ui.use_state(1)
	local ROW = 40
	local function button(options)
		return ui.button {
			absolute = true,
			bottom = 0,
			left = options.left,
			right = options.right,
			w = options.w,
			h = ROW,
			p = 0,
			bg = "#00000000",
			hover = {},
			active = {},
			label = options.label,
			items = "center",
			justify = "center",
			overflow = "hidden",
			opacity = options.opacity,
			scale = options.scale,
			pointer_events = options.pointer_events,
			aria_hidden = options.aria_hidden,
			on_click = options.on_click,
			options.symbol,
		}
	end
	return ios.keyboard_bar {
		absolute = true,
		left = channel(focused, 36, 8),
		right = channel(focused, 36, 8),
		bottom = props.bottom or 100,
		ui.glass {
			radius = channel(focused, 20, 24),
			interactive = true,
			direction = "column",
			-- `log_motion`: its size each frame in the gesture log, to compare with
			-- the keyboard's.
			on_resize = props.log_motion and function(size)
				ui.gesture_log(string.format("COMPOSER %.1f w %.1f h %.1f", ui.now(), size.w, size.h))
			end or nil,
			opacity = if still then ui.tween(fade, { duration = 120 }) else nil,
			-- The text: beside the buttons at rest, above them open.
			ui.div {
				ml = channel(focused, 38, 14),
				mr = channel(focused, 42, 14),
				mt = channel(focused, 9, 12),
				mb = 9,
				h = eased(height),
				overflow = "hidden",
				-- The lines' height comes from their text, laid out unseen beneath
				-- the editor, which fills what that makes and scrolls past the limit.
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
					ref = props.ref,
					wrap = true,
					p = 0,
					bg = "#00000000",
					border = 0,
					focus = {},
					on_focus = function()
						if props.log_motion then
							ui.gesture_log(string.format("COMPOSER %.1f focus", ui.now()))
						end
						focused:set(true)
						if still then
							fade:set(0.6)
							ui.after(120, function()
								fade:set(1)
							end)
						end
					end,
					on_blur = function()
						if props.log_motion then
							ui.gesture_log(string.format("COMPOSER %.1f blur", ui.now()))
						end
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
			-- The buttons' row: nothing at rest, where they sit beside the text.
			ui.div { h = channel(focused, 0, ROW - 9) },
			button {
				left = 4,
				w = 32,
				label = "Add",
				on_click = props.on_plus,
				symbol = ui.symbol { name = "plus", size = 18, weight = 500, color = "$text.secondary" },
			},
			button {
				right = 4,
				w = 36,
				label = "Send",
				opacity = channel(has_text, 0, 1),
				scale = channel(has_text, 0.6, 1),
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
				symbol = ui.symbol { name = "arrow.up.circle.fill", size = 30, color = "$ios.blue" },
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
	-- Collapsed once the large title has scrolled under the bar.
	local collapsed = ui.computed(function()
		return offset:get() >= BAR_LARGE - BAR
	end)
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
					return safe:get().top + BAR
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
					mt = LARGE_TITLE_TOP - BAR,
					mb = BAR_LARGE - LARGE_TITLE_TOP - LARGE_TITLE_HEIGHT,
					text_size = 34,
					weight = 700,
					color = "$text.primary",
					opacity = ui.tween(collapsed, { duration = 200, ease = LARGE_TITLE_EASE, from = 1, to = 0 }),
				},
				children,
			},
		},
		-- The system's edge under the bar as content passes under it, then its
		-- small title and its buttons on glass, above it.
		ios.scroll_edge {
			size = function()
				return safe:get().top + BAR
			end,
			shown = function()
				return offset:get() > 0
			end,
		},
		ui.div {
			absolute = true,
			left = 0,
			right = 0,
			top = function()
				return safe:get().top + SMALL_TITLE_TOP
			end,
			h = SMALL_TITLE_HEIGHT,
			pointer_events = false,
			opacity = ui.spring(collapsed, { stiffness = TITLE_SPRING.stiffness, damping = TITLE_SPRING.damping }),
			y = ui.spring(
				collapsed,
				{ stiffness = TITLE_SPRING.stiffness, damping = TITLE_SPRING.damping, from = SMALL_TITLE_RISE, to = 0 }
			),
			items = "center",
			justify = "center",
			ui.text {
				props.title,
				text_size = 17,
				weight = 600,
				color = "$text.primary",
				aria_hidden = true,
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
	-- A button's starts at a navigation controller's speed; a change of mind keeps
	-- the speed it had.
	local function from_button(direction, carried)
		if carried ~= 0 then
			return { stiffness = NAV_BUTTON.stiffness, damping = NAV_BUTTON.damping, velocity = carried }
		end
		return {
			stiffness = NAV_BUTTON.stiffness,
			damping = NAV_BUTTON.damping,
			velocity = direction * NAV_BUTTON_SPEED * width,
			delay = if direction > 0 then NAV_POP_START else NAV_PUSH_START,
		}
	end
	local function run_drag(target, done, spring)
		local carried = if running_drag then running_drag() else 0
		running_drag = settle(drag, target, function()
			running_drag = nil
			if done then
				done()
			end
		end, nil, spring or from_button(1, carried))
	end
	local function run_enter()
		local carried = if running_enter then running_enter() else 0
		running_enter = settle(entering, 0, function()
			running_enter = nil
		end, nil, from_button(-1, carried))
	end
	-- A swipe's page follows the finger through a spring, and a release carries
	-- that spring's speed home.
	local function follow(target)
		local carried = if running_drag then running_drag() else 0
		running_drag = settle(drag, target, function()
			running_drag = nil
		end, nil, { stiffness = NAV_TRACKING.stiffness, damping = NAV_TRACKING.damping, velocity = carried })
	end
	-- Where the page was when the finger landed.
	local start = 0
	-- The finger's samples, as UIKit's interactive pop keeps them.
	local stats = swipe_statistics()
	local function sample(e)
		stats.sample(ui.now() / 1000, clamp(start + e.x, 0, width) / width, e.vx / width)
	end
	-- From where it is, at the speed UIKit would give it.
	local function released(speed)
		if running_drag then
			running_drag()
		end
		running_drag = nil
		return { stiffness = NAV_RELEASE.stiffness, damping = NAV_RELEASE.damping, velocity = speed }
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
	-- The back button's chevron, 0 to 1.
	local chevron = ui.use_state(0)
	local running_chevron
	local function fade_chevron(to, delay)
		if running_chevron then
			running_chevron()
		end
		running_chevron = settle(chevron, to, function()
			running_chevron = nil
		end, nil, { stiffness = BACK_FADE.stiffness, damping = BACK_FADE.damping, delay = delay })
	end
	-- `log_motion`: each position in the gesture log, to compare with UIKit's.
	if props.log_motion then
		ui.effect(function()
			ui.gesture_log(
				string.format(
					"POS %.1f drag %.2f enter %.2f chevron %.3f",
					ui.now(),
					drag:get(),
					entering:get(),
					chevron:get()
				)
			)
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
		if #list == 2 then
			chevron:set(0)
			fade_chevron(1, NAV_PUSH_START + BACK_FADE_LAG)
		end
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
		if #pages:get() == 2 then
			fade_chevron(0, NAV_POP_START)
		end
		check_soon()
	end
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
				stats = swipe_statistics()
				sample(e)
				if running_chevron then
					running_chevron()
					running_chevron = nil
				end
			elseif e.phase == "changed" then
				sample(e)
				local at = clamp(start + e.x, 0, width)
				follow(at)
				-- Not yet read from UIKit: under a finger, the chevron goes as the page.
				if #pages:get() == 2 then
					chevron:set(1 - at / width)
				end
			elseif e.phase == "ended" then
				holding = false
				check_soon()
				sample(e)
				local at = clamp(start + e.x, 0, width)
				local speed = stats.release_speed(width)
				local last = #pages:get() == 2
				if stats.goes_back(at / width, width) then
					finish_pop(released(speed))
					if last then
						fade_chevron(0)
					end
				else
					run_drag(0, nil, released(speed))
					if last then
						fade_chevron(1)
					end
				end
			else
				holding = false
				check_soon()
				run_drag(0, nil, released(0))
				if #pages:get() == 2 then
					fade_chevron(1)
				end
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
				-- A page in motion is a card with a navigation controller's faint shadow.
				shadow = function()
					return if drag:get() + entering:get() ~= 0
						then { x = 0, y = -3, blur = 12, color = "#0000000a" }
						else false
				end,
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
		-- One back button over the pages, as a navigation controller's: its glass
		-- shows from a push and goes when the pop is done; its chevron fades.
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
				opacity = chevron,
				on_click = function()
					nav.pop()
				end,
				ui.symbol { name = "chevron.backward", size = 18, weight = 600, color = "$text.primary" },
			},
		},
	}
end)

-- A sheet that rises from the bottom over a dimmed page, on glass where the
-- system's are: `open`, a state; `detents`, "medium" and "large" (or
-- heights in points), smallest first; `build`, its content. A drag on it moves
-- between them, and past the smallest closes it.
--
-- As UIKit's (`UISheetPresentationController`, iOS 26), read from it: every
-- move is a critically damped spring of response 0.344 s (mass 1, stiffness
-- 333.33, damping 36.515: the springs it adds to the sheet's layers on a
-- detent change; present and dismiss follow the same curve, within 1 pt).
-- The largest detent is the window less its safe area at both ends; medium is
-- 0.56 of that (0.63 in a window 568 points tall or less), as its detent block
-- has it; the sheet is a detent tall plus the bottom safe area. Medium is the
-- full-width sheet scaled to leave 8 points at the sides and bottom, top
-- corners 56.2 (62 at large), bottom ones 38; large is opaque, on the page's
-- background. The page under it dims to 20% black.
local SHEET_SPRING = { stiffness = 333.3333, damping = 36.5148 }
local SHEET_MEDIUM, SHEET_MEDIUM_SHORT, SHEET_SHORT = 0.56, 0.63, 568
local SHEET_INSET = 8
local SHEET_CORNERS_MEDIUM, SHEET_CORNERS_LARGE, SHEET_CORNERS_BOTTOM = 56.2, 62, 38
local SHEET_DIM = 0.2
-- A drag's release, read from `_UISheetInteraction draggingEndedInSource:` and
-- its `_UIHyperInteractor`: a speed under 250 points a second counts as none;
-- the release projects as a scroll view does at deceleration rate 0.99
-- (speed x 0.99 / 0.01 / 1000) and goes to the detent nearest that, the
-- bottom of the window (dismissed) one of them, on the release spring. A flick
-- of 1000 a second or more whose projection stays nearest the detent it
-- left, already past that detent its way, goes one detent on instead, on the
-- high-speed spring (damping 0.8).
local SHEET_MIN_SPEED, SHEET_FLICK = 250, 1000
local SHEET_PROJECTION = 0.99 / 0.01 / 1000
local SHEET_RELEASE = uikit_spring(1, 0.3441442326)
local SHEET_RELEASE_FAST = uikit_spring(0.8, 0.3441442326)
-- Past the largest detent it stretches, coefficient 0.55 read from its
-- interactor; the curve is UIScrollView's, not yet read from the sheet's.
local SHEET_RUBBER = 0.55
ios.sheet = ui.component(function(props)
	local open = props.open
	local detents = props.detents or { "medium", "large" }
	local rise = ui.use_state(0)
	local safe = ui.window.safe_area
	local window = ui.window.size
	local detent = 1
	local grabbed = 0
	local function height()
		return window:get().height
	end
	-- The sheet's height at detent `index`.
	local function target(index)
		local area = safe:get()
		local most = height() - area.top - area.bottom
		local d = detents[index]
		local value = if d == "large"
			then most
			elseif d == "medium" then most * (if height() > SHEET_SHORT then SHEET_MEDIUM else SHEET_MEDIUM_SHORT)
			else math.min(d, most)
		return value + area.bottom
	end
	-- How far from the smallest detent toward the largest it is, 0 to 1.
	local function largeness()
		local low, high = target(1), target(#detents)
		if high <= low then
			return 1
		end
		return clamp((rise:get() - low) / (high - low), 0, 1)
	end
	-- `log_motion`: its top each frame in the gesture log, to compare with UIKit's.
	if props.log_motion then
		ui.effect(function()
			ui.gesture_log(string.format("SHEET %.1f top %.2f", ui.now(), height() - rise:get()))
		end)
	end
	local corners = ui.computed(function()
		local top = SHEET_CORNERS_MEDIUM + (SHEET_CORNERS_LARGE - SHEET_CORNERS_MEDIUM) * largeness()
		return { tl = top, tr = top, br = SHEET_CORNERS_BOTTOM, bl = SHEET_CORNERS_BOTTOM }
	end)
	local running
	local function run(state, to, done, speed, spring)
		local carried = if running then running() else 0
		running = settle(state, to, done, speed or carried, spring or SHEET_SPRING)
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
					return SHEET_DIM * clamp(rise:get() / target(1), 0, 1)
				end,
				on_click = dismiss,
			},
			ui.glass {
				absolute = true,
				left = 0,
				right = 0,
				bottom = 0,
				h = function()
					return math.max(rise:get(), 1)
				end,
				-- Medium is the whole sheet made smaller, about its bottom edge,
				-- and lifted off it: 8 points clear at the sides and bottom.
				origin = { 0.5, 1 },
				scale = function()
					local width = math.max(window:get().width, 1)
					local small = (width - 2 * SHEET_INSET) / width
					return small + (1 - small) * largeness()
				end,
				y = function()
					return -SHEET_INSET * (1 - largeness())
				end,
				corners = corners,
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
								local wanted = grabbed - e.y
								local most = target(#detents)
								if wanted > most then
									-- Stretching past the largest, as a scroll view past its end.
									local room = height()
									wanted = most + (1 - 1 / ((wanted - most) * SHEET_RUBBER / room + 1)) * room
								end
								rise:set(math.max(wanted, 0))
							else
								-- In UIKit's terms: the sheet's top, down positive.
								local h = height()
								local top = h - rise:get()
								local v = if e.phase == "ended" then e.vy else 0
								local moving = if math.abs(v) >= SHEET_MIN_SPEED then v else 0
								local projected = top + moving * SHEET_PROJECTION
								-- Its places, top first: each detent, then dismissed.
								local places = {}
								for index = #detents, 1, -1 do
									table.insert(places, h - target(index))
								end
								table.insert(places, h)
								local function nearest(y)
									local best, gap = 1, math.huge
									for index, at in ipairs(places) do
										if math.abs(at - y) < gap then
											best, gap = index, math.abs(at - y)
										end
									end
									return best
								end
								local left, landing = nearest(top), nearest(projected)
								local spring = SHEET_RELEASE
								if
									math.abs(v) >= SHEET_FLICK
									and left == landing
									and (v < 0) ~= (top >= places[left])
								then
									landing = clamp(left + (if v > 0 then 1 else -1), 1, #places)
									spring = SHEET_RELEASE_FAST
								end
								local speed = -v
								if landing == #places then
									run(rise, 0, function()
										open:set(false)
									end, speed, spring)
									return
								end
								detent = #detents + 1 - landing
								run(rise, target(detent), nil, speed, spring)
							end
						end,
					}
				end,
				-- Large is opaque, on the page's background.
				ui.div {
					absolute = true,
					left = 0,
					right = 0,
					top = 0,
					bottom = 0,
					bg = "$surface.base",
					opacity = largeness,
				},
				-- UIKit's grabber: 36 by 5, 5 points below the top.
				ui.div {
					h = 15,
					pt = 5,
					items = "center",
					label = "Sheet handle",
					role = "button",
					ui.div { w = 36, h = 5, rounded = 2.5, bg = "$text.tertiary" },
				},
				props.build(),
			},
		}
	)
end)

return ios
