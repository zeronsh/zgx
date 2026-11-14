const t0 = performance.now();
import init from "./out/luau.js";
const wasm = await init();
window.wasmMemory = wasm.memory;
window.loadMs = performance.now() - t0;
window.ready = true;
