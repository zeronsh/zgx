---
section: performance
---
- **A Markdown view's appends look at its blocks without allocating** (zgx-script): each update of `ui.markdown_view` made a string of every block's kind to find where it is kept; it now reads the kind as it lies and keys each block by plain numbers. An append into a reply with 256 paragraphs above it costs 19% fewer instructions (8% with a 6 KB paragraph streaming), and its keys hold no part of the blocks.
