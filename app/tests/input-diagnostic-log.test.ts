import test from "node:test";
import assert from "node:assert/strict";
import {InputDiagnosticLog} from "../src/input-diagnostic-log.ts";
test("read errors retain cause and owner, suppress repeats, and record recovery once",()=>{
  const sent:object[]=[];const log=new InputDiagnosticLog(async payload=>{sent.push(payload);});
  log.report("live-journal","a",null);
  for(let i=0;i<20;i++)log.report("live-journal","a",Error("Access denied (os error 5)"),{command:"get_recording_input_history_status"});
  log.report("live-journal","b",Error("Access denied (os error 5)"));
  log.report("live-journal","a",null);log.report("live-journal","a",null);
  assert.equal(sent.length,3);assert.match(JSON.stringify(sent[0]),/os error 5/);
  assert.deepEqual(sent[2],{stage:"live-journal",sessionId:"a",message:null});
  log.report("live-journal","a",Error("Access denied (os error 5)"));assert.equal(sent.length,4);
});
