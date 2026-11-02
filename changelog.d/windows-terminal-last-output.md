---
section: fixed
---
- **A Windows program's last output shows after it exits** (zgx-terminal): ConPTY renders what a program last wrote after the program has gone, and an engine-drawn terminal stopped reading at the exit, so a program's last lines were often lost: ping's statistics after Ctrl+C, a build's final error. The terminal now closes the pseudoconsole and reads its output to the end (five seconds at most) before reporting the exit. `zgx-alacritty-terminal` is 0.26.1. The output pipe now ends as the pseudoconsole closes (zgx no longer keeps its own copies of the console's ends of the pipes), so an exit is reported as soon as the last output is read.
