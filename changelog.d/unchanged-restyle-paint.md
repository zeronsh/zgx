---
section: performance
---
- **A style re-run with the same result writes no paint** (zgx): a reactive style that sets no paint and ran again to the same styles (a box whose colour a moved selection reads, but which did not change colour) re-laid and re-wrote its unchanged effects, transform, clip and mask each time; after one such run it writes nothing, as a box changing only its size already did, and leaves what else set them since. `update_bench` function-prop, where 198 of 200 boxes keep their colour each update, falls 13.6% of its updates (26.8 M instructions in 200).
