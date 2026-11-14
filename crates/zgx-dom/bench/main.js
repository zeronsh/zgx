import init from "./out/bench.js";
const wasm = await init();
window.wasmMemory = wasm.memory;
window.ready = true;
