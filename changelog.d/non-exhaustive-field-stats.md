---
section: changed
---
- **`theme::Field` and `DebugCacheStats` are `#[non_exhaustive]`** (zgx, zgx-gpu): a token can come to set another property, and the stats to count another cache, without breaking an app. A `match` on a `Field` needs a `_` arm and a pattern naming the stats' fields ends with `..`; only zgx builds a `DebugCacheStats`. New since 0.1.10: `Field`'s `GapX`, `GapY`, `MinWidth`, `MaxWidth`, `MinHeight`, `MaxHeight` and `OutlineInnerColor`, and the stats' `images_softer`, `layers_coarser`, `shader_pipelines` and `surface_collections`. Naming and reading them is as it was.
