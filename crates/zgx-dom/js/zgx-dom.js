// zgx-dom's browser side: applies a frame's DOM operations, written by
// Rust into wasm memory, in one call, and delegates events from the root.
// The op codes match `src/ops.rs`.

const nodes = [];
const classNames = [];
const decoder = new TextDecoder();
let sheet = null;
let dispatch = null;
let flush = null;
let scheduled = false;
let root = null;
// Scroll reads wait for the next frame, apart from the writes.
const scrolled = new Set();
let scrollFrame = 0;

const TAGS = ["div", "span", "button", "input", "textarea", "progress", "label", "img", "canvas"];

export function zgx_init(baseCss, onDispatch, onFlush) {
  root = document;
  dispatch = onDispatch;
  flush = onFlush;
  const style = document.createElement("style");
  style.textContent = baseCss;
  document.head.appendChild(style);
  sheet = style.sheet;
  document.addEventListener("click", onClick);
  document.addEventListener("input", onInput);
  document.addEventListener("change", onChange);
  document.addEventListener("scroll", onScroll, true);
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
