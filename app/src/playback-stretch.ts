import type { AudioPacket } from "./playback.ts";

// WSOLA uses a shared alignment for every channel, so stereo phase is kept.
// Each grain supplies one hop of output and one hop of overlap for the next.
export class PlaybackStretch {
  private readonly rate: number;
  private readonly hop: number;
  private readonly radius: number;
  private readonly stride: number;
  private readonly overlap: Float32Array[];
  private readonly outputHop: Float32Array[];
  private history: Float32Array[] = [];
  private inputFrames = 0;
  private outputFrames = 0;
  private consumedInHop: number;

  constructor(rate: number, sampleRate: number, channels: number) {
    this.rate = rate;
    this.hop = Math.round(sampleRate * 0.02);
    this.radius = Math.round(sampleRate * 0.012);
    this.stride = Math.max(1, Math.floor(sampleRate / 4000));
    this.overlap = Array.from({ length: channels }, () => new Float32Array(this.hop));
    this.outputHop = Array.from({ length: channels }, () => new Float32Array(this.hop));
    this.consumedInHop = this.hop;
  }

  process(packet: AudioPacket, next: AudioPacket | null): AudioPacket {
    const historyFrames = this.history[0]?.length ?? 0;
    const lookAhead = Math.min(next?.frames ?? 0, 2 * this.hop + this.radius + 2);
    const inputStart = this.inputFrames - historyFrames;
    const available = historyFrames + packet.frames + lookAhead;
    const input = packet.channels.map((samples, channel) => {
      // The final grain can extend past EOF. Zero padding keeps its overlap valid.
      const window = new Float32Array(available + this.hop);
      if (historyFrames) window.set(this.history[channel]);
      window.set(samples, historyFrames);
      if (next) window.set(next.channels[channel].subarray(0, lookAhead), historyFrames + packet.frames);
      return window;
    });

    this.inputFrames += packet.frames;
    // Round the cumulative length so fractional frames never accumulate drift.
    const frames = Math.round(this.inputFrames / this.rate) - this.outputFrames;
    const channels = packet.channels.map(() => new Float32Array(frames));
    for (let written = 0; written < frames;) {
      if (this.consumedInHop === this.hop) {
        const outputPosition = this.outputFrames + written;
        const remaining = frames - written;
        const hopFrames = next ? this.hop : Math.min(this.hop, remaining);
        const lastStart = Math.max(0, available - hopFrames);
        const nominal = Math.max(0, Math.min(lastStart, Math.round(outputPosition * this.rate) - inputStart));
        const start = outputPosition === 0 ? 0 : this.alignedStart(input, nominal, lastStart);
        for (let channel = 0; channel < channels.length; channel++) {
          for (let i = 0; i < this.hop; i++) {
            const weight = (i + 1) / (this.hop + 1);
            const sample = input[channel][start + i];
            this.outputHop[channel][i] = outputPosition === 0 ? sample : (1 - weight) * this.overlap[channel][i] + weight * sample;
            this.overlap[channel][i] = input[channel][start + this.hop + i] ?? 0;
          }
        }
        this.consumedInHop = 0;
      }
      const count = Math.min(frames - written, this.hop - this.consumedInHop);
      for (let channel = 0; channel < channels.length; channel++) {
        channels[channel].set(this.outputHop[channel].subarray(this.consumedInHop, this.consumedInHop + count), written);
      }
      written += count;
      this.consumedInHop += count;
    }
    // The next call may start its search just before the PCM packet boundary.
    this.history = next ? packet.channels.map(samples => samples.slice(-(this.radius + 2))) : [];
    this.outputFrames += frames;
    return { sampleRate: packet.sampleRate, channels, frames };
  }

  private alignedStart(input: Float32Array[], nominal: number, lastStart: number) {
    // Correlate on the strongest channel in the overlap. Summing channels
    // could cancel a valid signal, and separate searches would alter phase.
    let bestChannel = 0;
    let referenceEnergy = 0;
    for (let channel = 0; channel < input.length; channel++) {
      let energy = 0;
      for (let i = 0; i < this.hop; i += this.stride) energy += this.overlap[channel][i] ** 2;
      if (energy > referenceEnergy) {
        bestChannel = channel;
        referenceEnergy = energy;
      }
    }
    if (referenceEnergy < 1e-12) return nominal;
    const reference = this.overlap[bestChannel];
    const samples = input[bestChannel];
    const score = (start: number) => {
      let dot = 0;
      let energy = 0;
      for (let i = 0; i < this.hop; i += this.stride) {
        const sample = samples[start + i];
        dot += reference[i] * sample;
        energy += sample * sample;
      }
      return energy < 1e-12 ? 0 : dot / Math.sqrt(referenceEnergy * energy);
    };
    const first = Math.max(0, nominal - this.radius);
    const last = Math.min(lastStart, nominal + this.radius);
    let best = nominal;
    let bestScore = score(nominal);
    const consider = (start: number) => {
      const candidate = score(start);
      if (candidate > bestScore + 1e-6 || (Math.abs(candidate - bestScore) <= 1e-6 && Math.abs(start - nominal) < Math.abs(best - nominal))) {
        best = start;
        bestScore = candidate;
      }
    };
    for (let start = first; start <= last; start += this.stride) consider(start);
    const coarse = best;
    for (let start = Math.max(first, coarse - this.stride + 1); start <= Math.min(last, coarse + this.stride - 1); start++) consider(start);
    return best;
  }
}
