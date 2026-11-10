---
section: added
---
- **A scroll view's or a list's scrollbar can be the app's own** (zgx, zgx-script): `scrollbar = { … }` in Luau and `View::scrollbar_style` in Rust set its thickness (under the pointer too), strip and side, how far its track stops short of each end, its shortest thumb, how long it lingers and fades (fading in too, on `outQuad`), a track shorter than the view, a hint that shows it, the theme's active thumb colour under the pointer, no track, a press beside the thumb that brings the thumb there, whether a press focuses it, and whether assistive technology has it, over the platform's look or zgx's own on every platform. An app's floating rail runs no script as its view scrolls: `track` and `hint` are read as what they read changes.
