---
section: added
---
- **Native modules reach their host: threads, timers, shared state and async actions** (zgx-module, zgx-script): a module's threads post jobs to the interface's thread through a `Send` handle (`Host::ui`, `Ui::post`), where values are made; `host.after` and `host.every` run on zgx's clock; `host.shared(key, initial)` is the host's shared state scripts read with `ui.shared`, set reconciling, appended to and observed; `#[export(async)]` takes a `Promise<T>` the module settles later, which scripts get from `ui.call` or the module's table, typed by `zgx types`. Nothing is made until a module uses it, and a module holding its host costs an idle app no wake. Unstable before 1.0, as native modules are. [The guide](docs/modules.md) says how.
