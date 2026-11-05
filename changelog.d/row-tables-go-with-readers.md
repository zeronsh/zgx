---
section: performance
---
- **A long list's rows are no copy of it in the VM** (zgx-script): a list's row table, read through `state:at` or `ui.list`, lives while a script holds it. The conversion cache held every row read for as long as the list stood, so scrolling a chat from end to end built a second copy of its transcript in the script heap, which a collection could not free. The cache also gives back its size as a host goes idle.
