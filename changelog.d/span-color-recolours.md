---
section: performance
---
- **A span's colour function recolours its paragraph instead of lowering it again** (zgx-script): `ui.span { color = function() … end }` changing (a streamed reply's words fading in from script) rewrites the paragraph's colours, shaped as it was, and damages only the runs whose colours changed. Before, each change made every span of the paragraph again and repainted its whole box. On the gate's streaming transcript, frames between chunks cost about a third less and damage about a tenth as much with boost off.
