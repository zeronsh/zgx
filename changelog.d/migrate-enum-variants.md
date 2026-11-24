---
section: changed
---
- **Enums with new variants** (zgx, zgx-desktop): a `match` naming every variant handles them, or adds a `_` arm: `InputEvent::PointerMotion` and `PointerLockChange` (a locked pointer's motion, and the lock given or let go), `NativeContent::Glass` (the system's glass, from `ui.glass`) and `GlassMaterial::Liquid`.
