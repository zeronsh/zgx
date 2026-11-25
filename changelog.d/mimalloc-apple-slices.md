---
section: performance
---
- **zgx's builds for Apple's platforms give mimalloc slices of one 16 KiB page** (`.cargo/config.toml`): what was freed goes back to the system in smaller runs. A heavy list on the iOS simulator rested 29 MB lower after a fling and launched 15 MB lower; a macOS window rests 5 to 8 MB lower. `docs/getting-started.md` says how an app sets the same.
