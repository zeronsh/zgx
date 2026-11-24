---
section: changed
---
- **A native module's write to a shared state whose host is gone fails** (zgx-module, zgx-script): `Shared::set` and `Shared::remove` return `Result<bool, Failed>`, and they, `append`, `upsert` and `Host::image` fail with "its host is gone" instead of reporting success, so a module can tell that what it wrote went nowhere. In the interface, `shared_set` and `shared_remove` return `HOST_GONE` (not a count) then, and the writes that return `DONE` or `FAILED` fail saying so. Native modules are unstable before 1.0.
