-- A native-feeling iOS app in zgx alone: tab bar, navigation stack with a large
-- title and a swipe back, grouped lists, a sheet, an alert and an action sheet,
-- a search field with the system's keyboard, pull to refresh, and glass.
local ios = ui.ios
local tab = ui.state("tab", 1)
local airplane = ui.state("airplane", false)
local query = ui.state("query", "")
local sheet_open = ui.state("sheet_open", false)
local refreshed = ui.state("refreshed", 0)
local note = ui.state("note", "")
local lower = ui.state("lower", "")
-- How the Notes page's fields clear the keyboard: UIKit's own scroll, or riding
-- its edge. `ZGX_APP_ARGS="lowfield attached"` starts attached.
local attached = ui.state("attached", ui.args[2] == "attached")

local function Detail(nav: any, title: string)
	local enabled = ui.use_state(true)
	return ios.page {
		title = title,
		nav = nav,
		ios.section {
			footer = "A page pushed on the stack. Drag from the left edge to go back.",
			ios.row { title = "Enabled", toggle = enabled },
			ios.row { title = "Version", detail = "26.0", chevron = false, last = true },
		},
		ios.section {
			ios.row {
				title = "Reset…",
				color = "$danger",
				chevron = false,
				last = true,
				on_click = function()
					ui.action_sheet {
						title = "Reset " .. title .. "?",
						message = "This can't be undone.",
						buttons = {
							{ title = "Reset", style = "destructive" },
							{ title = "Cancel", style = "cancel" },
						},
						on_choose = function(button)
							if button == 1 then
								ui.haptic("success")
							end
						end,
					}
				end,
			},
		},
	}
end
local DetailPage = ui.component(function(props: { nav: any, title: string })
	return Detail(props.nav, props.title)
end)

local PAGE = [[<html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head>
<body style="font:-apple-system-body;margin:20px"><h1>A real web page</h1>
<p>WKWebView, in a zgx box with rounded corners. It scrolls on its own, inside the zgx page.</p>
<div style="height:1400px;border-radius:16px;background:linear-gradient(#fdb,#bdf)"></div></body></html>]]
local WebPage = ui.component(function(props: { nav: any })
	return ios.page {
		title = "Web",
		nav = props.nav,
		ui.webview { html = PAGE, mx = 20, h = 520, rounded = 26 },
	}
end)

-- Whether a row answers what is typed in the search field.
local function unmatched(title: string)
	return function()
		local typed = string.lower(query:get())
		return typed ~= "" and string.find(string.lower(title), typed, 1, true) == nil
	end
end

