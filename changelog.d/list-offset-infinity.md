---
section: fixed
---
- **`math.huge` as a list's offset is its end** (zgx, zgx-script): every kind of `ui.list` (fixed or measured rows, positional or keyed) now takes an offset past its end as the end, and `-math.huge` or NaN as the top, as `ui.scroll` does. Before, a list took any non-finite offset as its top. Setting a bound number to NaN no longer hangs the app: the state and the widget passed it back and forth for ever, since NaN never equals itself.
