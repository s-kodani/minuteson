import assert from "node:assert/strict";
import test from "node:test";
import { parseMeetingDeletionAvailability } from "../src/meeting-deletion-availability.ts";
import { registerPlaybackStop, stopMeetingPlayback } from "../src/playback-barrier.ts";

test("meeting-specific availability keeps unrelated jobs independent and validates complete results", () => {
  const rows = [{meetingId:"a",available:true,audioDeleted:false,blockers:[]},{meetingId:"b",available:false,audioDeleted:true,blockers:[{kind:"job",reason:"保存完了待ち",jobId:"running-job"}]}];
  assert.deepEqual(parseMeetingDeletionAvailability(rows,["a","b"]),rows);
  for (const invalid of [null, rows.slice(0,1),[rows[0],rows[0]],[rows[0],{...rows[1],available:true}],[rows[0],{...rows[1],blockers:[{kind:"job",reason:"",jobId:null}]}]]) {
    assert.throws(() => parseMeetingDeletionAvailability(invalid,["a","b"]));
  }
  assert.doesNotThrow(() => parseMeetingDeletionAvailability([{ ...rows[0], audioDeleted: null }], ["a"]));
  for (const audioDeleted of [undefined, "false", 0, {}]) assert.throws(() => parseMeetingDeletionAvailability([{ ...rows[0], audioDeleted }], ["a"]));
});
test("meeting playback barrier drains only the target, retains failures for retry", async () => {
  const calls: string[]=[];
  let refuse=true;
  const a=registerPlaybackStop(async()=>{calls.push("a"); if(refuse)throw Error("target close failed");},"a");
  const b=registerPlaybackStop(async()=>{calls.push("b");},"b");
  const unrelated=registerPlaybackStop(async()=>{calls.push("capture-test");});
  try {
    await assert.rejects(stopMeetingPlayback("a"),/target close failed/);
    assert.deepEqual(calls,["a"]);
    refuse=false; await stopMeetingPlayback("a");
    assert.deepEqual(calls,["a","a"]);
  } finally {a();b();unrelated();}
});
