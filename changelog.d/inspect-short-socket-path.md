---
section: fixed
---
- **Devtools serve from an iOS simulator app** (zgx-script): its temporary directory is about 150 bytes, past the 103 a Unix socket's path may be on Apple's systems (107 on Linux), so the socket never bound and `zgx inspect` found no app. Where the temporary directory is too long, an app now serves from `/tmp/zgx-inspect-<uid>` (its user's alone, as before), and `zgx inspect` looks there too; `ZGX_INSPECT_DIR` names another directory for both; a path still too long is an error naming the limit and the override.
