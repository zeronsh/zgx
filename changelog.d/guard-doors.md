---
section: added
---
- **`ScriptHost::guard` covers terminals, web pages and what else leaves the interface** (zgx-script): new `GuardKind`s `Term` (`ui.term`, by its command or the shell: refused, it raises and nothing starts), `TermInput` (a terminal's `:input`, `:paste`, `:binding`), `Web` (`src`, `:load`, `:html`, `html`, and each navigation the platform starts: a link, a form, a redirect, a page's script, history), `Clipboard`, `Dialog`, `File`, `Folder` and `Notification`. A refused `src` leaves the page where it was; a refused navigation is cancelled and reported. What a page fetches once loaded, and in-page `history` steps, no platform shows: `docs/plugins.md` says so, and lists what isn't asked.
