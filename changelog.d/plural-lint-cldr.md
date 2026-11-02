---
section: fixed
---
- **`i18n-plural` asks each locale for CLDR's plural cases in every build** (zgx-script): without the `i18n` feature (the `zgx` command, test hosts) the lint asked every locale for English's `one`, so a Chinese, Japanese, Korean, Thai or Vietnamese plural with `other` alone, which is complete, was reported (50 times in one catalog). It now reads ICU4X's categories for each catalog's locale whatever the features.
- **A debug build's lint findings never cover the interface** (zgx-script): they showed as the "Script error" card, a modal that took the pointer and hid what lay under it. They now count in a strip in the window's lower corner, never as an error nor over a modal, and a test host (`testing::TestApp`, or `ScriptHost::lint_strip(false)`) shows none.
