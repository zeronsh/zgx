---
section: added
---
- **A web view can forget a site** (zgx-script, zgx-desktop): `web:clear_site_data(url)`, a promise of how many records went, removes everything the page's profile keeps for the url's site, its registrable domain and the subdomains under it (cookies, local and session storage, IndexedDB, caches, service workers), for every web view on that profile, as a browser's "Remove site data" does: an app's "Forget login for this site". Pages open keep what they hold in memory until they reload. macOS and Linux remove WebKit's own records for the site; Windows clears the host's origins and the cookies a request to it carries, so storage of the site's other subdomains, and cookies one set for itself alone, stay there.
