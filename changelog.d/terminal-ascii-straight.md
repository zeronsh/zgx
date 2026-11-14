---
section: performance
---
- **Plain ASCII output is dispatched without a UTF-8 check** (zgx-terminal): each run of output between escapes was checked as UTF-8 and then walked again for its printable text; ASCII now goes straight, and the check starts only at the first byte that isn't ASCII. Printing a coloured 20 MB log takes 4% fewer instructions.
