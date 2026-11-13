---
section: added
---
- **`ui.locale:format_list`, `format_number`, `format_date`, `format_relative`** (zgx, zgx-script, zgx-run): `Intl`-style text in the locale shown, with ECMA-402's option names (list type and style; decimal, percent and currency numbers with fraction digits and grouping; date and time styles or components; relative times with `numeric = "auto"`), by ICU4X. Compared with Node's `Intl` output for en, de, ja and fi (228 cases, all equal, with V8's plain space in place of CLDR's narrow no-break space before AM/PM). Behind the `i18n-intl` feature (zgx, zgx-script, zgx-run), which adds about 1.7 MB to a stripped release build with every locale's data; `zgx::i18n::intl` has the same functions for Rust.
