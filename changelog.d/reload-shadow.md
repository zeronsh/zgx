---
section: added
---
- **`zgx inspect reloads` says what each hot reload would keep** (zgx-script): while a host watches its files, each load records its modules (what each requires, and what its top level registers that outlives the call), and each reload says what a reload keeping the modules a save didn't change would have done: `partial`, with the modules that would run again and how many kept, or `full` and why (plugins, a file that isn't a module, a module setting a global or starting a task or animation at its top level). Nothing is kept yet: this is the record the next step is judged by (#1369). A host that never watches records nothing.
