---
section: fixed
---
- **Screen readers hear all of a terminal's new output, and not its echo** (zgx-terminal): an engine-drawn terminal told readers of new output by setting its live region at each read, and output read in two parts before the frame's accessibility update (a command's output, then the prompt after it) was heard as its last part alone: Orca said `$` and never the output. It now says everything read since its last update with the next frame's. And the shell's echo of each typed character was read out, once a read with nothing new had dropped what was typed: what was typed now waits for its echo.
