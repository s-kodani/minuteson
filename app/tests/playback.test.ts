import { test } from "node:test";
import assert from "node:assert/strict";
import { ChunkPlayer, decodeAudioPacket, playbackStartFrame, playbackPacketFrames, validateMissingRanges, type AudioPacket, type PlaybackClock, type PlaybackInfo } from "../src/playback.ts";

function packet(frames: number, channels = 1, rate = 48000) {
  const buffer = new ArrayBuffer(12 + frames * channels * 4);
  const view = new DataView(buffer);
  view.setUint32(0, rate, true);
  view.setUint32(4, channels, true);
  view.setUint32(8, frames, true);
  for (let frame = 0; frame < frames; frame++) {
    for (let channel = 0; channel < channels; channel++) view.setFloat32(12 + (frame * channels + channel) * 4, channel === 0 ? 0.25 : -0.5, true);
  }
  return buffer;
}

const info: PlaybackInfo = { token: "token", sampleRate: 48000, channels: 1, totalFrames: 65536 * 5, startUs: 0, endUs: 10000000, trackKind: "mic", alignment: "unverified" };

class FakeClock implements PlaybackClock {
  time = 0;
  parts: { at: number; rate: number; packet: AudioPacket; ended: () => void; stopped: boolean }[] = [];
  now() { return this.time; }
  async resume() {}
  schedule(packet: AudioPacket, at: number, rate: number, ended: () => void) {
    const part = { packet, at, rate, ended, stopped: false };
    this.parts.push(part);
    return () => { part.stopped = true; };
  }
  end(index: number) {
    const part = this.parts[index];
    this.time = part.at + part.packet.frames / part.packet.sampleRate / part.rate;
    part.ended();
  }
}

async function tick() { for (let i = 0; i < 8; i++) await Promise.resolve(); }

test("starting at a reference continues beyond its end until manual pause or recording EOF", async () => {
  const clock = new FakeClock(), reads:number[] = [];
  const full = {...info,totalFrames:240000,endUs:5000000};
  const player = new ChunkPlayer(clock, async (_,offset) => { reads.push(offset);return packet(Math.min(65536,full.totalFrames-offset)); }, () => {});
  player.load(full);player.setRate(1.5);player.seek(playbackStartFrame(full,1000000));
  await player.play();
  assert.equal(reads[0],48000);
  // The clicked reference was 1s–1.25s. Its end is not the playback boundary.
  clock.time=clock.parts[0].at+0.5;
  assert.ok(player.snapshot().frame>60000);
  assert.equal(player.snapshot().playing,true);
  player.pause();const paused=player.snapshot().frame;
  assert.equal(player.snapshot().playing,false);await player.play();
  assert.equal(reads[3],paused);
  for(let i=3;i<clock.parts.length;i++) { clock.end(i);await tick(); }
  assert.equal(player.snapshot().frame,full.totalFrames);
  assert.equal(player.snapshot().playing,false);
  assert.equal(player.snapshot().rate,1.5);
});

test("binary PCM is deinterleaved and malformed packets are rejected", () => {
  const decoded = decodeAudioPacket(packet(3, 2));
  assert.deepEqual([...decoded.channels[0]], [0.25, 0.25, 0.25]);
  assert.deepEqual([...decoded.channels[1]], [-0.5, -0.5, -0.5]);
  assert.throws(() => decodeAudioPacket(new ArrayBuffer(4)), /短すぎ/);
  assert.throws(() => decodeAudioPacket(packet(0)), /形式/);
  const corrupt = packet(1);
  new DataView(corrupt).setFloat32(12, NaN, true);
  assert.throws(() => decodeAudioPacket(corrupt), /サンプル/);
});

test("long utterances schedule only three packets and refill without audio-clock gaps", async () => {
  const clock = new FakeClock();
  const reads: number[] = [];
  const player = new ChunkPlayer(clock, async (_, offset) => { reads.push(offset); return packet(Math.min(65536, info.totalFrames - offset)); }, () => {});
  player.load(info);
  await player.play();
  assert.deepEqual(reads, [0, 65536, 131072]);
  assert.equal(clock.parts[1].at, clock.parts[0].at + 65536 / 48000);
  clock.end(0);
  await tick();
  assert.deepEqual(reads, [0, 65536, 131072, 196608]);
  assert.equal(clock.parts[3].at, clock.parts[2].at + 65536 / 48000);
  clock.end(1);
  await tick();
  clock.end(2);
  clock.end(3);
  clock.end(4);
  assert.equal(player.snapshot().playing, false);
  assert.equal(player.snapshot().frame, info.totalFrames);
});

