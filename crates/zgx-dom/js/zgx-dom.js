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

const TAGS = ["div", "span", "button", "input", "textarea", "progress", "label", "img", "canvas"];

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
  document.addEventListener("keydown", onKeymaps, true);
  document.addEventListener("keydown", onGroupKey);
  document.addEventListener("focusin", onGroupFocus);
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
        const id = ops[i++];
        nodes[id].style.cssText = str();
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
      case 26: { // VISIBLE id handler margin(f32 bits)
        const node = nodes[ops[i++]];
        const handler = ops[i++];
        const margin = new Float32Array(new Uint32Array([ops[i++]]).buffer)[0];
        observeVisible(node, handler, margin);
        break;
      }
      default:
        throw new Error("zgx-dom: unknown op " + ops[i - 1]);
    }
  }
}

// What the benchmark and tests read: how many nodes the table holds.
export function zgx_live_nodes() {
  let n = 0;
  for (const node of nodes) if (node !== undefined) n++;
  return n;
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
  drag = { source: node, handler, over: null, ghost, dx: event.clientX - r.left, dy: event.clientY - r.top };
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
}

function dragMove(event) {
  if (!drag) return;
  const x = event.clientX, y = event.clientY;
  drag.ghost.style.left = x - drag.dx + "px";
  drag.ghost.style.top = y - drag.dy + "px";
  elementEvent(drag.handler, 15 | (1 << 8), 1, x, y, 0, 0, undefined);
  dragOver(x, y);
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
