---
section: fixed
---
- **A scroll group isn't left scrolled past its lines** (zgx): a code block scrolled sideways, then cut short by an edit made elsewhere (`EditorValue::set`, undo), showed nothing, its lines left of its frame. It comes back to as far as its lines reach, and `EditorHandle::on_group_scroll` is told once the draw is done.
