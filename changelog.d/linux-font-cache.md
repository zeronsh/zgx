---
section: performance
---
- **Linux reads its system fonts from a cache** (zgx-gpu), as macOS and Windows do. At every launch fontdb parsed fontconfig's configuration and opened every installed font. With 113 fonts that cost 11.7 M instructions; from the cache it costs 0.46 M (-96%). The cache lives in `$XDG_CACHE_HOME/zgx/` (or `~/.cache/zgx/`). It is used while the configuration files fontconfig read, the font directories they name and the environment fontdb reads (`HOME`, `XDG_CONFIG_HOME`, `FONTCONFIG_FILE`) are unchanged, and it keeps the generic families fontconfig's aliases chose. A launch that finds them changed loads the fonts as before and writes the cache again, once. The cache file's format is now 4 on every platform, so each rebuilds its cache once.
