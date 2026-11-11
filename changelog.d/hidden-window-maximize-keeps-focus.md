---
section: fixed
---
- **A window that reopens maximized keeps the keyboard focus on Windows** (winit): a hidden window that was maximized before it showed was shown, focused and hidden again by `SW_MAXIMIZE`, leaving the thread with no focus, so every key dinged until the window was reactivated. It now maximizes as it shows (`SW_SHOWMAXIMIZED`) and keeps the size it restores to.
