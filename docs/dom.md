# zgx in the browser: `zgx-dom`

`zgx-dom` mounts a Rust-composed zgx view as real browser DOM, from WebAssembly. The browser lays it out, measures and shapes its text, scrolls it, selects in it, handles IME and reads it to assistive technology; zgx's reactive graph, which already knows what changed, writes only the nodes that read it. It is new and partial: Rust views built with `zgx::compose` mount, and so do Luau apps (`zgx-script`, with zgx-dom's `luau` feature: [Luau in the browser](#luau-in-the-browser)). GPU content shows as a labelled placeholder ([what doesn't map](#what-doesnt-map)).

```rust,ignore
use zgx::{compose::prelude::*, reactive::Runtime};

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    let runtime = Runtime::new();
    let count = runtime.signal(0);
    let shown = count.clone();
    let view = row()
        .p(24.)
        .gap(12.)
        .child(button().child(text("Add")).on_click(move || {
            count.set(count.get() + 1);
        }))
        .child(text_signal(move || format!("Clicked {} times", shown.get())));
    std::mem::forget(zgx_dom::mount_in_body(&runtime, view));
}
```

Build it for `wasm32-unknown-unknown` and run `wasm-bindgen --target web` on the result ([the benchmark's build script](../crates/zgx-dom/bench/build.sh) does both). `zgx_dom::mount(&runtime, &element, view)` mounts into any element and returns a handle whose `unmount` removes the view and everything it holds; `zgx_dom::flush()` applies pending DOM writes at once (they are applied at the end of the current task otherwise).

## How it plugs in

A `View` is a description: its kind, styles, children, handlers and accessibility. Mounting it into a `Ui` builds scene nodes; `zgx-dom` builds DOM elements from the same description instead, with zgx's own `Runtime`, signals, components, providers and slots. No `Ui`, `Scene`, zgx layout or text measurement runs in the browser. zgx's `dom` feature (off by default, and turned on only by `zgx-dom` when it builds for wasm32) reads views out for it (`zgx::compose::__dom`, hidden: plumbing, not API) and keeps the regions that change shape (`switch`, `keyed`, `child_when`, `virtual_list`) in a form the DOM can mount beside the closures a scene mounts. With the feature off, zgx compiles as it did: no gate scene's code changes. zgx's clock (`zgx::clock`, animations, tracing) reads the browser's `performance.now()` on wasm32, where std's clock panics.

## How updates reach the DOM

There is no virtual DOM and nothing is diffed. Each dynamic part of a view (a bound text, a reactive style, a followed attribute, an editor's value, a scroll offset) is one zgx effect that owns one DOM node: a signal write runs exactly the effects that read it, and each writes its node. The techniques, and where they come from:

- **Fine-grained signals bound straight to nodes** (as Solid): a component runs once; there is no re-render.
- **One boundary crossing per frame**: effects write operations into a buffer in wasm memory (node ids, op codes, and strings in one text block), and a small JS side applies the frame's buffer in one call at the end of the task. A bound text is a `nodeValue` write, a class change one `className` write.
- **Templates stamped with `cloneNode(true)`** (as Solid, without a compiler): the structure of each keyed row or switch branch is captured as it mounts, without its texts and values; the second time a shape is seen it becomes a detached template, and every later block of that shape is one deep clone, with only the texts, styles and attribute values that differ from the template's written. A clone reaches only the nodes something writes (an effect, a handler, a region, an observer, a value of its own), by `firstChild` and `nextSibling` from its root: the others hold no id and no entry in the JS side's table. Shapes are compared whole (tags, classes, attribute names, where values go), so rows that differ in any of them never share a template.
- **Static parts hoisted into classes**: an element's static styles, with its hover, pressed, focus and disabled ones, become one class, made once and shared by every element with the same look (looks are compared as styles, so a look seen before costs no CSS text). Hover, pressed and focus styles are `:hover`, `:active` and `:focus-visible` rules and run no wasm. A `reactive_style` writes only its patch, inline over the class, and only when it changes. The classes live in one constructed sheet the document adopts (`adoptedStyleSheets`), so a page whose Content-Security-Policy blocks inline styles (`style-src` without `'unsafe-inline'`) still gets them; inline patches are CSSOM writes, which such a policy allows. A browser that can't adopt a sheet gets a `<style>` element instead.
- **Event delegation at the root**: one listener per event kind on the document; an element carries its handler's number.
- **Keyed lists that move the fewest rows**: rows whose old places increase along the new order stay, and the others move (a longest increasing subsequence); appending, clearing (`textContent = ""`, one operation) and removing alone take fast paths.
- **Block-level ownership** (as Ripple's blocks): each keyed row, switch branch and virtualized row is a block that owns its nodes, effects, handlers and nested regions, and goes as one.
- **Reads apart from writes**: scroll positions are read in the next animation frame, all of them before any write they cause; ResizeObserver and IntersectionObserver reports are applied together in the task's one flush; input applies at once, so the next read sees it.

## Luau in the browser

With zgx-dom's `luau` feature, a Luau app runs in the browser: zgx-script's host loads the script (`ScriptHost::inline` for source, `ScriptHost::bundled` for scripts compiled into the app with `include_bundle!`), Luau's VM runs it, and zgx-dom mounts the view it returns, its state bound to the nodes that show it as in a Rust view.

```rust,ignore
let runtime = zgx::reactive::Runtime::new();
let script = zgx_script::ScriptHost::inline(&runtime, "app.luau", APP);
script.reload()?;
std::mem::forget(zgx_dom::mount_in_body(&runtime, script.view()));
```

`zgx_dom::install_fetcher(&script)` gives the host the page's `fetch` for `ui.fetch`. Relative URLs resolve against the page's `document.baseURI` as it is when it's installed, and `install_fetcher_at(&script, base)` takes the URL the app is served under instead. Either way, a later navigation doesn't move them. The desktop's rules apply in the same places:

- a plugin reaches it only with `Capability::Fetch`;
- the app's `FetchPolicy` is asked of the URL and of each header the script set. With a policy, redirects are refused rather than followed, since a page can't show each hop to ask. `FetchPolicy::address` isn't asked, as the browser resolves names itself; the page's origin rules (CORS, `connect-src`) stand in for it;
- a network failure, a refused CORS request or an abort rejects the script's promise as a `"network"` error, and an HTTP status is a response.

The browser drops the request headers it forbids pages to set (`Host`, `Content-Length`, `Cookie` and the rest of its list) without saying so, where the desktop sends them. Credentials go to the page's own origin alone (`credentials: "same-origin"`). A fetch the script cancels, or a reload stops, is aborted. A page with a strict Content-Security-Policy needs `connect-src` for the hosts its app fetches from. The fetcher's JavaScript (`js/zgx-fetch.js`) is bound only with `luau`, so a Rust-only build carries none of it.


`zgx_dom::install_audio(&script)` gives the app's `audio` (with zgx-dom's `audio` feature, or `audio-effects`, `audio-vorbis`, `audio-mp3`, `audio-opus`, `audio-streams`) the desktop's mixer, zgx-audio's, pulled by an audio worklet. The same samples, effects, fades and frame-exact plays come out as on the desktop. A sound's file is fetched through the host's transport (`install_fetcher` first) and decoded in Rust, and a plugin's stays confined to its own directory, as there. Nothing is made until the first sound: then the `AudioContext` is, and the worklet asks for blocks over its port as its queue runs low. The main thread mixes them about 120 ms ahead; a port's messages aren't throttled in a background tab as timers are. Once the quiet hold has passed, nothing more is asked for and the context is suspended: a silent page runs no audio callback and posts nothing. A browser starts sound only after the page has been pressed, so the first pointer or key press resumes a context that's waiting. The worklet is made from a Blob URL, which a strict Content-Security-Policy refuses unless it allows `script-src blob:`. For such a page, serve `js/zgx-audio-worklet.js` and name it with `install_audio_at`. `ui.scene3d` isn't in the browser yet (it draws on the GPU), so a plugin's `"scene3d"` grant is an unknown one there.

Luau is C++, so it is built with [wasi-sdk](https://github.com/WebAssembly/wasi-sdk) (34 or later): source `crates/zgx-dom/luau-env.sh` with `WASI_SDK` naming it, and the wasm32 build compiles Luau against the wasi sysroot's libc and libc++, with WebAssembly exception handling (Luau throws on a script error; browsers have it: Chrome 137, Firefox 131, Safari 18.4 and later), and links them. Native builds read none of it. In the module, zgx-script answers wasi-libc's system calls itself (no environment, no files, the browser's clock) and runs C++'s static constructors before the first VM, so the page needs no WASI host. Without threads, a script call's time budget is checked by the VM's interrupt from the call's start, rather than by a watchdog thread setting it when the budget nears. zgx-script's renderer dependency (zgx-gpu, for image decoding and shader checks) isn't linked for wasm32.

## Layout and styles

zgx's layout defaults are CSS flexbox's, so styles map nearly one to one: a `column` is `display: flex; flex-direction: column`, a `row` a row, an `overlay` a one-cell grid, and `grid` CSS grid (zgx's track lists are CSS's syntax). Sizes are border-box, children stretch and shrink and stop at `min-width: auto` as in zgx, and an `absolute` view positions against its parent (made `position: relative` for it) unless a nearer ancestor is `relative`; padding, margins, gaps, insets, percentages, flex grow, shrink, basis and wrap, alignment, `absolute` and `sticky_top`, right to left, `display_contents`, `z`, aspect ratio, min and max sizes, backgrounds and linear gradients, borders and their styles, corner radii, shadows, outlines, opacity, transforms, filters, backdrop blur and edge fades all become their CSS. Typography inherits as CSS inherits, from the theme's text colour, size and family at the root.

## Elements, input, focus and accessibility

| zgx | DOM |
| --- | --- |
| `div`, `column`, `row`, `overlay` | `<div>` |
| `text`, `text_signal` | a text node, or a `<span>` when styled |
| `rich_text`, `rich_text_signal` | one paragraph: a span with nothing of its own is a text node, others a `<span>` with a class for its look (weight, italic, size, family, colour, letter spacing, background, padding, radius, underline and strikethrough with their style, thickness, colour and offset), a link an `<a>`: a web or mail `href` opens in a tab of its own, a click that says it handled the link (`on_click_handled`) keeps it from opening, and a link with no `href` is `role=link`, focusable and pressed with Enter. A `hover` style is a `:hover` rule, an icon a masked box before the text (tinted, or as drawn, on its well), and `fade_in` a CSS animation. As the spans change, those that still look the same keep their nodes (streamed text appends) |
| `button` | `<button type=button>` (keyboard activation is the browser's) |
| `text_input`, `text_area` | `<input>`, `<textarea>`: the browser's editing, selection, clipboard, spellcheck and IME; the value is written only when it differs, so a composition is never interrupted |
| `checkbox`, `slider`, `progress` | `<input type=checkbox>` in a `<label>`, `<input type=range>`, `<progress>` (NaN: indeterminate) |
| `scroll`, `scroll_x` | `overflow: auto`, its offset signal both ways |
| `virtual_list`, `variable_virtual_list`, `measured_virtual_list`, `measured_rows` | a scroller holding only the rows that show, positioned in a spacer as tall as all of them, each at its own height. A measured list's rows are as tall as they lay out, reported by ResizeObserver into its heights (at least a pixel). As heights above the view change, the row first in view stays where it shows, or the end for heights anchored there (`anchor_end`); resting at the top, nothing anchors |
| `canvas` | a `<canvas>` filling the element, drawn with Canvas 2D at the element's size (ResizeObserver) and the screen's pixel ratio: fills and strokes (caps, joins, miter limit, dashes) of rectangles, rounded rectangles, ellipses, lines and paths, solid, linear-gradient and striped brushes, nonzero and even-odd rules, clips, and text over the shapes. `canvas::measure_text` measures with Canvas 2D while it draws. It draws again as the size or what the drawing reads changes, and only when the commands differ |
| `image` | a `<canvas>` with the image's pixels, `role=img` and its label |
| `switch`, `keyed`, `child_when` | a region (`display: contents` unless styled) |
| roles, labels, levels, live regions, checked, selected, expanded, values, descriptions, hidden, modal | ARIA: `role`, `aria-label`, `aria-level`, `aria-live`, `aria-checked`… |
| `focusable` | `tabindex` |
| `selectable` | `user-select` |
| `modal` | a `<dialog>` shown with `showModal()`: the browser traps focus, makes the page behind inert, closes it on Escape and gives focus back; a click on its backdrop closes it unless `dismiss_backdrop(false)`; its backdrop is the theme's |
| `popover`, `menu`, `submenu` | an auto popover in the browser's top layer, placed against its anchor on its `placement` side (the other side when it doesn't fit there), kept in the viewport and placed again as the page scrolls or resizes. Escape and a click outside close it as the browser closes popovers; a press on its anchor closes it without reopening it. A menu (`role=menu`) opens from a bare button, focuses its first item and moves among its items with the arrows, Home and End; Right opens a submenu and Left closes it, the pointer opens a submenu's trigger's submenu, and Tab leaves a menu, closing it |
| `lazy` | an overlay's panel is filled the first time it opens, and kept: until then its children aren't built (tooltips are lazy) |
| `arrow` | a popover's arrow: a square turned 45° in the panel's own background, on the side facing the anchor (flipping with it) under the anchor's centre and kept off the corners; the panel sits its size further off |
| `menu_item` | a `<button role=menuitem>` (`menuitemcheckbox`, `menuitemradio` when checkable) that closes its menu and the menus it opened from, then runs its click; focus goes back to the menu's button |
| `View::tooltip` | a manual popover (`role=tooltip`) shown as the pointer rests on the view for its delay (at once within a second of another) or keyboard focus reaches it, gone as the pointer leaves or presses or a key is pressed; a text tip is the view's `aria-description` |
| `on_event`, `on_action` | the browser's events at the element, each phase a call (capture, at it, bubbling): pointer down, up, move, enter, leave and cancel, the wheel (lines and pages as points), keys (`key` and `code`), focus and blur (`focusin`, `focusout`) and activation (`click`). A listener's `prevent_default`, `stop_propagation`, `stop_immediate_propagation`, `focus` and pointer capture apply to the event at once. An event for something inside the element has a `target` of its own, not the element's |
| `on_drag`, `start_drag`, `on_drop`, `on_drag_end` | a drag a listener starts: a copy of its source follows the pointer; what's under it, from the innermost element with listeners out, hears `Over` until one takes it (`accept_drag`), then `Leave` and, as the button comes up there, `Drop`; the source hears `Start`, `Move` and `End` (accepted or not), and the press ends in no click. A `drag_preview` isn't built yet: the copy stands for it. Text dragged in from another app (the browser's own drag and drop) reaches the same destinations: one that takes `DraggedText` hears it over it and dropped, with its text and, where offered, its HTML (`DraggedText::html`); elsewhere the browser's own drop is left alone. A drag carrying HTML and no `text/plain` (rare: a link offers both) is left to the browser |
| `keymap` | zgx's keymap resolution (bindings by context, sequences of up to four keys, `NoAction`): a key goes to the keymaps from the focused element out before anything hears it; a sequence's first keys are taken, and a match's actions go out in turn from the focused element until a listener handles one (`prevent_default`) |
| `focus_group`, and list boxes, tab lists, toolbars and radio groups | one Tab stop (roving `tabindex`, on the checked or selected member first); the arrows of its axis, Home and End move among its own members (right to left, Left and Right swap); a radio group's wrap and choose |
| `context_menu`, `context_menu_with_span`, `TextSpan::context_menu` | a secondary click builds the entries and opens them as a menu with its corner at the pointer, replacing any menu already open; none lets the browser's own menu show. A span's own menu comes before its paragraph's, and `context_menu_with_span` is told the span (its place, link and text) |
| `Platform` (`PlatformService`, Luau's `ui.clipboard`) | provided around every mounted view, as a desktop window provides its own: the clipboard's text and HTML (`clipboard_text`, `clipboard_html`, `set_clipboard_text`, `set_clipboard_rich_text`) and `open_url` (a tab of its own). A page reads the clipboard only inside a paste event, so a read gives the paste being handled, else what the page last copied, cut, pasted or wrote; a key a keymap binds to paste (⌘V, Ctrl+V) runs its action as the paste comes. Writes go through `navigator.clipboard.write`, `text/plain` with `text/html`. File dialogs, the clipboard's images and files, notifications and badges aren't there yet |
| `on_mount` handles | `observe_size`, `observe_bounds` (ResizeObserver), `observe_visibility`, `observe_visibility_within` (IntersectionObserver with that `rootMargin`), `is_mounted`, `retain` |

## What doesn't map

Shown as a labelled placeholder box, or left out:

- **GPU content**: WGSL fills and layer shaders, `gpu_view`, zgx-3d scenes, native surfaces, terminals, web views and video export draw with a GPU or a platform view zgx-dom doesn't have. A `<canvas>` with WebGPU could host them later.
- **Luau apps' host services**: sockets, files, plugins from the disk, hot reload and `zgx check` need hosts zgx-dom doesn't have yet; images in Luau apps show their labels (zgx-script decodes them with zgx-gpu, which isn't linked in the browser).
- **Overlays**: `at_caret`, `hoist` and `modal_placement` placement; menus have no typeahead; an arrow has no border on its outer edges.
- **Rich text**: a link's middle click.
- **Input**: `drag_preview` builders (a copy of the source stands for them), file drops, `drag_export` and pinch; text and IME stay the browser's own in `<input>` and `<textarea>`. `reactive_interaction` isn't applied.
- **Lists**: the `_watched` variants' watchers aren't called.
- **Canvas**: a canvas view's pan and zoom (`canvas_view`) isn't applied.
- **Also not yet mapped**: SVG, and a `NodeHandle`'s layout, editor and scene methods.
- **Sizes worked out** (`w_calc` and the like) aren't written as CSS yet.

## Performance

`crates/zgx-dom/bench` is a table benchmark in the spirit of js-framework-benchmark: rows of an id, a label and a remove control, created, replaced, updated, selected, swapped, removed, appended and cleared through one zgx view. `bench/run.mjs` times any pages with the same buttons in headless Chrome, each operation from the click to a forced style and layout (script, style and layout, not paint), the median of ten after three warm-ups, and reads the JS heap and wasm memory after a forced collection. The baseline is the same table written with Solid (1.9, `createSignal` labels, `createSelector` selection, `<For>`), with the same DOM and CSS, timed the same way.

On an Apple M5, chrome-headless-shell 156, `release` profile:

| Operation (ms) | zgx-dom | Solid |
| --- | --- | --- |
| create 1,000 rows | 11.75 | 9.20 |
| replace 1,000 rows | 12.00 | 10.50 |
| update every 10th row | 1.60 | 1.65 |
| select a row | 0.30 | 0.20 |
| swap two rows | 1.20 | 1.20 |
| remove a row | 0.90 | 1.00 |
| create 10,000 rows | 110.7 | 97.1 |
| append 1,000 rows | 11.40 | 10.30 |
| clear 1,000 rows | 1.10 | 1.15 |
| clear 10,000 rows | 13.75 | 13.95 |
| create 10,000 rows in a virtual list | 1.30 | |

| Memory (MB, JS heap + wasm memory) | zgx-dom | Solid |
| --- | --- | --- |
| ready | 2.9 | 1.9 |
| 1,000 rows | 4.1 | 3.0 |
| 10,000 rows | 16.7 | 13.5 |
| cleared | 14.9 | 1.9 |
| 10,000 rows in a virtual list (a page of its own) | 5.2 | |

The page's wasm is 641 KB (213 KB gzipped).

Updates, selection, swaps, removals and clears cost what Solid's do: the work is the same few node writes. Where zgx-dom loses:

- **Creating rows** costs about 2.6 ms of script per 1,000 against Solid's 1.4: zgx's builders move each view through their calls, its effects are Rust structures with their own allocations, and each row's description is read out before its template is stamped. Style and layout cost about 1 ms more than Solid's although the DOM and CSS are the same: each page's own rows, inserted again as static HTML, lay out alike (8.2–8.4 ms against 8.1–8.2), so the difference is in how rows are inserted, not yet found.
- **Memory after a clear**: wasm memory grows but never shrinks, so what 10,000 rows took (about 1 KB a row: zgx's effects and signals, the row's block, its keys) stays reserved after they go. It is reused, not grown: clearing and making 10,000 rows again keeps the same peak. zgx-dom keeps that peak low (a long frame applies its operations as it goes, rows freed from the top of the node table shrink it rather than filling a free list), but a page that showed 10,000 rows keeps about 13 MB of wasm memory.

**Lists of thousands of rows belong in a virtual list** (`virtual_list`; Luau's `ui.list` once Luau runs here), as zgx's guidance has it everywhere: only the rows in view are mounted, so memory follows the window, not the list. 10,000 rows through one take 5.2 MB in all with 17 rows mounted, are made in 1.3 ms, and a 280-pixel scroll step costs 0.48 ms of script, style and layout.

**Why one buffer of operations a frame**: measured against applying each operation as its own call into JS (as one web-sys call a DOM operation would), the buffer was faster where there is work (create 1,000 rows 10.6 against 11.7 ms, replace 11.6 against 13.5, create 10,000 rows 113 against 128, append 11.5 against 12.8, clear 10,000 14.2 against 15.0) and level or within 0.15 ms on the single-row operations.

Run it: `crates/zgx-dom/bench/build.sh` (needs `wasm-bindgen-cli` of the version `Cargo.lock` pins), then `npm install` and `CHROME=/path/to/chrome node run.mjs zgx=crates/zgx-dom/bench other=path/to/another/page` from `bench/`.

## Tests

`cargo test -p zgx-dom --target wasm32-unknown-unknown` with `wasm-bindgen-test-runner` as the target's runner and `chromedriver` on the `PATH` runs its tests in headless Chrome: bound text writing its own node, styles as shared classes and computed CSS, delegated clicks, keyed reorders moving the fewest rows and clearing whole, switches, editors and checkboxes both ways, scroll offsets, virtual lists, ARIA, observers, and zgx's clock and tracing on the browser's clock. CI's Linux job runs them, lints the crate for wasm32 and builds the benchmark.

The audio tests (`--features audio --test audio`) need an `AudioContext` that runs without a press: a browser starts one only after a trusted press, and a test's `click()` isn't one. Give Chrome `--autoplay-policy=no-user-gesture-required` in your `webdriver.json`'s `goog:chromeOptions` args. CI writes it for that step.
