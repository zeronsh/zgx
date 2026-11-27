# Plugins: scripts that extend an app

A zgx-script app can be extended by scripts that run beside it, each in a VM of its own, and add to the app, filter what it shows or replace parts of it. They can reach places the app never prepared, the way WordPress plugins reach into a theme. zgx has no idea of a plugin: every script is equal, and each reaches everything the app's own script does. What tells them apart is mechanism, not permission: each has its own VM, its own errors, its own limits, and reloads on its own. This guide walks through the example in `crates/zgx-script/examples/plugins` (run it with `cargo run --release -p zgx-script --example plugins`). The reference is [state.md](reference/state.md#plugins-and-slots), and the design is [design/plugin-hooks.md](design/plugin-hooks.md).

## The example

A task list. Two scripts extend it:

- **focus** adds a Focus button beside the app's Add button. While focus is on, it filters the list down to the tasks not yet done.
- **stamps** puts a mark before the title and replaces the empty state with its own. It also replaces the app's row module with one that wraps the original and adds a "done" badge from a package the app uses.

```
examples/plugins/          the app
  app.luau
  components/task_row.luau required as @app/components/task_row
  plugins/focus/init.luau
  plugins/stamps/init.luau, task_row.luau
examples/plugins_kit/      a package (`kit`), outside the app
examples/support/plugins_host.rs   the host: tasks, actions, the scripts it loads
```

## Loading scripts

The script the host is made with is the root script: its view is the one that mounts. Others load beside it, each in a VM of its own:

```rust
host.set_scripts(vec![
    Script::file("focus", plugins.join("focus/init.luau")),
    Script::file("stamps", plugins.join("stamps/init.luau")),
]);
```

- `Script::file(name, path)` loads a file, which hot reloading watches; `Script::code(name, code)` loads source held in memory. The name is what its errors, its saved state and the reports below call it.
- `set_scripts` takes effect from the next `reload`. A script no longer given is unloaded: its effects, timers, windows, sockets and calls end before the next load is in.
- `ScriptHost::scripts_in(dir, limits)` loads every `.luau` file in a folder beside the root, by name, before those `set_scripts` gives; a file added to the folder loads as the host hot-reloads.
- Scripts beside the root load before it, so their slots and targets are there as the root lowers. One that fails to load is left out and its error reported; the root and the others load.
- A script's files are source, always: Luau checks no bytecode it loads, so a script's bytecode is read as source and fails to parse, rather than run. Only an app's bundle carries bytecode, its build's.

**Limits.** Each script runs within its own `Limits`, which the host sets: `Script::file(..).limits(Limits { … })` for one beside the root, `ScriptHost::limits` for the root's.

| `Limits` field | What it bounds |
| --- | --- |
| `memory` | its VM's heap: past it an allocation fails as the script's error |
| `load` | how long evaluating the script as it loads may take |
| `call` | how long one handler or computed call may take |
| `voices` | how many sounds it plays at once |

`count_state` counts what the script's states hold in Rust against `memory` too (sounds and effects with them): the meter, which weighs each state write, so it's off unless set.

`Limits::default()` has none of them: a script given no limits has no limit at all. `Limits::app_budgets()` is what `zgx run`, `zgx test` and `zgx audit` give the root script: 500 ms to load and 50 ms a call (ten times that in a debug build), no memory limit.

## Rules are the app's

zgx enforces no rule between scripts: any script may call any host action, read and write any shared state, fetch anything, open windows, replace any module. An app that wants rules (a plugin store, a template someone shares, a manifest a person approves) enforces them itself, where it decides what a script's request does:

- **A host action, function or async action** can read `host.calling_script()`: the name of the script calling it now, inside the call, and `None` outside one. So an action can answer each script in its own way, or refuse one.
- **A transport** (`streaming_fetcher`, `socket_connector`) reads `Body::script()` and `Socket::script()`: the script that made the request, the root's included. So a host's fetcher decides where each script may reach.
- **Which scripts load, and within what limits,** is the host's: `set_scripts` and `Limits`.

The terminal example (`crates/zgx-desktop/examples/terminal/plugins.rs`) is a whole system on top: a folder per plugin with a manifest saying what it needs, a sheet asking the person to allow it, and its panels opened from ⌘K. The list is for the person to decide by; running, a plugin reaches what zterm's own scripts do.

## Making an app extensible

**Every element with an `id` is a target.** A script names it as `#id`, and adds before or after it, replaces it or filters it. Nothing else is needed: an app with ids is already extensible.

**Declare the points you mean to keep.** `hook = "toolbar.add"` names a stable point that scripts target as `hook:toolbar.add`. Ids are your internals: a script targeting one works today and may break when the app changes, which the docs of an extensible app should say plainly. Declared hooks are the ones you promise to keep. The example's Add button carries both:

