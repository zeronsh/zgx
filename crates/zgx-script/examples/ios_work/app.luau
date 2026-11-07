-- Scenes of real work for scripts/ios/threads.py, to see what holds up the
-- UI thread with Luau interpreted. `ui.args[1]`:
--   list: a long list of text rows scrolling
--   stream: a long markdown answer arriving in pieces
--   cells: 300 labels changing from a script loop
--   sort: a script sorting 20,000 rows each second
--   doc: a 40 KB markdown document appearing at once, each two seconds
--   mount: 1,000 rows of text mounted at once, each two seconds
local mode = ui.args[1] or "list"
-- How often `doc` and `mount` change, in milliseconds.
local every = tonumber(ui.args[2]) or 2000
local words = "the quick brown fox jumps over the lazy dog while streaming text arrives in small pieces"

if mode == "list" then
	local offset = ui.state("offset", 0)
	local time = 0
	ui.every(16, function()
		time += 0.016
		offset:set((math.sin(time * 1.2) + 1) * 2500)
	end)
	return ui.list {
		w = "100%",
		h = "100%",
		count = 5000,
		height = 60,
		offset = offset,
		row = function(index)
			return ui.row {
				h = 60,
				px = 16,
				items = "center",
				ui.text { "Row " .. index .. " " .. words, color = "$text.primary", truncate = true },
			}
		end,
	}
elseif mode == "stream" then
	local answer = ui.state("answer", "")
	local parts = string.split(words, " ")
	local count = 0
	ui.every(16, function()
		count += 1
		local piece = parts[count % #parts + 1] .. " "
		if count % 40 == 0 then
			piece ..= "\n\n## Section " .. count .. "\n\n- a **bold** item with `code`\n\n"
		end
		answer:append(piece)
		if #answer:get() > 40000 then
			answer:set("")
		end
	end)
	return ui.scroll { w = "100%", h = "100%", ui.markdown_view { "{answer}", p = 16 } }
elseif mode == "doc" then
	local document = ui.state("document", "")
	local pieces: { string } = {}
	for index = 1, 400 do
		pieces[index] = "## Section " .. index .. "\n\n" .. words .. " **bold** and `code` " .. words .. "\n\n"
	end
	local full = table.concat(pieces)
	local shown = false
	ui.every(every, function()
		shown = not shown
		document:set(shown and full or "")
	end)
	return ui.scroll { w = "100%", h = "100%", ui.markdown_view { "{document}", p = 16 } }
elseif mode == "mount" then
	local shown = ui.state("shown", false)
	ui.every(every, function()
		shown:set(not shown:get())
	end)
	local rows: { number } = {}
	for index = 1, 1000 do
		rows[index] = index
	end
	return ui.scroll {
		w = "100%",
		h = "100%",
		ui.show(
			shown,
			ui.column {
				ui.each(rows, function(index: number)
					return ui.row { h = 44, px = 16, items = "center", ui.text { "Row " .. index .. " " .. words } }
				end),
			}
		),
	}
elseif mode == "cells" then
	local tick = ui.state("tick", 0)
	ui.every(33, function()
		tick:set(tick:get() + 1)
	end)
	local cells: { any } = {}
	for index = 1, 300 do
		cells[index] = ui.text {
			function()
				local total = 0
				for step = 1, 40 do
					total += (tick:get() * index * step) % 7
				end
				return tostring(total)
			end,
			w = 48,
			h = 22,
			text_size = 12,
		}
	end
	return ui.row { wrap = true, w = "100%", pt = 60, cells }
else
	local status = ui.state("status", "sorting")
	local rows: { number } = {}
	for index = 1, 20000 do
		rows[index] = (index * 7919) % 20011
	end
	ui.every(1000, function()
		local copy = table.clone(rows)
		table.sort(copy, function(a: number, b: number)
			return a < b
		end)
		status:set("sorted, first " .. copy[1] .. " at " .. os.clock())
	end)
	return ui.column { pt = 80, p = 20, ui.text { "{status}" } }
end
