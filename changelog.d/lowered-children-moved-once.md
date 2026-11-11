---
section: performance
---
- **A script's children are moved once as they lower** (zgx-script): each child's view goes from `lower` straight into a vector the parent keeps (`View::children_vec`), where it was moved again by `children`. The mount benchmark does 0.17% fewer instructions.
