---
section: changed
---
- **A script's errors say where** (zgx-script, zgx-desktop): an error names the script line it came from, or the function that failed and what zgx called it for, the same in the terminal, the window's banner and `zgx inspect`.
  - A function returning what zgx can't take leads with its definition: `app.luau:40: the row function of ui.each returned nothing; return a ui node`. This covers each and list rows, lazy `ui.show`/`ui.switch` branches, components and presence content.
  - An error inside a function zgx called names its role first: `in the key function of ui.each (defined at app.luau:7):`, above the script's own line and traceback. This covers keys, handlers, effects, `ui.computed`, computed props and callbacks.
  - A refused argument to a `ui` function names the script line that called it: `runtime error: app.luau:3: …`.
  - A contribution's or plugin page's error, found as the interface builds, names the line that made it.
  - A desktop app's contained panic leads its banner with where it panicked.
