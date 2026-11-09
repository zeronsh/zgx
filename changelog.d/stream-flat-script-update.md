---
section: performance
---
- **A streamed Markdown chunk's script and update cost the same at 800 sections as at 100** (zgx, zgx-script): `ui.markdown` given a state answers its blocks as a list grown from the last answer, in chunks it shares with it (held folded, so no other value pays for it), and `ui.each` hands `keyed` only the keys after those that stayed (`keyed_grown`, `Keys::Grown`), which it moves and orders alone (`Scene::reorder_children_from`). An edit to a long text finds the line breaks of what it adds alone, and a text without carriage returns is not read for them. Per chunk, script 98K → 103K and update 137K → 135K instructions from 100 to 800 sections (they had grown to 127K and 367K); `md_chunk_flat` holds both flat.
