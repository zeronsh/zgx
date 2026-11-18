// zgx-dom's browser side: applies a frame's DOM operations, written by
// Rust into wasm memory, in one call, and delegates events from the root.
// The op codes match `src/ops.rs`.

const nodes = [];
const classNames = [];
const decoder = new TextDecoder();
let sheet = null;
let dispatch = null;
let flush = null;
let elementEvent = null;
let resolveKeys = null;
let scheduled = false;
let root = null;
// Scroll reads wait for the next frame, apart from the writes.
const scrolled = new Set();
let scrollFrame = 0;
// Set by a click's handler (`PREVENT`): the click's default is prevented.
let prevented = false;

const TAGS = ["div", "span", "button", "input", "textarea", "progress", "label", "img", "canvas", "dialog", "a"];

// One device pixel in CSS pixels, for a translation that rests on whole
// device pixels (`View::pixel_snap`, its CSS rounded to it): kept as the
// window moves to a screen of another resolution.
function devicePixel() {
  const ratio = window.devicePixelRatio || 1;
  document.documentElement.style.setProperty("--zgx-device-px", 1 / ratio + "px");
  if (typeof matchMedia === "function") {
    matchMedia(`(resolution: ${ratio}dppx)`).addEventListener("change", devicePixel, { once: true });
  }
}

// The page's sheet: base rules, then each class's as it is first used. A
// constructed sheet, adopted by the document, applies under a strict
// `style-src` that blocks a <style> element; where a browser can't adopt
// one, a <style> element holds the rules instead.
function adoptSheet(css) {
  if ("adoptedStyleSheets" in document && typeof CSSStyleSheet === "function") {
    try {
      const constructed = new CSSStyleSheet();
      constructed.replaceSync(css);
      document.adoptedStyleSheets = [...document.adoptedStyleSheets, constructed];
      return constructed;
    } catch (_) {
      // A sheet this browser can't construct: the element below.
    }
  }
  const style = document.createElement("style");
  style.textContent = css;
  document.head.appendChild(style);
  return style.sheet;
}

export function zgx_init(baseCss, onDispatch, onFlush, onEvent, onKeys) {
  root = document;
  dispatch = onDispatch;
  flush = onFlush;
  elementEvent = onEvent;
  resolveKeys = onKeys;
  sheet = adoptSheet(baseCss);
  devicePixel();
  document.addEventListener("click", onClick);
  document.addEventListener("input", onInput);
  document.addEventListener("change", onChange);
  document.addEventListener("scroll", onScroll, true);
  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("click", onClickCapture, true);
  document.addEventListener("pointerover", onPointerOver);
  document.addEventListener("pointerout", onPointerOut);
  document.addEventListener("focusin", onFocusIn);
  document.addEventListener("focusout", onFocusOut);
  document.addEventListener("keydown", onKeyDown);
  addEventListener("resize", placeAll);
  document.addEventListener("scroll", placeAll, true);
  document.addEventListener("keydown", onKeymaps, true);
  document.addEventListener("contextmenu", onContextMenu);
  document.addEventListener("copy", onCopy, true);
  document.addEventListener("cut", onCopy, true);
  document.addEventListener("paste", onPaste, true);
  document.addEventListener("keydown", onGroupKey);
  document.addEventListener("focusin", onGroupFocus);
  document.addEventListener("keydown", onLinkKey);
  document.addEventListener("dragover", onOutsideOver, true);
  document.addEventListener("dragleave", onOutsideLeave, true);
  document.addEventListener("drop", onOutsideDrop, true);
  // A contenteditable over a model (a text area with atoms): its edits are
  // the model's.
  document.addEventListener("beforeinput", onBeforeInput, true);
  document.addEventListener("compositionstart", onCompositionStart, true);
  document.addEventListener("compositionend", onCompositionEnd, true);
  document.addEventListener("keydown", onEditKey, true);
}

// A link with no href (`role=link`): Enter presses it, as it does a link.
// A run that only acts (`role=button`): Enter and Space, as a button's.
function onLinkKey(event) {
  const node = event.target;
  if (node.$$c === undefined || !node.getAttribute) return;
  const role = node.getAttribute("role");
  if ((event.key === "Enter" && (role === "link" || role === "button")) || (event.key === " " && role === "button")) {
    event.preventDefault();
    node.click();
  }
}

// A node made outside: the element a view mounts into.
export function zgx_adopt(id, element) {
  nodes[id] = element;
}

// Rust's tasks (zgx's executor) and timers (zgx::timer), which the page's
// event loop runs: it has no thread for them.
let runTasks = null;
let fireTimers = null;
let deliverFrame = null;
let frameAsked = 0;
let tasksQueued = false;
let tasksChannel = null;
let timer = 0;

export function zgx_tasks_init(onRun, onFire, onFrame) {
  runTasks = onRun;
  fireTimers = onFire;
  deliverFrame = onFrame;
}

// An animation awaits the next display refresh: one frame asked for, however
// many ask. A hidden tab's the browser holds back.
export function zgx_frame_ask() {
  if (frameAsked) return;
  frameAsked = requestAnimationFrame(onFrame);
}

function onFrame(at) {
  frameAsked = 0;
  deliverFrame(at);
}

// The idle trim's one timeout (src/idle.rs): armed at most once at a time.
export function zgx_trim_arm(fire, ms) {
  setTimeout(fire, ms);
}

export function zgx_frame_asked() {
  return frameAsked !== 0;
}

// A task woke: run the ready ones in a microtask, or, for one woken while they
// ran (a task yielding), after the browser's next task, so input and paint
// come between.
export function zgx_tasks_wake(yielded) {
  if (tasksQueued) return;
  tasksQueued = true;
  if (!yielded) {
    queueMicrotask(drainTasks);
    return;
  }
  if (!tasksChannel) {
    tasksChannel = new MessageChannel();
    tasksChannel.port1.onmessage = drainTasks;
  }
  tasksChannel.port2.postMessage(0);
}

function drainTasks() {
  tasksQueued = false;
  runTasks();
}

// The one timeout for zgx's earliest timer, `ms` from now; below zero, none.
export function zgx_timer_arm(ms) {
  if (timer) clearTimeout(timer);
  timer = ms < 0 ? 0 : setTimeout(onTimer, ms);
}

function onTimer() {
  timer = 0;
  fireTimers();
}

export function zgx_timer_armed() {
  return timer !== 0;
}

export function zgx_schedule() {
  if (scheduled) return;
  scheduled = true;
  queueMicrotask(() => {
    scheduled = false;
    flush();
  });
}

function onClick(event) {
  if (offCount !== 0 && event.target.closest?.("[data-off]")) return;
  let node = event.target;
  while (node && node !== root) {
    const handler = node.$$c;
    if (handler !== undefined) {
      const modifiers =
        (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
      dispatch(handler, 0, modifiers, undefined);
      // A link's click that handled it: its href doesn't open.
      if (prevented) {
        prevented = false;
        event.preventDefault();
      }
      return;
    }
    node = node.parentNode;
  }
}

// Disabled elements (`OFF`) and what they hold, as a disabled fieldset's
// controls are: no input or focus, heard disabled. Each control inside is
// marked as the element is disabled (or as it is added, while it is),
// keeping its own state in `$$off` to give back when no disabled element
// holds it any more.
let offCount = 0;
const OFFABLE = "button,input,select,textarea,a[href],[tabindex],[contenteditable]";
const NATIVE = /^(BUTTON|INPUT|SELECT|TEXTAREA)$/;

function setOff(el, on) {
  if (on === el.hasAttribute("data-off")) return;
  if (on) {
    offCount++;
    el.setAttribute("data-off", "");
    el.setAttribute("aria-disabled", "true");
    for (const inside of el.querySelectorAll(OFFABLE)) offMark(inside);
    el.$$offWatch = new MutationObserver((records) => {
      for (const record of records)
        for (const added of record.addedNodes) {
          if (added.nodeType !== 1) continue;
          if (added.matches(OFFABLE)) offMark(added);
          for (const inside of added.querySelectorAll(OFFABLE)) offMark(inside);
        }
    });
    el.$$offWatch.observe(el, { childList: true, subtree: true });
  } else {
    offCount--;
    el.$$offWatch.disconnect();
    el.$$offWatch = undefined;
    el.removeAttribute("data-off");
    // Still inside another disabled element: heard disabled through it.
    if (el.$$off === undefined) el.removeAttribute("aria-disabled");
    for (const inside of el.querySelectorAll(OFFABLE)) offRestore(inside);
  }
}

function offMark(el) {
  if (el.$$off !== undefined) return;
  el.$$off = [el.disabled, el.getAttribute("tabindex"), el.getAttribute("aria-disabled"), el.getAttribute("contenteditable")];
  if (NATIVE.test(el.tagName)) {
    el.disabled = true;
    return;
  }
  if (el.tagName === "A") el.setAttribute("tabindex", "-1");
  else el.removeAttribute("tabindex");
  if (el.hasAttribute("contenteditable")) el.setAttribute("contenteditable", "false");
  el.setAttribute("aria-disabled", "true");
}

function offRestore(el) {
  const was = el.$$off;
  if (was === undefined || el.parentElement?.closest("[data-off]")) return;
  el.$$off = undefined;
  if (NATIVE.test(el.tagName)) {
    el.disabled = was[0];
    return;
  }
  const put = (name, value) => (value === null ? el.removeAttribute(name) : el.setAttribute(name, value));
  put("tabindex", was[1]);
  put("aria-disabled", el.hasAttribute("data-off") ? "true" : was[2]);
  if (was[3] !== null) el.setAttribute("contenteditable", was[3]);
}

function onInput(event) {
  // A contenteditable over a model the browser wrote into (`UNPREVENTED`),
  // composing no more: shown again from the model.
  const host = editHost(event.target);
  if (host) {
    if (host.$$dirty && host.$$cs === undefined) {
      host.$$dirty = false;
      sendEdit(host, "render", "", 0, 0);
    }
    return;
  }
  const node = event.target;
  const handler = node.$$i;
  if (handler === undefined) return;
  if (node.type === "range") dispatch(handler, 2, +node.value, undefined);
  else if (node.type !== "checkbox") dispatch(handler, 1, 0, node.value);
}

function onChange(event) {
  const node = event.target;
  const handler = node.$$i;
  if (handler !== undefined && node.type === "checkbox") dispatch(handler, 3, node.checked ? 1 : 0, undefined);
}

function onScroll(event) {
  const node = event.target;
  if (node.$$s === undefined) return;
  scrolled.add(node);
  if (!scrollFrame) scrollFrame = requestAnimationFrame(readScrolls);
}

function readScrolls() {
  scrollFrame = 0;
  // Every read first, then the writes they lead to (one flush).
  const reads = [];
  for (const node of scrolled) reads.push(node.$$s, node.$$h ? node.scrollLeft : node.scrollTop);
  scrolled.clear();
  for (let i = 0; i < reads.length; i += 2) dispatch(reads[i], 4, reads[i + 1], undefined);
}

const resized = new Map();
let resizeObserver = null;
function observeSize(node, handler) {
  if (!resizeObserver) {
    resizeObserver = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const h = entry.target.$$r;
        if (h === undefined) continue;
        const box = entry.borderBoxSize && entry.borderBoxSize[0];
        const w = box ? box.inlineSize : entry.contentRect.width;
        const ht = box ? box.blockSize : entry.contentRect.height;
        const r = entry.target.getBoundingClientRect();
        dispatch(h, 5, w, ht + "," + r.x + "," + r.y);
      }
    });
  }
  node.$$r = handler;
  resizeObserver.observe(node);
}

