---
section: added
---
- **An app's windows by name, and the one that opens first, from `zgx.json`** (zgx-run): `"named_windows": { "main": { … }, "intro": { …, "script": "intro.luau" } }` names windows, each with the window keys and platform blocks of the top level and a script of its own if it opens one; `"first": "main"` opens that one, and `zgx run --window intro app/` (`zgx-run --window`) another, a name it doesn't have an error listing those it does (`ui.args` are as they were). A key comes from the first that has it of the named window's platform block, the window, the top level's platform block and the top level.
