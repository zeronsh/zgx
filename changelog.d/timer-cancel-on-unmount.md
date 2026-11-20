---
section: fixed
---
- **A stopped timer lets go of what it holds when it stops** (zgx-script): `ui.every` and `ui.after` made by a component ended as it unmounted only when they next fell due, a task sleeping the whole period with the callback and everything it captured (a tab's states, closures and their upvalues). A component polling every minute or a toast timing out in five held its state that long, a leak of 100 and more states a chat tab as Anara closes tabs faster than the timers fall due. Stopping a timer (unmount, a reload, the returned stop function) now wakes its task to end at once. A test mounts and unmounts such a component 40 times and finds no states left.
