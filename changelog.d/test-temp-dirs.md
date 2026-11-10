---
section: fixed
---
- **`zgx test` leaves nothing in the temp directory** (zgx-script): each test's storage and each mount's files go once the test's apps have (an app saves as it goes, which wrote its storage again after the test removed it), what is left goes as each file's VM does, a failing or panicking test's too, and on Ctrl-C (Unix) the run removes what it made before it ends. `zgx test` had left a `zgx-test-storage-*` directory for nearly every test, tens of thousands on a machine that runs them often. zgx-script's own test suites leave theirs no more either.
