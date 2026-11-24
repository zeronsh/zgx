---
section: changed
---
- **`WindowCapabilities` is `#[non_exhaustive]`** (zgx-desktop): apps read it and only zgx builds one, so it can gain fields without breaking an app. Code that built one with a struct literal starts from `let mut can = WindowCapabilities::default();` and assigns its fields (a literal with `..Default::default()` isn't allowed outside zgx-desktop either), and a pattern naming its fields ends with `..`.
