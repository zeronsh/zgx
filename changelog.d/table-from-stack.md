---
section: performance
---
- **A table a script returns reaches Rust without a copy of its type check** (zgx-mlua 0.12.7, zgx-script): a code editor's highlighter result, a row builder's and every other table result is referenced as it lies on the stack. app-code and app-code-search run about 150K fewer instructions, script-update 562K.
