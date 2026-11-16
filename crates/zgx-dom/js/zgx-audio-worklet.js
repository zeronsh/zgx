// zgx-dom's audio worklet: plays the blocks the main thread mixes, and is
// the clock that asks for them. It asks (`low`) when what it holds falls
// under its mark, once until the answer comes, and not at all once told to
// stop, so a silent page sends nothing. A gap (the main thread late) fades
// out over 5 ms and back in rather than clicking, and is counted.
class ZgxAudio extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const { ahead } = options.processorOptions;
    this.mark = Math.max(128, Math.round(ahead * sampleRate));
    this.queue = [];
    this.at = 0;
    this.held = 0;
    this.asked = false;
    this.going = false;
    this.gain = 0;
    this.step = 1 / (0.005 * sampleRate);
    this.underruns = 0;
    this.told = 0;
    this.drained = false;
    // Whether a block has come since the last start: a queue empty before
    // the first is waiting to begin, not running dry.
    this.primed = false;
    // The least it has held since it last asked: how close it came to
    // running dry (reported with each ask).
    this.lowest = Infinity;
    this.port.onmessage = (event) => {
      const message = event.data;
      if (message.block) {
        this.queue.push(message.block);
        this.held += message.block.length / 2;
        this.primed = true;
        this.asked = false;
        if (!message.going) this.going = false;
        this.ask();
      } else if (message.start) {
        if (!this.going && this.held === 0) this.primed = false;
        this.going = true;
        this.ask();
      }
    };
  }

  ask() {
    if (this.going && !this.asked && this.held < this.mark) {
      this.asked = true;
      this.port.postMessage({
        low: this.mark * 2 - this.held,
        held: this.held,
        lowest: this.lowest === Infinity ? this.held : this.lowest,
      });
      this.lowest = Infinity;
    }
  }

  process(_inputs, outputs) {
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
    if (dry) this.underruns += 1;
    if (this.underruns !== this.told) {
      this.told = this.underruns;
      this.port.postMessage({ underruns: this.underruns });
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
