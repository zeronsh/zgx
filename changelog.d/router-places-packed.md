---
section: performance
---
- **`ui.router`'s history keeps each place in its own bytes** (zgx-script): a place behind or ahead is kept as one string (its route's name and path, interned), not the route's tables. Those were made as the app moved, among what the move made and dropped, so with unlimited history (`history = math.huge`) each one held a Luau heap page long after the rest of it was free: 1,200 moves held 1.1 MB more than keeping one place, and now hold about 30–130 KB more. Back and forward return the same routes, params read from the path as `navigate` makes them.
