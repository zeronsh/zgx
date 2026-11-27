---
title: CLI reference
description: Every command of zgx and zgx-run, as their --help prints it, with exit codes and the lint rules.
---

Each block below is read from the binary's source when the site builds, so it says what `--help` says.

## Install

```sh
cargo install zgx-run
cargo install zgx-script --bin zgx
```

From crates.io; `--git https://github.com/zeronsh/zgx` before a crate's name installs what isn't released yet.

`zgx check` needs [luau-lsp](https://github.com/JohnnyMorganz/luau-lsp/releases) on the `PATH`; `zgx lint` and `zgx inspect` need nothing else.

## zgx-run

<!-- usage crates/zgx-run/src/main.rs USAGE -->

## zgx

<!-- usage crates/zgx-script/src/bin/zgx.rs USAGE -->

| Command | Exit 0 | Exit 1 | Exit 2 | Exit 3 |
| --- | --- | --- | --- | --- |
| `zgx check` | no errors | type errors | bad usage | luau-lsp not installed |
| `zgx lint` | no findings | findings | bad usage or an unreadable file | |
| `zgx inspect` | success | the app answered with an error | bad usage | no app answers |

## zgx inspect

<!-- usage crates/zgx-script/src/bin/zgx.rs INSPECT_USAGE -->

`--json` wraps every answer as `{"ok":true,"text":…,"data":…}` or `{"ok":false,"error":…}`. The commands are explained in [reading](/devtools/reading) and [acting](/devtools/acting), the selector syntax in [targets](/devtools/targets).

## Lint rules

`zgx lint --rules` prints these. `-- zgx-lint: allow(rule)` on a finding's line, or the line above it, accepts a deliberate exception.

<!-- lint-rules -->
