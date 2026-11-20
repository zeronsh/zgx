---
section: fixed
---
- **`cargo test` leaves no temp dirs behind** (tooling): each test binary runs under its own `TMPDIR` (`scripts/test-runner.sh`), removed when it exits, and a run killed before that is swept by the next.
