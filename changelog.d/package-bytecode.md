---
section: performance
---
- **A package's modules compile once for every host that requires them** (zgx-script): a script host made after another or beside it, on any thread, loads a package's bytecode (`ScriptHost::package`, zgx.json's `"packages"`; zgx-video's `@video` and `@fx`) rather than compiling it again: a video's stages, its render workers and a player building its stage again compile `@video` once. Each host holds what its last load required; what no host holds is released as a host goes idle, so a process at rest keeps none of it.
