---
section: fixed
---
- **A spellchecked editor lets go of what it holds as it unmounts** (zgx): the editor kept the closure that rechecks its spelling, and that closure held the editor, so a spellchecked field was never freed, and with it everything its handlers captured (a chat composer's state). The editor now holds the recheck weakly.
