---
section: added
---
- **Plugin targets by class and inside other elements** (zgx-script): `ui.contribute { target }` and `ui.filter` take `.class` (every element with it) and descendant chains, `#sidebar >> .session-row`, in the selector syntax tests use. A target matching several elements contributes at each, and a chain only where its ancestors hold, including inside a list's rows. `zgx lint` warns of an id, hook or class in a target, a chain's parts too, that no element of the app declares; under strict types an app's classes are targets (`HookTarget`) and a chain is any text (`HookChain`).
