---
section: performance
---
- **A list's rows are stamped without re-reading their shape** (zgx-dom): each row is checked against the shape the last one was stamped from, part by part as it's read, and is stamped from that template directly when it matches, with no key built, hashed or looked up. Creating 10,000 rows of the table benchmark takes 2% less time, and its script 7% less.
