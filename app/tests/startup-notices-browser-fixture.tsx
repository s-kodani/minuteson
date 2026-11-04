// Synthetic IPC and tiny model metadata. No model, file, network or process IO.
import React,{useState} from "react";
import {createRoot} from "react-dom/client";
import {mockIPC} from "./startup-ready-mocks";
import {ModelInspectionPanel} from "../src/model-inspection";
import {ToastHost,NotificationList} from "../src/toast";
import {downloadDigest,packageReviewHash,downloadByteTotals} from "../src/model-download-state";
(window as any).skipModelDownloadFixtureMain=true;
(window as any).startupNoticesFixture={};
const {modelDownloadIPC}=await import("./model-download-browser-fixture");
const f=(window as any).modelDownloadFixture;
const recipe=JSON.parse(f.definition);
Object.assign(recipe.manifest,{engine:"llama.cpp",engineVersion:"b11960",modelName:"合成LLM",quantization:"Q4_K_M"});
recipe.manifest.files[1].role="llm";
f.definition=JSON.stringify(recipe);
const recipeHash=await downloadDigest(recipe),entry=f.entries[0],oldId=entry.packageId;
Object.assign(entry,{engine:"llama.cpp",engineVersion:"b11533",modelName:"合成LLM",modelVersion:"1",quantization:"Q4_K_M",sourceType:"download",download:{operationId:"33333333-3333-4333-8333-333333333333",recipeHash:"f".repeat(64)},modelFiles:[{role:"llm",size:17,sha256:"b".repeat(64)}]});
entry.compatibility={operationId:"22222222-2222-4222-8222-222222222222",packageId:entry.packageId,reviewHash:entry.reviewHash,adapterRevision:1,state:"succeeded",errorMessage:null};
Object.assign(f.profiles[0],{id:"44444444-4444-4444-8444-444444444444",profileVersion:1,components:[{roles:["llm"],engine:"llama.cpp",packageId:oldId,modelName:entry.modelName,modelVersion:entry.modelVersion,checkOperationId:entry.compatibility.operationId,settingsMatch:true,compatibilityCurrent:true}]});
const optionId="llama-b11533-qwen25-15b-q4km";
(window as any).windowsNativeModelOptions=[{id:optionId,label:"議事録生成（合成）",description:"合成のランタイム更新",definition:f.definition,review:{recipeSchemaVersion:4,recipeHash,packageReviewHash:await packageReviewHash(recipe.manifest),manifest:recipe.manifest,sources:recipe.sources,totalBytes:33,...downloadByteTotals(recipe),installed:false,ready:false,publisherAuthenticity:"not_verified"},tokenLimits:{maxInputTokens:32768,maxOutputTokens:8192,maxContextTokens:32768,tokenizer:"utf8_byte_bpe"}}];
const runtime={id:"llama-runtime-update",revision:"a".repeat(64),kind:"model_runtime_update",title:"議事録生成エンジンの更新",message:"モデルデータを再利用して更新します。",priority:0,deferred:false,optionId,packageId:oldId,recipeHash};
const information={id:"feature-information",revision:"d".repeat(64),kind:"information",title:"機能変更のお知らせ（合成）",message:"読むだけの合成案内です。",priority:10,deferred:false,optionId:null,packageId:null,recipeHash:null};
f.deferred=new Set<string>();f.acknowledged=false;
mockIPC(async(command,args:any)=>{
 if(command==="get_startup_notices"){
  f.noticeReads=(f.noticeReads??0)+1;
  if(document.querySelector('dialog[open][aria-labelledby="startup-recovery-title"]'))throw Error("synthetic recovery blocks notice reads");
  const rows=[];
  if(f.profiles[0].components.some((c:any)=>c.packageId===oldId))rows.push(runtime);
  if(!f.acknowledged)rows.push(information);
  return rows.map(n=>({...n,deferred:f.deferred.has(n.id)}));
 }
 if(command==="respond_startup_notice"){
  f.calls.push({command,args});
  const notice=[runtime,information].find(n=>n.id===args.id&&n.revision===args.revision);
  if(!notice||args.response==="acknowledge"&&notice.kind!=="information")throw Error("合成の案内対象不一致");
  if(args.response==="acknowledge")f.acknowledged=true;
  f.deferred.add(args.id);return;
 }
 return modelDownloadIPC(command,args);
},{shouldMockEvents:true});
function Fixture(){
 const [key,setKey]=useState(0),[locked,setLocked]=useState(false),[ready,setReady]=useState(false);
 f.remount=()=>setKey(v=>v+1);f.setLocked=setLocked;f.setReady=setReady;
 return <main><ToastHost/><NotificationList visible/><button onClick={()=>setKey(v=>v+1)}>画面を再生成</button><ModelInspectionPanel startupNoticesReady={ready} key={key} visible={false} locked={locked} startupNoticesLocked={locked} recordingActive={false}/></main>;
}
createRoot(document.getElementById("root")!).render(<Fixture/>);
