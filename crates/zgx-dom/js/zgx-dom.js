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

export function zgx_init(baseCss, onDispatch, onFlush, onEvent, onKeys) {
  root = document;
  dispatch = onDispatch;
  flush = onFlush;
  elementEvent = onEvent;
  resolveKeys = onKeys;
  const style = document.createElement("style");
  style.textContent = baseCss;
  document.head.appendChild(style);
  sheet = style.sheet;
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
  document.addEventListener("keydown", onGroupKey);
  document.addEventListener("focusin", onGroupFocus);
  document.addEventListener("keydown", onLinkKey);
}

// A link with no href (`role=link`): Enter presses it, as it does a link.
function onLinkKey(event) {
  const node = event.target;
  if (event.key === "Enter" && node.$$c !== undefined && node.getAttribute && node.getAttribute("role") === "link") {
    event.preventDefault();
    node.click();
  }
}

// A node made outside: the element a view mounts into.
export function zgx_adopt(id, element) {
  nodes[id] = element;
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

function onInput(event) {
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
        node[name] = ops[i++] === 1;
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
        const anchor = anchorId === 0xffffffff ? null : nodes[anchorId];
        setUpOverlay(panel, { kind, anchor, handler, side, align, offset, flags, delay, backdrop });
        break;
      }
      case 30: { // SHOW panel on
        const panel = nodes[ops[i++]];
        show(panel, ops[i++] === 1);
        break;
      }
      case 31: // PREVENT: the click being handled doesn't do its default
        prevented = true;
        break;
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
  if (!o || !o.anchor || o.kind === LAYER || !o.anchor.isConnected) return;
  const a = o.anchor.getBoundingClientRect();
  let pos = o.kind === TOOLTIP && !(o.flags & 8) ? "pointer-events:none;" : "";
  if (o.flags & 2) {
    pos += "min-width:" + a.width + "px;";
    panel.style.minWidth = a.width + "px";
  }
  const w = panel.offsetWidth;
  const h = panel.offsetHeight;
  const vw = document.documentElement.clientWidth;
  const vh = document.documentElement.clientHeight;
  const off = o.offset;
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
        show(sub, true);
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
  show(tip, true);
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
    show(sub, true);
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
  if (outcome & 16 && event.pointerId !== undefined) node.setPointerCapture(event.pointerId);
  if (outcome & 32 && event.pointerId !== undefined && node.hasPointerCapture(event.pointerId))
    node.releasePointerCapture(event.pointerId);
}

// Keymaps (`View::keymap`): a key down goes to the keymaps from the focused
// element out before anything hears it. A sequence's first keys are taken;
// a match's actions go out in turn as `zgxaction` events from the focused
// element, until one is handled (a listener prevents its default).
function onKeymaps(event) {
  const path = [];
  for (let n = event.target; n && n !== document; n = n.parentNode) if (n.$$k !== undefined) path.push(n.$$k);
  if (!path.length) return;
  const mods =
    (event.shiftKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.altKey ? 4 : 0) | (event.metaKey ? 8 : 0);
  const found = resolveKeys(path.join(","), event.key, event.code, mods | (event.repeat ? 16 : 0));
  if (found === 0) return;
  if (found > 1) {
    let handled = false;
    for (let c = 0; c < found - 2 && !handled; c++) {
      const action = new CustomEvent("zgxaction", { bubbles: true, cancelable: true, detail: c });
      event.target.dispatchEvent(action);
      handled = action.defaultPrevented;
    }
    if (!handled) return;
  }
  event.preventDefault();
  event.stopPropagation();
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
