---
section: fixed
---
- **An input method asking about an editor that is gone no longer ends the app** (zgx, zgx-desktop, Windows): Windows' Text Services Framework can ask where text is from inside its own message handling, after the focused editor left the scene and before zgx tells it focus moved. `Scene::text_range_area` now answers `None` for a node no longer in the scene, and `EditorHandle::input_method_area` an empty rect, where both panicked (`stale scene node`). A panic in the text store's COM methods now fails that call instead of aborting the process, which a panic can't unwind out of.
