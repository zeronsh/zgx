---
section: fixed
---
- **A host name registered twice says so** (zgx-script): in a debug build, an `action`, `function` or `async_action` registered again from another place in the host's code warns once, naming both places, on stderr, in `ScriptHost::warnings` and in the lint strip. The later handler answers, as before. One registered again from the same place (a host rebuilt) stays silent. A release build records nothing.
