---
section: added
---
- **Snapshots picture what Direct3D 11 drew, and Wayland's popups** (zgx-gpu, zgx-desktop): `zgx inspect snapshot` (`Ui::snapshot`) read nothing from a window drawn with Direct3D 11 directly, nor from a menu or popup on Wayland. A window on Direct3D 11 now reads the region from the swap chain's buffer it showed (an offscreen renderer, its target), copying that region alone; and a Wayland popup keeps what it showed, as X11's and Windows' do.
