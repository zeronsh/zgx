---
section: fixed
---
- **A module first required after the load reloads when it is saved** (zgx-script): a `require` in a build that runs later (a page built when it opens, a `ui.show` branch) or in a handler added the module to what a reload reads, but `ScriptHost::watch` asked the system to watch only what the last load had required, so saving it reloaded nothing until another file changed. Requiring a module now has the watch look at its files again, once for all the modules a step requires; a host not watching does nothing more.
