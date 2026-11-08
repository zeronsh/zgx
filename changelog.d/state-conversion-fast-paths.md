---
section: performance
---
- **A table a script puts in state is read faster** (zgx-script): its strings take the fast UTF-8 check, and the converter's address-keyed maps (strings by their Luau string, values held by the cache) hash by a multiply, not SipHash. The mail example's script load, which puts 4,000 messages in state, does 9.7% fewer instructions (221.4M → 200.0M), with the same values and tables.
