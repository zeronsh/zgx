---
section: fixed
---
- **Chinese text draws on macOS 26 again** (zgx-cosmic-text, zgx-gpu): a font face with no outlines zgx can draw, such as macOS 26's PingFang (its shapes only in Apple's `hvgl` table), is passed by in fallback, and Chinese takes Heiti SC or TC instead. In 0.1.7 every Chinese character on macOS 26 drew as blank space.
