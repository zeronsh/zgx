---
section: fixed
---
- **`ui.write_file` writes any bytes, and `ui.read_file` reads them** (zgx-script): a string with a byte that isn't UTF-8 (a PNG, any binary file) failed with "error converting Lua string to String" and wrote nothing; it is now written as its bytes. `ui.write_file(file, { base64 = … })` writes bytes given in base64 (a rendered canvas's PNG), up to 256 MiB. `ui.read_file(file, { base64 = true })` reads a file's bytes whole, in base64, where text replaced each byte that isn't UTF-8.