// One IntersectionObserver for each margin (its rootMargin).
const visibilityObservers = new Map();
function observeVisible(node, handler, margin) {
  let observer = visibilityObservers.get(margin);
  if (!observer) {
    observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const h = entry.target.$$v && entry.target.$$v.get(margin);
          if (h !== undefined) dispatch(h, 6, entry.isIntersecting ? 1 : 0, undefined);
        }
      },
      { rootMargin: margin + "px" },
    );
    visibilityObservers.set(margin, observer);
  }
  (node.$$v || (node.$$v = new Map())).set(margin, handler);
  observer.observe(node);
}

export function zgx_apply(memory, opsPtr, opsLen, strPtr, strLen) {
  const ops = new Uint32Array(memory.buffer, opsPtr, opsLen);
  const floats = new Float32Array(memory.buffer, opsPtr, opsLen);
  const text = strLen ? decoder.decode(new Uint8Array(memory.buffer, strPtr, strLen)) : "";
  let i = 0;
  const str = () => {
    const at = ops[i++];
    const n = ops[i++];
    return text.substring(at, at + n);
  };
  while (i < opsLen) {
    switch (ops[i++]) {
      case 1: { // CREATE id tag
        const id = ops[i++];
        nodes[id] = document.createElement(TAGS[ops[i++]]);
        break;
      }
      case 2: { // TEXT id str
        const id = ops[i++];
        nodes[id] = document.createTextNode(str());
        break;
      }
      case 3: { // SET_TEXT id str
        const id = ops[i++];
        nodes[id].nodeValue = str();
        break;
      }
      case 4: { // CLASS id name: a class name by index
        const id = ops[i++];
        nodes[id].className = classNames[ops[i++]];
        break;
      }
      case 5: { // STYLE id str
        const node = nodes[ops[i++]];
        node.style.cssText = str();
        // An overlay's place, kept under its styles.
        if (node.$$o) node.style.cssText += node.$$o.pos;
        break;
      }
      case 6: { // ATTR id name value
        const id = ops[i++];
        const name = str();
        nodes[id].setAttribute(name, str());
        break;
      }
      case 7: { // REMOVE_ATTR id name
        const id = ops[i++];
        nodes[id].removeAttribute(str());
        break;
      }
      case 8: { // APPEND parent child
        const parent = nodes[ops[i++]];
        parent.appendChild(nodes[ops[i++]]);
        break;
      }
      case 9: { // INSERT parent child before
        const parent = nodes[ops[i++]];
        const child = nodes[ops[i++]];
        parent.insertBefore(child, nodes[ops[i++]]);
        break;
      }
      case 10: { // REMOVE id
        nodes[ops[i++]].remove();
        break;
      }
      case 11: { // FREE n ids…
        const n = ops[i++];
        for (let k = 0; k < n; k++) {
          const id = ops[i++];
          const node = nodes[id];
          // Observers let go too: a handler number they hold may be reused.
          if (node && node.$$r !== undefined) {
            resizeObserver.unobserve(node);
            node.$$r = undefined;
          }
          if (node && node.$$o) {
            // Gone with its block: it reports nothing more.
            shown.delete(node);
            pending.delete(node);
            node.$$o = undefined;
          }
          if (node && node.$$v) {
            for (const margin of node.$$v.keys()) visibilityObservers.get(margin).unobserve(node);
            node.$$v = undefined;
          }
          nodes[id] = undefined;
        }
        break;
      }
      case 12: { // CLEAR id
        nodes[ops[i++]].textContent = "";
        break;
      }
      case 13: { // RULE str
        const rule = str();
        try {
          sheet.insertRule(rule, sheet.cssRules.length);
        } catch (_) {
          // A declaration this browser rejects drops its rule alone.
        }
        break;
      }
      case 14: { // HANDLER id kind handler
        const node = nodes[ops[i++]];
        const kind = ops[i++];
        const handler = ops[i++];
        if (kind === 0) node.$$c = handler;
        else if (kind === 1) node.$$i = handler;
        else if (kind === 2) node.$$s = handler;
        else if (kind === 3) node.$$h = 1;
        else if (kind === 4) observeSize(node, handler);
        else if (kind === 5) listen(node, handler);
        else if (kind === 6) node.$$k = handler;
        else if (kind === 7) group(node, handler);
        else if (kind === 9) node.$$m = handler;
        else if (kind === 10) node.$$si = handler;
        else if (kind === 11) node.$$e = handler;
        break;
      }
      case 15: { // VALUE id str: an editor's text, unless it holds it
        const node = nodes[ops[i++]];
        const value = str();
        if (node.value !== value) node.value = value;
        break;
      }
      case 16: { // CHECKED id on
        const node = nodes[ops[i++]];
        node.checked = ops[i++] === 1;
        break;
      }
      case 17: { // NUMBER id prop f32bits
        const node = nodes[ops[i++]];
        const prop = ops[i++];
        const bits = ops[i++];
        const value = new Float32Array(new Uint32Array([bits]).buffer)[0];
        if (prop === 0) node.value = value;
        else if (prop === 1) {
          if (Math.abs(node.scrollTop - value) >= 1) node.scrollTop = value;
        } else if (prop === 2) {
          if (Math.abs(node.scrollLeft - value) >= 1) node.scrollLeft = value;
        } else if (prop === 3) {
          if (Number.isNaN(value)) node.removeAttribute("value");
          else node.value = value;
        }
        break;
      }
      case 18: { // PROP id name on: a boolean property
        const node = nodes[ops[i++]];
        const name = str();
        const on = ops[i++] === 1;
        // A control inside a disabled element stays disabled: its own
        // value waits for the element to be enabled (`offRestore`).
        if (node.$$off !== undefined && name === "disabled") node.$$off[0] = on;
        else node[name] = on;
        break;
      }
      case 19: { // CLONE id from: a template's copy
        const id = ops[i++];
        nodes[id] = nodes[ops[i++]].cloneNode(true);
        break;
      }
      case 20: { // FIRST id from: its first child
        const id = ops[i++];
        nodes[id] = nodes[ops[i++]].firstChild;
        break;
      }
      case 21: { // NEXT id from: its next sibling
        const id = ops[i++];
        nodes[id] = nodes[ops[i++]].nextSibling;
        break;
      }
      case 22: { // IMAGE id width height ptr len: RGBA pixels into a canvas
        const node = nodes[ops[i++]];
        const w = ops[i++];
        const h = ops[i++];
        const ptr = ops[i++];
        const len = ops[i++];
        node.width = w;
        node.height = h;
        const pixels = new Uint8ClampedArray(memory.buffer, ptr, len).slice();
        node.getContext("2d").putImageData(new ImageData(pixels, w, h), 0, 0);
        break;
      }
      case 23: { // FOCUS id
        nodes[ops[i++]].focus();
        break;
      }
      case 24: { // CLASS_NAME index str
        const index = ops[i++];
        classNames[index] = str();
        break;
      }
      case 25: { // CLASS_ADD id name
        const id = ops[i++];
        nodes[id].classList.add(classNames[ops[i++]]);
        break;
      }
      case 27: { // TRUNCATE length: the table's free end
        nodes.length = ops[i++];
        break;
      }
      case 28: { // WALK id from n index…: a descendant, by child indices
        const id = ops[i++];
        let node = nodes[ops[i++]];
        const n = ops[i++];
        // By siblings: `childNodes` would make a live list each step.
        for (let k = 0; k < n; k++) {
          node = node.firstChild;
          for (let s = ops[i++]; s > 0; s--) node = node.nextSibling;
        }
        nodes[id] = node;
        break;
      }
      case 26: { // VISIBLE id handler margin(f32 bits)
        const node = nodes[ops[i++]];
        const handler = ops[i++];
        const margin = new Float32Array(new Uint32Array([ops[i++]]).buffer)[0];
        observeVisible(node, handler, margin);
        break;
      }
      case 29: { // OVERLAY panel backdrop kind anchor handler side align offset flags delay
        const panel = nodes[ops[i++]];
        const backdrop = str();
        const kind = ops[i++];
        const anchorId = ops[i++];
        const handler = ops[i++];
        const side = ops[i++];
        const align = ops[i++];
        const offset = new Float32Array(new Uint32Array([ops[i++]]).buffer)[0];
        const flags = ops[i++];
        const delay = ops[i++];
        const arrow = new Float32Array(new Uint32Array([ops[i++]]).buffer)[0];
        const anchor = anchorId === 0xffffffff ? null : nodes[anchorId];
        setUpOverlay(panel, { kind, anchor, handler, side, align, offset, flags, delay, backdrop, arrow });
        break;
      }
      case 30: { // SHOW panel on
        const panel = nodes[ops[i++]];
        show(panel, ops[i++] === 1);
        break;
      }
      case 33: { // MENU_AT holder x y: a context menu's panel at the pointer
        const holder = nodes[ops[i++]];
        const x = new Float32Array(new Uint32Array([ops[i++]]).buffer)[0];
        const y = new Float32Array(new Uint32Array([ops[i++]]).buffer)[0];
        const panel = holder.querySelector(".zp");
        if (panel && panel.$$o) {
          panel.$$o.at = { x, y };
          if (panel.matches(":popover-open")) place(panel);
        }
        break;
      }
      case 34: { // SELECT root anchor focus: a contenteditable's selection, in UTF-16 units
        const el = nodes[ops[i++]];
        const anchor = ops[i++];
        const focus = ops[i++];
        selectIn(el, anchor, focus);
        break;
      }
      case 35: { // OFF id on: an element disabled, with what it holds
        const el = nodes[ops[i++]];
        setOff(el, ops[i++] === 1);
        break;
      }
      case 31: // PREVENT: the click being handled doesn't do its default
        prevented = true;
        break;
      case 32: { // PAINT canvas width height count commands…
        const node = nodes[ops[i++]];
        const width = floats[i++];
        const height = floats[i++];
        const count = ops[i++];
        const reader = { next: () => ops[i++], num: () => floats[i++], str, memory };
        paint(node, width, height, count, reader);
        break;
      }
      default:
        throw new Error("zgx-dom: unknown op " + ops[i - 1]);
    }
  }
  // Overlays asked to show before they were in the document show now.
  for (const panel of pending) {
    if (panel.isConnected) {
      pending.delete(panel);
      show(panel, true);
    }
  }
}

