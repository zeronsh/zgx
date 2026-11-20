---
section: added
---
- **Markdown links GFM's autolink literals, when asked** (zgx-script): `ui.markdown(text, { autolinks = true })`, `ui.markdown_view { text, autolinks = true }` and `markdown::Options::autolinks` link `https://…` as written, `www.…` to `http://…` and emails to `mailto:`, leaving out trailing punctuation, an unbalanced `)` and an entity-like `&name;` as GFM's spec says, and keeping the text's marks. As remark-gfm (what a web chat renders with), `ftp://` is no link. Code, links and files hold none, and without the option nothing changes.
