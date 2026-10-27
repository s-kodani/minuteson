import test from "node:test";
import assert from "node:assert/strict";
import { meetingPlaybackModes, meetingPlaybackLabel, meetingPlaybackStart, preferredMeetingPlaybackMode, validateMeetingPlayback, type MeetingPlaybackStartReference } from "../src/meeting-playback-state.ts";
import { playbackStartFrame, type PlaybackInfo } from "../src/playback.ts";

const info: PlaybackInfo = { token:"meeting", sampleRate:16000, channels:1, totalFrames:57600000, startUs:0, endUs:3600000000, trackKind:"mixed", alignment:"unverified" };

test("all audio-bearing meetings expose only the canonical mixed source",()=>{
  assert.deepEqual(meetingPlaybackModes("recording"),["mixed"]);
  assert.deepEqual(meetingPlaybackModes("audio_import"),["mixed"]);
  assert.deepEqual(meetingPlaybackModes(null),[]);
  assert.equal(preferredMeetingPlaybackMode(["mixed"]),"mixed");
  assert.equal(preferredMeetingPlaybackMode([]),null);
  assert.equal(meetingPlaybackLabel("mixed"),"会議音声");
});

test("meeting playback uses complete native timeline and accepts end-position paused restore", () => {
  for (const rate of [0.75, 1, 1.25, 1.5, 2]) validateMeetingPlayback(info, "mixed", { frame:info.totalFrames,rate,playing:false,endUs:info.endUs });
  validateMeetingPlayback({ ...info,endUs:100001,totalFrames:1601 }, "mixed", { frame:0,rate:1,playing:true });
});

test("wrong meeting output contract, unsafe metadata and fabricated synchronization are rejected", () => {
  for (const patch of [{ token:"" }, { startUs:1 }, { endUs:0 }, { endUs:Number.MAX_SAFE_INTEGER + 1 }, { endUs:0.5 }, { sampleRate:48000 }, { channels:2 }, { totalFrames:1 }, { totalFrames:172800000 }, { totalFrames:NaN }, { trackKind:"mic" }, { alignment:"verified" }]) {
    assert.throws(() => validateMeetingPlayback({ ...info,...patch }, "mixed", { frame:0,rate:1,playing:true }));
  }
});

test("mode switch never clamps invalid position or reuses a changed source duration", () => {
  for (const patch of [{ frame:-1 }, { frame:info.totalFrames + 1 }, { frame:0.5 }, { frame:NaN }, { rate:3 }, { endUs:info.endUs - 1 }]) {
    assert.throws(() => validateMeetingPlayback(info, "mixed", { frame:240000,rate:1.5,playing:false,endUs:info.endUs,...patch }));
  }
});

test("canonical playback refuses a response that still asks the player to skip gaps",()=>{
  const gap={trackKind:"system" as const,startUs:100000,endUs:200000,startFrame:1600,endFrame:3200,reasonCode:"capture_timestamp_gap"};
  assert.throws(()=>validateMeetingPlayback({...info,missingRanges:[gap]},"mixed",{frame:6000,rate:1,playing:true}));
});

test("all playback entry points resolve an absolute start within the full recording", () => {
  const references: MeetingPlaybackStartReference[] = [
    { kind:"utterance", analysisId:"a", utteranceId:"u", expectedContentHash:"hash", startUs:123456, endUs:765432 },
    { kind:"draftUtterance", request:{ meetingId:"m", analysisId:"a", utteranceId:"child", expectedContentHash:"hash", expectedDraft:{id:"d",lockVersion:2}, expectedStartUs:123456, expectedEndUs:765432 } },
    { kind:"interval", startUs:123456, endUs:765432, expectedDurationUs:1000000 },
  ];
  for (const reference of references) {
    assert.equal(meetingPlaybackStart(reference, 1000000), 123456);
    assert.equal(playbackStartFrame(info, meetingPlaybackStart(reference, 1000000)), 1975);
    assert.throws(() => meetingPlaybackStart(reference, 500000));
    assert.throws(() => meetingPlaybackStart(reference, NaN));
  }
  assert.throws(() => meetingPlaybackStart({kind:"interval",startUs:0,endUs:1,expectedDurationUs:1000001},1000000));
  for (const range of [{startUs:-1,endUs:2}, {startUs:1,endUs:1}, {startUs:0.5,endUs:2}, {startUs:1,endUs:NaN}]) {
    assert.throws(() => meetingPlaybackStart({kind:"interval",...range,expectedDurationUs:1000000},1000000));
  }
});

test("reference starts near EOF and within the final sample stay on the 16 kHz timeline", () => {
  const short = {...info,endUs:1000000,totalFrames:16000};
  for (const [startUs, frame] of [[750000,12000], [999999,15999]]) {
    const start = meetingPlaybackStart({kind:"interval",startUs,endUs:1000000,expectedDurationUs:1000000},short.endUs);
    validateMeetingPlayback(short,"mixed",{frame:0,rate:1,playing:true,endUs:short.endUs});
    assert.equal(playbackStartFrame(short,start),frame);
  }
});
