---
section: performance
---
- **A package's modules compile once for every host that requires them** (zgx-script): a script host made after another or beside it, on any thread, loads a package's bytecode (`ScriptHost::package`, zgx.json's `"packages"`; zgx-video's `@video` and `@fx`) rather than compiling it again: a video's stages, its render workers and a player building its stage again compile `@video` once. No host holds it: the shared table keeps it until a host goes idle, which releases it all, so an app at rest holds what it did before. A hot-reloading host keeps its packages' modules by path, as its other files.
