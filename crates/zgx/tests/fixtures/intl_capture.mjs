// Captures what Node's Intl gives for the cases crates/zgx/tests/.../intl.rs
// checks: `node intl_capture.mjs > intl_node.json`. Node 22.20 (ICU 77).
const locales = ["en", "de", "ja", "fi"];
const t = Date.UTC(2026, 10, 12, 15, 30, 45);
const out = { node: process.version, icu: process.versions.icu, cldr: process.versions.cldr, cases: [] };
const add = (fn, locale, args, f) => out.cases.push({ fn, locale, args, expected: f() });
for (const l of locales) {
  for (const type of ["conjunction", "disjunction", "unit"])
    for (const style of ["long", "short", "narrow"])
      for (const items of [["a", "b", "c"], ["a", "b"], ["a"]])
        add("list", l, { items, options: { type, style } }, () =>
          new Intl.ListFormat(l, { type, style }).format(items));
  for (const [n, o] of [
    [1234567.891, {}],
    [1234567.891, { maximumFractionDigits: 0 }],
    [1234.5, { minimumFractionDigits: 2 }],
    [1234567.891, { useGrouping: false }],
    [-1234.5, {}],
    [0.256, { style: "percent" }],
    [0.256, { style: "percent", maximumFractionDigits: 1 }],
    [1234.5, { style: "currency", currency: "USD" }],
    [1234.5, { style: "currency", currency: "EUR" }],
    [1234.5, { style: "currency", currency: "JPY" }],
    [-1234.5, { style: "currency", currency: "USD" }],
  ]) add("number", l, { n, options: o }, () => new Intl.NumberFormat(l, o).format(n));
  for (const o of [
    { dateStyle: "full" }, { dateStyle: "long" }, { dateStyle: "medium" }, { dateStyle: "short" },
    { timeStyle: "short" }, { timeStyle: "medium" },
    { dateStyle: "medium", timeStyle: "short" },
    { month: "short", day: "numeric" }, { year: "numeric", month: "long" },
    { weekday: "long" }, { month: "long", day: "numeric", year: "numeric" },
  ]) add("date", l, { t, options: o }, () => new Intl.DateTimeFormat(l, { timeZone: "UTC", ...o }).format(t));
  for (const [d, u, numeric, style] of [
    [-1, "day", "always", "long"], [2, "hour", "always", "long"], [-3, "month", "always", "long"],
    [1, "week", "always", "long"], [-5, "minute", "always", "short"], [-1, "day", "auto", "long"],
    [0, "day", "auto", "long"], [1, "year", "auto", "long"],
  ]) add("relative", l, { delta: d, unit: u, options: { numeric, style } }, () =>
    new Intl.RelativeTimeFormat(l, { numeric, style }).format(d, u));
}
console.log(JSON.stringify(out, null, 1));