```lua
ui.button { id = "add", hook = "toolbar.add", "Add", on_click = add }
```

**Keep the host's own controls on top.** A view a host composes around the app's (`render`) can hold `modal(label, open).top_layer()` (`View::top_layer`): a layer filling the window, above whatever a script opens (`ui.modal`, `ui.layer`, a popover, a fixed node at any `z`), and live under them, so its controls take the pointer while a script's modal is up. The pointer passes through it but for its controls, and it takes no focus. A script's `z` orders its element among its siblings and can't reach it.

Scripts may also target an element by role and name, as the app writes them (`role:button "Add"`, or `"Add"`). That is convenient and the most fragile form; prefer declared hooks.

**Classes and chains.** `.session-row` targets every element with that class (`class = "session-row"`), and a contribution goes beside or in place of each. `#sidebar >> .session-row` targets those inside `#sidebar`, at any depth: each part before a `>>` is an element above the one before it. Parts combine as one element's (`#save .primary`, `role:button "Add"`). A chain is checked as the element mounts, so one in a row of a `ui.each` inside the sidebar matches, and the same class elsewhere doesn't. `ui.filter` takes the same targets.

Targets match only the root script's elements, never what another script contributed: a replacement matching its own target would replace itself without end.

**Let modules be replaced where that makes sense.** A module the app requires as `@app/<path>` (from its own directory) can be replaced by any script. A relative `require("./x")` never can. The example requires its row as `@app/components/task_row` on purpose.

## What scripts do

| | call |
|---|---|
| add beside an element | `ui.contribute { target, before = node }`, `after = node` |
| put a node in its place | `ui.contribute { target, replace = node }` |
| pass a prop through a function | `ui.filter { target, text = fn, … }` (text, label, hidden, disabled, tooltip, items, order) |
| replace an app module | `ui.override { module = "@app/…", with = "./file" }` |
| add to a slot the app made | `ui.contribute(name, node)` |

From the focus script:

```lua
ui.contribute { target = "hook:toolbar.add", after = ui.button { id = "focus", on_click = ui.toggle(focus), … } }
ui.filter {
	target = "#tasks",
	items = function(items)
		if not focus:get() then return items end
		…
	end,
}
```

A filter runs in the script's VM. It runs again whenever what it reads changes: the list it filters, and `focus`, the shared state it reads. It never runs per frame. A filter that fails is reported as the script's error, and the value shows as it was.

Before and after work among a container's children. The app's root element, and the panes of a split or a library component, are not targets. The root of a `ui.each` or `ui.list` row takes them a row each: what is added lays out as the list's own items, moves with its row as the list reorders, goes with it when it is removed, and reads what the row's root provides (`provide`), live. So does the root of a `ui.show`'s or `ui.switch`'s branch, or of a component: the additions are siblings in the parent's flow, come and go with the branch, and read what the root provides. So does a `ui.presence`'s content: what is added enters with it, and as it leaves, stays where it was until its exit animation ends, then goes in the same frame. Where a root can't have siblings (the app's root, a window's root), adding before or after it is an error in `host.contributions()` and to the script.

What a script adds reads what the app provides above where it lands (`provide`, `ui.use_context`), nearest first. A value arrives as a copy in the script's own VM; a state arrives as the same state, so what reads it follows the app's changes and a write changes the app's; a name nobody provides gives the default. A function can't cross into another script: providing one where another script's node reads it is an error naming it.

**A module override** runs in the app's VM, as the app's own code, wherever the app requires the module. That is why stamps' row can `require("@app-original/components/task_row")` (the module it replaces, to wrap it) and `require("@kit/badge")`. Of several scripts replacing one module, the last one's wins. `with` is one of the script's own files.

## Reports

**Where a script can put something, and what it can read there**, is `host.elements(&ui, &handle)`: the app's named elements (ids, classes, hooks) grouped by the elements that `provide` context, the app's root outermost. Each group gives a selector that reaches it (its id, or its classes inside the nearest element with one: `#sidebar >> .session-row`), the context names it provides and those it inherits, and its named elements down to the next group. Rows a list repeats are one group with a count; names the source declares that aren't showing (a page not open) are listed `mounted: false`. `zgx inspect elements` (MCP `zgx_elements`) gives the same, a line each and then as JSON. The mounted tree is kept only while scripts beside the root are loaded; without, it lists the source's names (`live: false`).

**What each contribution does now** is `host.contributions()`: for each `ui.contribute { target }` and `ui.filter`, in the order made, a `ContributionReport { script, target, place, matches, error }`: the script, its target, its place (`before`, `after`, `replace`, `filter`), how many elements it applies at now, and its latest error. A target matching nothing reads `matches: 0`, so a host can say "not shown here" without reading the script's source. The count follows the interface: an element a `ui.show` hides drops out, and one in each row of a list counts a match a row. `zgx inspect contributions` (MCP `zgx_contributions`) gives the same, a line each and then as JSON. Without scripts beside the root the list is empty and nothing is counted.

