---
section: performance
---
- **A terminal tells a screen reader of new output for what changed, not the whole screen** (zgx-terminal): each read's news compared every row on screen and searched what was typed for the whole of it, so a line streaming in at the bottom cost the screen's text, more with every row. Rows that didn't all change are compared alone, and output longer than what was typed is said without searching for its echo. On a full screen of identical lines, a line rewritten in place is now said alone: the search for a scroll took it, and every line below it, for lines scrolled in.
