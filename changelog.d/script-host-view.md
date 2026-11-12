---
section: added
---
- **A script shows a view the host makes in Rust** (zgx-script): `zgx_script::host_view(name, || View)` registers a view and `ui.host_view { name = "…" }` mounts it in the script's tree, so it lays out and clips like the script's own elements and stays under its popups, dialogs and layers, as a view composed beside the script's can't. A name nothing registered is an empty box.
