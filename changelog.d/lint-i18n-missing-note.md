---
section: changed
---
- **`zgx lint` notes a locale's untranslated keys once, and still passes** (zgx-script): `i18n-missing` was a warning a key and failed the run, so a catalog mid-translation (thousands of keys across 30 locales) buried every other finding. It is now one note a locale (how many of the source's keys it lacks, and the first few), printed and in `--format json` with its `severity`, and neither counted as failing nor in a debug build's lint strip. `Severity::Note` and `Report::passes` are new.
