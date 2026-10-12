import test from "node:test";
import assert from "node:assert/strict";
import { MarkdownSaveController, MAX_MINUTES_BYTES, markdownAudioInterval, playableMarkdownInterval,
  type MarkdownDocument, type MarkdownSaveRequest } from "../src/minutes-markdown-state.ts";

const op = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12,"0")}`;
const initial: MarkdownDocument = { meetingId:"a", markdown:"保存済み", contentHash:"a".repeat(64), lockVersion:1, operationId:op(1), updatedAt:10 };
function acknowledge(request: MarkdownSaveRequest): MarkdownDocument {
  return { meetingId:request.meetingId, markdown:request.markdown, contentHash:"b".repeat(64),
    lockVersion:(request.expected?.lockVersion ?? 0)+1, operationId:request.operationId, updatedAt:11 };
}
test("explicit save preserves exact human text including empty body and never autosaves",async()=>{
  const calls:MarkdownSaveRequest[]=[];
  const queue=new MarkdownSaveController("a",async r=>{calls.push(r);return acknowledge(r);},()=>op(2));
  queue.load(initial); queue.edit("\r\n# 点検日\n  \t<script>x</script>\n ");
  await Promise.resolve(); assert.equal(calls.length,0);
  const exact=queue.snapshot().text;
  await queue.save(); assert.equal(calls[0].markdown,exact); assert.equal(queue.snapshot().text,exact);
  assert.equal(queue.snapshot().dirty,false);
  queue.edit(""); await queue.save(); assert.equal(queue.snapshot().document?.markdown,"");
});
test("one clicked save does not drain or overwrite newer text typed during its response",async()=>{
  let release!:()=>void; const gate=new Promise<void>(resolve=>{release=resolve;}); const calls:MarkdownSaveRequest[]=[];
  const queue=new MarkdownSaveController("a",async r=>{calls.push(r);await gate;return acknowledge(r);},()=>op(2));
  queue.load(initial); queue.edit("クリック時の本文"); const saving=queue.save(); assert.equal(queue.save(),saving);
  queue.edit("待ち時間の追記"); release(); await saving;
  assert.equal(calls.length,1); assert.equal(queue.snapshot().document?.markdown,"クリック時の本文");
  assert.equal(queue.snapshot().text,"待ち時間の追記"); assert.equal(queue.snapshot().dirty,true);
});
test("unknown save response retries only its original operation/body/base, then leaves latest text dirty",async()=>{
  const calls:MarkdownSaveRequest[]=[]; let fail=true; let count=2;
  const queue=new MarkdownSaveController("a",async r=>{calls.push(r);if(fail)throw Error("lost response");return acknowledge(r);},()=>op(count++));
  queue.load(initial); queue.edit("送信した本文"); await assert.rejects(queue.save(),/lost response/);
  queue.edit("失敗後も人間の入力を保持"); assert.equal(queue.snapshot().dirty,true);
  fail=false; await queue.save(); assert.deepEqual(calls[1],calls[0]); assert.equal(count,3);
  assert.equal(queue.snapshot().text,"失敗後も人間の入力を保持"); assert.equal(queue.snapshot().dirty,true);
  await queue.save(); assert.equal(calls[2].markdown,"失敗後も人間の入力を保持"); assert.equal(calls[2].expected?.lockVersion,2);
  assert.equal(queue.snapshot().dirty,false);
});
test("reverting local text cannot dismiss an unknown save or start a new operation",async()=>{
  let calls=0; const queue=new MarkdownSaveController("a",async()=>{calls++;throw Error("unknown");},()=>op(2));
  queue.load(initial); queue.edit("変更"); await assert.rejects(queue.save()); queue.edit(initial.markdown);
  assert.equal(queue.snapshot().dirty,true); await assert.rejects(queue.save()); assert.equal(calls,2);
});
test("external generation/edit cannot displace dirty input, but clean preview follows confirmed saved text",()=>{
  const queue=new MarkdownSaveController("a",async r=>acknowledge(r)); queue.load(initial);
  queue.edit("未保存の人間の入力"); const next={...initial,markdown:"生成の結果",lockVersion:2,contentHash:"b".repeat(64),operationId:op(2)};
  assert.equal(queue.observe(next),false); assert.equal(queue.snapshot().text,"未保存の人間の入力");
  queue.load(initial); assert.equal(queue.observe(next),true); assert.equal(queue.snapshot().text,next.markdown);
  const copy=queue.snapshot(); copy.document!.markdown="外から変更"; assert.equal(queue.snapshot().document?.markdown,next.markdown);
});
test("save failure and wrong acknowledgment retain exact input and acknowledged state",async()=>{
  for (const tamper of [ (r:MarkdownSaveRequest)=>({...acknowledge(r),meetingId:"b"}),
    (r:MarkdownSaveRequest)=>({...acknowledge(r),markdown:"別の本文"}),
    (r:MarkdownSaveRequest)=>({...acknowledge(r),operationId:op(99)}),
    (r:MarkdownSaveRequest)=>({...acknowledge(r),lockVersion:99}),
    (r:MarkdownSaveRequest)=>({...acknowledge(r),contentHash:"G".repeat(64)}),
  ]) {
    const queue=new MarkdownSaveController("a",async r=>tamper(r),()=>op(2));queue.load(initial);queue.edit("保持する本文");
    await assert.rejects(queue.save());assert.deepEqual(queue.snapshot().document,initial);assert.equal(queue.snapshot().text,"保持する本文");assert.equal(queue.snapshot().dirty,true);
  }
});
test("invalid pasted input is retained but never sent, and fixed input can be saved",async()=>{
  let calls=0;const queue=new MarkdownSaveController("a",async r=>{calls++;return acknowledge(r);},()=>op(2));queue.load(initial);
  for (const text of ["あ".repeat(Math.floor(MAX_MINUTES_BYTES/3)+1),"NUL\0本文"]) {
    queue.edit(text);await assert.rejects(queue.save());assert.equal(queue.snapshot().text,text);assert.equal(calls,0);
  }
  queue.edit("修正");await queue.save();assert.equal(calls,1);
});
test("initial document uses an absent base and clean refresh cannot interrupt an in-flight save",async()=>{
  let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve;});let sent:MarkdownSaveRequest|null=null;
  const queue=new MarkdownSaveController("a",async r=>{sent=r;await gate;return acknowledge(r);},()=>op(2));
  queue.load(null);queue.edit("新規本文");const saving=queue.save();
  assert.throws(()=>queue.load(initial),/保存中/);assert.equal(queue.observe(initial),false);
  release();await saving;assert.equal(sent!.expected,null);assert.equal(queue.snapshot().document?.lockVersion,1);
});
test("strict interval notation supports exact microseconds and long hours without transcript IDs",()=>{
  assert.deepEqual(markdownAudioInterval("audio:00:12:30-00:13:15"),{startUs:750_000_000,endUs:795_000_000});
  assert.deepEqual(markdownAudioInterval("audio:00:00:00.000001-00:00:00.999999"),{startUs:1,endUs:999999});
  assert.deepEqual(markdownAudioInterval("audio:100:00:00.1-100:00:01.01"),{startUs:360_000_100_000,endUs:360_001_010_000});
  for(const url of ["audio:00:00:02-00:00:01","audio:00:00:01-00:00:01","audio:00:60:00-01:00:01","audio:0:00:01-00:00:02","audio:00:00:00.1234567-00:00:02","audio:9999999999999999:00:00-9999999999999999:00:01","audio:00:00:01-00:00:02\n","javascript:alert(1)","https://x/audio:00:00:01-00:00:02","audio:%30%30:00:01-00:00:02"]) assert.equal(markdownAudioInterval(url),null,url);
});
test("audio interval is inactive when audio missing, duration invalid or range outside saved audio",()=>{
  const url="audio:00:00:01-00:00:02";
  assert.deepEqual(playableMarkdownInterval(url,2_000_000),{startUs:1_000_000,endUs:2_000_000});
  for(const duration of [null,0,-1,1_999_999,NaN,Infinity,2_000_000.5])assert.equal(playableMarkdownInterval(url,duration),null);
});
