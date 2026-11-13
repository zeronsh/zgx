---
section: fixed
---
- **`zgx check` passes again, and `unknown-token` ignores tables of data** (zgx-script): the definitions typed a `repeat` field bare, which Luau's parser rejects (a keyword), failing every check; it is `["repeat"]` now. The `unknown-token` lint took a key of a plain table (`SELECTED = { outline = "$colour" }`, keyed by variant) for the style field of that name and called the token the wrong kind; a table given to a name that is no style table is data now. (zgx-desktop: an item after the test module moved above it, for clippy.)
