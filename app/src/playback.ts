export interface PlaybackInfo {
  token: string;
  sampleRate: number;
  channels: number;
  totalFrames: number;
  startUs: number;
  endUs: number;
  trackKind: string;
  alignment: string;
  missingRanges?: PlaybackMissingRange[];
}

export interface PlaybackMissingRange {
  trackKind: "mic" | "system" | "imported";
  startUs: number;
  endUs: number;
  startFrame: number;
  endFrame: number;
  reasonCode: string;
}

export function validateMissingRanges(info: PlaybackInfo): PlaybackMissingRange[] {
  if (info.missingRanges === undefined) return [];
  if (!Array.isArray(info.missingRanges) || info.missingRanges.length > 2048
    || info.startUs !== 0 || !Number.isSafeInteger(info.endUs) || info.endUs <= 0
    || info.sampleRate !== 48000 || info.channels !== 1 || info.alignment !== "unverified"
    || !["mic", "system", "both", "imported"].includes(info.trackKind)
    || !Number.isSafeInteger(info.totalFrames)
    || BigInt(info.totalFrames) !== (BigInt(info.endUs) * 48000n + 999999n) / 1000000n) {
    throw Error("会議音声の欠損情報が不正です。");
  }
  let previous: PlaybackMissingRange | null = null;
  const counts = new Map<string,number>();
  for (const gap of info.missingRanges) {
    if (!gap || typeof gap !== "object" || Object.keys(gap).sort().join(",") !== "endFrame,endUs,reasonCode,startFrame,startUs,trackKind"
      || !["mic", "system", "imported"].includes(gap.trackKind)
      || (info.trackKind === "both" ? gap.trackKind === "imported" : gap.trackKind !== info.trackKind)
      || !Number.isSafeInteger(gap.startUs) || !Number.isSafeInteger(gap.endUs)
      || gap.startUs < 0 || gap.endUs <= gap.startUs || gap.endUs > info.endUs
      || !Number.isSafeInteger(gap.startFrame) || !Number.isSafeInteger(gap.endFrame)
      || gap.startFrame < 0 || gap.endFrame <= gap.startFrame || gap.endFrame > info.totalFrames
      || BigInt(gap.startFrame) !== BigInt(gap.startUs) * 48000n / 1000000n
      || BigInt(gap.endFrame) !== (BigInt(gap.endUs) * 48000n + 999999n) / 1000000n
      || typeof gap.reasonCode !== "string" || !gap.reasonCode || new TextEncoder().encode(gap.reasonCode).length > 256 || /\p{Cc}/u.test(gap.reasonCode)
      || previous && (gap.startFrame < previous.startFrame || gap.startFrame === previous.startFrame && gap.endFrame < previous.endFrame)) {
      throw Error("会議音声の欠損範囲と保存時間軸が一致しません。");
    }
    const count=(counts.get(gap.trackKind) ?? 0)+1;
    if(count>1024)throw Error("会議音声の欠損記録が多すぎます。");
    counts.set(gap.trackKind,count);
    previous = gap;
  }
  return info.missingRanges;
}

export function missingAtFrame(info: PlaybackInfo, frame: number): PlaybackMissingRange[] {
  return (info.missingRanges ?? []).filter(gap => gap.startFrame <= frame && frame < gap.endFrame);
}

export function playbackPacketFrames(info: PlaybackInfo, frame: number): number {
  if (missingAtFrame(info, frame).length) return 0;
  let frames = Math.min(65536, info.totalFrames - frame);
  for (const gap of info.missingRanges ?? []) if (gap.startFrame > frame) frames = Math.min(frames, gap.startFrame - frame);
  return frames;
}

export function nextSavedFrame(info: PlaybackInfo, frame: number): number | null {
  let next = frame;
  for (const gap of info.missingRanges ?? []) if (gap.startFrame <= next && next < gap.endFrame) next = gap.endFrame;
  return next < info.totalFrames ? next : null;
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
    const missingRanges = validateMissingRanges(info);
    this.info = { ...info, missingRanges:missingRanges.length ? missingRanges.map(gap=>({...gap})) : undefined };
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
    if (missingAtFrame(this.info, this.position).length) { this.error=null; this.changed(); return; }
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
        const frames = playbackPacketFrames(info, from);
        // Stop only after preceding scheduled PCM has finished. Prefetch must
        // not cut off known audio early or silently jump over missing time.
        if (frames === 0) {
          if (this.scheduled.length === 0) { this.position=from; this.playing=false; this.changed(); }
          break;
        }
        const bytes = await this.read(info.token, from);
        if (generation !== this.generation) return;
        const packet = decodeAudioPacket(bytes);
        if (packet.sampleRate !== info.sampleRate || packet.channels.length !== info.channels || packet.frames !== frames) {
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