local function SettingsPage(nav: any)
	local function open(title: string)
		return function()
			nav.push(function(inner: any)
				return DetailPage { nav = inner, title = title }
			end)
		end
	end
	return ios.page {
		title = "Settings",
		nav = nav,
		on_refresh = function(done)
			ui.after(900, function()
				refreshed:set(refreshed:get() + 1)
				done()
			end)
		end,
		ios.search { value = query, mx = 20 },
		ui.div { h = 16 },
		ios.section {
			ios.row {
				icon = "airplane",
				tint = "$ios.orange",
				title = "Airplane Mode",
				hidden = unmatched("Airplane Mode"),
				toggle = airplane,
			},
			ios.row {
				icon = "wifi",
				tint = "$ios.blue",
				title = "Wi-Fi",
				hidden = unmatched("Wi-Fi"),
				detail = "Home",
				on_click = open("Wi-Fi"),
			},
			ios.row {
				icon = "wave.3.right",
				tint = "$ios.blue",
				title = "Bluetooth",
				hidden = unmatched("Bluetooth"),
				detail = "On",
				on_click = open("Bluetooth"),
			},
			ios.row {
				icon = "antenna.radiowaves.left.and.right",
				tint = "$ios.green",
				title = "Cellular",
				hidden = unmatched("Cellular"),
				last = true,
				on_click = open("Cellular"),
			},
		},
		ios.section {
			ios.row {
				icon = "bell.badge.fill",
				menu = function()
					return {
						{
							title = "Share",
							symbol = "square.and.arrow.up",
							on_choose = function()
								ui.share { text = "A zgx app on iOS", url = "https://github.com" }
							end,
						},
						{ title = "Mute", symbol = "bell.slash" },
						{ title = "Remove", symbol = "trash", style = "destructive" },
					}
				end,
				tint = "$ios.red",
				title = "Notifications",
				hidden = unmatched("Notifications"),
				on_click = open("Notifications"),
			},
			ios.row {
				icon = "speaker.wave.3.fill",
				tint = "$ios.pink",
				title = "Sounds & Haptics",
				hidden = unmatched("Sounds & Haptics"),
				on_click = function()
					ui.haptic("success")
					ui.alert {
						title = "Sounds & Haptics",
						hidden = unmatched("Sounds & Haptics"),
						message = "You felt that one on a phone.",
						buttons = { { title = "OK" } },
					}
				end,
			},
			ios.row {
				icon = "moon.fill",
				tint = "$ios.indigo",
				title = "Focus",
				hidden = unmatched("Focus"),
				on_click = open("Focus"),
			},
			ios.row {
				icon = "hourglass",
				tint = "$ios.indigo",
				title = "Screen Time",
				hidden = unmatched("Screen Time"),
				last = true,
				on_click = open("Screen Time"),
			},
		},
		ios.section {
			ios.row {
				icon = "safari",
				tint = "$ios.blue",
				title = "Web page",
				hidden = unmatched("Web page"),
				on_click = function()
					nav.push(function(inner: any)
						return WebPage { nav = inner }
					end)
				end,
			},
			ios.row {
				icon = "gear",
				tint = "$ios.gray",
				title = "A sheet",
				hidden = unmatched("A sheet"),
				on_click = function()
					sheet_open:set(true)
				end,
			},
			ios.row {
				icon = "accessibility",
				tint = "$ios.blue",
				title = "Accessibility",
				hidden = unmatched("Accessibility"),
				last = true,
				on_click = open("Accessibility"),
			},
		},
	}
end

-- `sheet` (`ZGX_APP_ARGS`): the sheet opens on its own at 1.5 s, as Ref's
-- `MODE=sheet` presents UIKit's, to compare the two.
if ui.args[1] == "sheet" then
	ui.after(1500, function()
		sheet_open:set(true)
	end)
end

-- `composer_loop` (`ZGX_APP_ARGS`): the composer opens and closes ten times on its
-- own, each frame logged, to compare with the keyboard on a phone.
local composer_editor = ui.ref()
if ui.args[1] == "composer_loop" then
	tab:set(2)
	ui.effect(function()
		for i = 0, 9 do
			ui.after(2000 + i * 1600, function()
				local el = composer_editor:peek()
				if el then
					el:focus()
				end
			end)
			ui.after(2800 + i * 1600, function()
				local el = composer_editor:peek()
				if el then
					el:blur()
				end
			end)
		end
	end)
end

local function Composer()
	local message = ui.use_state("")
	return ios.composer {
		value = message,
		placeholder = "Message",
		log_motion = true,
		ref = composer_editor,
	}
end

