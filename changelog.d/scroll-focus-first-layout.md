---
section: fixed
---
- **A modal whose first control is in a scroll view lays the view out at its own size from the start** (zgx): focusing the control as the modal opened laid the scene out before the view's styles reached it, so its content was first 320 points wide (a scroll view's default) and a ref in it, or a spring following one, heard 320 before the view's own width. A first control below the fold is also shown as the modal opens: through nested scroll views it wasn't.
