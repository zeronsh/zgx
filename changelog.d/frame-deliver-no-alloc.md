---
section: performance
---
- **A frame's delivery no longer allocates or hashes with SipHash** (zgx): `FrameClock::deliver` worked out each rate's answer in a map made every refresh; it now keeps the handful there are in a list on the stack. The executor's task map and wake set, hashed at every wake, and the clock's cadence use Fx, as task ids and divisors are small integers.
