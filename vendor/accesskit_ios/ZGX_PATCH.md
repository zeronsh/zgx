# zgx patch to accesskit_ios 0.1.2

Vendored from crates.io `accesskit_ios` 0.1.2 (MIT OR Apache-2.0; the published
crate ships no license files) and applied through `[patch.crates-io]` in the
workspace `Cargo.toml`. Changes are marked "zgx patch" in the source.

Each change was found by reading what a running zgx app offers assistive
technology beside what UIKit offers for the same screen (`ZGX_IOS_AX_DUMP=1`).

1. **Static text and headings speak a label.** AccessKit keeps their text as
   the value, and iOS read that as a value with no label; UILabel has a label
   and no value (`NodeWrapper::label`, `value`).
2. **A switch reads as UIKit's cell with a UISwitch**: a button and static
   text, with the toggle trait and a value of 0 or 1 (`traits`).