// What the benchmark and tests read: how many nodes the table holds.
export function zgx_live_nodes() {
  let n = 0;
  for (const node of nodes) if (node !== undefined) n++;
  return n;
}

// Overlays (`modal`, `popover`, `menu`, `submenu`, tooltips): a modal is a
// <dialog> shown with showModal(), the rest popovers in the browser's top
// layer, placed against their anchors. The browser's closing one (Escape,
// a click outside, a dialog's cancel) is reported to its handler (kind 7).
const DIALOG = 0, POPOVER = 1, MENU = 2, SUBMENU = 3, TOOLTIP = 4, LAYER = 5;
// Asked to show before they were in the document.
const pending = new Set();
// Popovers showing, placed again as the page scrolls or resizes.
const shown = new Set();
let placeFrame = 0;
let panelObserver = null;
// What a press on an open overlay's anchor closed: its click is eaten, so
// it doesn't open it again (as a popovertarget button's).
let pressedAnchor = null;

function report(panel, on) {
  const o = panel.$$o;
  if (o) dispatch(o.handler, 7, on ? 1 : 0, undefined);
}

function setUpOverlay(panel, o) {
  o.open = false;
  o.closedAt = -1e9;
  o.pos = o.backdrop ? "--zbd:" + o.backdrop + ";" : "";
  if (o.kind === TOOLTIP && !(o.flags & 8)) o.pos += "pointer-events:none;";
  panel.$$o = o;
  if (o.arrow > 0) panel.classList.add("za");
  if (o.pos) panel.style.cssText += o.pos;
  if (o.kind === DIALOG) {
    panel.addEventListener("close", () => {
      o.open = false;
      report(panel, false);
    });
    if (o.flags & 1) {
      // A click on the backdrop lands on the dialog, outside its box.
      panel.addEventListener("click", (event) => {
        if (event.target !== panel) return;
        const r = panel.getBoundingClientRect();
        const inside =
          event.clientX >= r.left && event.clientX <= r.right && event.clientY >= r.top && event.clientY <= r.bottom;
        if (!inside) panel.close();
      });
    }
    return;
  }
  panel.popover = o.kind === TOOLTIP || o.kind === LAYER ? "manual" : "auto";
  if (o.kind === LAYER) panel.setAttribute("data-layer", "");
  panel.addEventListener("beforetoggle", (event) => {
    if (event.newState !== "closed" || !panel.$$o) return;
    o.open = false;
    o.closedAt = performance.now();
    // Focus inside goes back to the anchor (a press elsewhere then moves
    // it on, as the press does).
    if (o.anchor && panel.contains(document.activeElement)) {
      const anchor = o.anchor;
      queueMicrotask(() => {
        const now = document.activeElement;
        if (anchor.isConnected && (!now || now === document.body || panel.contains(now)))
          anchor.focus({ preventScroll: true });
      });
    }
  });
  panel.addEventListener("toggle", (event) => {
    if (!panel.$$o) return;
    const on = event.newState === "open";
    if (on) watch(panel);
    else unwatch(panel);
    if (o.anchor && o.kind !== TOOLTIP) o.anchor.setAttribute("aria-expanded", on ? "true" : "false");
    report(panel, on);
  });
  if (o.anchor) {
    if (o.kind === TOOLTIP) o.anchor.$$tip = panel;
    else {
      o.anchor.$$p = panel;
      o.anchor.setAttribute("aria-haspopup", o.kind === MENU || o.kind === SUBMENU ? "menu" : "dialog");
      o.anchor.setAttribute("aria-expanded", "false");
    }
  }
}

// What the page opens itself (a tooltip as the pointer rests, a submenu
// from the pointer or a key): a lazy panel (flag 16) is told to open, so it
// is filled before it shows; others show at once.
function open(panel) {
  const o = panel && panel.$$o;
  if (!o) return;
  if (o.flags & 16) report(panel, true);
  else show(panel, true);
}

function isShown(panel) {
  const o = panel.$$o;
  return o.kind === DIALOG ? panel.open : panel.matches(":popover-open");
}

function show(panel, on) {
  const o = panel && panel.$$o;
  if (!o) return;
  if (!panel.isConnected) {
    if (on) pending.add(panel);
    else pending.delete(panel);
    return;
  }
  if (on === isShown(panel)) return;
  if (o.kind === DIALOG) {
    o.open = on;
    if (on) panel.showModal();
    else panel.close();
    return;
  }
  if (!on) {
    panel.hidePopover();
    return;
  }
  o.open = true;
  panel.showPopover();
  place(panel);
  if (o.kind === MENU || o.kind === SUBMENU) {
    const first = items(panel)[0];
    (first || panel).focus({ preventScroll: true });
  } else if (o.kind === POPOVER && o.flags & 4) {
    const first = panel.querySelector(
      'button:not([disabled]),input:not([disabled]),textarea:not([disabled]),select,[tabindex]:not([tabindex="-1"])',
    );
    (first || panel).focus({ preventScroll: true });
  }
}

function watch(panel) {
  shown.add(panel);
  if (!panelObserver) panelObserver = new ResizeObserver(placeAll);
  panelObserver.observe(panel);
}

function unwatch(panel) {
  shown.delete(panel);
  if (panelObserver) panelObserver.unobserve(panel);
}

function placeAll() {
  if (placeFrame || !shown.size) return;
  placeFrame = requestAnimationFrame(() => {
    placeFrame = 0;
    for (const panel of shown) place(panel);
  });
}

// A popover against its anchor: on its side (the other if it doesn't fit
// there and fits better), lined up at its start, centre or end, and kept
// in the viewport.
function place(panel) {
  const o = panel.$$o;
  if (!o || o.kind === LAYER) return;
  if (!o.at && (!o.anchor || !o.anchor.isConnected)) return;
  // At a point (a context menu), else against its anchor.
  const a = o.at
    ? { left: o.at.x, right: o.at.x, top: o.at.y, bottom: o.at.y, width: 0, height: 0 }
    : o.anchor.getBoundingClientRect();
  let pos = o.kind === TOOLTIP && !(o.flags & 8) ? "pointer-events:none;" : "";
  if (o.flags & 2) {
    pos += "min-width:" + a.width + "px;";
    panel.style.minWidth = a.width + "px";
  }
  const w = panel.offsetWidth;
  const h = panel.offsetHeight;
  const vw = document.documentElement.clientWidth;
  const vh = document.documentElement.clientHeight;
  // An arrow's room: the panel sits that much further off.
  const off = o.offset + (o.arrow > 0 ? o.arrow : 0);
  let side = o.side;
  const room = [vh - a.bottom, a.top, a.left, vw - a.right];
  const need = side < 2 ? h + off : w + off;
  const other = side ^ 1;
  if (need > room[side] && room[other] > room[side]) side = other;
  let x, y;
  const line = (start, size, end, own) => (o.align === 0 ? start : o.align === 1 ? start + (size - own) / 2 : end - own);
  if (side < 2) {
    y = side === 0 ? a.bottom + off : a.top - off - h;
    x = line(a.left, a.width, a.right, w);
  } else {
    x = side === 3 ? a.right + off : a.left - off - w;
    y = line(a.top, a.height, a.bottom, h);
  }
  const m = 4;
  x = Math.max(m, Math.min(x, vw - w - m));
  y = Math.max(m, Math.min(y, vh - h - m));
  pos += "left:" + x + "px;top:" + y + "px;";
  if (o.arrow > 0) {
    // On the side facing the anchor, at its centre, kept off the corners.
    const s = o.arrow;
    const keep = (v, size) => Math.max(s * 1.5, Math.min(v, size - s * 1.5));
    let ax, ay;
    if (side < 2) {
      ax = keep(a.left + a.width / 2 - x, w);
      ay = side === 0 ? 0 : h;
    } else {
      ay = keep(a.top + a.height / 2 - y, h);
      ax = side === 3 ? 0 : w;
    }
    pos += "--zx:" + ax + "px;--zy:" + ay + "px;--zs:" + s * Math.SQRT2 + "px;";
    panel.style.setProperty("--zx", ax + "px");
    panel.style.setProperty("--zy", ay + "px");
    panel.style.setProperty("--zs", s * Math.SQRT2 + "px");
  }
  o.pos = pos;
  panel.style.left = x + "px";
  panel.style.top = y + "px";
}

