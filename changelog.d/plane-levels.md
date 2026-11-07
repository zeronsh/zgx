---
section: added
---
- Native views that show over one another get planes between them: what paints over a native view that is itself over another goes on a further overlay (`PlaneSplit` levels, `Plane::Above`), only where views and paint overlap. A plane's drawable is checked to fit its plane before it shows (`zgx-gpu`'s plane fit checks), so a late frame is never stretched over a new size.
