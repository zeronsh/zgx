import init from "./out/luau_bench.js";
const wasm = await init();
window.wasmMemory = wasm.memory;
window.ready = true;
