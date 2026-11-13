---
section: changed
---
- **`zgx inspect gc` says when `--since-mark` has no mark and when counts are since it** (zgx-script): a reload gives the script a new VM, which drops the mark (the objects it remembered are gone), and the counts then are all of them. The no-mark line now says so, and a `--retained` answer says "(made since the mark)" when it is a delta.
