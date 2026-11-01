---
section: fixed
---
- **A broken Ghostty checkout in zgx-ghostty-sys's cache is fetched again**, not a failed build: one left without its files (as a cache keeping only part of a directory leaves it) is deleted and fetched anew, with a `cargo:warning` saying so. Only a checkout the build script made, marked by its `.zgx-ghostty-checkout` file, is ever deleted; an unmarked one stops the build with what to do.
