export interface PlaybackInfo {
  token: string;
  sampleRate: number;
  channels: number;
  totalFrames: number;
  startUs: number;
  endUs: number;
  trackKind: string;
  alignment: string;
}

export interface AudioPacket {
  sampleRate: number;
  channels: Float32Array[];
  frames: number;
}

export function playbackStartFrame(info: PlaybackInfo, absoluteUs: number): number {
  if (!Number.isSafeInteger(absoluteUs) || absoluteUs < info.startUs || absoluteUs >= info.endUs || info.totalFrames < 1) throw Error("推定区間の再生位置が発言の範囲外です。");
  return Math.min(info.totalFrames-1,Math.floor((absoluteUs-info.startUs)*info.sampleRate/1_000_000));
}

export function decodeAudioPacket(bytes: ArrayBuffer | number[]): AudioPacket {
  const buffer = bytes instanceof ArrayBuffer ? bytes : Uint8Array.from(bytes).buffer;
  if (buffer.byteLength < 12) throw new Error("音声パケットが短すぎます");
  const view = new DataView(buffer);
  const sampleRate = view.getUint32(0, true);
  const channels = view.getUint32(4, true);
  const frames = view.getUint32(8, true);
  if (sampleRate < 8000 || sampleRate > 384000 || channels < 1 || channels > 8 || frames < 1 || frames > 65536 || buffer.byteLength !== 12 + frames * channels * 4) {
    throw new Error("音声パケットの形式が不正です");
  }
  const samples = Array.from({ length: channels }, () => new Float32Array(frames));
  for (let frame = 0; frame < frames; frame++) {
    for (let channel = 0; channel < channels; channel++) {
      const sample = view.getFloat32(12 + (frame * channels + channel) * 4, true);
      if (!Number.isFinite(sample) || Math.abs(sample) > 1) throw new Error("音声サンプルが不正です");
      samples[channel][frame] = sample;
    }
  }
  return { sampleRate, channels: samples, frames };
}

export interface PlaybackClock {
  now(): number;
  resume(): Promise<void>;
  schedule(packet: AudioPacket, at: number, rate: number, ended: () => void): () => void;
}

interface ScheduledAudio {
  from: number;
  frames: number;
  at: number;
  until: number;
  stop: () => void;
}

// Buffer at most three small packets. Scheduling uses the audio clock, rather
// than JS timers, so adjoining packets can play without timer-induced gaps.
export class ChunkPlayer {
  private info: PlaybackInfo | null = null;
  private position = 0;
  private nextFrame = 0;
  private nextTime = 0;
  private rate = 1;
  private playing = false;
  private readingGeneration: number | null = null;
  private generation = 0;
  private scheduled: ScheduledAudio[] = [];
  private error: string | null = null;
  private clock: PlaybackClock;
  private read: (token: string, offset: number) => Promise<ArrayBuffer | number[]>;
  private changed: () => void;

  constructor(clock: PlaybackClock, read: (token: string, offset: number) => Promise<ArrayBuffer | number[]>, changed: () => void) {
    this.clock = clock;
    this.read = read;
    this.changed = changed;
  }

  snapshot() {
    const now = this.clock.now();
    const active = this.scheduled.find((part) => now >= part.at && now < part.until);
    const elapsed = this.scheduled.filter((part) => now >= part.until).at(-1);
    const frame = active && this.info ? Math.min(active.from + active.frames, active.from + Math.floor((now - active.at) * this.info.sampleRate * this.rate)) : elapsed ? elapsed.from + elapsed.frames : this.position;
    return { info: this.info, frame, playing: this.playing, buffering: this.playing && this.scheduled.length === 0, rate: this.rate, error: this.error };
  }

  load(info: PlaybackInfo) {
    this.clear();
    if (!Number.isSafeInteger(info.totalFrames) || info.totalFrames <= 0 || !Number.isSafeInteger(info.sampleRate) || info.sampleRate < 8000 || info.sampleRate > 384000 || !Number.isInteger(info.channels) || info.channels < 1 || info.channels > 8) {
      throw new Error("再生情報が不正です");
    }
    this.info = info;
    this.changed();
  }

  clear() {
    this.pause();
    this.info = null;
    this.position = 0;
    this.error = null;
    this.changed();
  }

  pause() {
    this.position = this.snapshot().frame;
    this.playing = false;
    this.generation++;
    const scheduled = this.scheduled;
    this.scheduled = [];
    for (const part of scheduled) part.stop();
    this.changed();
  }

  seek(frame: number) {
    if (!this.info || !Number.isFinite(frame)) return;
    const resume = this.playing;
    this.pause();
    this.position = Math.max(0, Math.min(this.info.totalFrames, Math.round(frame)));
    this.changed();
    if (resume && this.position < this.info.totalFrames) void this.play();
  }

  setRate(rate: number) {
    if (![0.75, 1, 1.25, 1.5, 2].includes(rate)) return;
    const resume = this.playing;
    this.pause();
    this.rate = rate;
    this.changed();
    if (resume) void this.play();
  }

  async play(): Promise<void> {
    if (!this.info || this.playing) return;
    if (this.position >= this.info.totalFrames) this.position = 0;
    this.playing = true;
    this.error = null;
    this.nextFrame = this.position;
    this.nextTime = 0;
    const generation = ++this.generation;
    this.changed();
    try {
      await this.clock.resume();
      if (generation === this.generation) await this.fill(generation);
    } catch (cause) {
      this.fail(generation, cause);
    }
  }

  private fail(generation: number, cause: unknown) {
    if (generation !== this.generation) return;
    this.pause();
    this.error = String(cause);
    this.changed();
  }

  private async fill(generation: number) {
    if (this.readingGeneration === generation) return;
    this.readingGeneration = generation;
    try {
      while (generation === this.generation && this.playing && this.info && this.scheduled.length < 3 && this.nextFrame < this.info.totalFrames) {
        const info = this.info;
        const from = this.nextFrame;
        const bytes = await this.read(info.token, from);
        if (generation !== this.generation) return;
        const packet = decodeAudioPacket(bytes);
        if (packet.sampleRate !== info.sampleRate || packet.channels.length !== info.channels || packet.frames !== Math.min(65536, info.totalFrames - from)) {
          throw new Error("再生情報と音声パケットが一致しません");
        }
        const at = Math.max(this.nextTime, this.clock.now() + 0.04);
        const until = at + packet.frames / packet.sampleRate / this.rate;
        const part: ScheduledAudio = { from, frames: packet.frames, at, until, stop: () => {} };
        part.stop = this.clock.schedule(packet, at, this.rate, () => {
          if (generation !== this.generation) return;
          this.scheduled = this.scheduled.filter((item) => item !== part);
          this.position = part.from + part.frames;
          if (this.nextFrame >= info.totalFrames && this.scheduled.length === 0) this.playing = false;
          this.changed();
          void this.fill(generation);
        });
        this.scheduled.push(part);
        this.nextFrame += packet.frames;
        this.nextTime = until;
        this.changed();
      }
    } catch (cause) {
      this.fail(generation, cause);
    } finally {
      if (this.readingGeneration === generation) this.readingGeneration = null;
    }
  }
}
