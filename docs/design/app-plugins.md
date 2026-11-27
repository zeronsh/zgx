# Scripts in a Luau app: `zgx.json`'s `scripts`

Status: built. `scripts` replaced 0.1.7's `expose` and `plugins` when every
script became equal: there is nothing left to expose or allow.

A Rust host loads scripts beside its root with `set_scripts` (or
`scripts_in`), each with its `Limits`. An app run by `zgx run` has no Rust
of its own, so its `zgx.json` names them instead:

```json
{
  "title": "Tasks",
  "scripts": {
    "focus": "plugins/focus",
    "stamps": { "path": "plugins/stamps", "limits": { "memory_mb": 32, "call_ms": 50 } }
  }
}
```

## `scripts`

- **The name** is what the script's errors, its saved state
  (`script:<name>`) and the reports call it.
- **The path** is its folder (its `init.luau`) or file, from the app's
  directory. One that is neither is an error at load.
- **`limits`** takes `memory_mb`, `load_ms`, `call_ms` and `voices`, as
  `Limits` does; each left out has none, and a script given none has no
  limit. An unknown key is an error naming it.
- **Every script reaches everything the app's does:** its shared state, its
  actions, its elements, its packages. `"packages"` is visible to every
  script.

## Actions

`ui.actions { complete = function(id) … end }` defines actions in Luau, for
an app with no Rust to define them in. Every script calls them with
`ui.invoke("complete", id)` or `ui.action("complete", id)`, as it calls a
Rust host's.

## The old keys

`plugins` and `expose` are errors that say what to do:

- ``"plugins"``: "`plugins` is now `scripts`: { "name": "path" }, or
  { "path", "limits" }; a script reaches everything the app's does, with no
  `allow`".
- ``"expose"``: "`expose` is gone: every script reaches every shared state,
  action and element of the app's; leave it out".

## One loader

`zgx_script::app_config::read(dir)` reads `zgx.json`'s `packages` and
`scripts`, and its `apply(host)` gives them to the host. Three callers
share it:

- `zgx run`;
- `zgx test`'s `mount()`;
- `zgx audit` (#988's `audit_app`).

zgx-run's own `zgx.json` reading (the window) sits beside it, so one parser
knows every key and an unknown key is reported once.

## Cost

Nothing for an app without `scripts`. `ui.actions` is a lazy `ui`
function, so a script that doesn't call it builds nothing. An action call
goes through the existing `invoke` path, finding a Luau function where it
would find a Rust one.

## Verified by

- `tests/app_plugins.rs`: a Luau app's scripts from its `zgx.json` alone,
  through `mount()`, with no Rust host: a script beside the app's reads its
  shared state, defines an action and filters a target, with nothing
  granted. A script that isn't there is an error naming it.
- `app_config`'s own tests: the old `plugins` and `expose` keys, and
  `limits`.
