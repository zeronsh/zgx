---
section: fixed
---
- **Watching scripts for hot reload holds no file open per script on macOS** (zgx-script): one FSEvents stream watches the folders that hold them, where each watched file took a descriptor of its own. An app the Finder starts may open only 256 files, and a few hundred watched scripts used them all, after which pictures, lazily loaded screens and themes failed to open. A file beside a watched one (an editor's swap file, the Finder's `.DS_Store`) or one written in place inside a plugins folder no longer wakes a reload.
