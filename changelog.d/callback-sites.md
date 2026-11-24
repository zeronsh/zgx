---
section: changed
---
- **A contained panic in a host's Rust callback names it and where it was registered** (zgx-script, zgx-desktop): a panic in an action, a function, an async action or the fetcher a script called ends its report with `(in a host action registered at src/main.rs:120)`, the line that handed it to `ScriptHost`, beside where it panicked (which may be deep in a library). It costs a callback nothing as it returns.
