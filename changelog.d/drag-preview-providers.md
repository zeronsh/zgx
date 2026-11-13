---
section: fixed
---
- **A drag preview built from a component finds its source's providers** (zgx): `drag_preview` mounted its view with no services, so a preview whose component asked for the frame clock (anything animated) panicked with "missing component provider for zgx::frame::FrameClock" as the drag began. It now mounts under the providers its source was built under.
