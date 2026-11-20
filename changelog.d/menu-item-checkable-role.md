---
section: fixed
---
- **`ui.menu_item { role = "menuitemcheckbox", checked = … }` works** (zgx-script): `ui.menu_item` takes `checked` into its options before the role is read, so the role then failed with "give it `checked`" even when it had one. The role now finds it there.
