---
section: performance
---
- **A pointer move hit tests once** (zgx): its dispatch and its cursor asked the scene the same, each walking it; the last answer is kept with the revisions it was found at. A clip changing, children reordered and a paint order changing now bump `Scene::interaction_revision`, which the host's cursor cache keys on too.
