# Scrolling: scroll, scroll_x, list

`scroll` and `scroll_x` are viewports over ordinary children, all of them mounted. `list` is virtualized: of its `count` rows, only those near the view are built, so it holds a million rows as cheaply as twenty.

```luau
ui.scroll(props: ScrollProps?) -> Node
ui.scroll_x(props: ScrollProps?) -> Node
ui.list(props: ListProps) -> Node
```

## scroll and scroll_x

`scroll` stacks its children top to bottom and scrolls vertically; `scroll_x` lays them in a row and scrolls horizontally. The wheel scrolls the innermost viewport that can still move, and passes to an outer one at an edge; focus moving into a viewport (Tab, a focus group's arrows, a press, `el:focus()`) scrolls it the least that shows the focused node, `scroll_padding` kept clear (`el:focus({ scroll = false })` leaves it where it is).

| Option | Type | Default | |
| --- | --- | --- | --- |
| children | `Child` | none | |
| `offset` | `State<number>` | its own | the scroll position in points, both ways: scrolling writes it, setting it scrolls (clamped) |
| `row_text` | `(index: number) -> string` | the row's own texts | the plain text of row `index`: in a selectable region, copying a selection that spans rows the list has not mounted (select-all over a long transcript) takes theirs from it. Without it, the list builds the row's description (not mounted) and takes its texts, a `row`'s children joined by tabs and others by line breaks, as a copy of the row on screen does; give it for rows drawn or built in ways that don't say their text |
| `paging` | `boolean` | `false` | it comes to rest only on whole pages, a page its own length along the way it scrolls, as `UIScrollView.pagingEnabled`: lifting a gesture goes on to the next page if quick (300 points a second or more), else to the nearest, and never more than one page from where it began, on UIKit's own spring; a mouse wheel goes a page a notch, and at the last page goes on to what's around it, as a touchpad does on X11 and Windows, which don't say when fingers lift; anything else that moves it (a key, a reveal, setting `offset`) settles on the nearest page once it stops; resized, it stays on its page |
| `follow` | `boolean` | `false` | stays at the end while the content grows, until scrolled away from it (a transcript, a log) |
| `scrollbar` | `boolean` \| [`Scrollbar`](#a-scrollbar-of-your-own) | `false` | an interactive scrollbar over the trailing edge: drag it, click its track to page; overlay style (hidden until scrolling) where the system has overlay scrollbars. A table draws and times it as the app's own |
| `overscroll` | `"auto"` \| `"contain"` | `"auto"` | `"contain"`: a wheel or trackpad gesture that starts over it stays with it at its edges, momentum included, as CSS's `overscroll-behavior: contain`: the scroll views around it neither move nor stretch (a transcript inside a page). `"auto"` hands what it can't scroll on to them |
| `scroll_padding` | `number` \| `{ top, bottom, left, right }` | `0` | points kept clear inside its edges as a focused control scrolls into view, as CSS's `scroll-padding`: what floats over them (a composer over the bottom, a title bar over the top) doesn't cover it. One number for every side; a side not given is 0. A state or a function saying it is followed: what floats over the edge growing (a composer to more lines) takes its room as it does |
| `w`, `h` | `Length` | `320` by `240` when nothing else sizes it | a stretching parent, `grow`, `w`/`h` or `w_full` size it instead |
| `gap`, `items`, `p`… | | | lay out the content as a column's (a row's for `scroll_x`) |
| `overflow` | `Overflow` | `"hidden"` | |

The content fills the viewport's width (`scroll`) or height (`scroll_x`) and is as long as its children make it.

A child with `sticky_top` holds at the top as the view scrolls past it, inside its parent: section headers ([sticky](properties.md#sticky)).

## list

| Option | Type | Default | |
| --- | --- | --- | --- |
| `count` | `Reactive<number>` | required without `heights` or `data` | how many rows |
| `data` | `Readable<{ any }>` | none | the list its rows show, as `ui.each`'s (not with `count`; with `heights`, one height an item): `key` is given an item, `row` the item and its index (a state). A two-parameter `row` is built again when its item changes; one taking a third parameter, `function(item, index, row)`, is built once and gets `row`, its item as a state kept current, so the row's functions update in place: a reply streaming into a transcript runs its own row alone |
| `row` | `(index: number) -> Node`, or with `key` `(key, index: State<number>) -> Node`, or with `data` `(item, index: State<number>, row: State<any>?) -> Node` | required | builds row `index`, from 1; with `key`, the row of item `key`, given its index as a state that follows the row; with `data`, the row of an item |
| `key` | `(index: number) -> number \| string \| boolean`, or with `data` a field's name | none | each row's item's key (with `data`, a function of the item, or the name of a field, `key = "id"`, which reads it without making any item a table: what a long list wants): a row follows its item as the list changes, moved rather than built or written again. With `measured` or `heights`, items added or removed above the view keep the rows in view where they are, as a browser's scroll anchoring does: a feed taking new items at its top while the reader is down it doesn't move under them; resting at the top, the new items show. Without it, rows are by position: a removal or an insertion writes every row after it with the item now there (its text, layout and paint done again). Keys are unique; not with `sticky` |
| `height` | `number` | `28` | every row's height, in points (positive); with `measured`, the estimate for a row not yet measured |
| `virtualize` | `"ancestor"` | none | it doesn't scroll itself but with the scroll view around it (a panel that scrolls as a whole, its sections lists): as tall as all its rows, it builds only those that view shows (with `overscan`), again as it scrolls or resizes. With `height` or `heights`, not `measured`; the view's wheel and scrollbar are the list's, and `offset`, `scrollbar` and the edge callbacks don't apply |
| `measured` | `boolean` | `false` | rows take their content's height (wrapped text, images), measured as they mount; not with `heights` |
| `estimate` | [`ListEstimate`](#estimates) | none | with `measured`, `data` and `key`: how a row's data makes its height, so the scrollbar tells the truth before rows are built |
| `follow` | `boolean` | `false` | with `measured` or `heights`: resting at the end, it stays at the end as rows are added and measured (a chat); scrolled away, it stays where it is |
| `heights` | `{ number }`, a state of one, or a function returning one | none | each row's own height instead, in order; the count is its length. A row of 0 takes no room and isn't built, nor one under a hundredth of a point: rows folded away cost nothing, however many |
| `sticky` | `{ number }`, a state of one, or a function returning one | none | with `heights`, and with or without `key`: rows (from 1) that stick to the top while the rows after them scroll past, each pushed up by the next (section headers, a diff's file headers); the one in charge is built again over the list, as any row is |
| `sticky_top` | `number` | `0` | how far below the top the sticky rows stick (CSS's sticky `top`): what scrolls above them still shows, so cover it if it shouldn't |
| `overscan` | `number` | `2` | rows built beyond each edge of the view, at the least: at once where a host has no time to spare, as `ahead`'s otherwise ([building ahead](#building-ahead)) |
| `ahead` | `number` | `1` | viewports of rows built beyond each edge of the view in time frames leave over, and kept as it scrolls away ([building ahead](#building-ahead)); `0`, it builds the rows in view and its `overscan` alone, and drops the rest |
| `offset` | `State<number>` | its own | the scroll position in points, as `scroll`'s |
| `scrollbar` | `boolean` \| [`Scrollbar`](#a-scrollbar-of-your-own) | `false` | an interactive scrollbar, as `scroll`'s |
| `overscroll` | `"auto"` \| `"contain"` | `"auto"` | as `scroll`'s: `"contain"` keeps a gesture over the list with it at its ends |
| `scroll_padding` | `number` \| `{ top, bottom, left, right }` | `0` | as `scroll`'s: focus moving to a control in a row (Tab past the rows in view, onto one built beyond them) scrolls the list to it, the offset following, this much kept clear |
| `keyboard` | `boolean` | `false` | its rows are one Tab stop, landing on the row last focused, and the arrows, Page Up/Down, Home and End move focus through every row (mounting it), as a list box's do; typing jumps to the next row whose text starts with what was typed (letters within 750 ms build a prefix, the same letter again cycles), rows not mounted read by `row_text`. A row with `role = "option"` (or `"listitem"`) takes the focus itself, so its `on_focus` can open it (selection following focus); other controls in a row keep their own keys |
| `on_end` | `() -> ()` | none | called as its end comes within `end_distance` points of the bottom of the view (an infinite scroll's next page, `ui.paged`'s `more`): once each time, and again once its rows have grown; while its rows are shorter than the view too, until they fill it or a call adds none. Watched in Rust: no script runs while it scrolls short of it |
| `end_distance` | `number` | `200` | how far from the end `on_end` is called |
| `on_start` | `() -> ()` | none | called as its start comes within `start_distance` points of the top of the view (a history's earlier page, react-virtuoso's `startReached`): once each time, and again once its rows have changed. Rows it adds above keep the rows in view where they are: a list with a `key` and `measured` (or `heights`) finds the row it showed by its key, so the view does not jump. Watched in Rust |
| `start_distance` | `number` | `200` | how far from the start `on_start` is called |
| `at_end` | `State<boolean>` | none | kept saying whether the list rests at its end, within a few points of it (react-virtuoso's `atBottomStateChange`): a "jump to the latest" button shows while it is false, and `scroll_to_row(count, { align = "end" })` takes the view there. Written only as it changes |
| `w`, `h` | `Length` | `320`, `240` | |

With `heights`, rows differ in height but are still never measured: the list knows where each starts, so a changed list (a section folded, rows inserted) re-lays out without building anything offscreen. A host can publish the heights as a shared value (`ScriptHost::shared`), and the script never copies them.

With `measured = true`, rows take the height of what they hold, at the list's width: a chat's or a mail's rows of wrapped text, which a script can't measure. A row is measured as it mounts; rows not yet seen are `height` tall until then, so the scrollbar's length is an estimate that settles as rows are seen. The rows in view stay where they are as rows above them are measured (scrolling up, or jumping far with `offset` or End): only the rows measured move. Positional rows keep heights by position; with `key`, heights go with their items, so a removal or an insertion keeps every measured height.

```lua
type Post = { id: number, author: string, text: string }
local posts = ui.state("posts", {} :: { Post })
return ui.list {
	count = function()
		return posts:len()
	end,
	measured = true,
	-- New posts at the end stay in view while it rests there.
	follow = true,
	-- About one line and a name: the estimate until a row is measured.
	height = 48,
	row = function(i: number)
		local post = posts:peek()[i]
		return ui.column {
			w_full = true,
			px = 12,
			py = 6,
			ui.text { post.author, literal = true, bold = true },
			ui.text { post.text, literal = true, wrap = true },
		}
	end,
}
```

To bring a row into view, give the list a `ref` and ask its element: `el:scroll_to_row(i, { align = "center" })`, `i` from 1, `align` `"start"` (its top at the top), `"center"`, `"end"` (its bottom at the bottom) or `"nearest"` (the default: the least scrolling, none if it shows). It works for every kind of list: one of measured rows measures the rows around the target as it lands, so the row ends exactly where asked however far it was.

```lua
local messages = ui.ref()
return ui.column {
	ui.button {
		on_click = function()
			local list = messages:peek()
			if list then
				list:scroll_to_row(5000, { align = "center" })
			end
		end,
		"Jump to 5000",
	},
	ui.list {
		ref = messages,
		count = 10000,
		height = 24,
		w = 300,
		h = 240,
		row = function(i)
			return ui.text { `Message {i}`, h = 24 }
		end,
	},
}
```

A row is built before it scrolls into view and dropped a while after it leaves ([building ahead](#building-ahead)); it owns what it makes, as a [component](state.md#components) does: its `ui.use_state`, effects and computed values end with it.

### Building ahead

A row can cost milliseconds to build (a reply's Markdown, a table), more than a frame at 120 Hz has to spare. So a window builds rows off the scroll frame, as a browser lays out and paints ahead of its viewport:

- **Ahead.** As a refresh begins, up to half of it from its start and before the frame's own work (what a window's frame otherwise spends waiting for its point in the refresh), the list builds and lays out the rows within `ahead` viewports of each edge of the view (and `overscan` rows at the least), the way the view is going first, one at a time. A row that doesn't fit waits for a frame it fits; one longer than that budget is built after a few frames, in a frame that has only just begun. Measured, its height is known before it shows, so what's in view doesn't jump as it comes in. It starts as the view first moves and goes quiet once they are built; a hidden or covered window waits until it shows.
- **At once.** A row in view that isn't built is built in that frame, as it must be: a jump far away, or a scroll faster than time allows.
- **Kept.** A row that leaves stays built while it is within 4 viewports more of the view, so scrolling back builds nothing, and costs little where it is: it is laid out once, and outside the list's clip it isn't painted or hit. Beyond that it is dropped, and past 256 rows or 8,000 nodes the farthest go first, though never those shown or to build. A new width drops what's out of view, to be built at it.
- **At rest.** A list that has never scrolled holds what shows and its `overscan`, as it always did, and builds nothing further. 30 seconds after its window's last frame, shown or covered, a list goes back to that and what it let go of is freed, until its view next moves.
- **Keys.** With `key`, rows kept follow their items when rows are added or removed around them, as those in view do.

A row built ahead is mounted like one in view: its effects run, what it fetches on mount is fetched, and its controls are reached by Tab in order. A row with a popover open from it, or with the keyboard's focus in a list with `keyboard`, goes as it leaves, as ever. A list whose rows shouldn't start before they show, or that holds something scarce, says `ahead = 0`.

A headless host (`TestApp`, `zgx test`) has no time left over: its lists build the rows in view and their `overscan`. In Rust, a host says it has some with `Ui::host_spare_time`, and spends it with `Ui::run_spare_work`.

`zgx inspect frames` shows spare time as `spare` entries of their own; the replay's and `summary`'s frames and work leave them out.

A list whose items are removed or inserted (a mailbox's messages archived, a feed's new posts) keys its rows, and reads each row's item by its key:

```lua
type Message = { id: number, subject: string }
-- Ids are places in the list (messages are only added at the end).
local messages = ui.state("messages", {} :: { Message })
local inbox = ui.state("inbox", {} :: { number })
return ui.list {
	count = function()
		return inbox:len()
	end,
	key = function(i)
		return inbox:at(i)
	end,
	row = function(id: number, index: State<number>)
		return ui.text {
			function()
				local m = messages:at(id)
				return if m then `{index:get()}. {m.subject}` else ""
			end,
		}
	end,
}
```


### Estimates

A list of `measured` rows knows a row's height once it's built. Until then the row stands at `height`, so the scrollbar is the wrong length and a dragged thumb lands far from where it points. `estimate` says once how a row's data makes its height. Rust runs it per row, with no script: each row's text is scanned once, and a new width is arithmetic over that. Rows still measure as they mount.

```luau
local rows = ui.state("rows", { { id = "a", text = "Hello", kind = "note" } })
return ui.list {
	data = rows,
	key = "id",
	measured = true,
	estimate = {
		height = 24,
		text = { field = "text", size = 14, line = 20, char = 0.49, inset = 24 },
		by = "kind",
		cases = { note = { height = 18 } },
		has = { attachments = { height = 60 } },
	},
	row = function(r)
		return ui.text { r.text, wrap = true, p = 12 }
	end,
}
```

A part adds any of these up:

| Field | Type | |
| --- | --- | --- |
| `height` | `number` | points it always adds |
| `text` | a text, or a list of them | wrapped text, below |
| `each`, `item` | field name, part | a part for each item of a list field |
| `by`, `cases`, `default` | field name, `{ [value]: part }`, part | a part by a field's value |
| `has` | `{ [field]: part }` | a part for each field that holds anything |

A text has:

| Field | Default | |
| --- | --- | --- |
| `field` | required | the field holding it |
| `size`, `line` | `14`, 1.4 times `size` | its font's size and a line's height |
| `char` | `0.5` | a narrow character's average width, in ems; wide characters (Han, kana, Hangul, emoji) count an em |
| `word` | `5` | an average word's length in characters: wrapping leaves about half of one unused on each line |
| `inset`, `most`, `minus`, `share` | `0`, none, `0`, `1` | its column: `share` of (the list less `inset`, at most `most`, less `minus`) |
| `most_lines`, `beyond` | none, `0` | clamped to this many lines, and what shows beyond them then |
| `markdown` | none | it's Markdown: `{ paragraph, heading, item, code, table }`, what each kind of source line adds. A paragraph's or an item's source lines flow as one, however they were wrapped; code and table lines stay one each |

It's an estimate: fit it to a few rows by eye or by measuring. A row's real height replaces it as the row mounts, without moving what's in view. A row's height can't come from anything but its data here: state, pictures not sized in the data and the like are only known once it's built.

## A scrollbar of your own

`scrollbar = { … }` draws and times the bar as the app's own: an app's floating rail, say, thin and clear of a card's corners. It's still zgx's bar, drawn and timed in Rust: a scroll runs no script for it, its edge `fade` doesn't fade it, and it lights up only for a scroll the user makes (an app placing its view, a kept place put back as rows are measured, or a reload leave it dark). Each field it sets replaces the platform's, in points and milliseconds:

| Field | Type | Platform's (zgx's own) | |
| --- | --- | --- | --- |
| `platform` | `boolean` | `true` | `false`: over zgx's own look on every platform, with no Windows arrow buttons and no widening delay |
| `thickness`, `active_thickness` | `number` | `6`, `8` | the thumb across, and under the pointer or dragged (`thickness` alone sets both) |
| `strip` | `number` | `8` | the strip at the edge that takes the pointer |
| `side` | `number` | centred | the thumb from the view's edge |
| `ends` | `number` | `0` | how far its track stops short of each end, the thumb whole within it (at least `min` drawn) |
| `inset` | `number` | `2` | how far the thumb's drawing is inset from its place at each end (so it's drawn `2 × inset` shorter) |
| `min` | `number` | `24` | the shortest thumb |
| `linger`, `fade` | `number` | `1000`, `200` | how long it stays once the view stops scrolling, and how long it fades |
| `fade_in` | `boolean` | `false` | it fades in over `fade` too, not at once |
| `ease` | `"linear"` \| `"outQuad"` \| `"outCubic"` | `"linear"` | its fade's curve |
| `track` | `State<number>` \| `() -> number` | the view's length | its track's length from the view's start (a transcript's rail stopping above a composer over its foot), read as it changes |
| `hint` | `State<boolean>` \| `() -> boolean` | none | shown while it holds (a table a pointer rests on), read as it changes |
| `active_color` | `boolean` | `false` | under the pointer or dragged, the thumb is `$scrollbar.thumb.active` (not its own colour, brighter) |
| `track_fill` | `boolean` | `true` | whether its track (`$scrollbar.track`) is drawn |
| `press` | `"page"` \| `"jump"` | `"page"` | a press beside the thumb pages, or brings the thumb's middle under the pointer and drags from there |
| `linger_from` | `"release"` \| `"scroll"` | `"release"` | let go (the pointer leaves, a hint ends), it lingers from then, or from the last scroll |
| `focusable` | `boolean` | `true` | whether a press focuses it for its keys; `false` leaves focus where it was (a composer keeps it) |
| `always_overlay` | `boolean` | `false` | shown as it scrolls even where the system always shows scrollbars |
| `accessible` | `boolean` | `true` | whether assistive technology has it, a scrollbar whose value is the offset; `false` leaves it out (the view itself still scrolls for a reader), as an app's rail may |

```luau
-- A floating rail: 3 across (5 under the pointer) 2 from the edge, its
-- ends 4 short of the view's, in and out over 260 ms 1.4 s after a scroll.
ui.list {
	count = #rows, height = 20, offset = at,
	scrollbar = {
		platform = false, thickness = 3, active_thickness = 5, strip = 10, side = 2,
		ends = 4, inset = 0, linger = 1400, fade = 260, fade_in = true, ease = "outQuad",
		active_color = true, track_fill = false, press = "jump", linger_from = "scroll",
		focusable = false, always_overlay = true, accessible = false,
		track = function() return seen:get() - composer:get() end,
	},
	row = function(i) return Row(rows[i]) end,
}
```

## Example

```lua
type Message = { from: string, text: string }
local messages = ui.state("messages", {} :: { Message })
local offset = ui.state("offset", 0)
return ui.row {
	p = 16,
	gap = 16,
	items = "start",
	-- A transcript: every message mounted, kept at the end as it grows.
	ui.column {
		gap = 8,
		ui.scroll {
			w = 260,
			h = 200,
			gap = 6,
			follow = true,
			ui.each(messages, function(m: Message)
				return ui.text { `{m.from}: {m.text}`, wrap = true }
			end),
		},
		ui.button {
			on_click = function()
				messages:push { from = "Ada", text = `Message {messages:len() + 1}` }
			end,
			"Send",
		},
	},
	-- A hundred thousand rows; about ten are built at a time.
	ui.column {
		gap = 8,
		ui.list {
			count = 100000,
			height = 24,
			w = 200,
			h = 200,
			offset = offset,
			row = function(i)
				return ui.text { `Row {i}`, w_full = true, h = 24, px = 8 }
			end,
		},
		ui.text { "{offset}" },
		ui.button { on_click = ui.set(offset, 0), "Top" },
	},
}
```

## Gotchas

- Every list row is `height` points: a taller row overlaps the next and a shorter one leaves a gap. For rows of different heights give `heights` (each row's own, still never measured), or `measured = true` for rows whose height depends on their text (`zgx lint` flags wrapped text in rows of fixed height).
- A list row is built once for its index, before it comes into view. Reading state in `row` (`items:get()[i]`) takes the value at that moment; to follow a change, read it in a function property or text inside the row (`ui.text(function() return items:get()[i].name end)`), or change `count`. Rows are keyed by index, so a row's own state stays with its position, not its data.
- In a `selectable` region, a selection runs through a list as through any text: dragging past its edge scrolls it, the selection outlives the rows scrolled away (each end keeps its place in its row), and copying takes the rows between, mounted or not (see `row_text`).
- List rows do not stretch across the list: give a row's root `w_full = true`.
- A `scroll` with no size and nothing sizing it is 320 by 240 along its scrolling axis; across it, a column or row stretches it (the default `items`), and inside one with `items = "start"` it keeps 320. Use `w_full = true`, `grow = 1` or a stretching parent.
- All of a `scroll`'s children stay mounted however far away they are; past a few thousand rows, use `list`.
- `offset` counts points from the top (the left for `scroll_x`). Setting it beyond the content clamps it, and a binding to a state that is not a number reads as 0.
- There is no scrollbar unless asked (`scrollbar = true`): a viewport scrolls by wheel, touchpad, keyboard focus and `offset`, and with the keys as a page does (Space, Shift+Space, Page Up/Down, Home/End, arrows) once it has been pressed in or is under the pointer, while nothing has focus (a focused control keeps its keys).

## Keeping a gesture

A scrolling box inside a scrolling page hands a gesture on at its edges unless it says otherwise. With `overscroll = "contain"` a transcript keeps it, momentum included:

```lua
local lines = ui.state("lines", {} :: { string })
for i = 1, 40 do
	lines:push(`Line {i}`)
end
return ui.scroll {
	w = "100%",
	h = 400,
	gap = 8,
	ui.text { "A page" },
	ui.scroll {
		overscroll = "contain",
		h = 200,
		ui.each(lines, function(line: string)
			return ui.text { line }
		end),
	},
	ui.div { h = 600 },
}
```
