---
section: added
---
- **An app sets how long its toasts stay** (zgx-components, zgx-script): `ui.toast_defaults { duration?, warning_duration?, error_duration? }` (milliseconds, `false` until dismissed; what is not given stays as it was) and `Toaster::set_defaults(ToastDefaults { info, warning, error })` set what toasts shown after it stay when they do not say, so an app can have every toast go after 5000 ms, errors included, as Metal's web toaster does. zgx's own defaults are unchanged (5 seconds, 7 for a warning, an error until dismissed) and a toast's own `duration` still wins.
