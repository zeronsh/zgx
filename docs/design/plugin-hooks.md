# Plugin hooks

Scripts add to an app at places the app did not prepare for, as WordPress plugins hook into a theme: next to the app's Save button, in place of its empty state, with a filter on a list's items. Slots (`ui.slot` and `ui.contribute(name, node)`) stay as they are, for the places an app does prepare. Every script is equal: any script beside the root may use any of this, with no check. This is mechanism, not permission.

Status: (a) hook points and before/after/replace; (b) filters; (c) `@app` module overrides and packages; and (e) the worked example (`crates/zgx-script/examples/plugins`) and the guide, [plugins.md](../plugins.md), are built; (d), types, follows.

## Targets

Every element with an `id` is a hook point. A script may also name an element the way the inspector and agents do (`crate::driver`'s parser, so there is one selector language):

| target | matches |
|---|---|
| `#save` | the element whose `id` is `save` |
| `.row` | every element with the class `row` |
| `hook:toolbar.save` | the element that declares `hook = "toolbar.save"` |
| `role:button "Save"` | a button whose name, as the app writes it, is `Save` |
| `"Save"` | any element named `Save` |
| `#sidebar >> .row` | those `.row` elements inside `#sidebar`, at any depth |

A role and a name are the ones the app's source gives: an element's `role` (or the role its kind has: a button is a `button`, an input a `textbox`), and its `label`, its literal text or its first child's literal text. A name from state is not static and is not matched; such an element is reached by its `id`. A chain is checked as the element mounts; `@n`, `row` and `[n]` are errors that say so.

**Stable points:** `hook = "name"` declares a point the app documents and keeps. Every other `id` and every selector is the app's internals: targeting them works, and may break with the app's next version. `zgx types` (d) emits the declared hooks and every id, so a script's checker sees which it uses.

Only the root script's elements are targets. What a script adds is never matched by another script's target, nor by its own: a replacement matching its own target would replace itself without end.

## Placing

```lua
ui.contribute { target = "#save", before = ui.button { label = "Preview", on_click = preview } }
ui.contribute { target = "hook:toolbar.save", after = ui.text { "Saved by sync" } }
ui.contribute { target = "#empty-state", replace = ui.text { "Nothing here yet" } }
```

- `before` and `after` add siblings, in the matched element's parent, in load order.
- `replace` puts the script's node in the element's place. Of several, the last script's wins.
- A contribution names exactly one of the three.

## Filtering

```lua
ui.filter { target = "#title", text = function(text) return string.upper(text) end }
ui.filter { target = "#save", disabled = function(disabled) return disabled or not signed_in:get() end }
```

A filter passes one of an element's props through a function of the script's: `text` (its own, or a button's first child's), `label`, `hidden`, `disabled`, `tooltip` (its written text), `items` (a `ui.each`'s or a `ui.list`'s `data`) and `order` (its children's, by id: those returned first, the rest after; a changed order rebuilds the children in a region, an equal one keeps them). The function takes the value as it is and returns it changed; each is a computed source of the app's script that calls the script's function, so it runs in that script's VM again when what either reads changes, never per frame. Several apply in load order. The element is lowered as a copy with those props changed; its handlers, styles and state are the app's. A filter that fails is reported as the script's error and the value shows unchanged.

## Matching and reactivity

Targets are matched as the interface is lowered, once per element built, never per frame. An element built again (a `ui.show` turning back on, a row of `ui.each` coming in) is matched again. So a match that goes takes its contributions with it, and one that appears gets them; nothing is re-run while the element stays. A script's reload, or a change to which scripts load, reloads the app, which lowers it again.

Before, after and replace work in the parent's children, as the parent lowers them: the app's root element, and children an element lowers itself (a split's panes, a library component's parts), are not reached in (a).

## Separation

Each script's nodes run in its own VM, against its own state, the shared state and what the app provides where they land (a state there is the same state, written as the app writes it). That is so a script reloads, fails and is limited on its own (`Limits`), not to keep it from anything: every script reaches every host action, shared state and element. An app that wants a rule enforces it in its own host (`ScriptHost::calling_script`, `Body::script`).

## Cost

With no script contributing to a target, lowering checks one `Option` per children list: a container's children are lowered as before, and with contributions by a separate, out-of-line (cold) loop. Elements themselves are not looked at. With contributions, each element built is looked up by id and hook name (hash maps) and against the selectors that name neither (a short list). Measured with 1,000 ids, with and without selector-bearing scripts, in the PR that lands each part; the exact gate is ≤0.

## Plan

- **(a)** Hook points, before/after/replace.
- **(b)** Filters: `ui.filter { target, text = fn, … }` over text, label, hidden, disabled, tooltip, list items and child order, run as computeds in the script's VM: when what they read changes, never per frame.
- **(c)** Overrides and packages: an `@app/…` module (only `@app` requires; a relative require is never replaced) replaced by a script's module, run in the app's VM, with `@app-original/…` reaching the original. Packages: `ScriptHost::package`, `zgx.json`-listed `.luaurc` aliases outside the app's directory, read only, which every script requires from; `zgx bundle` includes them.
- **(d)** Types: `zgx types` emits `HookName`, each hook's context type and typed `ui.contribute` overloads.
- **(e)** Docs and a worked example.

Not yet: a function form of `replace` (given the app's node), and `first = true` to go before every other script.