function overlayOf(node) {
  for (; node && node !== document; node = node.parentNode) {
    if (node.$$p && node.$$p.$$o) return node;
  }
  return null;
}

function onPointerDown(event) {
  hideTip();
  // A press on an open overlay's anchor closes it (the browser's light
  // dismiss, or here), and its click is eaten.
  const anchor = overlayOf(event.target);
  pressedAnchor = null;
  if (anchor) {
    const o = anchor.$$p.$$o;
    if (o.kind !== SUBMENU && (o.open || performance.now() - o.closedAt < 50)) {
      pressedAnchor = anchor;
      if (isShown(anchor.$$p)) show(anchor.$$p, false);
    }
  }
}

function onClickCapture(event) {
  const anchor = pressedAnchor;
  pressedAnchor = null;
  if (anchor && anchor.contains(event.target)) {
    event.stopPropagation();
    event.preventDefault();
  }
}

// A menu's items, in order: its own, not its submenus'.
function items(panel) {
  const all = panel.querySelectorAll('[role^="menuitem"]');
  const out = [];
  for (const item of all) {
    if (item.closest(".zp") === panel && !item.disabled && item.getAttribute("aria-disabled") !== "true") out.push(item);
  }
  return out;
}

function menuOf(node) {
  const panel = node && node.closest && node.closest(".zp");
  return panel && panel.$$o && (panel.$$o.kind === MENU || panel.$$o.kind === SUBMENU) ? panel : null;
}

function onKeyDown(event) {
  hideTip();
  const item = event.target;
  const panel = menuOf(item);
  if (!panel) return;
  const list = items(panel);
  const at = list.indexOf(item);
  let to = -1;
  switch (event.key) {
    case "ArrowDown":
      to = at < 0 || at === list.length - 1 ? 0 : at + 1;
      break;
    case "ArrowUp":
      to = at <= 0 ? list.length - 1 : at - 1;
      break;
    case "Home":
      to = 0;
      break;
    case "End":
      to = list.length - 1;
      break;
    case "ArrowRight": {
      const sub = item.$$p;
      if (sub && sub.$$o && sub.$$o.kind === SUBMENU) {
        open(sub);
        event.preventDefault();
      }
      return;
    }
    case "ArrowLeft":
      if (panel.$$o.kind === SUBMENU) {
        panel.hidePopover();
        event.preventDefault();
      }
      return;
    case "Tab":
      // Tab leaves a menu, closing it, as a native menu does.
      closeMenus(panel);
      return;
    default:
      return;
  }
  event.preventDefault();
  if (list[to]) list[to].focus({ preventScroll: true });
}

function closeMenus(panel) {
  let root = panel;
  for (let p = panel; p; p = menuOf(p.$$o.anchor)) root = p;
  if (isShown(root)) root.hidePopover();
}

// Tooltips: shown as the pointer rests on their anchor (at once within a
// second of another), or keyboard focus reaches it; one at a time, gone as
// the pointer leaves or presses, or a key is pressed.
let tip = null;
let tipTimer = 0;
let tipClosed = -1e9;

function tipAnchorOf(node) {
  for (; node && node !== document; node = node.parentNode) {
    if (node.$$tip && node.$$tip.$$o) return node;
  }
  return null;
}

function showTip(anchor) {
  clearTimeout(tipTimer);
  if (tip && tip !== anchor.$$tip) hideTip();
  tip = anchor.$$tip;
  open(tip);
}

function hideTip() {
  clearTimeout(tipTimer);
  tipTimer = 0;
  if (!tip) return;
  if (tip.$$o && isShown(tip)) show(tip, false);
  tip = null;
  tipClosed = performance.now();
}

function onPointerOver(event) {
  const anchor = tipAnchorOf(event.target);
  if (anchor && (!tip || tip !== anchor.$$tip)) {
    const from = tipAnchorOf(event.relatedTarget);
    if (from !== anchor) {
      const soon = tip !== null || performance.now() - tipClosed < 1000;
      clearTimeout(tipTimer);
      if (soon) showTip(anchor);
      else tipTimer = setTimeout(() => anchor.isConnected && showTip(anchor), anchor.$$tip.$$o.delay);
    }
  } else if (tip && tip.contains(event.target)) {
    clearTimeout(tipTimer);
  }
  // In a menu, the pointer opens a submenu's trigger's submenu and closes
  // the others.
  const item = event.target.closest && event.target.closest('[role^="menuitem"]');
  const panel = item && menuOf(item);
  if (!panel) return;
  for (const sub of panel.querySelectorAll(".zp")) {
    if (sub.$$o && sub.$$o.kind === SUBMENU && sub.$$o.anchor !== item && isShown(sub) && menuOf(sub.$$o.anchor) === panel)
      sub.hidePopover();
  }
  const sub = item.$$p;
  if (sub && sub.$$o && sub.$$o.kind === SUBMENU && !isShown(sub)) {
    open(sub);
    item.focus({ preventScroll: true });
  }
}

function onPointerOut(event) {
  if (!tip && !tipTimer) return;
  const to = event.relatedTarget;
  const anchor = tipAnchorOf(event.target);
  if (!anchor || tipAnchorOf(to) === anchor) return;
  // An interactive card: the pointer may cross into it.
  const card = anchor.$$tip;
  if (card.$$o.flags & 8) {
    setTimeout(() => {
      if (!anchor.matches(":hover") && !card.matches(":hover")) hideTip();
    }, 120);
    return;
  }
  hideTip();
}

function onFocusIn(event) {
  const anchor = tipAnchorOf(event.target);
  if (anchor && event.target === anchor && anchor.matches(":focus-visible")) showTip(anchor);
}

function onFocusOut(event) {
  if (tip && tipAnchorOf(event.target) && !tip.contains(event.relatedTarget)) hideTip();
}

// An element's `on_event` listeners: the events zgx routes, heard in each
// phase (at the element once), each a call into its listeners, whose
// decisions (prevent, stop, focus, capture) apply at once.
const EVENTS = {
  pointerdown: 0,
  pointerup: 1,
  pointermove: 2,
  pointerenter: 3,
  pointerleave: 4,
  wheel: 5,
  keydown: 6,
  keyup: 7,
  focusin: 8,
  focusout: 9,
  click: 10,
  zgxaction: 11,
  pointercancel: 12,
};

function listen(node, handler) {
  node.$$e = handler;
  for (const type in EVENTS) {
    node.addEventListener(type, heardCapturing, true);
    node.addEventListener(type, heard, false);
  }
}

function heardCapturing(event) {
  // At the element itself, the bubbling listener runs it, once.
  if (event.eventPhase !== 2) run(this, event, 0);
}

function heard(event) {
  run(this, event, event.eventPhase === 2 ? 1 : 2);
}

