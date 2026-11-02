---
section: fixed
---
- **Terminals opened at once all start on macOS** (zgx-terminal): macOS's `openpty` isn't safe to call from two threads at once while ptys are in use, and an engine-drawn terminal could fail to start ("Terminal · couldn't start: Unknown error: -6"), about once in 4,000 concurrent opens. Ptys are now opened one at a time. `zgx-alacritty-terminal` is 0.26.1.
