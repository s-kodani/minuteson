import test from "node:test";
import assert from "node:assert/strict";
import { checkedVadOptions, defaultVadOptions, sameVadOptions } from "../src/vad-state.ts";
test("VAD accepts all boundary values and keeps options when disabled", () => {
  assert.equal(checkedVadOptions(defaultVadOptions), defaultVadOptions);
  for (const patch of [{ enabled: false }, {thresholdMilli:10}, {thresholdMilli:990}, {minSpeechDurationMs:1}, {minSpeechDurationMs:5000}, {minSilenceDurationMs:1}, {minSilenceDurationMs:5000}, {maxSpeechDurationS:1}, {maxSpeechDurationS:120}, {speechPadMs:0}, {speechPadMs:1000}, {samplesOverlapMs:0}, {samplesOverlapMs:1000}]) {
    assert.doesNotThrow(() => checkedVadOptions({...defaultVadOptions,...patch}));
  }
  assert.ok(!sameVadOptions(defaultVadOptions, {...defaultVadOptions,enabled:false}));
});
test("VAD rejects blank, unknown, nonfinite, fractional, off-step and contradictory values", () => {
  for (const patch of [{enabled:1}, {extra:1}, {thresholdMilli:0}, {thresholdMilli:1000}, {thresholdMilli:601}, {minSpeechDurationMs:0}, {minSpeechDurationMs:5001}, {minSilenceDurationMs:0}, {minSilenceDurationMs:5001}, {maxSpeechDurationS:0}, {maxSpeechDurationS:121}, {speechPadMs:-1}, {speechPadMs:1001}, {samplesOverlapMs:-1}, {samplesOverlapMs:1001}, {samplesOverlapMs:101}, {minSpeechDurationMs:2000,maxSpeechDurationS:1}, {speechPadMs:NaN}, {speechPadMs:Infinity}, {speechPadMs:0.5}, {speechPadMs:""}]) {
    assert.throws(() => checkedVadOptions({...defaultVadOptions,...patch}));
  }
});
