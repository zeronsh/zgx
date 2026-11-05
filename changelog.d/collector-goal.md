---
section: performance
---
- **The collector leaves less behind** (zgx-script): a script VM's collector aims for 125% of the heap in use, where Luau's default lets it double. It can't see what a script's objects hold in Rust, so a doubled heap keeps long-gone rows and all they hold. In a long session of a real app, about 40 MB less of the script heap at its peak and 10 MB less at rest.
