---
section: added
---
- **`zgx lint` flags whole-reply passes around a streamed `ui.markdown`** (zgx-script): `computed-markdown-blocks` for a computed reading all of a markdown state's blocks, `computed-markdown-rewrite` for `ui.markdown` given text a computed rewrites whole. Each makes a chunk cost the reply's length; the row builder and rewriting each piece as it's appended stay flat.
