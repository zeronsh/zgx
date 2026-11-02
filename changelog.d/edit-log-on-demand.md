---
section: performance
---
- **A long text keeps its edits only for a reader that follows them** (zgx): every edit of a long text (a keystroke in an editor on a long document) recorded itself in the text's log, read or not; a text now records edits only once a reader that follows it by them asks (`Text::keep_edits`: an editor bound to it, when the value changes from outside, and `ui.markdown_view`), and otherwise only takes a new id. A stream into a bound editor takes its first token whole, then each one as it is.
