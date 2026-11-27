import test from "node:test";
import assert from "node:assert/strict";
import {RecorderWaveformHistory,waveformBars,type WaveformSample} from "../src/recorder-waveform-state.ts";
import type {VisibleLevel} from "../src/input-level-state.ts";
const measured=(peak:number):VisibleLevel=>({state:"measured",rms:peak/2,peak,rmsDbFS:null,peakDbFS:null,clipping:peak>=1});
const missing=(state:"stale"|"waiting"|"unavailable"):VisibleLevel=>({state});
// Real clocks: the WebView's performance.now() and an epoch-sized Date.now().
const startMono=50_000,startWall=1_795_740_000_000;
// Drives the history the way the panel does: one read per 100 ms paint, and a
// new native revision every `publishMs` while `live(ms)` is true.
function panel(publishMs:number){
  const history=new RecorderWaveformHistory();
  let ms=0,revision=0,last:readonly WaveformSample[]=[];
  const run=(duration:number,live:(at:number)=>boolean,state:VisibleLevel=missing("stale"),connected=true)=>{
    for(const end=ms+duration;ms<end;){
      const published=Math.floor(ms/publishMs);ms+=100;
      if(live(ms)&&Math.floor(ms/publishMs)>published)revision++;
      last=history.read(revision||null,live(ms)?measured(.5):state,connected,startMono+ms,startWall+ms);
    }
    return last;
  };
  return {run,read:(level:VisibleLevel,connected=true)=>last=history.read(revision||null,level,connected,startMono+ms,startWall+ms)};
}
const bars=(values:readonly WaveformSample[])=>values.filter(value=>value!==null).length;
const blanks=(values:readonly WaveformSample[])=>values.filter(value=>value===null).length;

test("waveform retains actual separate samples and cannot animate repeated or older publications",()=>{
  const mic=new RecorderWaveformHistory(),pc=new RecorderWaveformHistory();
  assert.deepEqual(mic.read(1,measured(.25),true,startMono,startWall),[.25]);
  assert.deepEqual(pc.read(1,measured(.75),true,startMono,startWall),[.75]);
  assert.deepEqual(mic.read(1,measured(.25),true,startMono+100,startWall+100),[.25]);
  assert.deepEqual(mic.read(0,measured(.5),true,startMono+200,startWall+200),[.25]);
  assert.deepEqual(mic.read(2,measured(0),true,startMono+250,startWall+250),[.25,0]);
  assert.deepEqual(pc.read(2,measured(2),true,startMono+250,startWall+250),[.75,1]);
});

test("the first read with epoch-sized clocks neither stalls nor invents gaps",()=>{
  const history=new RecorderWaveformHistory();
  const started=performance.now();
  assert.deepEqual(history.read(null,missing("unavailable"),false,startMono,startWall),[]);
  assert.deepEqual(history.read(1,measured(.4),true,startMono+100,startWall+100),[.4]);
  assert.ok(performance.now()-started<50,"first read walked the epoch as blank slots");
});

test("every missing state blanks the right edge at once and keeps earlier bars",()=>{
  for(const [level,connected] of [[missing("stale"),true],[missing("waiting"),true],[missing("unavailable"),true],[measured(.5),false]] as const){
    const view=panel(250);
    assert.deepEqual(view.run(1000,()=>true),[.5,.5,.5,.5]);
    const gap=view.read(level,connected);
    assert.deepEqual(gap,[.5,.5,.5,.5,null],`${level.state}/${connected}`);
  }
  // A paint without any revision (for example right after a clock rebuild)
  // must not leave the last bar at the right edge either.
  const history=new RecorderWaveformHistory();
  history.read(1,measured(.5),true,startMono,startWall);
  assert.deepEqual(history.read(null,measured(.5),true,startMono+100,startWall+100),[.5,null]);
});

test("gap width equals outage time divided by the publication spacing on 100 ms and 250 ms publishers",()=>{
  for(const publishMs of [100,250]){
    const view=panel(publishMs);
    view.run(2000,()=>true);
    const before=view.run(0,()=>true);
    // Slots keep entering on the right while the outage lasts.
    const during=view.run(1000,()=>false);
    assert.ok(blanks(during)>=Math.floor(1000/publishMs),`${publishMs} ms: gap did not advance during the outage`);
    const after=view.run(1000,()=>false);
    assert.equal(blanks(after),Math.floor(2000/publishMs),`${publishMs} ms: gap is not proportional to the outage`);
    assert.equal(bars(after),bars(before));
    // Resuming adds the next bar after the gap without counting the gap again:
    // only the time until that bar's publication (under two slots) is added.
    const resumed=view.run(publishMs,()=>true);
    assert.equal(resumed.at(-1),.5);
    assert.ok(blanks(resumed)-blanks(after)<=2,`${publishMs} ms: resume double-counted the gap`);
  }
});

test("a missed outage still leaves a proportional gap when the next bar arrives",()=>{
  const history=new RecorderWaveformHistory();
  for(let i=1;i<=5;i++)history.read(i,measured(.5),true,startMono+i*250,startWall+i*250);
  // A suspended timer never observed the outage; 3 s later a new value arrives.
  const resumed=history.read(6,measured(.2),true,startMono+4250,startWall+4250);
  assert.equal(resumed.at(-1),.2);
  assert.equal(blanks(resumed),11);
});

test("a long outage drains the history and a new owner starts empty",()=>{
  const view=panel(100);
  view.run(2000,()=>true);
  assert.ok(bars(view.run(0,()=>true))>0);
  assert.deepEqual(view.run(100*waveformBars+100,()=>false),[]);
  assert.deepEqual(new RecorderWaveformHistory().read(null,missing("unavailable"),false,startMono,startWall),[]);
});

test("history stays bounded and unchanged reads keep the same array",()=>{
  const view=panel(100);
  const full=view.run(20_000,()=>true);
  assert.equal(full.length,waveformBars);
  assert.equal(view.read(measured(.5)),full);
});

test("a paused monotonic clock still advances on wall time and a reversed wall clock never rewinds",()=>{
  const history=new RecorderWaveformHistory();
  history.read(1,measured(.5),true,startMono,startWall);
  history.read(2,measured(.5),true,startMono+250,startWall+250);
  // Sleep: performance.now() paused while the wall clock moved 1 s.
  assert.deepEqual(history.read(2,missing("stale"),true,startMono+250,startWall+1250),[.5,.5,null,null,null,null]);
  // A wall clock stepped backwards adds no slots and keeps the history.
  assert.deepEqual(history.read(2,missing("stale"),true,startMono+250,startWall-60_000),[.5,.5,null,null,null,null]);
});

test("a brief disconnect never re-adds the same revision when it reconnects",()=>{
  const history=new RecorderWaveformHistory();
  history.read(1,measured(.5),true,startMono,startWall);
  history.read(2,measured(.4),true,startMono+250,startWall+250);
  assert.deepEqual(history.read(2,measured(.4),false,startMono+350,startWall+350),[.5,.4,null]);
  assert.deepEqual(history.read(2,measured(.4),true,startMono+600,startWall+600),[.5,.4,null,null]);
  assert.deepEqual(history.read(3,measured(.6),true,startMono+850,startWall+850),[.5,.4,null,null,null,.6]);
});
