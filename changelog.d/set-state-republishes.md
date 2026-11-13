---
section: changed
---
- **`app:set_state` and `zgx inspect set` write as a host republish does** (zgx-script): the parts equal to the state's value are kept, so tests and inspect model `Shared::set` and a script's next read reuses the tables of the rows that did not change.
