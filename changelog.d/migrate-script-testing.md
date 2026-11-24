---
section: changed
---
- **`TestApp` is made by its functions, and an inspect `Server` isn't unwind safe** (zgx-script): `TestApp` has private fields, so a literal of it no longer compiles (`TestApp::inline` and its other constructors are as they were). `inspect::Server` is no longer `UnwindSafe` or `RefUnwindSafe`: wrap what captures one in `AssertUnwindSafe` to pass it to `catch_unwind`.
