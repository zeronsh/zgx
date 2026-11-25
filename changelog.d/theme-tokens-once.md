---
section: performance
---
- **`ui.theme.tokens` evaluates the tokens once a call** (zgx, zgx-script): each token it declared evaluated every token there is, so a design system's declarations cost the square of its size. What reads tokens sees those of a call declared together, where it ran at each of them.
