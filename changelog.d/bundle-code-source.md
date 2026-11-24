---
section: added
---
- **A host can refuse a bundle's bytecode** (zgx-script, hidden for now): with `ScriptHost::code(Code::Source)`, a bundle's scripts load as source text only, as files from the disk always do, and one in bytecode form fails to load. Luau checks no bytecode it loads, so only bytecode the app's own build compiled (`Code::VerifiedBytecode`, what `bundled` keeps) should run: scripts from anywhere else, an update downloaded, are compiled where they run.
