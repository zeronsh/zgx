---
section: performance
---
- **A script element's style keys are found by their atoms** (zgx-script): each property key a Luau table names (`w`, `bg`, `text_size`) is now known by the atom Luau keeps in the key string, given once per string, instead of through a map keyed by the string's heap address. A lookup costs the same whatever the heap's layout (an unrelated change no longer moves it) and less: `fill_from` −20%, micro-vscroll-variable-luau −0.48%, script-update −0.14%.
