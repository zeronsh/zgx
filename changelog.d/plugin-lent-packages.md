---
section: added
---
- **A host lends a package to plugins** (zgx-script): `Capability::Package("ds")` lends package `ds` to a plugin, and `ScriptHost::grant_all_plugins` grants a capability to every plugin, those loaded later too. zgx.json allows `"package:<name>"`. The plugin requires `@ds/…` into its own VM, with module state of its own. A lent module requires only inside lent packages, never `@app/…`, and an error names the missing grant. What it calls is gated by the plugin's grants. An edit to the package reloads its plugins, and `contributions()` lists what each plugin was lent. `zgx check` resolves a plugin's lent packages, and `zgx types` adds their aliases to the plugin's `.luaurc`.
