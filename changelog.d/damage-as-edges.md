---
section: performance
---
- **Damage is kept by its edges**: a rectangle joining a frame's damage compares with each one held, and worked out each one's far edges to do it. They're now kept, so a frame that damages many places scans for less: `animate` −1.7%, `style` −0.8%, `anime` −0.8%, on arm64. The damage itself is the same, to the bit.
