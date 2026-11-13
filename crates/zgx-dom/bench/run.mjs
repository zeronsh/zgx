// Times table pages in headless Chrome: each page has buttons with
// data-id run, runlots, add, update, clear and swaprows, and a
// [data-id=rows] element whose children are rows (a label second, a remove
// control third), as examples/bench.rs builds.
//
//   node run.mjs name=dir [name=dir…]   (each dir served, its index.html run)
//
// CHROME names the browser binary (a chrome-headless-shell, say); RUNS and
// WARMUP set the counts. Each operation is timed from the click to a forced
// style and layout (`document.body.offsetHeight`): script, style and layout,
// not paint, the same way for every page. Prints a Markdown table.
import { chromium } from "playwright-core";
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const RUNS = +(process.env.RUNS || 10);
const WARMUP = +(process.env.WARMUP || 3);
const pages = process.argv.slice(2).map((arg) => {
  const [name, dir] = arg.split("=");
  return { name, dir: path.resolve(dir) };
});
if (!pages.length) {
  console.error("usage: node run.mjs name=dir [name=dir…]");
  process.exit(2);
}

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm", ".css": "text/css" };
const server = http.createServer((req, res) => {
  const [, name, ...rest] = decodeURIComponent(req.url.split("?")[0]).split("/");
  const page = pages.find((p) => p.name === name);
  if (!page) return res.writeHead(404).end();
  let file = path.join(page.dir, rest.join("/") || "index.html");
  if (!file.startsWith(page.dir)) return res.writeHead(403).end();
  if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, "index.html");
  fs.readFile(file, (err, data) => {
    if (err) return res.writeHead(404).end();
    res.writeHead(200, { "content-type": TYPES[path.extname(file)] || "application/octet-stream" });
    res.end(data);
  });
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const base = `http://127.0.0.1:${server.address().port}`;

const browser = await chromium.launch({
  executablePath: process.env.CHROME,
  headless: true,
  args: ["--js-flags=--expose-gc", "--disable-gpu", "--no-sandbox"],
});

// Run in the page: setup, then one timed action.
const OPS = {
  "create 1,000 rows": { setup: ["clear"], act: ["run"] },
  "replace 1,000 rows": { setup: ["run"], act: ["run"] },
  "update every 10th row (of 1,000)": { setup: ["run"], act: ["update"] },
  "select a row": { setup: ["run"], act: ["select"] },
  "swap two rows": { setup: ["run"], act: ["swaprows"] },
  "remove a row": { setup: ["run"], act: ["remove"] },
  "create 10,000 rows": { setup: ["clear"], act: ["runlots"] },
  "append 1,000 rows (to 1,000)": { setup: ["clear", "run"], act: ["add"] },
  "clear 1,000 rows": { setup: ["run"], act: ["clear"] },
  "clear 10,000 rows": { setup: ["runlots"], act: ["clear"] },
  "create 10,000 rows (virtual list)": { setup: ["clear"], act: ["runvirtual"], optional: true },
};

async function timeOp(page, op, run) {
  return page.evaluate(
    async ({ setup, act, run }) => {
      const q = (id) => document.querySelector(`[data-id="${id}"]`);
      const rows = () => q("rows");
      const press = (step) => {
        if (step === "select") rows().children[(run * 7) % 1000].children[1].click();
        else if (step === "remove") rows().children[4 + (run % 10)].children[2].click();
        else q(step).click();
      };
      const frame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
      for (const step of setup) {
        press(step);
        await frame();
      }
      gc();
      await frame();
      const t0 = performance.now();
      for (const step of act) press(step);
      void document.body.offsetHeight;
      const t1 = performance.now();
      await frame();
      return t1 - t0;
    },
    { ...op, run },
  );
}

async function memory(page, cdp) {
  await page.evaluate(() => {
    gc();
    gc();
  });
  const { metrics } = await cdp.send("Performance.getMetrics");
  const m = Object.fromEntries(metrics.map((x) => [x.name, x.value]));
  const wasm = await page.evaluate(() => (window.wasmMemory ? window.wasmMemory.buffer.byteLength : 0));
  return { heap: m.JSHeapUsedSize, wasm, nodes: m.Nodes };
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  const mid = s.length >> 1;
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
};

const results = {};
const memories = {};
const scrolls = {};
for (const { name } of pages) {
  const page = await browser.newPage();
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  await page.goto(`${base}/${name}/`);
  await page.waitForFunction(() => window.ready === true);
  results[name] = {};
  const virtual = await page.evaluate(() => !!document.querySelector('[data-id="runvirtual"]'));
  for (const [label, op] of Object.entries(OPS)) {
    if (op.optional && !virtual) {
      results[name][label] = NaN;
      continue;
    }
    for (let i = 0; i < WARMUP; i++) await timeOp(page, op, i);
    const times = [];
    for (let i = 0; i < RUNS; i++) times.push(await timeOp(page, op, WARMUP + i));
    results[name][label] = median(times);
  }
  // Memory on a fresh page: ready, 1,000 rows, 10,000 rows, cleared.
  await page.goto(`${base}/${name}/`);
  await page.waitForFunction(() => window.ready === true);
  const mem = {};
  mem.ready = await memory(page, cdp);
  await page.evaluate(() => document.querySelector('[data-id="run"]').click());
  mem.rows1k = await memory(page, cdp);
  await page.evaluate(() => document.querySelector('[data-id="runlots"]').click());
  mem.rows10k = await memory(page, cdp);
  await page.evaluate(() => document.querySelector('[data-id="clear"]').click());
  mem.cleared = await memory(page, cdp);
  if (virtual) {
    // 10,000 rows through a virtual list, on a page of its own: what it
    // holds, and what a scroll step costs (script, style and layout per
    // step, from Chrome's own counters), scrolled 280 px a frame for 60.
    await page.goto(`${base}/${name}/`);
    await page.waitForFunction(() => window.ready === true);
    await page.evaluate(() => document.querySelector('[data-id="runvirtual"]').click());
    mem.virtual10k = await memory(page, cdp);
    const before = await cdp.send("Performance.getMetrics");
    const steps = 60;
    const mounted = await page.evaluate(async (steps) => {
      const list = document.querySelector('[data-id="vrows"]');
      const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      for (let i = 0; i < steps; i++) {
        list.scrollTop += 280;
        await frame();
      }
      // The rows: the children of the list's spacer.
      return list.firstElementChild.childElementCount;
    }, steps);
    const after = await cdp.send("Performance.getMetrics");
    const delta = (key) =>
      after.metrics.find((m) => m.name === key).value - before.metrics.find((m) => m.name === key).value;
    scrolls[name] = {
      perStep: ((delta("ScriptDuration") + delta("LayoutDuration") + delta("RecalcStyleDuration")) * 1000) / steps,
      mounted,
    };
  }
  memories[name] = mem;
  await page.close();
}
await browser.close();
server.close();

const names = pages.map((p) => p.name);
console.log(`| Operation (median of ${RUNS}, ms) | ${names.join(" | ")} |`);
console.log(`| --- | ${names.map(() => "---").join(" | ")} |`);
for (const label of Object.keys(OPS)) {
  console.log(`| ${label} | ${names.map((n) => (Number.isNaN(results[n][label]) ? "–" : results[n][label].toFixed(2))).join(" | ")} |`);
}
console.log();
const mb = (b) => (b / 1048576).toFixed(2);
console.log(`| Memory (MB: JS heap + wasm memory; DOM nodes, which a collection may not yet have freed) | ${names.join(" | ")} |`);
console.log(`| --- | ${names.map(() => "---").join(" | ")} |`);
for (const key of ["ready", "rows1k", "rows10k", "cleared", "virtual10k"]) {
  if (!names.some((n) => memories[n][key])) continue;
  console.log(
    `| ${key} | ${names
      .map((n) => {
        const m = memories[n][key];
        if (!m) return "–";
        return `${mb(m.heap)} + ${mb(m.wasm)} = ${mb(m.heap + m.wasm)}; ${m.nodes}`;
      })
      .join(" | ")} |`,
  );
}
for (const n of names) {
  if (scrolls[n]) {
    console.log(`\n${n}: virtual list of 10,000 rows, ${scrolls[n].mounted} rows mounted after scrolling; ${scrolls[n].perStep.toFixed(2)} ms of script, style and layout per 280 px scroll step`);
  }
}
