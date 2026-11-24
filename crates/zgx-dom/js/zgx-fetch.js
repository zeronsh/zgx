// zgx-dom's fetcher for Luau apps (`ui.fetch`, and the files a script asks
// for by path): the browser's fetch, bound only when the `luau` feature is
// on, so a Rust-only build ships none of this.
//
// A relative URL resolves against the base the app was installed with,
// never the page's location at the time of the request. Credentials go to
// the page's own origin alone. A request whose Rust future is dropped is
// aborted (`abort`), so a sound replaced or a view gone stops downloading.
// A strict Content-Security-Policy needs `connect-src` for remote URLs.

let pending = 0;

// The URL `url` names from `base`, as [href, scheme, host, port]; throws
// for one that isn't a URL.
export function zgx_fetch_url(url, base) {
  const u = new URL(url, base);
  return [u.href, u.protocol.replace(/:$/, ""), u.hostname, u.port];
}

// Starts a request; returns { promise, abort }. The promise resolves to
// { status, headers: [name, value, ...], bytes: Uint8Array }, and rejects
// with a message for a network failure, a refused CORS request, a refused
// redirect or an abort.
export function zgx_fetch(href, method, headers, body, redirect) {
  const controller = new AbortController();
  const init = {
    method,
    headers: [],
    credentials: "same-origin",
    redirect,
    signal: controller.signal,
  };
  for (let i = 0; i + 1 < headers.length; i += 2) {
    init.headers.push([headers[i], headers[i + 1]]);
  }
  if (body !== undefined && body !== null) init.body = body;
  pending += 1;
  // The whole body, read at once: what sounds and most requests want. A
  // long or chunked response could be written as it arrives
  // (`response.body.getReader()`), as a streaming fetcher may.
  //
  // An abort rejects this promise after Rust has dropped the future
  // awaiting it. That's safe: js-sys's JsFuture gives `then` callbacks made
  // with `Closure::once` whose state lives in an `Rc`, so they outlive the
  // future and are freed when one is called (js-sys src/futures/mod.rs).
  const promise = (async () => {
    try {
      const response = await fetch(href, init);
      const bytes = new Uint8Array(await response.arrayBuffer());
      const flat = [];
      response.headers.forEach((value, name) => flat.push(name, value));
      return { status: response.status, headers: flat, bytes };
    } catch (error) {
      throw String((error && error.message) || error);
    } finally {
      pending -= 1;
    }
  })();
  return { promise, abort: () => controller.abort() };
}

// Requests started and not yet settled: what a test reads to see an
// abort leave nothing behind.
export function zgx_fetches_pending() {
  return pending;
}
