---
section: added
---
- **Snapshots on Linux and Windows** (zgx-desktop, zgx-gpu): `zgx inspect snapshot`, `Ui::snapshot` and an element's `snapshot()` picture what the window last showed there too, not only on macOS, so CI and headless machines get real pictures of an app's screens. The window's GPU frame is read back for the node's box alone (`ShownFrame::read`), or the software raster's pixels where the window draws on the raster (a machine with no GPU), and a node in a popup's own window (X11, Windows) from that popup's frame. A window drawn with Direct3D 11 takes none yet.
