---
section: fixed
---
- **A finished animation of a state or a table lets its properties go** (zgx-script): as a cancelled one did already. The engine kept an owner for every state or table property any animation had played, for as long as the app ran, and with each one the animation's allocation. An app animating a fresh state each time (a reply's clock, a row's value) held one per use. A memory report now counts the clips playing and the state and table properties owned (`anime` lines).