function run(node, event, phase) {
  const handler = node.$$e;
  if (handler === undefined) return;
  // Inside a disabled element, as on the desktop: no input reaches it.
  if (offCount !== 0 && event.target.closest?.("[data-off]")) return;
  const kind = EVENTS[event.type];
  const mods =
    (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
  const clicks = Math.min(event.detail | 0, 255);
  const bits = kind | (phase << 8) | (mods << 12) | (clicks << 16) | (event.repeat ? 1 << 24 : 0);
  let x = 0, y = 0, dx = 0, dy = 0, keys;
  if (event.clientX !== undefined) {
    x = event.clientX;
    y = event.clientY;
  }
  if (kind === 5) {
    // Lines and pages, as points.
    const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? innerHeight : 1;
    dx = event.deltaX * unit;
    dy = event.deltaY * unit;
  } else if (kind <= 1) dx = event.button;
  else if (kind === 11) dx = event.detail;
  else if (kind === 6 || kind === 7) keys = event.key + "\n" + event.code;
  const self = event.target === node || kind === 3 || kind === 4 ? 1 : 0;
  const outcome = elementEvent(handler, bits, self, x, y, dx, dy, keys);
  if (outcome & 1) event.preventDefault();
  if (outcome & 4) event.stopImmediatePropagation();
  else if (outcome & 2) event.stopPropagation();
  if (outcome & 8) node.focus({ preventScroll: true });
  try {
    if (outcome & 16 && event.pointerId !== undefined) node.setPointerCapture(event.pointerId);
    if (outcome & 32 && event.pointerId !== undefined && node.hasPointerCapture(event.pointerId))
      node.releasePointerCapture(event.pointerId);
  } catch (_) {
    // No such pointer (it went, or the event was made by a script).
  }
  if (outcome & 64 && !drag) startDrag(node, handler, event);
}

// A drag a listener started (`start_drag`): its source hears Start, Move
// and End; what's under the pointer, from the innermost element with
// listeners out, hears Over until one takes it (`accept_drag`), then
// Leave as the pointer leaves it, and Drop as the button comes up over it.
// A copy of the source follows the pointer.
let drag = null;

function startDrag(node, handler, event) {
  const r = node.getBoundingClientRect();
  if (node.hasPointerCapture && event.pointerId !== undefined && node.hasPointerCapture(event.pointerId))
    node.releasePointerCapture(event.pointerId);
  const ghost = node.cloneNode(true);
  ghost.style.cssText +=
    ";position:fixed;margin:0;left:" + r.left + "px;top:" + r.top + "px;width:" + r.width + "px;height:" + r.height +
    "px;opacity:.8;pointer-events:none;z-index:2147483647;";
  document.body.appendChild(ghost);
  drag = {
    source: node,
    handler,
    over: null,
    ghost,
    dx: event.clientX - r.left,
    dy: event.clientY - r.top,
    x: event.clientX,
    y: event.clientY,
  };
  elementEvent(handler, 18 | (1 << 8), 1, r.left, r.top, 0, 0, undefined);
  document.addEventListener("pointermove", dragMove, true);
  document.addEventListener("pointerup", dragUp, true);
  document.addEventListener("pointercancel", dragUp, true);
  dragOver(event.clientX, event.clientY);
}

function dragOver(x, y) {
  let taken = null;
  const hit = document.elementFromPoint(x, y);
  for (let n = hit; n && n !== document; n = n.parentNode) {
    if (n.$$e === undefined) continue;
    const outcome = elementEvent(n.$$e, 13 | ((n === hit ? 1 : 2) << 8), n === hit ? 1 : 0, x, y, 0, 0, undefined);
    if (outcome & 128) {
      taken = n;
      break;
    }
    if (outcome & 2) break;
  }
  if (drag.over && drag.over !== taken && drag.over.$$e !== undefined)
    elementEvent(drag.over.$$e, 14 | (1 << 8), 1, x, y, 0, 0, undefined);
  drag.over = taken;
  return hit;
}

function dragMove(event) {
  if (!drag) return;
  const x = event.clientX, y = event.clientY;
  drag.ghost.style.left = x - drag.dx + "px";
  drag.ghost.style.top = y - drag.dy + "px";
  elementEvent(drag.handler, 15 | (1 << 8), 1, x, y, 0, 0, undefined);
  drag.x = x;
  drag.y = y;
  armDragScroll(dragOver(x, y));
  event.stopPropagation();
}

function dragUp(event) {
  if (!drag) return;
  const { handler, over, ghost } = drag;
  const x = event.clientX, y = event.clientY;
  let accepted = 0;
  if (over && over.$$e !== undefined && event.type === "pointerup") {
    accepted = elementEvent(over.$$e, 16 | (1 << 8), 1, x, y, 0, 0, undefined) & 1;
    elementEvent(over.$$e, 14 | (1 << 8), 1, x, y, accepted, 0, undefined);
  }
  drag = null;
  stopDragScroll();
  ghost.remove();
  document.removeEventListener("pointermove", dragMove, true);
  document.removeEventListener("pointerup", dragUp, true);
  document.removeEventListener("pointercancel", dragUp, true);
  elementEvent(handler, 17 | (1 << 8), 1, x, y, accepted, 0, undefined);
  // The press ended in a drag, not a click.
  const eat = (click) => {
    click.stopPropagation();
    click.preventDefault();
  };
  document.addEventListener("click", eat, { capture: true, once: true });
  setTimeout(() => document.removeEventListener("click", eat, { capture: true }), 0);
}

// A drag held near a scroll view's edge scrolls it, as the core's
// (drag_scroll.rs): the innermost one under the pointer that can still move
// toward that edge, up to 24 px a 16 ms frame within 48 px of it (full speed
// from 12 in), per time, a stall making up for 50 ms at most; then what is
// under the pointer hears the drag over it again. A frame is asked for only
// while the pointer is in such a band: none in the middle, at the end of
// what scrolls, or once the drag ends. (The browser scrolls for its own
// drags, files and text from other apps.)
const SCROLL_BAND = 48, SCROLL_FULL = 12, SCROLL_STEP = 24, SCROLL_FRAME = 16, SCROLL_STALL = 50;
let dragScroll = null;

function edgeSpeed(at, start, extent) {
  if (!(at >= start && at <= start + extent)) return 0;
  const ramp = (inside) => Math.min(Math.max((SCROLL_BAND - inside) / (SCROLL_BAND - SCROLL_FULL), 0), 1) * SCROLL_STEP;
  const fromStart = at - start, fromEnd = start + extent - at;
  if (fromEnd < SCROLL_BAND && fromEnd <= fromStart) return ramp(fromEnd);
  if (fromStart < SCROLL_BAND) return -ramp(fromStart);
  return 0;
}

// What the drag at x, y scrolls: { node, dx, dy } in px a frame, or null;
// from `hit`, what is under it, if known (dragOver's).
function dragScrollPlan(x, y, hit = document.elementFromPoint(x, y)) {
  for (let n = hit; n && n.nodeType === 1; n = n.parentElement) {
    const across = n.scrollWidth > n.clientWidth, down = n.scrollHeight > n.clientHeight;
    if (!across && !down) continue;
    const style = getComputedStyle(n);
    const scrolls = (overflow) => overflow === "auto" || overflow === "scroll";
    const r = n.getBoundingClientRect();
    let dx = across && scrolls(style.overflowX) ? edgeSpeed(x, r.left + n.clientLeft, n.clientWidth) : 0;
    let dy = down && scrolls(style.overflowY) ? edgeSpeed(y, r.top + n.clientTop, n.clientHeight) : 0;
    if ((dx < 0 && n.scrollLeft <= 0) || (dx > 0 && n.scrollLeft >= n.scrollWidth - n.clientWidth - 0.5)) dx = 0;
    if ((dy < 0 && n.scrollTop <= 0) || (dy > 0 && n.scrollTop >= n.scrollHeight - n.clientHeight - 0.5)) dy = 0;
    if (dx || dy) return { node: n, dx, dy };
  }
  return null;
}

// Ask for a frame if the drag is in a band, and stop asking if it left one.
function armDragScroll(hit) {
  if (!drag || !dragScrollPlan(drag.x, drag.y, hit)) {
    stopDragScroll();
    return;
  }
  // Its first tick moves a frame's worth (rAF's time is its frame's start,
  // before now).
  if (!dragScroll) dragScroll = { frame: requestAnimationFrame(dragScrollTick), last: null };
}

function dragScrollTick(now) {
  const last = dragScroll && dragScroll.last !== null ? dragScroll.last : now - SCROLL_FRAME;
  dragScroll = null;
  if (!drag) return;
  const plan = dragScrollPlan(drag.x, drag.y);
  if (!plan) return;
  const frames = Math.min(Math.max(now - last, 0), SCROLL_STALL) / SCROLL_FRAME;
  if (frames > 0) {
    const { node } = plan, before = node.scrollLeft + "," + node.scrollTop;
    node.scrollBy({ left: plan.dx * frames, top: plan.dy * frames, behavior: "instant" });
    // It could not move (it scrolls along the other axis only): until the
    // pointer moves.
    if (node.scrollLeft + "," + node.scrollTop === before) return;
    dragOver(drag.x, drag.y);
  }
  dragScroll = { frame: requestAnimationFrame(dragScrollTick), last: Math.max(now, last) };
}

function stopDragScroll() {
  if (dragScroll) cancelAnimationFrame(dragScroll.frame);
  dragScroll = null;
}

// Whether a frame is asked for a drag's autoscroll (for tests: none outside
// a band).
export function zgx_drag_scroll_pending() {
  return dragScroll !== null;
}

// Text dragged in from another app or the desktop (the browser's own drag
// and drop): what's under it, from the innermost element with listeners out,
// hears Over as an in-app drag's destination does (its text not yet known),
// and the one that takes it (`accept_drag`) hears Drop with its text and
// HTML. Elements that take none leave the browser's own drop alone (an input
// takes text itself). Skipped while an in-app drag runs: it has the pointer.
const OUTSIDE = 1 << 25;
let outside = null;

function carriesText(event) {
  const types = event.dataTransfer ? Array.from(event.dataTransfer.types) : [];
  return types.includes("text/plain") && !types.includes("Files");
}

function onOutsideOver(event) {
  if (drag || !carriesText(event)) return;
  const x = event.clientX, y = event.clientY;
  if (!outside) {
    // Given up without a drop or a leave the page hears (the source app
    // ending it): over a page, dragover comes every 350 ms at most, so a
    // second without one ends it.
    const watch = setInterval(() => {
      if (outside && performance.now() - outside.last > 1000) endOutside(outside.x, outside.y);
    }, 250);
    outside = { over: null, watch, last: 0, x, y };
  }
  outside.last = performance.now();
  outside.x = x;
  outside.y = y;
  const path = event.composedPath();
  let taken = null;
  for (const n of path) {
    if (n === document) break;
    if (n.$$e === undefined) continue;
    const self = n === path[0];
    const outcome = elementEvent(n.$$e, 13 | ((self ? 1 : 2) << 8) | OUTSIDE, self ? 1 : 0, x, y, 0, 0, undefined);
    if (outcome & 128) {
      taken = n;
      break;
    }
    if (outcome & 2) break;
  }
  if (outside.over && outside.over !== taken && outside.over.$$e !== undefined)
    elementEvent(outside.over.$$e, 14 | (1 << 8) | OUTSIDE, 1, x, y, 0, 0, undefined);
  outside.over = taken;
  if (taken) {
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
  }
}

function endOutside(x, y) {
  if (!outside) return;
  if (outside.over && outside.over.$$e !== undefined)
    elementEvent(outside.over.$$e, 14 | (1 << 8) | OUTSIDE, 1, x, y, 0, 0, undefined);
  clearInterval(outside.watch);
  outside = null;
  // No element's handler: the drag in the document ends.
  elementEvent(-1 >>> 0, 17 | OUTSIDE, 0, x, y, 0, 0, undefined);
}

function onOutsideLeave(event) {
  if (!outside) return;
  const x = event.clientX, y = event.clientY;
  // Left the document, or given up over it (Escape): no element entered.
  // Else left one element for another; outside the window, ended all the
  // same.
  if (event.relatedTarget === null || x <= 0 || y <= 0 || x >= innerWidth || y >= innerHeight)
    endOutside(x, y);
}

function onOutsideDrop(event) {
  if (drag || !outside) return;
  const over = outside.over;
  const x = event.clientX, y = event.clientY;
  if (over && over.$$e !== undefined) {
    event.preventDefault();
    const data = event.dataTransfer;
    const text = "\n" + data.getData("text/plain") + "\u0000" + data.getData("text/html");
    elementEvent(over.$$e, 16 | (1 << 8) | OUTSIDE, 1, x, y, 0, 0, text);
  }
  endOutside(x, y);
}

// Keymaps (`View::keymap`): a key down goes to the keymaps from the focused
// element out before anything hears it. A sequence's first keys are taken;
// a match's actions go out in turn as `zgxaction` events from the focused
// element, until one is handled (a listener prevents its default).
function onKeymaps(event) {
  const path = [];
  for (let n = event.target; n && n !== document; n = n.parentNode) if (n.$$k !== undefined) path.push(n.$$k);
  if (!path.length) return;
  // Paste (⌘V, Ctrl+V): resolved as the paste event comes, so an action
  // bound to it reads what is pasted (the page reads the clipboard only
  // then). The key itself goes on to make that event.
  pastePending = null;
  if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === "v") {
    pastePending = { event, path };
    return;
  }
  if (resolveFor(event, event.target, path)) {
    event.preventDefault();
    event.stopPropagation();
  }
}

