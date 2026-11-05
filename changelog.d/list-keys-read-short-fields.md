---
section: fixed
---
- **A list's keys leave long rows off the screen out of the script** (zgx-script): a `ui.list` of `data` with a `key` copied every row into the script to ask their keys, before showing any. A row of 2 KB or more now gives `key` its numbers, booleans and short strings, so 2,000 rows holding 40 MB of text grow the script's heap by under 3 MB, not 41 MB. A key that reads anything else is given the whole item, as before.
