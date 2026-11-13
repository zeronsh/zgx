---
section: fixed
---
- **A drag preview is built with what its source's tree provides** (zgx): the preview was mounted as a tree of its own with no services, so a component in it that asked for the window's frame clock (an animated mask, an animation) panicked with "missing component provider for zgx::frame::FrameClock", which showed the error banner and dropped the inspector's connection as a drag started. It is mounted with its source's services and theme scope now.