// A key's keymaps (`path`) resolved and their actions routed from `target`:
// whether something took it (a sequence's first keys, or an action handled).
function resolveFor(event, target, path) {
  const mods =
    (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
  const found = resolveKeys(path.join(","), event.key, event.code, mods | (event.repeat ? 16 : 0));
  if (found === 0) return false;
  if (found === 1) return true;
  for (let c = 0; c < found - 2; c++) {
    const action = new CustomEvent("zgxaction", { bubbles: true, cancelable: true, detail: c });
    target.dispatchEvent(action);
    if (action.defaultPrevented) return true;
  }
  return false;
}

// Focus groups (`View::focus_group`, list boxes, tab lists, toolbars, radio
// groups): one Tab stop, the arrows (and Home, End) moving among its own
// members; a radio group's wrap and choose.
function group(node, bits) {
  node.$$fg = bits;
  queueMicrotask(() => node.isConnected && rove(node, null));
}

function groupOf(node) {
  for (let n = node && node.parentNode; n && n !== document; n = n.parentNode) if (n.$$fg !== undefined) return n;
  return null;
}

function members(group) {
  const out = [];
  for (const el of group.querySelectorAll("button,input,select,textarea,a[href],[tabindex]")) {
    if (el.disabled || el.getAttribute("aria-disabled") === "true" || groupOf(el) !== group) continue;
    if (el.tabIndex < 0 && !el.$$rove) continue;
    out.push(el);
  }
  return out;
}

// One member a Tab stop: `to`, else the checked or selected one, else the first.
function rove(group, to) {
  const all = members(group);
  if (!all.length) return;
  const stop =
    to ||
    all.find((el) => el.getAttribute("aria-checked") === "true" || el.getAttribute("aria-selected") === "true" || el.checked) ||
    all[0];
  for (const el of all) {
    el.$$rove = true;
    el.tabIndex = el === stop ? 0 : -1;
  }
}

function onGroupFocus(event) {
  const group = groupOf(event.target);
  if (group && event.target.$$rove) rove(group, event.target);
}

function onGroupKey(event) {
  if (event.defaultPrevented) return;
  const group = groupOf(event.target);
  if (!group) return;
  const axis = group.$$fg & 3;
  const wraps = (group.$$fg & 4) !== 0;
  const vertical = axis & 1;
  const horizontal = axis & 2;
  const rtl = getComputedStyle(group).direction === "rtl";
  let step;
  switch (event.key) {
    case "ArrowUp":
      if (vertical) step = -1;
      break;
    case "ArrowDown":
      if (vertical) step = 1;
      break;
    case "ArrowLeft":
      if (horizontal) step = rtl ? 1 : -1;
      break;
    case "ArrowRight":
      if (horizontal) step = rtl ? -1 : 1;
      break;
    case "Home":
      step = "first";
      break;
    case "End":
      step = "last";
      break;
  }
  if (step === undefined) return;
  const all = members(group);
  const at = all.indexOf(event.target);
  if (at < 0) return;
  let to;
  if (step === "first") to = all[0];
  else if (step === "last") to = all[all.length - 1];
  else if (wraps) to = all[(at + step + all.length) % all.length];
  else to = all[at + step];
  if (!to || to === event.target) return;
  event.preventDefault();
  rove(group, to);
  to.focus();
  if (wraps) to.click();
}

// A canvas's drawing (`src/canvas.rs`) with Canvas 2D, at its size in CSS
// pixels and the screen's pixel ratio. Text goes over the shapes, unclipped.
function rgba(word) {
  return "rgba(" + (word >>> 24) + "," + ((word >>> 16) & 255) + "," + ((word >>> 8) & 255) + "," + (word & 255) / 255 + ")";
}

// Canvas pictures by image id (`Draw::Image`), the most recent kept.
const pictures = new Map();

function picture(r, lo, hi, w, h, ptr, len) {
  const key = hi * 4294967296 + lo;
  let canvas = pictures.get(key);
  if (canvas) pictures.delete(key);
  else {
    canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const pixels = new Uint8ClampedArray(r.memory.buffer, ptr, len).slice();
    canvas.getContext("2d").putImageData(new ImageData(pixels, w, h), 0, 0);
    if (pictures.size >= 64) pictures.delete(pictures.keys().next().value);
  }
  pictures.set(key, canvas);
  return canvas;
}

function paint(node, width, height, count, r) {
  const ratio = devicePixelRatio || 1;
  const w = Math.max(1, Math.round(width * ratio));
  const h = Math.max(1, Math.round(height * ratio));
  if (node.width !== w) node.width = w;
  if (node.height !== h) node.height = h;
  const ctx = node.getContext("2d");
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  const path = () => {
    const p = new Path2D();
    switch (r.next()) {
      case 0: {
        const x = r.num(), y = r.num(), pw = r.num(), ph = r.num(), radius = r.num();
        if (radius > 0) p.roundRect(x, y, pw, ph, radius);
        else p.rect(x, y, pw, ph);
        break;
      }
      case 1: {
        const x = r.num(), y = r.num(), pw = r.num(), ph = r.num();
        p.ellipse(x + pw / 2, y + ph / 2, pw / 2, ph / 2, 0, 0, Math.PI * 2);
        break;
      }
      case 2:
        p.moveTo(r.num(), r.num());
        p.lineTo(r.num(), r.num());
        break;
      default: {
        const n = r.next();
        for (let k = 0; k < n; k++) {
          switch (r.next()) {
            case 0:
              p.moveTo(r.num(), r.num());
              break;
            case 1:
              p.lineTo(r.num(), r.num());
              break;
            case 2:
              p.quadraticCurveTo(r.num(), r.num(), r.num(), r.num());
              break;
            case 3:
              p.bezierCurveTo(r.num(), r.num(), r.num(), r.num(), r.num(), r.num());
              break;
            default:
              p.closePath();
          }
        }
      }
    }
    return p;
  };
  const brush = () => {
    const kind = r.next();
    switch (kind) {
      case 0:
        return rgba(r.next());
      case 1: {
        const g = ctx.createLinearGradient(r.num(), r.num(), r.num(), r.num());
        const n = r.next();
        for (let k = 0; k < n; k++) g.addColorStop(r.num(), rgba(r.next()));
        return g;
      }
      case 3:
      case 4: {
        // Radial or conic, drawn in the unit space `matrix` maps onto the
        // canvas (`cover`): Canvas 2D's gradients take no matrix of their own.
        const matrix = [r.num(), r.num(), r.num(), r.num(), r.num(), r.num()];
        const g = kind === 3 ? ctx.createRadialGradient(0, 0, 0, 0, 0, 1) : ctx.createConicGradient(0, 0, 0);
        const n = r.next();
        for (let k = 0; k < n; k++) g.addColorStop(r.num(), rgba(r.next()));
        return { g, matrix };
      }
      default: {
        // Diagonal stripes: a tile of one, repeated.
        const background = rgba(r.next()), foreground = rgba(r.next());
        const spacing = Math.max(1, r.num()), stripe = r.num();
        const size = Math.max(1, Math.round(spacing * ratio));
        const tile = document.createElement("canvas");
        tile.width = tile.height = size;
        const t = tile.getContext("2d");
        t.fillStyle = background;
        t.fillRect(0, 0, size, size);
        t.strokeStyle = foreground;
        t.lineWidth = stripe * ratio;
        t.beginPath();
        for (const at of [-size, 0, size]) {
          t.moveTo(at, size);
          t.lineTo(at + size, 0);
        }
        t.stroke();
        const pattern = ctx.createPattern(tile, "repeat");
        pattern.setTransform(new DOMMatrix([1 / ratio, 0, 0, 1 / ratio, 0, 0]));
        return pattern;
      }
    }
  };
  // Fills `to` (its clip or what it is drawn into) with a radial or conic
  // brush: the canvas's bounds taken into the brush's unit space and filled.
  const cover = (to, { g, matrix }) => {
    const back = new DOMMatrix(matrix).inverse();
    const corners = [[0, 0], [width, 0], [0, height], [width, height]].map(([x, y]) => back.transformPoint({ x, y }));
    const xs = corners.map((p) => p.x), ys = corners.map((p) => p.y);
    const x = Math.min(...xs), y = Math.min(...ys);
    to.transform(...matrix);
    to.fillStyle = g;
    to.fillRect(x, y, Math.max(...xs) - x, Math.max(...ys) - y);
  };
  // A second canvas the canvas's size, cleared, drawn in CSS pixels: where a
  // gradient stroke or a shadow's shape is made before it is drawn here.
  let spare = null;
  const scratch = () => {
    if (!spare) {
      spare = document.createElement("canvas");
      spare.width = w;
      spare.height = h;
    }
    const s = spare.getContext("2d");
    s.setTransform(1, 0, 0, 1, 0, 0);
    s.globalCompositeOperation = "source-over";
    s.clearRect(0, 0, w, h);
    s.setTransform(ratio, 0, 0, ratio, 0, 0);
    return s;
  };
  const unscaled = (draw) => {
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    draw();
    ctx.restore();
  };
  const caps = ["butt", "round", "square"];
  const joins = ["miter", "round", "bevel"];
  let depth = 0;
  const texts = [];
  for (let c = 0; c < count; c++) {
    switch (r.next()) {
      case 1: {
        const rule = r.next() === 1 ? "evenodd" : "nonzero";
        const style = brush();
        const shape = path();
        if (style.matrix) {
          ctx.save();
          ctx.clip(shape, rule);
          cover(ctx, style);
          ctx.restore();
        } else {
          ctx.fillStyle = style;
          ctx.fill(shape, rule);
        }
        break;
      }
      case 2: {
        ctx.lineWidth = r.num();
        ctx.lineCap = caps[r.next()];
        ctx.lineJoin = joins[r.next()];
        ctx.miterLimit = r.num();
        const dashes = [];
        for (let n = r.next(); n > 0; n--) dashes.push(r.num());
        ctx.setLineDash(dashes);
        ctx.lineDashOffset = r.num();
        const style = brush();
        const shape = path();
        if (style.matrix) {
          // The line drawn alone, then the gradient kept only where it is.
          const s = scratch();
          s.lineWidth = ctx.lineWidth;
          s.lineCap = ctx.lineCap;
          s.lineJoin = ctx.lineJoin;
          s.miterLimit = ctx.miterLimit;
          s.setLineDash(ctx.getLineDash());
          s.lineDashOffset = ctx.lineDashOffset;
          s.stroke(shape);
          s.globalCompositeOperation = "source-in";
          cover(s, style);
          unscaled(() => ctx.drawImage(spare, 0, 0));
        } else {
          ctx.strokeStyle = style;
          ctx.stroke(shape);
        }
        break;
      }
      case 3: {
        const rule = r.next() === 1 ? "evenodd" : "nonzero";
        ctx.save();
        depth++;
        ctx.clip(path(), rule);
        break;
      }
      case 4:
        if (depth > 0) {
          ctx.restore();
          depth--;
        }
        break;
      case 6: {
        // A picture placed by `fit` in its box, cut to the box's corners.
        const at = [r.next(), r.next(), r.next(), r.next(), r.next(), r.next()];
        const [x, y, bw, bh, radius, opacity, px, py, pw, ph] = Array.from({ length: 10 }, r.num);
        if (at[5] === 0 || !(bw > 0 && bh > 0 && opacity > 0)) break;
        const box = new Path2D();
        if (radius > 0) box.roundRect(x, y, bw, bh, radius);
        else box.rect(x, y, bw, bh);
        ctx.save();
        ctx.clip(box);
        ctx.globalAlpha = Math.min(1, opacity);
        ctx.drawImage(picture(r, ...at), px, py, pw, ph);
        ctx.restore();
        break;
      }
      case 7: {
        // A shadow: its shape (grown or shrunk by `spread`) made alone, then
        // drawn far off the canvas with Canvas 2D's shadow brought back over
        // it, blurred (`shadowBlur` is twice the sigma, in device pixels).
        const color = rgba(r.next()), dx = r.num(), dy = r.num(), blur = r.num(), spread = r.num();
        const s = scratch();
        const shape = path();
        s.fillStyle = s.strokeStyle = "#000";
        s.setLineDash([]);
        s.fill(shape);
        if (spread !== 0) {
          s.lineWidth = Math.abs(spread) * 2;
          s.lineJoin = "round";
          if (spread < 0) s.globalCompositeOperation = "destination-out";
          s.stroke(shape);
        }
        unscaled(() => {
          const far = 2 * w + 2 * h;
          ctx.shadowColor = color;
          ctx.shadowBlur = Math.max(0, blur) * 2 * ratio;
          ctx.shadowOffsetX = dx * ratio + far;
          ctx.shadowOffsetY = dy * ratio;
          ctx.drawImage(spare, -far, 0);
        });
        break;
      }
      default:
        texts.push([r.str(), r.str(), r.num(), r.num(), r.next(), r.next(), r.next()]);
    }
  }
  while (depth-- > 0) ctx.restore();
  for (const [text, font, x, y, color, anchor, baseline] of texts) {
    ctx.font = font;
    ctx.fillStyle = rgba(color);
    ctx.textAlign = ["left", "center", "right"][anchor];
    ctx.textBaseline = ["top", "middle", "bottom"][baseline];
    ctx.fillText(text, x, y);
  }
}

// A text's width and height in `font`, as Canvas 2D draws it
// (`canvas::measure_text` while a canvas draws).
let measurer = null;
export function zgx_measure(font, text) {
  if (!measurer) measurer = document.createElement("canvas").getContext("2d");
  measurer.font = font;
  const m = measurer.measureText(text);
  return new Float64Array([m.width, m.fontBoundingBoxAscent + m.fontBoundingBoxDescent]);
}

// A secondary click: the nearest element with a context menu (a span's own
// before its paragraph's) builds its entries, told where and the span it
// was on; one that opens a menu prevents the browser's.
function onContextMenu(event) {
  let span = null;
  for (let n = event.target; n && n !== document; n = n.parentNode) {
    if (n.$$si !== undefined && !span) span = n;
    if (n.$$m === undefined) continue;
    const r = n.getBoundingClientRect();
    const index = span ? span.$$si : -1;
    const href = span && span.getAttribute ? span.getAttribute("href") || "" : "";
    const text = span ? span.textContent : "";
    prevented = false;
    dispatch(n.$$m, 8, 0, [event.clientX, event.clientY, event.clientX - r.left, event.clientY - r.top, index, href, text].join("\n"));
    if (prevented) {
      prevented = false;
      event.preventDefault();
    }
    return;
  }
}

// The clipboard (`src/platform.rs`). A page reads it only inside a paste
// event: reads give that paste's, else what the page last copied, cut,
// pasted or wrote.
let clipboard = { text: null, html: null };
let pastePending = null;

export function zgx_clipboard_text() {
  return clipboard.text;
}

export function zgx_clipboard_html() {
  return clipboard.html;
}

export function zgx_clipboard_write(text, html) {
  clipboard = { text, html: html === undefined ? null : html };
  const c = navigator.clipboard;
  if (!c) return false;
  const done = html != null && typeof ClipboardItem !== "undefined"
    ? c.write([
        new ClipboardItem({
          "text/plain": new Blob([text], { type: "text/plain" }),
          "text/html": new Blob([html], { type: "text/html" }),
        }),
      ])
    : c.writeText(text);
  // Refused (no permission, the page not focused): what was asked stays
  // what this page reads.
  done.catch(() => {});
  return true;
}

// A tag for each element a focus snapshot saw, to tell it from an element
// made later in the slot it freed, however many snapshots came between.
const focusTags = new WeakMap();
let nextFocusTag = 0;
const encoder = new TextEncoder();

// A field whose value has a selection (text, search, a textarea…).
function selects(el) {
  return (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) && el.selectionStart !== null;
}

// A contenteditable over a model (`atom_editor.rs`): the element that takes
// `target`'s edits, if it is in one.
function editHost(target) {
  let node = target;
  while (node && node.$$e === undefined) node = node.parentNode;
  return node || null;
}

// Where `node`'s `offset` is in `el`'s text, in UTF-16 units: its chips'
// labels are its text.
function textOffset(el, node, offset) {
  const range = document.createRange();
  range.setStart(el, 0);
  range.setEnd(node, offset);
  return range.toString().length;
}

// `el`'s selection in UTF-16 units of its text, in order.
function editSelection(el) {
  const selection = document.getSelection();
  if (!selection || selection.rangeCount === 0 || !el.contains(selection.anchorNode)) {
    const end = el.textContent.length;
    return [end, end];
  }
  const range = selection.getRangeAt(0);
  return [
    textOffset(el, range.startContainer, range.startOffset),
    textOffset(el, range.endContainer, range.endOffset),
  ];
}

// A DOM point at `at` UTF-16 units into `el`'s text: never inside a chip
// (contenteditable false), whose edges stand for it.
function pointAt(el, at) {
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let seen = 0;
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const length = node.nodeValue.length;
    const chip = node.parentNode !== el && node.parentNode.isContentEditable === false
      ? node.parentNode
      : null;
    if (at <= seen + length) {
      if (!chip) return [node, at - seen];
      const index = Array.prototype.indexOf.call(chip.parentNode.childNodes, chip);
      return at === seen ? [chip.parentNode, index] : [chip.parentNode, index + 1];
    }
    seen += length;
  }
  return [el, el.childNodes.length];
}

