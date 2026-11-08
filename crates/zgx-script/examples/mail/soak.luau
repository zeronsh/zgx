-- `ZGX_APP_ARGS=soak`: the app drives itself, for a long session's memory
-- and CPU: mailboxes, opening and marking unread, scrolling the list, a draft
-- typed with the keyboard up and discarded, a search and its clearing. Each
-- cycle leaves the data as it found it, so memory comes back to where it was.
local state = require("./state")
local actions = require("./actions")

-- The first messages of each mailbox, found once.
local boxes = { "Inbox", "Archive", "Sent" }
local firsts: { [string]: { number } } = {}
for _, m in state.messages:peek() do
	local list = firsts[m.mailbox] or {}
	if #list < 3 then
		table.insert(list, m.id)
		firsts[m.mailbox] = list
	end
end

local cycle = 0
local step = 0
local steps: { () -> () } = {}
for _, box in boxes do
	table.insert(steps, function()
		state.mailbox:set(box)
	end)
	for _, id in firsts[box] or {} do
		table.insert(steps, function()
			actions.open(id)
		end)
		table.insert(steps, function()
			actions.mark_unread(id)
		end)
	end
	-- Down the list and back: the last row mounted, then the first, focused.
	table.insert(steps, function()
		local rows = state.rows:peek() and state.rows:elements() or {}
		local last = rows[#rows]
		if last then
			last:focus()
		end
	end)
	table.insert(steps, function()
		local rows = state.rows:peek() and state.rows:elements() or {}
		local first = rows[1]
		if first then
			first:focus()
		end
	end)
end
table.insert(steps, function()
	actions.compose("soak@example.com", "Soak " .. cycle)
end)
table.insert(steps, function()
	local body = state.body:peek()
	if body then
		body:focus()
	end
end)
table.insert(steps, function()
	state.draft_body:set(string.rep("A line typed into the draft. ", 20))
end)
table.insert(steps, function()
	local body = state.body:peek()
	if body then
		body:blur()
	end
	ui.batch(function()
		state.composing:set(false)
		state.draft_to:set("")
		state.draft_subject:set("")
		state.draft_body:set("")
	end)
end)
table.insert(steps, function()
	state.query:set("Subject 1")
end)
table.insert(steps, function()
	state.query:set("")
end)
table.insert(steps, function()
	state.selected:set(nil)
	state.mailbox:set("Inbox")
	cycle += 1
	print(string.format("SOAK cycle %d", cycle))
end)

ui.every(500, function()
	step = step % #steps + 1
	steps[step]()
end)
