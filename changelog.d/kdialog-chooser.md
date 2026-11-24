---
section: added
---
- **kdialog's file chooser where no portal answers** (zgx-desktop, Linux): on a KDE session (`XDG_CURRENT_DESKTOP` naming KDE) with no `org.freedesktop.portal.FileChooser`, open, save and folder dialogs show kdialog's chooser, KDE's own, before zenity's; elsewhere kdialog stands in where zenity is missing. It is run, attached to the window on X11 and ended with it as zenity's is.
