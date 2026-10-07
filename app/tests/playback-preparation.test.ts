import test from "node:test";
import assert from "node:assert/strict";
import { PlaybackPreparationPool,type PreparationBridge } from "../src/playback-preparation.ts";
import type { PlaybackInfo } from "../src/playback.ts";
const info:PlaybackInfo={token:"audio",sampleRate:48000,channels:1,totalFrames:48000,startUs:0,endUs:1000000,trackKind:"mic",alignment:"unverified"};
function deferred<T>(){let resolve!:(v:T)=>void,reject!:(e:Error)=>void;const promise=new Promise<T>((a,b)=>{resolve=a;reject=b;});return{promise,resolve,reject};}
const tick=async()=>{for(let i=0;i<12;i++)await Promise.resolve();};
function fixture(patch:Partial<PreparationBridge>={}){
  const calls:string[]=[],closed:string[]=[];let sequence=0;
  const bridge:PreparationBridge={register:async id=>{calls.push(`register:${id}`);},prepare:async(command,args)=>{calls.push(`prepare:${command}:${args.preparationId}`);return info;},cancel:async id=>{calls.push(`cancel:${id}`);},acknowledge:async(id,token)=>{calls.push(`ack:${id}:${token}`);},close:async token=>{closed.push(token);},...patch};
  return {pool:new PlaybackPreparationPool(bridge,()=>`id-${++sequence}`),calls,closed};
}
test("all five preparation paths reserve then prepare then acknowledge the exact received token",async()=>{
  const f=fixture();
  for(const command of ["prepare_utterance_playback","prepare_draft_utterance_playback","prepare_minutes_evidence_playback","prepare_minutes_transcript_playback","prepare_meeting_playback"]){
    assert.equal(await f.pool.run(command,{request:{fixed:"reference"},preparationId:"untrusted"}),info);
  }
  assert.equal(f.calls.length,15);for(let i=0;i<5;i++){assert.equal(f.calls[i*3],`register:id-${i+1}`);assert.match(f.calls[i*3+1],new RegExp(`:id-${i+1}$`));assert.equal(f.calls[i*3+2],`ack:id-${i+1}:audio`);}
  await f.pool.stop();assert.equal(f.calls.length,15);assert.deepEqual(f.closed,[]);
});
test("cancel while registration is pending waits for registration and never starts expensive preparation",async()=>{
  const reg=deferred<void>();const f=fixture({register:async id=>{f.calls.push(`register:${id}`);await reg.promise;}});
  const job=f.pool.run("prepare",{}),rejection=assert.rejects(job,/取り消/);f.pool.cancelAll();let stopped=false;const stop=f.pool.stop().then(()=>{stopped=true;});
  await tick();assert.deepEqual(f.calls,["register:id-1"]);assert.equal(stopped,false);reg.resolve();await rejection;await stop;
  assert.deepEqual(f.calls,["register:id-1","cancel:id-1"]);assert.equal(stopped,true);assert.deepEqual(f.closed,[]);
});
test("cancel acknowledgement does not mean blocked native work ended; late replies close without adoption",async()=>{
  const pending=deferred<PlaybackInfo>();const f=fixture({prepare:async()=>pending.promise});
  const job=f.pool.run("prepare",{}),rejection=assert.rejects(job,/取り消/);await tick();f.pool.cancelAll();let stopped=false;const stop=f.pool.stop().then(()=>{stopped=true;});
  await tick();assert.equal(stopped,false);pending.resolve(info);await rejection;await stop;
  assert.equal(f.calls.some(x=>x.startsWith("ack:")),false);assert.deepEqual(f.closed,["audio"]);assert.equal(stopped,true);
  assert.equal(f.calls.filter(x=>x==="cancel:id-1").length,1,"release stays alive until the late result, not a second cancellation after premature cleanup");
});
test("lost registration and preparation replies release by the already-known reservation id",async()=>{
  for(const phase of ["register","prepare"] as const){
    const f=fixture({[phase]:async()=>{throw Error(`lost ${phase}`);}});await assert.rejects(f.pool.run("prepare",{}),/lost/);await f.pool.stop();
    assert.ok(f.calls.includes("cancel:id-1"));assert.equal(f.calls.some(x=>x.startsWith("ack:")),false);assert.deepEqual(f.closed,[]);
  }
});
test("lost receipt acknowledgement closes the known token and never schedules apparent success",async()=>{
  const f=fixture({acknowledge:async()=>{throw Error("lost ack");}});await assert.rejects(f.pool.run("prepare",{}),/lost ack/);await f.pool.stop();
  assert.deepEqual(f.closed,["audio"]);assert.ok(f.calls.includes("cancel:id-1"));
});
test("failed reservation cancellation stays owned and global stop explicitly retries it",async()=>{
  let attempts=0;const f=fixture({prepare:async()=>{throw Error("native failed");},cancel:async()=>{if(++attempts===1)throw Error("cancel lost");}});
  await assert.rejects(f.pool.run("prepare",{}),/native failed/);await tick();assert.equal(attempts,1);await f.pool.stop();assert.equal(attempts,2);
  await f.pool.stop();assert.equal(attempts,2);
});
test("failed token close after unknown acknowledgement is retained for explicit barrier retry",async()=>{
  let closes=0;const f=fixture({acknowledge:async()=>{throw Error("lost ack");},close:async()=>{if(++closes<=2)throw Error("close lost");}});
  await assert.rejects(f.pool.run("prepare",{}),/lost ack/);await tick();await assert.rejects(f.pool.stop(),/close lost/);await f.pool.stop();assert.equal(closes,3);
});
test("a newer preparation is not cancelled or released by the old late reply",async()=>{
  const old=deferred<PlaybackInfo>();let n=0;const f=fixture({prepare:async()=>++n===1?old.promise:{...info,token:"new"}});
  const first=f.pool.run("prepare",{}),rejection=assert.rejects(first,/取り消/);await tick();f.pool.cancelAll();
  assert.equal((await f.pool.run("prepare",{})).token,"new");old.resolve(info);await rejection;await f.pool.stop();
  assert.deepEqual(f.closed,["audio"]);assert.ok(f.calls.includes("ack:id-2:new"));assert.equal(f.calls.includes("cancel:id-2"),false);
});
test("cancellation during acknowledgement closes even if native ownership already transferred",async()=>{
  const ack=deferred<void>();const f=fixture({acknowledge:async()=>ack.promise});
  const job=f.pool.run("prepare",{}),rejection=assert.rejects(job,/取り消/);await tick();f.pool.cancelAll();await tick();ack.resolve();await rejection;await f.pool.stop();
  assert.ok(f.closed.length>=1);assert.ok(f.closed.every(t=>t==="audio"));
});
test("malformed result does not invent a token; reservation cleanup handles unseen native publication",async()=>{
  const f=fixture({prepare:async()=>({...info,token:null}) as unknown as PlaybackInfo});await assert.rejects(f.pool.run("prepare",{}),/応答が不正/);await f.pool.stop();
  assert.deepEqual(f.closed,[]);assert.ok(f.calls.includes("cancel:id-1"));assert.equal(f.calls.some(x=>x.startsWith("ack:")),false);
});
