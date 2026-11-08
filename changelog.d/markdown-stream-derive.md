---
section: performance
---
- **A streamed Markdown reply converts only its new blocks** (zgx-script): each chunk of a keyed `ui.markdown` stream answered a new list, which a script reading it converted whole, weighing and walking every block it held; its table is now derived from the last answer's, converting and weighing only what changed. A 400-section reply streamed in costs 21.5% fewer instructions (`md-stream`, a new gate cell).