**Commands** share one namespace: a command `id` two scripts declare is declared twice, an error. A shortcut two commands claim, spelled the same or not, stays with the root script's command first, then with the others' in load order, and the later one is published without it. The contributions list says so after the contributions: place `shortcut`, the command's id as its target, and an error naming the key, why ("taken by the root script", or "taken by the script `focus`") and the command that holds it. `host.dropped_shortcuts()` gives the same typed (`DroppedShortcut { script, command, key, reason, holder }`), to read as `host.commands()` changes.

**Where it shows now** is `host.contribution_bounds("focus")`: a box in the window for each thing that script has mounted in a slot or before, after or in place of an element (what a `ui.filter` changes stays the app's own), for a host to say whose a control is or outline it.

**Removing a script for good**, `host.forget_state("focus")` leaves nothing of its state: its `ui.state` and `ui.persist` values, what waits to be saved and its saved files. Call it once the script is no longer loaded (`set_scripts` without it, then `reload`); loaded again, it starts from its `initial`. A script's saved values are kept under `script:<name>`; one saved by an older zgx under `plugin:<name>` is moved there on its first load.

## Shared state and actions

Each script's `ui.state` is its own. What scripts share with each other and with Rust:

- **Shared state.** `host.shared("tasks", …)` is state Rust and every script share, read and written by each (`ui.shared("tasks", {})`). Its first value is the host's, then the root script's default, then the first other script's. The example's tasks live in Rust: the focus script counts the open ones on its button and filters the list; `focus` is shared state the focus script sets.
- **Actions.** `host.action("complete", …)` is a Rust handler that every script calls with `ui.action("complete", id)` (as a handler) or `ui.invoke`. A name registered twice keeps the later handler; in a debug build, a second registration from another place in the host's code warns (stderr, `ScriptHost::warnings`, the lint strip), naming both places. One from the same place (a host rebuilt) doesn't. An action that should answer one script otherwise than another reads `host.calling_script()`.
- **Everything else** (fetching, the clipboard, files, windows, web views, terminals, audio) every script reaches as the root does.

## Scripts in a Luau app: zgx.json

An app run with `zgx run` has no Rust of its own. Its `zgx.json` names the scripts it loads beside its own, and `zgx test` and `zgx audit` load the same:

```json
{
  "packages": { "kit": "../plugins_kit" },
  "scripts": {
    "focus": "plugins/focus",
    "stamps": { "path": "plugins/stamps", "limits": { "memory_mb": 32, "call_ms": 50 } }
  }
}
```

- **`scripts`** maps each script's name to its path from the app's directory: a folder with an `init.luau`, or a file. Or to `{ "path", "limits" }`, where `limits` takes `memory_mb`, `load_ms`, `call_ms`, `voices` and `count_state` (`true` or `false`); each left out has none. A missing path is an error as the app loads.
- `plugins` and `expose`, from older zgx versions, are errors that say what replaced them.
- **The app's script defines its actions** with `ui.actions`, as a Rust host defines its own. Every script calls them as it calls a Rust host's: `ui.invoke("complete", id)`, or `ui.action("complete", id)` as a handler.

  ```lua
  local tasks = ui.shared("tasks", {})
  ui.actions {
      complete = function(id)
          tasks:update(function(list)
              for _, task in list do
                  if task.id == id then
                      task.done = true
                  end
              end
          end)
      end,
  }
  ```

## Packages

Modules several apps share go in a package, a folder outside the app: `ScriptHost::package("kit", dir)` or zgx.json's `"packages": { "kit": "../plugins_kit" }`. Every script requires them as `@kit/…`, and a `.luaurc` alias into the folder lets `zgx check` follow them. Packages are read only and are bundled with the app. A module of one loads in the VM of the script that requires it, with state of its own there. When a package's files change, every script that required from it reloads.

An app using its design system lets each script match whichever template is active:

```lua
local Badge = require("@ds/badge")
ui.contribute { target = "#header", after = Badge { label = "beta" } }
```

`zgx types <script folder>` adds the app's packages as aliases to that script's `.luaurc`, pointing at the folders zgx.json gives, so an editor's luau-lsp completes `@ds/…`. It never changes an alias already there.

## Checking

`zgx lint <dir>` and `zgx check <dir>` cover every script as they do the app. `check` resolves `@app/…` and `@app-original/…` against the app's directory, the nearest holding an `app.luau`. The example's test (`tests/plugins_example.rs`) runs the app, its host and both scripts headless: each contribution, the filter following shared state, the host's actions, and the replaced empty state.
