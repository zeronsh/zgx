---
section: added
---
- **`ui.fetch` in the browser** (zgx-dom, `luau`): `zgx_dom::install_fetcher(&host)` gives a Luau app the page's `fetch`. Relative URLs resolve against the app's base (`document.baseURI` when installed, or the URL `install_fetcher_at` names). The desktop's grant and `FetchPolicy` checks apply. Failures reject as `"network"` errors, and a cancelled fetch is aborted. Its JavaScript ships only with `luau`.
