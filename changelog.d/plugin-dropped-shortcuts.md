---
section: added
---
- **A plugin command's dropped shortcut says so** (zgx-script): when a plugin's command declares a shortcut the app's commands (or an earlier plugin's) hold, spelled the same or not, `ScriptHost::dropped_shortcuts` records the key, why ("taken by the app") and the command that holds it, and `ScriptHost::contributions` (`zgx inspect contributions`) lists it as place `shortcut`, for the app to tell the plugin's author.
