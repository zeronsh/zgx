---
section: fixed
---
- **A terminal names the program in front after it execs another** (zgx-terminal): an engine-drawn terminal kept the name it first read for a foreground process as long as the process's pid stayed in front, but a process keeps its pid when it runs another program in its place. So the shell, read in the moment between its session forking it and its running, could be named after the app for the whole of a quiet command (`sleep 30`), and `exec less` went on showing the shell. The name is now read again at each check, and the check runs once output settles (a moment after the latest output, not the first of a burst), so a command that goes quiet at once, like `sleep 30` typed at a prompt, is named too. Checks still follow output alone: nothing polls.
