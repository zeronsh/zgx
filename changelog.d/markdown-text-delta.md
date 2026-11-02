---
section: performance
---
- **A Markdown view follows a long reply by its appends** (zgx, zgx-script): `ui.markdown_view { "{reply}" }` reads the state's text where it lies, rather than copying it into a string each update; a long reply streamed in with `reply:append(token)` (Rust: the new `Shared::append`) is parsed from the inserts its text keeps (the new `Text::id` and `Text::inserts_since`), with no copy and no comparison of the document. With 256 paragraphs, an update costs 2.4% less when the reply is `set` whole (at the same or lower peak heap) and 6.7% less when it is appended to (the long text keeps its last edits, about 4 KB). Any other edit reads the text whole, as before.
