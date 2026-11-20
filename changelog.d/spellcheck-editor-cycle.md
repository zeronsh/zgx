---
section: fixed
---
- **A spellchecked editor lets go of what it holds as it unmounts** (zgx): the editor kept the closure that rechecks its spelling, and that closure held the editor, so a spellchecked field was never freed, and with it everything its `on_caret` handler captured (a chat composer's state module: about 118 States and 240 KB per closed chat view). The editor now holds the recheck weakly. A test counts the handler's references after an unmount.
