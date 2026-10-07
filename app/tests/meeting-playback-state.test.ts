import test from "node:test";
import assert from "node:assert/strict";
import { meetingPlaybackModes, meetingPlaybackLabel, validateMeetingPlayback } from "../src/meeting-playback-state.ts";
import type { PlaybackInfo } from "../src/playback.ts";

const info: PlaybackInfo = { token:"meeting", sampleRate:48000, channels:1, totalFrames:172800000, startUs:0, endUs:3600000000, trackKind:"both", alignment:"unverified" };

test("meeting source modes are explicit and imported channels do not imply mic/system", () => {
  assert.deepEqual(meetingPlaybackModes("recording"), ["mic", "system", "both"]);
  assert.deepEqual(meetingPlaybackModes("audio_import"), ["imported"]);
  assert.deepEqual(meetingPlaybackModes(null), []);
  assert.deepEqual(meetingPlaybackModes(), []);
  assert.equal(meetingPlaybackLabel("both"), "マイク＋PC音声");
});

test("meeting playback uses complete native timeline and accepts end-position paused restore", () => {
  for (const rate of [0.75, 1, 1.25, 1.5, 2]) validateMeetingPlayback(info, "both", { frame:info.totalFrames,rate,playing:false,endUs:info.endUs });
  validateMeetingPlayback({ ...info,endUs:100001,totalFrames:4801 }, "both", { frame:0,rate:1,playing:true });
});

test("wrong meeting output contract, unsafe metadata and fabricated synchronization are rejected", () => {
  for (const patch of [{ token:"" }, { startUs:1 }, { endUs:0 }, { endUs:Number.MAX_SAFE_INTEGER + 1 }, { endUs:0.5 }, { sampleRate:16000 }, { channels:2 }, { totalFrames:1 }, { totalFrames:NaN }, { trackKind:"mic" }, { alignment:"verified" }]) {
    assert.throws(() => validateMeetingPlayback({ ...info,...patch }, "both", { frame:0,rate:1,playing:true }));
  }
});

test("mode switch never clamps invalid position or reuses a changed source duration", () => {
  for (const patch of [{ frame:-1 }, { frame:info.totalFrames + 1 }, { frame:0.5 }, { frame:NaN }, { rate:3 }, { endUs:info.endUs - 1 }]) {
    assert.throws(() => validateMeetingPlayback(info, "both", { frame:240000,rate:1.5,playing:false,endUs:info.endUs,...patch }));
  }
});