local function TextPage(nav: any)
	local function page(ride: "native" | "attached")
		return ios.page {
			title = "Notes",
			nav = nav,
			keyboard_ride = ride,
			ios.section {
				header = "KEYBOARD",
				footer = "Off: the page scrolls once, as UIKit's do. On: a field rides the keyboard's edge.",
				ios.row { title = "Field rides the keyboard", toggle = attached, last = true },
			},
			ios.section {
				header = "A NOTE",
				ui.div {
					px = 16,
					py = 12,
					ui.textarea {
						value = note,
						label = "Note",
						placeholder = "Write something",
						h = 160,
						bg = "#00000000",
						border = 0,
						focus = {},
					},
				},
			},
			ui.text { "Refreshed {refreshed} times", wrap = true, mx = 36, mb = 160, color = "$text.secondary" },
			ios.section {
				header = "LOW ON THE PAGE",
				footer = "The page scrolls this field above the keyboard.",
				ui.div {
					px = 16,
					py = 12,
					ui.input {
						-- `ZGX_APP_ARGS=lowfield`: focused by itself, to test on a phone.
						on_mount = if ui.args[1] == "lowfield"
							then function(element: any)
								ui.after(1500, function()
									tab:set(2)
								end)
								ui.after(2200, function()
									element:focus()
								end)
							end
							else nil,
						value = lower,
						label = "A field low on the page",
						placeholder = "Tap to type",
						bg = "#00000000",
						border = 0,
						focus = {},
					},
				},
			},
		}
	end
	return ui.div {
		w = "100%",
		h = "100%",
		ui.switch(function()
			return if attached:get() then "attached" else "native"
		end, { native = page("native"), attached = page("attached") }),
		Composer(),
	}
end

local function GlassPage(_nav: any)
	local apart = ui.use_state(8)
	local rows: { any } = {}
	for index = 1, 30 do
		rows[index] = ui.row {
			h = 88,
			bg = index % 2 == 0 and "$ios.blue" or "$ios.orange",
			px = 20,
			items = "center",
			ui.text { "Row " .. index, color = "#ffffff", weight = 600, text_size = "$font.size.lg" },
		}
	end
	return ui.div {
		w = "100%",
		h = "100%",
		ui.scroll { w = "100%", h = "100%", ui.column { rows } },
		ui.glass_group {
			absolute = true,
			left = 76,
			bottom = 260,
			w = 260,
			h = 96,
			spacing = 40,
			ui.glass { absolute = true, left = 0, top = 0, w = 96, h = 96, radius = 48, interactive = true },
			ui.glass {
				absolute = true,
				left = 96,
				top = 0,
				w = 96,
				h = 96,
				radius = 48,
				interactive = true,
				x = ui.spring(apart),
			},
			-- A pane joining the group grows out of its glass, and melts back.
			ui.show(function()
				return apart:get() < 40
			end, ui.glass { absolute = true, left = 192, top = 0, w = 68, h = 96, radius = 34 }),
		},
		ui.glass {
			absolute = true,
			left = 76,
			right = 76,
			bottom = 180,
			h = 56,
			radius = 28,
			interactive = true,
			ui.button {
				w = "100%",
				h = "100%",
				rounded = 28,
				bg = "#00000000",
				hover = {},
				active = {},
				items = "center",
				justify = "center",
				on_click = function()
					apart:set(if apart:get() < 40 then 90 else 8)
				end,
				ui.text { "Merge", weight = 600, color = "$text.primary" },
			},
		},
	}
end

local function page(index: number)
	return ui.div {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		hidden = function()
			return tab:get() ~= index
		end,
		ios.stack {
			-- Each position in the gesture log, to compare with UIKit's.
			log_motion = true,
			root = if index == 1 then SettingsPage elseif index == 2 then TextPage else GlassPage,
			active = function()
				return tab:get() == index
			end,
		},
	}
end

return ui.div {
	w = "100%",
	h = "100%",
	bg = "$ios.grouped",
	page(1),
	page(2),
	page(3),
	ui.native_tabs {
		absolute = true,
		left = 0,
		right = 0,
		bottom = 0,
		h = 83,
		value = tab,
		items = {
			{ title = "Settings", symbol = "gear" },
			{ title = "Notes", symbol = "note.text" },
			{ title = "Glass", symbol = "circle.hexagongrid.fill" },
		},
	},
	ios.sheet {
		open = sheet_open,
		detents = { "medium", "large" },
		log_motion = true,
		build = function()
			return ui.column {
				p = 20,
				gap = 12,
				ui.text { "A sheet", heading = 2, text_size = "$font.size.lg", weight = 600, color = "$text.primary" },
				ui.text {
					"Drag the handle between its detents, or down to close.",
					wrap = true,
					color = "$text.secondary",
				},
			}
		end,
	},
}
