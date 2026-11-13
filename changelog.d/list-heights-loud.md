---
section: fixed
---
- **`ui.list`'s `heights` says so when it isn't a list of numbers** (zgx-script): a function, `ui.computed` or state giving nil, a number or a list holding anything but numbers reported an empty list's zero rows without a word; it is a script error now (a `ui.computed` of numbers works, as before). The types and docs say that with `key`, `row` receives the key (`row(key, index)`), not the index.
