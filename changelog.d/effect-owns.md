---
section: changed
---
- **An effect's run owns what it makes** (zgx, zgx-script): a `ui.on_cleanup`, `ui.after` or `ui.every` made in a `ui.effect` now goes before the effect runs again, and with the effect, as the docs said and a React effect's cleanup does. Before, a run's cleanups and timers went to whatever was being built at the time, or to the script until it reloaded: content a `ui.presence` swapped out kept its effects' cleanups, and an effect's cleanups didn't run between its runs. A `ui.computed` run owns what it makes until its next run (start timers in an effect or a handler). `Runtime::own` and `Runtime::running` let a host give a resource to the running effect.
