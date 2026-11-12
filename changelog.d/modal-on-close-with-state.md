---
section: fixed
---
- **A `ui.modal` with a state `open` runs its `on_close`** (zgx, zgx-script): `on_close` was only heard when `open` was a function, so a modal given a state and an `on_close` (the usual shape) never ran it on Escape or a press on the backdrop. It now runs as the person closes it, after the state is written, and not when the app sets the state itself. `View::on_dismiss` is the Rust side.
