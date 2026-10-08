import { test } from "node:test";
import assert from "node:assert/strict";
import { ChunkPlayer, decodeAudioPacket, missingAtFrame, nextSavedFrame, playbackPacketFrames, validateMissingRanges, type PlaybackMissingRange, type AudioPacket, type PlaybackClock, type PlaybackInfo } from "../src/playback.ts";

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

function gap(startUs:number,endUs:number,trackKind:PlaybackMissingRange["trackKind"]="mic"):PlaybackMissingRange {
  return {startUs,endUs,trackKind,startFrame:Math.floor(startUs*48000/1e6),endFrame:Math.ceil(endUs*48000/1e6),reasonCode:"capture_timestamp_gap"};
}
function missingInfo(ranges:PlaybackMissingRange[]):PlaybackInfo {
  return {...info,endUs:10_000_000,totalFrames:480000,missingRanges:ranges};
}
function missingPlayer(data:PlaybackInfo) {
  const clock=new FakeClock(),reads:number[]=[];
  const player=new ChunkPlayer(clock,async(_,offset)=>{reads.push(offset);const frames=playbackPacketFrames(data,offset);assert.ok(frames>0,"no IPC read in a missing interval");return packet(frames);},()=>{});
  player.load(data);return {clock,reads,player};
}

test("missing metadata preserves microsecond boundaries and conservatively excludes partial frames",()=>{
  const data=missingInfo([gap(100001,100002)]);
  validateMissingRanges(data);
  assert.deepEqual(missingAtFrame(data,4800),data.missingRanges);
  assert.equal(playbackPacketFrames(data,0),4800);
  assert.equal(playbackPacketFrames(data,4800),0);
  assert.equal(nextSavedFrame(data,4800),4801);
  assert.deepEqual(missingAtFrame(data,4801),[]);
});

test("missing metadata rejects forged bounds, shapes, foreign tracks, fabricated alignment and excess records",()=>{
  const data=missingInfo([gap(100000,200000)]),original=data.missingRanges![0];
  for(const patch of [{startUs:-1},{endUs:10000001},{endUs:original.startUs},{startUs:0.5},{startFrame:0},{endFrame:9601},{endFrame:NaN},{trackKind:"system"},{reasonCode:""},{reasonCode:"x\n"},{reasonCode:"語".repeat(86)},{foreign:true}]) {
    assert.throws(()=>validateMissingRanges({...data,missingRanges:[{...original,...patch} as PlaybackMissingRange]}));
  }
  for(const patch of [{sampleRate:44100},{channels:2},{startUs:1},{alignment:"verified"},{trackKind:"processed"},{totalFrames:479999},{missingRanges:null}])assert.throws(()=>validateMissingRanges({...data,...patch} as PlaybackInfo));
  assert.throws(()=>validateMissingRanges(missingInfo([gap(200000,300000),gap(100000,200000)])));
  assert.throws(()=>validateMissingRanges(missingInfo(Array.from({length:1025},()=>gap(100000,200000)))));
  assert.deepEqual(validateMissingRanges(info),[]);
});

test("both-track missing ranges advance through overlaps and adjacent boundaries without collapsing meeting time",()=>{
  const data={...missingInfo([gap(100000,200000),gap(150000,300000,"system"),gap(300000,400000)]),trackKind:"both"};
  validateMissingRanges(data);
  assert.equal(nextSavedFrame(data,8000),19200);
  assert.equal(missingAtFrame(data,8000).length,2);
  assert.equal(nextSavedFrame(data,500),500);
  const tail=missingInfo([gap(9_000_000,10_000_000)]);
  assert.equal(nextSavedFrame(tail,432000),null);
  assert.equal(nextSavedFrame(tail,480000),null);
});

test("leading missing audio never reads or schedules silence and explicit seek stays paused",async()=>{
  const {clock,reads,player}=missingPlayer(missingInfo([gap(0,200000)]));
  await player.play();assert.deepEqual(reads,[]);assert.equal(clock.parts.length,0);
  assert.equal(player.snapshot().playing,false);assert.equal(player.snapshot().error,null);
  player.seek(9600);assert.equal(player.snapshot().frame,9600);assert.equal(player.snapshot().playing,false);
  await player.play();assert.equal(reads[0],9600);assert.equal(clock.parts[0].packet.channels[0][0],0.25);
  player.clear();
});

test("prefetch shortens the packet but lets all preceding PCM finish before pausing at a gap",async()=>{
  const {clock,reads,player}=missingPlayer(missingInfo([gap(3_000_000,4_000_000)]));
  await player.play();assert.deepEqual(reads,[0,65536,131072]);
  assert.equal(clock.parts[2].packet.frames,144000-131072);
  assert.equal(player.snapshot().playing,true);
  clock.end(0);await tick();clock.end(1);await tick();
  assert.equal(player.snapshot().playing,true);assert.equal(player.snapshot().frame,131072);
  clock.end(2);await tick();assert.equal(player.snapshot().playing,false);assert.equal(player.snapshot().frame,144000);
  assert.deepEqual(reads,[0,65536,131072]);assert.equal(player.snapshot().error,null);
  await player.play();assert.equal(reads.length,3);
  player.seek(192000);assert.equal(player.snapshot().playing,false);
  player.setRate(2);await player.play();assert.equal(reads[3],192000);assert.equal(clock.parts[3].rate,2);
  assert.equal(clock.parts[4].at,clock.parts[3].at+65536/48000/2);player.clear();
});

test("a gap shorter than one packet pauses after known audio, and trailing missing audio never loops automatically",async()=>{
  const {clock,reads,player}=missingPlayer(missingInfo([gap(100000,10_000_000)]));
  await player.play();assert.deepEqual(reads,[0]);assert.equal(clock.parts[0].packet.frames,4800);assert.equal(player.snapshot().playing,true);
  clock.end(0);await tick();assert.equal(player.snapshot().frame,4800);assert.equal(player.snapshot().playing,false);
  assert.equal(nextSavedFrame(player.snapshot().info!,4800),null);
  await player.play();assert.deepEqual(reads,[0]);player.clear();
});

test("seek into missing audio invalidates an in-flight packet without scheduling stale sound",async()=>{
  const data=missingInfo([gap(100000,200000)]),clock=new FakeClock(),reads:number[]=[];
  let release!:(data:ArrayBuffer)=>void;
  const pending=new Promise<ArrayBuffer>(resolve=>{release=resolve;});
  const player=new ChunkPlayer(clock,async(_,offset)=>{reads.push(offset);return pending;},()=>{});
  player.load(data);const playing=player.play();await tick();player.seek(6000);release(packet(4800));await playing;await tick();
  assert.deepEqual(reads,[0]);assert.equal(clock.parts.length,0);assert.equal(player.snapshot().frame,6000);assert.equal(player.snapshot().playing,false);player.clear();
});

test("native packet crossing a known gap fails before any PCM is scheduled",async()=>{
  const clock=new FakeClock(),player=new ChunkPlayer(clock,async()=>packet(65536),()=>{});
  player.load(missingInfo([gap(100000,200000)]));await player.play();
  assert.equal(clock.parts.length,0);assert.match(player.snapshot().error!,/一致しません/);player.clear();
});

test("loaded missing metadata is an owned snapshot rather than a mutable IPC object",async()=>{
  const data=missingInfo([gap(0,200000)]),{reads,player}=missingPlayer(data);
  data.missingRanges![0].endFrame=1;data.missingRanges!.length=0;
  await player.play();assert.deepEqual(reads,[]);assert.equal(player.snapshot().info!.missingRanges![0].endFrame,9600);player.clear();
});
