---
section: performance
---
- **A VM's collector runs tighter, goal 150 and step 400, and `app:gc()` reads the collected heap** (zgx-script): a component mounted and unmounted in a loop, holding a 2,000-item table in a handler, a function prop or a state, left the heap between collections at 8 to 20 MB over a 0.6 MB live one, as Luau's goal of 200 let garbage pile up for the two cycles its closures take to go. The collected heap was flat all along (a collection frees it all); it is the uncollected one that now stays under 1 to 4 MB. A test mounts and unmounts each kind 400 times and asserts both. `zgx test` gains `app:gc()`, a full collection returning the heap in KB.
