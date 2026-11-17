// zgx-dom's audio worklet: plays the blocks the main thread mixes, and is
// the clock that asks for them. It asks (`low`) when what it holds falls
// under its mark, once until the answer comes, and not at all once told to
// stop, so a silent page sends nothing. A gap (the main thread late) fades
// out over 5 ms and back in rather than clicking, and is counted.
class ZgxAudio extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const { ahead } = options.processorOptions;
    // What it holds before it asks: `ahead` to start with, grown by half
    // each time it runs dry (once a gap), up to half a second, and back to
    // `ahead` with each new or resumed context. Only a device whose main
    // thread has answered late pays more latency, and only until then.
    this.base = Math.max(128, Math.round(ahead * sampleRate));
    this.cap = Math.round(0.5 * sampleRate);
    this.mark = this.base;
    this.grown = false;
    // For tests: hold the answer to the next ask back until this many
    // frames after the ask, as a main thread that late would.
    this.stall = 0;
    this.withheld = null;
    this.queue = [];
    this.at = 0;
    this.held = 0;
    this.asked = false;
    this.going = false;
    this.gain = 0;
    this.step = 1 / (0.005 * sampleRate);
    this.underruns = 0;
    this.told = 0;
    this.toldLate = 0;
    this.drained = false;
    // Whether a block has come since the last start: a queue empty before
    // the first is waiting to begin, not running dry.
    this.primed = false;
    // The least it has held since it last asked: how close it came to
    // running dry (reported with each ask).
    this.lowest = Infinity;
    // When it last asked (in frames) and what it held then; how long the
    // answer took; and how many answers came later than what it held then
    // lasted (`late`). It runs dry only after a late answer: a main thread
    // busy past the queue, not the queue itself.
    this.askedAt = 0;
    this.heldAtAsk = 0;
    this.lastWait = 0;
    this.late = 0;
    this.port.onmessage = (event) => {
      const message = event.data;
      if (message.block) {
        if (this.stall > 0 && this.asked) {
          this.withheld = message;
          return;
        }
        this.answered(message);
      } else if (message.start) {
        // Started again: it says when it has drained again, even if the
        // first answer says stop before any block plays (a sound already
        // over), so the context is suspended rather than left running.
        this.drained = false;
        if (message.fresh) this.mark = this.base;
        if (!this.going && this.held === 0) this.primed = false;
        this.going = true;
        this.ask();
      } else if (message.stall) {
        this.stall = message.stall;
      }
    };
  }

  // A block mixed for its ask.
  answered(message) {
    if (this.asked) {
      this.lastWait = currentFrame - this.askedAt;
      if (this.lastWait > this.heldAtAsk) this.late += 1;
    }
    this.queue.push(message.block);
    this.held += message.block.length / 2;
    this.primed = true;
    this.asked = false;
    this.grown = false;
    if (!message.going) this.going = false;
    this.ask();
  }

  ask() {
    if (this.going && !this.asked && this.held < this.mark) {
      this.asked = true;
      this.askedAt = currentFrame;
      this.heldAtAsk = this.held;
      this.port.postMessage({
        low: this.mark * 2 - this.held,
        held: this.held,
        lowest: this.lowest === Infinity ? this.held : this.lowest,
        wait: this.lastWait,
        mark: this.mark,
      });
      this.lowest = Infinity;
    }
  }

  process(_inputs, outputs) {
    // A withheld answer (a test's stall) let go once its time has come.
    if (this.withheld && currentFrame - this.askedAt >= this.stall) {
      const message = this.withheld;
      this.withheld = null;
      this.stall = 0;
      this.answered(message);
    }
    const out = outputs[0];
    const left = out[0];
    const right = out[1] || out[0];
    let dry = false;
    for (let i = 0; i < left.length; i++) {
      const block = this.queue[0];
      let l = 0;
      let r = 0;
      if (block) {
        l = block[this.at * 2];
        r = block[this.at * 2 + 1];
        this.at += 1;
        this.held -= 1;
        if (this.at * 2 >= block.length) {
          this.queue.shift();
          this.at = 0;
        }
        this.gain = Math.min(1, this.gain + this.step);
      } else {
        if (this.going && this.primed) dry = true;
        this.gain = Math.max(0, this.gain - this.step);
      }
      left[i] = l * this.gain;
      if (right !== left) right[i] = r * this.gain;
    }
    if (this.going && this.primed) this.lowest = Math.min(this.lowest, this.held);
    if (dry) {
      this.underruns += 1;
      // Ran dry: hold more before asking from now on, once a gap.
      if (!this.grown) {
        this.grown = true;
        this.mark = Math.min(this.cap, Math.round(this.mark * 1.5));
      }
    }
    if (this.underruns !== this.told || this.late !== this.toldLate) {
      this.told = this.underruns;
      this.toldLate = this.late;
      this.port.postMessage({ underruns: this.underruns, late: this.late });
    }
    this.ask();
    // Stopped and played out: the main thread suspends the context.
    if (!this.going && this.held === 0 && !this.drained) {
      this.drained = true;
      this.port.postMessage({ drained: true });
    }
    if (this.going) this.drained = false;
    return true;
  }
}

registerProcessor("zgx-audio", ZgxAudio);
