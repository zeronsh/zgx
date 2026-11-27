-- Focus: a button beside the app's Add (its declared hook) and, while it
-- is on, a filter on the list hiding what is done. `focus` and `tasks` are
-- shared state: the button counts what is open.
local focus = ui.shared("focus", false)
local tasks = ui.shared("tasks", {} :: { { done: boolean } })

ui.contribute {
	target = "hook:toolbar.add",
	after = ui.button {
		id = "focus",
		on_click = ui.toggle(focus),
		ui.text {
			function()
				if focus:get() then
					return "Show all"
				end
				local open = 0
				for _, task in tasks:get() do
					if not task.done then
						open += 1
					end
				end
				return `Focus ({open})`
			end,
		},
	},
}

ui.filter {
	target = "#tasks",
	items = function(items)
		if not focus:get() then
			return items
		end
		local open = {}
		for _, task in items do
			if not task.done then
				table.insert(open, task)
			end
		end
		return open
	end,
}
