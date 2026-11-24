---
section: performance
---
- **An app's scripts are compiled ahead of its first load on several threads, and more of them** (zgx-script, `compile_ahead`, which zgx-run calls): a module named by a string (`pcall(require, "@app/theme")`, a list of screens a function of the script's own requires) is compiled ahead as a literal `require` is, where only literals were followed, and up to four workers (at most half the machine's processors) take the files in the order the load requires them. On Zeron's main window, 286 modules, the first frame comes 52 ms sooner on later launches and 45 ms on the first (Linux). A module named and never required is let go as the load ends, as are the workers.
