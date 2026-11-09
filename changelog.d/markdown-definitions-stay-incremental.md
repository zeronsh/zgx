---
section: performance
---
- **A link definition no longer makes every later chunk of a streamed Markdown reply parse the whole reply** (zgx-script): what only looks like one (`[x]: y` in a code block) costs nothing, a real one with no reference ahead of it costs nothing beyond its tail, its later references following it, and one with a reference ahead of it parses the whole text as it lands. Per chunk at 800 sections, 27M instructions in update become 120K. A whole parse keeps the values of the blocks that come out the same.
