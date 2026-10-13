import test from "node:test";
import assert from "node:assert/strict";
import { MarkdownSaveController, type MarkdownDocument } from "../src/minutes-markdown-state.ts";
import { confirmMarkdownLeave } from "../src/minutes-markdown-leave.ts";
import type { ConfirmationOptions } from "../src/confirmation-state.ts";
import { SavedEditsCancelled } from "../src/confirmed-action.ts";
const op=(n:number)=>`00000000-0000-4000-8000-${String(n).padStart(12,"0")}`;
const initial:MarkdownDocument={meetingId:"m",markdown:"保存済み",lockVersion:1,contentHash:"a".repeat(64),operationId:op(1),updatedAt:1};
function fixture() {
  let saved={...initial},calls=0,fail=false;
  const controller=new MarkdownSaveController("m",async r=>{calls++;if(fail)throw Error("unknown");saved={...saved,markdown:r.markdown,operationId:r.operationId,lockVersion:(r.expected?.lockVersion??0)+1};return {...saved};},()=>op(2));
  controller.load(saved);
  return {controller,read:async()=>({...saved}),calls:()=>calls,setFail:(v:boolean)=>{fail=v;}};
}
test("clean navigation never prompts, reads or writes",async()=>{
  const f=fixture();await confirmMarkdownLeave(f.controller,"会議",async()=>{throw Error("unexpected read");},async()=>{throw Error("unexpected prompt");});assert.equal(f.calls(),0);
});
test("leaving requires explicit save or discard; cancellation preserves exact input without a write",async()=>{
  const f=fixture();f.controller.edit("人間の入力\n");let options!:ConfirmationOptions;
  await assert.rejects(confirmMarkdownLeave(f.controller,"会議",f.read,async o=>{options=o;return "cancelled";}),SavedEditsCancelled);
  assert.equal(f.controller.snapshot().text,"人間の入力\n");assert.equal(f.calls(),0);assert.equal(options.alternativeAction?.label,"編集を破棄して続ける");
  await confirmMarkdownLeave(f.controller,"会議",f.read,async o=>{assert.equal(o.validate!(),true);await o.action();return "completed";});
  assert.equal(f.calls(),1);assert.equal(f.controller.snapshot().dirty,false);
});
test("an unconfirmed save remains a failure, not a quiet cancellation",async()=>{
  const f=fixture();f.controller.edit("入力を保持");
  await assert.rejects(confirmMarkdownLeave(f.controller,"会議",f.read,async()=>"unconfirmed"),cause=>cause instanceof Error && !(cause instanceof SavedEditsCancelled));
  assert.equal(f.controller.snapshot().text,"入力を保持");assert.equal(f.calls(),0);
});
test("discard reads current saved body and does not save unwanted input",async()=>{
  const f=fixture();f.controller.edit("不要な入力");await confirmMarkdownLeave(f.controller,"会議",f.read,async o=>{await o.alternativeAction!.action();return "completed";});
  assert.equal(f.controller.snapshot().text,initial.markdown);assert.equal(f.calls(),0);
});
test("discard failure, wrong meeting or input change during read retains input and pending decision",async()=>{
  for(const mode of ["read-error","wrong-meeting","new-input"]) {
    const f=fixture();f.controller.edit("入力を保持");const read=async()=>{
      if(mode==="read-error")throw Error("read failed");if(mode==="new-input")f.controller.edit("新しい入力");return {...initial,meetingId:mode==="wrong-meeting"?"other":"m"};
    };
    await assert.rejects(confirmMarkdownLeave(f.controller,"会議",read,async o=>{await o.alternativeAction!.action();return "completed";}));
    assert.equal(f.controller.snapshot().text,mode==="new-input"?"新しい入力":"入力を保持");assert.equal(f.calls(),0);
  }
});
test("unknown save cannot be dismissed by old-base read; newer committed stamp prevents late CAS overwrite",async()=>{
  const f=fixture();f.controller.edit("送信した入力");f.setFail(true);await assert.rejects(f.controller.save());f.controller.edit("新しい入力");
  assert.throws(()=>f.controller.discard(initial),/保存結果が未確認/);assert.equal(f.controller.snapshot().text,"新しい入力");
  f.controller.discard({...initial,lockVersion:2,operationId:op(9),contentHash:"b".repeat(64),markdown:"後続の保存本文"});
  assert.equal(f.controller.snapshot().text,"後続の保存本文");assert.equal(f.controller.snapshot().dirty,false);assert.equal(f.calls(),1);
});
test("composition is never implicitly saved or discarded; changes after prompt invalidate the review",async()=>{
  const f=fixture();f.controller.edit("変換中");f.controller.setComposing(true);
  await assert.rejects(confirmMarkdownLeave(f.controller,"会議",f.read,async()=>{throw Error("unexpected prompt");}),/日本語入力/);
  await assert.rejects(f.controller.save(),/日本語入力/);assert.throws(()=>f.controller.discard(initial),/日本語入力/);assert.equal(f.calls(),0);
  f.controller.setComposing(false);let options!:ConfirmationOptions;
  await assert.rejects(confirmMarkdownLeave(f.controller,"会議",f.read,async o=>{options=o;f.controller.edit("変更");assert.equal(o.validate!(),false);return "cancelled";}));assert.equal(options.validate!(),false);
});
