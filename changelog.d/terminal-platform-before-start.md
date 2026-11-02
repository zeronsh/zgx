---
section: fixed
---
- **A terminal program's first clipboard request is answered** (zgx-terminal): an engine-drawn terminal was given the app's platform (its clipboard) only after its session had started and been read once, so on a busy machine a program that copied to the clipboard or asked to read it (OSC 52) as it started could have its copy dropped or its question answered empty. The platform is now the terminal's before its session starts.
