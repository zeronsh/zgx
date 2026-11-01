---
section: fixed
---
- **A keystroke in a long bound document no longer reads all of it** (zgx). Whether the bound text needed its line breaks normalized was found by scanning every chunk, on every keystroke. A text now counts its carriage returns where it is made and in the chunks an edit makes again, so the check is constant time: a keystroke on a 100,000-line file costs 1.8 M instructions, down from 5.3 M. A short bound text now takes each edit where it is, rather than a new copy of the whole text, and the editor doesn't compare the value it has just written.
