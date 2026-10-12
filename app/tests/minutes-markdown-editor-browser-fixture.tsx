// Synthetic memory only. No Tauri bridge, DB, AI process or audio playback.
import { createRoot } from "react-dom/client";
import { MinutesMarkdownEditor } from "../src/minutes-markdown-editor";
import { MarkdownSaveController, type MarkdownDocument, type MarkdownSaveRequest } from "../src/minutes-markdown-state";
import "../src/style.css";
import "../src/prototype-alignment.css";
import "../src/minutes-markdown-editor.css";

let document:MarkdownDocument={meetingId:"synthetic-plain-minutes",markdown:"# 点検会議\n\n- 点検日を確認する\n\n[区間](audio:00:00:01-00:00:02)\n",lockVersion:1,contentHash:"a".repeat(64),operationId:crypto.randomUUID(),updatedAt:1};
const fixture={mode:"success",calls:[] as MarkdownSaveRequest[],plays:[] as unknown[],release:()=>{},controller:null as MarkdownSaveController|null};
const controller=new MarkdownSaveController(document.meetingId,async request=>{
  fixture.calls.push(structuredClone(request));
  if(fixture.mode==="held")await new Promise<void>(resolve=>{fixture.release=resolve;});
  if(fixture.mode==="failure")throw Error("合成保存失敗");
  const same=document.operationId===request.operationId&&document.markdown===request.markdown;
  if(!same){
    if(request.expected?.lockVersion!==document.lockVersion||request.expected?.contentHash!==document.contentHash)throw Error("合成保存競合");
    document={...document,markdown:request.markdown,lockVersion:document.lockVersion+1,contentHash:"b".repeat(64),operationId:request.operationId,updatedAt:document.updatedAt+1};
  }
  if(fixture.mode==="lost")throw Error("合成保存成功応答の喪失");
  return structuredClone(document);
});
controller.load(document);fixture.controller=controller;
Object.assign(window,{plainMinutesFixture:fixture});
createRoot(documentElement()).render(<main style={{height:"100dvh",display:"flex",flexDirection:"column",padding:24}}><MinutesMarkdownEditor meetingId={document.meetingId} controller={controller} durationUs={10_000_000} onPlay={interval=>fixture.plays.push(interval)} /></main>);
function documentElement(){return window.document.getElementById("root")!;}
