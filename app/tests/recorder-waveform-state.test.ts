import test from "node:test";
import assert from "node:assert/strict";
import {RecorderWaveformHistory,waveformBars} from "../src/recorder-waveform-state.ts";
import type {VisibleLevel} from "../src/input-level-state.ts";
const measured=(peak:number):VisibleLevel=>({state:"measured",rms:peak/2,peak,rmsDbFS:null,peakDbFS:null,clipping:peak>=1});

test("waveform retains actual separate samples and cannot animate repeated or older publications",()=>{
  const mic=new RecorderWaveformHistory(),pc=new RecorderWaveformHistory();
  assert.deepEqual(mic.read(1,measured(.25),true),[.25]);
  assert.deepEqual(pc.read(1,measured(.75),true),[.75]);
  assert.deepEqual(mic.read(1,measured(.25),true),[.25]);
  assert.deepEqual(mic.read(0,measured(.5),true),[.25]);
  assert.deepEqual(mic.read(2,measured(0),true),[.25,0]);
  assert.deepEqual(pc.read(2,measured(2),true),[.75,1]);
});

test("waveform drops old signals on stale, missing or disconnected input and waits for a new measurement",()=>{
  for(const state of ["stale","waiting","unavailable","disconnected"]){
    const history=new RecorderWaveformHistory();
    history.read(1,measured(.5),true);
    const level:VisibleLevel=state==="disconnected"?measured(.5):{state:state as "stale"|"waiting"|"unavailable"};
    assert.deepEqual(history.read(1,level,state!=="disconnected"),[]);
    assert.deepEqual(history.read(1,measured(.5),true),[]);
    assert.deepEqual(history.read(2,measured(.2),true),[.2]);
  }
});

test("waveform history stays bounded and a new owner starts empty",()=>{
  const history=new RecorderWaveformHistory();
  for(let revision=1;revision<=500;revision++)history.read(revision,measured(revision/1000),true);
  const values=history.read(500,measured(.5),true);
  assert.equal(values.length,waveformBars);
  assert.equal(values[0],.437);
  assert.equal(values.at(-1),.5);
  assert.deepEqual(new RecorderWaveformHistory().read(null,{state:"unavailable"},false),[]);
});
