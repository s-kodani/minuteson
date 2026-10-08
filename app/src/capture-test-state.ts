import {validMicrophoneUID} from "./capture-input-state.ts";
import {checkedCaptureApplicationIdentity,type CaptureScope} from "./capture-scope-state.ts";

export type CaptureTestPhase="preparing"|"recording"|"checking"|"cancelling"|"ready"|"cancelled"|"error";
export type CaptureTestTrack={frames:number;sampleRate:number;channels:number;durationMs:number;hasGaps:boolean};
export type CaptureTestSnapshot={schemaVersion:1;revision:number;runId:string;phase:CaptureTestPhase;busy:boolean;
  microphoneUid:string;microphoneName:string|null;captureScope:CaptureScope;applicationName:string|null;
  microphone:CaptureTestTrack|null;system:CaptureTestTrack|null;cleanupPending:boolean;errorMessage:string|null};
export type CaptureTestRequest={runId:string;microphoneUid:string;captureScope:CaptureScope;
  expectedImport:{token:string|null;revision:number;busy:boolean}};
export const validCaptureTestId=(v:unknown):v is string=>typeof v==="string"&&/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(v);
const integer=(v:unknown,min:number,max=Number.MAX_SAFE_INTEGER):v is number=>typeof v==="number"&&Number.isSafeInteger(v)&&v>=min&&v<=max;
const record=(v:unknown,keys:string[])=>{
  if(!v||typeof v!=="object"||Array.isArray(v)||Object.keys(v).sort().join("|")!==keys.slice().sort().join("|"))throw Error("録音前テストの応答形式が不正です。");
  return v as Record<string,unknown>;
};
const optionalText=(v:unknown,maximum=1024):v is string|null=>v===null||typeof v==="string"&&!!v.trim()&&!v.includes("\0")&&new TextEncoder().encode(v).length<=maximum;
function scope(value:unknown):CaptureScope{
  const v=record(value,["mode","application"]);
  if(v.mode==="system"&&v.application===null)return {mode:"system",application:null};
  if(v.mode!=="application")throw Error("テストのPC音声の取得範囲が不正です。");
  record(v.application,["pid","bundleIdentifier","launchTimeUs"]);
  return {mode:"application",application:checkedCaptureApplicationIdentity(v.application)};
}
function track(value:unknown):CaptureTestTrack|null{
  if(value===null)return null;
  const v=record(value,["frames","sampleRate","channels","durationMs","hasGaps"]);
  if(!integer(v.sampleRate,8000,384000)||!integer(v.frames,1,v.sampleRate*5)||!integer(v.channels,1,8)||
    !integer(v.durationMs,0,5000)||v.durationMs!==Math.floor(v.frames*1000/v.sampleRate)||typeof v.hasGaps!=="boolean")throw Error("テスト音声の形式・長さが不正です。");
  return {frames:v.frames,sampleRate:v.sampleRate,channels:v.channels,durationMs:v.durationMs,hasGaps:v.hasGaps};
}
export function checkedCaptureTest(value:unknown,expectedRunId?:string):CaptureTestSnapshot{
  const v=record(value,["schemaVersion","revision","runId","phase","busy","microphoneUid","microphoneName","captureScope","applicationName","microphone","system","cleanupPending","errorMessage"]);
  if(v.schemaVersion!==1||!integer(v.revision,1)||!validCaptureTestId(v.runId)||expectedRunId!==undefined&&v.runId!==expectedRunId||
    !validMicrophoneUID(v.microphoneUid)||!optionalText(v.microphoneName)||!optionalText(v.applicationName)||!optionalText(v.errorMessage,16384)||
    typeof v.phase!=="string"||!["preparing","recording","checking","cancelling","ready","cancelled","error"].includes(v.phase)||typeof v.busy!=="boolean"||typeof v.cleanupPending!=="boolean")throw Error("録音前テストの対象・状態が不正です。");
  const phase=v.phase as CaptureTestPhase,captureScope=scope(v.captureScope),microphone=track(v.microphone),system=track(v.system);
  const busy=["preparing","recording","checking","cancelling"].includes(phase);
  if(v.busy!==busy||busy&&v.cleanupPending||phase==="ready"&&(v.cleanupPending||v.errorMessage!==null)||
    (phase==="ready")!==(microphone!==null&&system!==null)||phase!=="ready"&&(microphone!==null||system!==null)||captureScope.mode==="system"&&v.applicationName!==null)throw Error("テストの保存・停止確認が状態と一致しません。");
  return {schemaVersion:1,revision:v.revision,runId:v.runId,phase,busy:v.busy,microphoneUid:v.microphoneUid as string,
    microphoneName:v.microphoneName,captureScope,applicationName:v.applicationName,microphone,system,cleanupPending:v.cleanupPending,errorMessage:v.errorMessage};
}
const transitions:Record<CaptureTestPhase,CaptureTestPhase[]>={
  preparing:["preparing","recording","checking","cancelling","ready","cancelled","error"],
  recording:["recording","checking","cancelling","ready","cancelled","error"],
  checking:["checking","cancelling","ready","cancelled","error"],
  cancelling:["cancelling","cancelled","error"],ready:["ready"],cancelled:["cancelled"],error:["error"],
};
// Polls can skip intermediate phases. They cannot replace the owner, revive a
// cancelled run, or publish ready audio for a different microphone/process.
export function reconcileCaptureTest(prior:CaptureTestSnapshot|null,value:unknown,expected:Pick<CaptureTestRequest,"runId"|"microphoneUid"|"captureScope">):CaptureTestSnapshot{
  const next=checkedCaptureTest(value,expected.runId);
  if(next.microphoneUid!==expected.microphoneUid||JSON.stringify(next.captureScope)!==JSON.stringify(scope(expected.captureScope)))throw Error("テストの入力が開始条件と一致しません。");
  if(!prior)return next;
  if(prior.runId!==expected.runId)throw Error("以前のテスト結果が別の開始に混在しています。");
  if(next.revision<prior.revision)return prior;
  if(next.revision===prior.revision){
    if(JSON.stringify(next)!==JSON.stringify(prior))throw Error("同じテストの版に異なる結果があります。");
    return prior;
  }
  if(!transitions[prior.phase].includes(next.phase)||prior.microphoneName!==null&&next.microphoneName!==prior.microphoneName||
    prior.applicationName!==null&&next.applicationName!==prior.applicationName)throw Error("テストの状態または入力表示が逆行しました。");
  if(prior.phase==="ready"&&JSON.stringify({...next,revision:0})!==JSON.stringify({...prior,revision:0}))throw Error("検証済みのテスト音声が同じ所有IDで変更されました。");
  return next;
}
