---
section: fixed
---
- **A plugin replaces the root of a presence's content** (zgx-script): `ui.contribute { target, replace }` did nothing, without an error, when its target was the root a `ui.presence` builder returns, at one level or nested; a child inside the branch was replaced. The root is now replaced where the branch builds it, each entry as it enters, leaving with it, as `before` and `after` there already were.
