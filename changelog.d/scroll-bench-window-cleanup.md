---
section: fixed
---
- **Scroll captures keep their original failure** (benchmarking): stop the test window before removing its staged files, and report cleanup errors separately so a failed refresh check cannot turn into a missing-fixture or directory-cleanup error.
