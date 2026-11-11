---
section: performance
---
- **What `compile_ahead` holds and hashes is lighter** (zgx-script): a module compiled ahead is let go as soon as its `require` has loaded it, not held to the end of the load. Each file's cache key is made on the worker, not on the main thread as the load takes it. A key hashes at most a source's first and last 64 KB, and a hit is still compared byte for byte. The worker finds a file's `require`s with a substring search, not a seven-byte comparison at every byte.
