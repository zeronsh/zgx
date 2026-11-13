---
section: fixed
---
- **`zgx test` gives an app seconds, not milliseconds, and clears what killed runs left** (zgx-script): a test's fixtures can be heavy and the machine busy (CPU time grows under load too), so a mounted app now gets 5 s per call and 30 s to load, where a loop that never ends is still stopped. A run also removes the `zgx-test-storage-<pid>-<n>` directories of runs that were killed (a process that is gone, or one older than a day): a SIGKILL, a crash or a power cut cannot remove its own, and thousands had piled up.
