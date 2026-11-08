---
section: added
---

Editors expose exact natural text metrics at a requested content width through
`EditorHandle::text_metrics` and script elements' `text_metrics(width)`. Metrics
include styled spans and IME composition, follow content and typography after
layout, and stay cached across focus, selection, scroll and viewport changes.
