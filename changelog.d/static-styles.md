---
section: performance
---
- **A style that can't change keeps nothing after it is applied** (zgx): a node styled only with literals (no theme tokens, interaction styles, disabled state, typography of its own, backdrop blur or reactive style; not text) is styled once, where its effect's first run would style it, and keeps no effect, subscription or memo; mounting rows of such boxes costs less and they keep less in memory.
