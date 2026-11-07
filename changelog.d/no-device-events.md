---
section: performance
---
- **A window no longer wakes for pointer moves outside it on X11** (zgx-desktop on Linux and Windows): zgx reads none of winit's device events, and now asks for none. X11 sent each move twice, once raw (XInput2) to the whole app wherever the pointer was, and Windows posted a `WM_INPUT` for each move and key of a focused window. On X11, a move outside the window cost the app about 16,000 instructions and now costs nothing, and a move over it costs about 3,600 (13%) fewer. From crates.io's winit, X11 stops raw moves once a window has had focus and lost it, not from the start.