test("pause and rate changes preserve position and stop previously scheduled sources", async () => {
  const clock = new FakeClock();
  const reads: number[] = [];
  const player = new ChunkPlayer(clock, async (_, offset) => { reads.push(offset); return packet(Math.min(65536, info.totalFrames - offset)); }, () => {});
  player.load(info);
  await player.play();
  clock.time = clock.parts[0].at + 0.5;
  player.pause();
  assert.equal(player.snapshot().frame, 24000);
  assert.ok(clock.parts.every((part) => part.stopped));
  player.setRate(2);
  await player.play();
  assert.equal(reads[3], 24000);
  assert.equal(clock.parts[3].rate, 2);
  assert.equal(clock.parts[4].at, clock.parts[3].at + 65536 / 48000 / 2);
  clock.time = clock.parts[3].at + 0.25;
  player.setRate(1);
  await tick();
  assert.equal(reads[6], 48000);
  assert.ok(clock.parts.slice(3, 6).every((part) => part.stopped));
  player.clear();
  assert.equal(player.snapshot().info, null);
});

test("seeking while a packet loads never schedules stale audio", async () => {
  let release!: (bytes: ArrayBuffer) => void;
  const first = new Promise<ArrayBuffer>((resolve) => { release = resolve; });
  const clock = new FakeClock();
  const reads: number[] = [];
  const player = new ChunkPlayer(clock, async (_, offset) => {
    reads.push(offset);
    return reads.length === 1 ? first : packet(Math.min(65536, info.totalFrames - offset));
  }, () => {});
  player.load(info);
  const playing = player.play();
  await tick();
  player.seek(100000);
  await tick();
  release(packet(65536));
  await playing;
  await tick();
  assert.deepEqual(reads, [0, 100000, 165536, 231072]);
  assert.equal(clock.parts.length, 3);
  assert.equal(player.snapshot().frame, 100000);
  player.clear();
});

test("audio errors stop playback and are retained for user-visible retry", async () => {
  const clock = new FakeClock();
  let fail = true;
  const player = new ChunkPlayer(clock, async (_, offset) => {
    if (fail) throw new Error("changed audio hash");
    return packet(Math.min(65536, info.totalFrames - offset));
  }, () => {});
  player.load(info);
  await player.play();
  assert.equal(player.snapshot().playing, false);
  assert.match(player.snapshot().error!, /changed audio hash/);
  fail = false;
  await player.play();
  assert.equal(player.snapshot().playing, true);
  assert.equal(player.snapshot().error, null);
  player.clear();
});

test("ending a stale source after seek cannot move the new position", async () => {
  const clock = new FakeClock();
  const player = new ChunkPlayer(clock, async (_, offset) => packet(Math.min(65536, info.totalFrames - offset)), () => {});
  player.load(info);
  await player.play();
  player.seek(info.totalFrames - 12000);
  await tick();
  clock.parts[0].ended();
  assert.equal(player.snapshot().frame, info.totalFrames - 12000);
  assert.equal(clock.parts.at(-1)?.packet.frames, 12000);
  player.clear();
});

test("seeking to the end stops instead of unexpectedly restarting", async () => {
  const clock = new FakeClock();
  const player = new ChunkPlayer(clock, async (_, offset) => packet(Math.min(65536, info.totalFrames - offset)), () => {});
  player.load(info);
  await player.play();
  player.seek(info.totalFrames);
  await tick();
  assert.equal(player.snapshot().playing, false);
  assert.equal(player.snapshot().frame, info.totalFrames);
  assert.equal(clock.parts.length, 3);
  player.seek(NaN);
  assert.equal(player.snapshot().frame, info.totalFrames);
});

test("canonical playback rejects missing-range metadata rather than skipping meeting time",()=>{
  const clock=new FakeClock();
  const player=new ChunkPlayer(clock,async()=>packet(1),()=>{});
  assert.throws(()=>player.load({...info,missingRanges:[{trackKind:"mic",startUs:100000,endUs:200000,startFrame:4800,endFrame:9600,reasonCode:"capture_timestamp_gap"}]}),/欠損補完済み/);
  assert.deepEqual(validateMissingRanges({...info,missingRanges:[]}),[]);
});

test("silent canonical PCM keeps every frame and never advances the cursor early",async()=>{
  const clock=new FakeClock(),reads:number[]=[];
  const data={...info,totalFrames:48000,endUs:1000000,trackKind:"mixed"};
  const player=new ChunkPlayer(clock,async(_,frame)=>{reads.push(frame);const p=packet(playbackPacketFrames(data,frame));new Uint8Array(p,12).fill(0);return p;},()=>{});
  player.load(data);await player.play();assert.deepEqual(reads,[0]);assert.equal(player.snapshot().frame,0);
  clock.end(0);await tick();assert.equal(player.snapshot().frame,48000);assert.equal(player.snapshot().playing,false);
});
