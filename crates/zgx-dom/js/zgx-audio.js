// zgx-dom's sound (the `audio` feature): an AudioContext and the worklet
// that pulls zgx-audio's blocks (js/zgx-audio-worklet.js), made at the
// first sound, never before. Bound only with `audio`.

let fill = null;
let setRate = null;
let source = "";
let url = null;
let ahead = 0.12;
let ctx = null;
let node = null;
let starting = null;
let wanted = false;
const state = { asked: 0, posted: 0, underruns: 0, depths: [] };

export function zgx_audio_install(fillFn, rateFn, workletSource, workletUrl, aheadSeconds) {
  // One engine and one context for the page: installed once.
  if (fill) return;
  fill = fillFn;
  setRate = rateFn;
  source = workletSource;
  url = workletUrl ?? null;
  ahead = aheadSeconds;
  // Browsers start a context only after the page has been used: the first
  // press resumes one that's wanted. Once, in the capture phase.
  const resume = () => {
    document.removeEventListener("pointerdown", resume, true);
    document.removeEventListener("keydown", resume, true);
    if (ctx && wanted && ctx.state === "suspended") ctx.resume();
  };
  document.addEventListener("pointerdown", resume, true);
  document.addEventListener("keydown", resume, true);
}

async function start() {
  if (!ctx) {
    ctx = new AudioContext();
    setRate(ctx.sampleRate);
    let module = url;
    let made = null;
    if (!module) {
      made = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
      module = made;
    }
    await ctx.audioWorklet.addModule(module);
    if (made) URL.revokeObjectURL(made);
    node = new AudioWorkletNode(ctx, "zgx-audio", {
      numberOfInputs: 0,
      outputChannelCount: [2],
      processorOptions: { ahead },
    });
    node.port.onmessage = (event) => {
      const message = event.data;
      if (message.low) {
        state.asked += 1;
        // What the worklet held as it asked, and the least since its last
        // ask (frames): kept, the last 64, for a test to read. Bounded and
        // cheap (two numbers an ask, about ten a second while sound plays).
        state.depths.push([message.held, message.lowest]);
        if (state.depths.length > 64) state.depths.shift();
        const answer = fill(message.low);
        if (!answer) return;
        const [block, going] = answer;
        state.posted += 1;
        if (!going) wanted = false;
        node.port.postMessage({ block, going }, [block.buffer]);
      } else if (message.underruns !== undefined) {
        state.underruns = message.underruns;
      } else if (message.drained) {
        drained();
      }
    };
    node.connect(ctx.destination);
  }
  // On the same chain as the suspends, so one still pending can't land
  // after this: resumed whenever sound is wanted (a running context's
  // resume does nothing).
  if (wanted) await ctx.resume().catch(() => {});
  node.port.postMessage({ start: true });
}

// The worklet has played out what it held: suspended, unless a sound has
// woken it meanwhile. On the chain the starts run on, in order.
function drained() {
  starting = (starting ?? Promise.resolve())
    .then(() => {
      if (!wanted && ctx) return ctx.suspend();
    })
    .catch(() => {});
}

// For a test: the drained message as if the hold had passed, and a sound
// waking in the same task.
export function zgx_audio_drain_and_wake(slowSuspendMs) {
  // The suspend made slow, as a browser's may be: a start that doesn't wait
  // for it would see the context running and skip its resume.
  if (ctx && slowSuspendMs > 0) {
    const suspend = ctx.suspend;
    ctx.suspend = () => {
      ctx.suspend = suspend;
      return new Promise((resolve) => setTimeout(resolve, slowSuspendMs)).then(() =>
        suspend.call(ctx),
      );
    };
  }
  wanted = false;
  drained();
  zgx_audio_wake();
}

// The engine has a sound to play: start asking for blocks.
export function zgx_audio_wake() {
  wanted = true;
  starting = (starting ?? Promise.resolve()).then(start).catch((error) => {
    console.error("zgx-audio:", error);
  });
}

export function zgx_audio_state() {
  return {
    context: ctx ? ctx.state : "none",
    running: wanted,
    asked: state.asked,
    posted: state.posted,
    underruns: state.underruns,
    sampleRate: ctx ? ctx.sampleRate : 0,
    depths: state.depths.slice(),
  };
}