// `el`'s selection set from its model's (UTF-16 units), while it has the focus.
function selectIn(el, anchor, focus) {
  const active = document.activeElement;
  if (active !== el && !el.contains(active)) return;
  const selection = document.getSelection();
  if (!selection) return;
  const [anchorNode, anchorOffset] = pointAt(el, anchor);
  const [focusNode, focusOffset] = pointAt(el, focus);
  selection.setBaseAndExtent(anchorNode, anchorOffset, focusNode, focusOffset);
}

// An edit of `el` for its model: `kind` (an input type), `data`, over bytes
// `start` to `end` of its text.
function sendEdit(el, kind, data, start, end) {
  // Both ends in one number, each below 2^26 (a text past 64 MiB keeps its
  // ends there rather than mixing them).
  const most = 67108863;
  dispatch(el.$$e, 20, Math.min(start, most) * 67108864 + Math.min(end, most), kind + "\0" + data);
}

// Input types whose range is not at the caret: a spelling correction or a
// text replacement, a drop, a drag's delete. Every other edit takes the
// selection, and the model decides what a delete takes (a chip whole, its
// own words and lines), as a native field does.
const TARGETED = new Set(["insertReplacementText", "insertFromDrop", "deleteByDrag"]);

// The edit's own range where it isn't the caret and the browser gives one,
// else the selection: UTF-16 units of `el`'s text, in order.
function editRange(el, event) {
  const ranges = TARGETED.has(event.inputType) && event.getTargetRanges ? event.getTargetRanges() : [];
  if (ranges.length) {
    const range = ranges[0];
    if (el.contains(range.startContainer) && el.contains(range.endContainer)) {
      return [
        textOffset(el, range.startContainer, range.startOffset),
        textOffset(el, range.endContainer, range.endOffset),
      ];
    }
  }
  return editSelection(el);
}

