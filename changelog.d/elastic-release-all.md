---
section: fixed
---
- **A scroll view stretched past its edge always springs back** (zgx, macOS trackpads): every stretched view springs back as a gesture ends, not only the last one a gesture took, so a list released past its end while another view takes the next gesture no longer stays shifted with empty space below it; a view whose content fits shows no stretch and lets go of any as the gesture ends. `zgx inspect node` gives a scroll view's offset within its range and any stretch (`scroll`), and `Ui::scroll_report` the same.
