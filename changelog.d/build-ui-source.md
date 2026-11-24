---
section: added
---
- **`zgx build --ui source` writes a bundle whose scripts stay source** (zgx-script, zgx-run): for an app whose people lay their own files over its interface, or edit it where it is installed, which a bundle of checked bytecode can't be. Every file of its folder ships (one that looks like a secret fails the build), and `ui/` is run as any folder is; its native modules are built, cached and checked as in any bundle. Bytecode stays the default, its format as it was. `Opened::files` is an `Option` now, `None` for source.
