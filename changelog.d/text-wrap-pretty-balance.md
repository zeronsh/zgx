---
section: added
---
- **`wrap = "pretty"`, and `"balance"` as Chrome breaks lines** (zgx, zgx-script): both run Blink's own line-break scorer (the Minikin-style optimal fit with its penalties, its four-line window for `pretty` and six-line limit for `balance`, and its fallback to greedy breaking), so the same widths break in the same places, with the box and its line count unchanged. `balance` no longer narrows the wrap width; lines break at the fitted spaces. `Styled::text_pretty` and `text_fit` join `text_balance`; `Scene::set_text_balance` is now `set_text_fit` and `Scene::text_wrap` is gone (use `fitted_breaks`, `fit_rich`, `fit_str`).
