---
section: fixed
---
- **At 120 Hz, scrolling and animation no longer drop a frame now and then** (zgx-desktop, macOS): frames the display paces tick from the view's own display link and hand over as the refresh begins, as a native Metal view's do, instead of just before the window server decides which refresh shows them. A frame shows a refresh later than before, as a native window's does. 60 Hz keeps its timing. `ZGX_VIEW_CLOCK=0` restores the old pacing.
