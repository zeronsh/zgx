---
section: fixed
---
- **A window drawing only video frames lets go of each it no longer shows as it draws on** (zgx-gpu): with no picture beside them, it kept every native surface it had shown until it went idle (up to the 64 MiB the cache allows), each holding a buffer its pool couldn't reuse, and where it did let go its only flush came before CoreVideo's cache would.
