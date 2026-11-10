---
section: fixed
---
- **A frame drawn from its kept frame no longer drops or doubles text**: on windows drawing in a single pass, its draws were batched after the kept frame had gone, by the rules for draws in order, so a line could go missing or draw twice, brighter.
