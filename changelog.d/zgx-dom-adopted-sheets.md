---
section: added
---
- **zgx-dom under a strict Content-Security-Policy** (`zgx-dom`): its classes go in a constructed style sheet the document adopts, so a page whose `style-src` blocks inline `<style>` still renders styled. Browsers that can't adopt a sheet keep a `<style>` element.
