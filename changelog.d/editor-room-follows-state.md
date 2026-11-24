---
section: fixed
---
- **Lines' room follows a highlighter state an edit above them changed** (zgx): a fence typed above a text area's lines, whose `line_insets` depend on the state before them, puts them in its scroll group and room at once; lines out of view kept their old height, and the group none, until the insets were set again. An edit measures again the lines its state reaches, and no more; a text area without line insets does nothing new.
