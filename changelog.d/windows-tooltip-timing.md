---
section: changed
---
- **Tooltips on Windows wait as WinUI's do** (zgx, zgx-desktop): twice the system's mouse hover time before showing (800 ms by default) and the hover time within 200 ms of another opening (WinUI's one and a half times truncates to one), read from `SPI_GETMOUSEHOVERTIME` with the system's other settings. macOS and Linux keep 300 ms and an immediate reshow within a second. The timing is `zgx::compose::TooltipTiming` (`set_tooltip_timing`); a tooltip's own `delay` still waits its own.
