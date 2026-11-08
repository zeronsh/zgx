---
section: changed
---
- **`WindowCapabilities` is `#[non_exhaustive]`** (zgx-desktop): apps read it and only zgx builds one, so it can gain fields without breaking an app. Code that built one with a struct literal should start from `WindowCapabilities::default()`.
