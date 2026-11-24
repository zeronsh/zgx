# Recovering a Luau call stopped by its time budget

Status: design only (internal). Nothing here is built. Asked by the Plugin
Store: an untrusted plugin rendering a 64 KiB README as markdown ran one
build call past the 50 ms release budget, and wanted to catch the stop and
draw the text plain instead.

## Today

`interrupt.rs`: once an outermost call's CPU time passes its budget `B`
(the watchdog sets the interrupt as its wall time passes), every 64th
interrupt raises "script exceeded its time budget". After a stop the
counter is set so that the very next safepoint checks again. A `pcall`
catches the error, but its handler is stopped at its first loop or call.
That is correct: any recovery inside the same call is more script time
past `B`. A runaway is stopped within `B`, the watchdog's wake-up and 64
interrupts.

Plugins are untrusted, so whatever recovers must keep a fixed bound on how
far a script can run past its budget.

## (a) A one-time grace for the call that caught the stop

After the first stop the outermost call gets `G` more (say 5 ms), once and
never renewed; then stops are final, as today.

- **Bound:** `B + G` plus the watchdog's wake-up and 64 interrupts. A loop
  of `pcall` and work cannot extend it: the grace is the outermost call's,
  spent once.
- **But:** every plugin can take `G` on purpose (pcall a runaway, then
  spend the grace), so it is a bigger budget for all scripts. The handler's
  fallback must also fit in `G`, which "draw it plain" may not for a large
  document. And it changes what the budget means everywhere.

## (b) A host fallback for a stopped subtree (recommended)

A boundary element:

```lua
ui.budgeted {
    fallback = ui.text { readme, wrap = true },
    build = function() return markdown(readme) end,
}
```

- `fallback` is built eagerly by the caller, as ordinary nodes, inside the
  caller's own budget. It is data, not code run after the stop.
- `build` runs as a guarded call of its own, with its own deadline,
  scheduled by the host after the parent's build returns, so a stop there
  unwinds that call alone.
- On a stop the host mounts `fallback`, or keeps the boundary's last good
  content if it has one. No script runs after the stop.

**Bound:** each boundary's build is bounded by its slice (`B_b` ≤ `B`). The
boundaries and their parent share one per-frame script budget (`B_frame`),
so N boundaries cannot multiply it. A fallback is never itself a
boundary's build, so nothing nests to extend time. The worst case per frame
is `B_frame` plus the watchdog's wake-up and 64 interrupts: today's bound,
with no extra time for untrusted code.

It fits the README case: the plain text is built cheaply up front, and the
markdown build stops at its slice without the plugin's own `pcall`.

## Decision

(b): its bound is today's, and the fallback is data. (a) gives every plugin
`G` more by design.

What (b) costs, when it is built:

- a new element (internal API first);
- the host running a boundary's build as a separate guarded call, which
  needs scheduling inside the build pass;
- a per-frame budget the boundaries share (today the budget is per call);
- the last good content: the boundary keeps its previous subtree, as a
  keyed swap does.