// Input types the browser won't let be prevented: what it writes is the
// model's at the composition's end, and the text is shown again whole
// after it (`onInput`).
const UNPREVENTED = new Set(["insertCompositionText", "deleteCompositionText", "insertFromComposition"]);

function onBeforeInput(event) {
  const el = editHost(event.target);
  if (!el) return;
  // An input method's text goes in as the browser shows it, its end to the
  // model (`onCompositionEnd`).
  if (UNPREVENTED.has(event.inputType) || event.isComposing) {
    el.$$dirty = true;
    return;
  }
  event.preventDefault();
  const text = el.textContent;
  const [start, end] = editRange(el, event);
  let data = event.data;
  // A paste or a drop: its plain text alone.
  if (data == null && event.dataTransfer) data = event.dataTransfer.getData("text/plain");
  sendEdit(el, event.inputType, data || "", utf8(text, start), utf8(text, end));
}

function onCompositionStart(event) {
  const el = editHost(event.target);
  if (!el) return;
  const text = el.textContent;
  const [start, end] = editSelection(el);
  el.$$cs = [utf8(text, start), utf8(text, end)];
  sendEdit(el, "compositionstart", "", el.$$cs[0], el.$$cs[1]);
}

function onCompositionEnd(event) {
  const el = editHost(event.target);
  if (!el) return;
  const [start, end] = el.$$cs || [0, 0];
  el.$$cs = undefined;
  el.$$dirty = false;
  sendEdit(el, "compositionend", event.data || "", start, end);
}

// Undo and redo by key in a contenteditable over a model: the model's, not
// the browser's (which holds none of it).
function onEditKey(event) {
  const el = editHost(event.target);
  if (!el || !(event.metaKey || event.ctrlKey) || event.altKey) return;
  const key = event.key.toLowerCase();
  const redo = (key === "z" && event.shiftKey) || (key === "y" && !event.shiftKey);
  if (key !== "z" && !redo) return;
  event.preventDefault();
  const text = el.textContent;
  const [start, end] = editSelection(el);
  sendEdit(el, redo ? "historyRedo" : "historyUndo", "", utf8(text, start), utf8(text, end));
}

// UTF-16 offset `at` of `text` in UTF-8 bytes, as Rust counts it.
function utf8(text, at) {
  return encoder.encode(text.slice(0, at)).length;
}

// UTF-8 offset `bytes` of `text` in UTF-16 units: -1 past its end or inside a
// character.
function utf16(text, bytes) {
  let b = 0;
  for (let i = 0; i < text.length; ) {
    if (b === bytes) return i;
    if (b > bytes) return -1;
    const c = text.codePointAt(i);
    b += c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
    i += c >= 0x10000 ? 2 : 1;
  }
  return b === bytes ? text.length : -1;
}

// `NodeHandle::focus_snapshot`: the node with the document's focus, its tag,
// and a field's selection as anchor and focus in UTF-8 bytes; none if focus
// is on no node of ours. Rare (a guide starting), so the node is looked up.
export function zgx_focus_snapshot() {
  const el = document.activeElement;
  const id = el ? nodes.indexOf(el) : -1;
  if (id < 0) return undefined;
  let tag = focusTags.get(el);
  if (tag === undefined) {
    tag = nextFocusTag++;
    focusTags.set(el, tag);
  }
  if (!selects(el)) return [id, tag];
  const start = utf8(el.value, el.selectionStart);
  const end = utf8(el.value, el.selectionEnd);
  return el.selectionDirection === "backward" ? [id, tag, end, start] : [id, tag, start, end];
}

// `NodeHandle::restore_focus`: focus back on the snapshot's node, its
// selection set, nothing scrolled (no container around it, the page or the
// field's own text). False, and nothing changed, if the node is gone or its
// value no longer holds the offsets (anchor -1: no selection to set).
export function zgx_restore_focus(id, tag, anchor, focus) {
  const el = nodes[id];
  if (!(el instanceof HTMLElement) || !el.isConnected || focusTags.get(el) !== tag) return false;
  let range = null;
  if (anchor >= 0) {
    if (!selects(el)) return false;
    const a = utf16(el.value, anchor);
    const f = utf16(el.value, focus);
    if (a < 0 || f < 0) return false;
    range = a <= f ? [a, f, "forward"] : [f, a, "backward"];
  }
  // Setting a selection scrolls an ancestor to it in some engines: every
  // scroll position around it is put back as it was.
  const kept = [];
  for (let node = el; node; node = node.parentElement) {
    kept.push([node, node.scrollLeft, node.scrollTop]);
  }
  const page = [window.scrollX, window.scrollY];
  el.focus({ preventScroll: true });
  if (document.activeElement !== el) return false;
  if (range) el.setSelectionRange(...range);
  for (const [node, left, top] of kept) {
    if (node.scrollLeft !== left) node.scrollLeft = left;
    if (node.scrollTop !== top) node.scrollTop = top;
  }
  if (window.scrollX !== page[0] || window.scrollY !== page[1]) window.scrollTo(page[0], page[1]);
  return true;
}

// `NodeHandle::scroll_into_view`: the browser's own, at once, so a script
// hears whether anything moved. Every scroll container around the node and
// the page are read before and after an instant scroll; one asked to ease
// (`animate` 1, or -1 and no prefers-reduced-motion) goes back and eases
// there from where it was. False for a node not in the document.
export function zgx_scroll_into_view(id, block, inline, animate) {
  const node = nodes[id];
  if (!(node instanceof Element) || !node.isConnected) return false;
  const around = [];
  for (let el = node.parentElement; el; el = el.parentElement) {
    around.push([el, el.scrollLeft, el.scrollTop]);
  }
  const page = [window.scrollX, window.scrollY];
  node.scrollIntoView({ block, inline, behavior: "instant" });
  const moved =
    around.some(([el, x, y]) => el.scrollLeft !== x || el.scrollTop !== y) ||
    window.scrollX !== page[0] ||
    window.scrollY !== page[1];
  const ease =
    animate === 1 ||
    (animate === -1 && !window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  if (moved && ease) {
    for (const [el, x, y] of around) {
      el.scrollLeft = x;
      el.scrollTop = y;
    }
    window.scrollTo(page[0], page[1]);
    node.scrollIntoView({ block, inline, behavior: "smooth" });
  }
  return moved;
}

export function zgx_open_url(url) {
  return window.open(url, "_blank", "noopener,noreferrer") !== null || true;
}

function onCopy() {
  const selection = document.getSelection();
  const text = selection ? selection.toString() : "";
  if (text) clipboard = { text, html: null };
}

function onPaste(event) {
  const data = event.clipboardData;
  if (data) {
    const text = data.getData("text/plain");
    const html = data.getData("text/html");
    clipboard = { text: text || null, html: html || null };
  }
  // A paste a keymap waited for (`onKeymaps`): its actions run now.
  const pending = pastePending;
  pastePending = null;
  if (pending && resolveFor(pending.event, pending.event.target, pending.path)) {
    event.preventDefault();
    event.stopPropagation();
  }
}
