// Binding status, PCM levels and CAF writes are deliberately separate evidence.
export type MicrophoneStatus={state:"connected"|"disconnected"|"reconnecting"|"unavailable"|"stopped";sourceEpoch:number;interruptions:number;reason:string|null};
export type InputKind="microphone"|"system";
export type InputNotice={id:string;input:InputKind;target:"main"|"recorder";kind:"interrupted"|"recovered";interruptions:number};
export type InputStatus={schemaVersion:2;ownerId:string;revision:number;reportAgeMs:number;microphone:MicrophoneStatus;system:MicrophoneStatus|null;notices:InputNotice[]};
export type VisibleInput={state:"unknown"|"stale"}|MicrophoneStatus;
const owner=(v:unknown)=>typeof v==="string"&&/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(v);
const integer=(v:unknown,min=0)=>typeof v==="number"&&Number.isSafeInteger(v)&&v>=min;
const time=(v:number)=>Number.isFinite(v)&&v>=0&&v<=Number.MAX_SAFE_INTEGER;
const reasons=new Set(["identity_changed","engine_changed","device_disconnected","identity_ambiguous","metadata_unavailable","permission_unavailable","restart_failed","format_changed"]);
const systemReasons=new Set(["stream_stopped","application_unavailable","acquisition_timeout","permission_unavailable","restart_failed","format_changed"]);
function record(v:unknown,keys:string[]){if(!v||typeof v!=="object"||Array.isArray(v)||Object.keys(v).sort().join("|")!==keys.slice().sort().join("|"))throw Error("入力状態の形式が不正です。");return v as Record<string,unknown>;}
export function checkedInputStatus(v:unknown,expectedOwner:string):InputStatus{
  const r=record(v,["schemaVersion","ownerId","revision","reportAgeMs","microphone","system","notices"]);
  if(r.schemaVersion!==2||!owner(r.ownerId)||r.ownerId!==expectedOwner||!integer(r.revision,1)||!integer(r.reportAgeMs))throw Error("入力状態の対象または版が不正です。");
  function input(value:unknown,kind:InputKind):MicrophoneStatus{
    const m=record(value,["state","sourceEpoch","interruptions","reason"]);
    if(!integer(m.sourceEpoch)||!integer(m.interruptions)||typeof m.state!=="string"||!["connected","disconnected","reconnecting","unavailable","stopped"].includes(m.state)||
      !(m.reason===null||typeof m.reason==="string"&&(kind==="microphone"?reasons:systemReasons).has(m.reason))||
      m.state==="connected"&&(m.sourceEpoch===0||Number(m.interruptions)>=Number(m.sourceEpoch)||m.reason!==null)||["disconnected","unavailable"].includes(m.state)&&(m.interruptions===0||m.reason===null))throw Error("入力の取得状態が不正です。");
    return m as MicrophoneStatus;
  }
  const microphone=input(r.microphone,"microphone"),system=r.system===null?null:input(r.system,"system");
  if(!Array.isArray(r.notices)||r.notices.length>2)throw Error("入力通知の形式が不正です。");
  const seen=new Set<InputKind>(),notices=r.notices.map(value=>{
    const n=record(value,["id","input","target","kind","interruptions"]);
    if(n.input!=="microphone"&&n.input!=="system"||seen.has(n.input as InputKind))throw Error("入力通知の対象が不正です。");
    const m=n.input==="microphone"?microphone:system;seen.add(n.input as InputKind);
    if(!m||typeof n.target!=="string"||!["main","recorder"].includes(n.target)||typeof n.kind!=="string"||!["interrupted","recovered"].includes(n.kind)||!integer(n.interruptions,1)||n.interruptions!==m.interruptions||
      n.id!==`${r.ownerId}:${n.input}:${n.interruptions}:${n.kind}`||m.state==="stopped"||(n.kind==="recovered")!==(m.state==="connected"))throw Error("入力通知と取得状態が一致しません。");
    return n as InputNotice;
  });
  return {schemaVersion:2,ownerId:r.ownerId as string,revision:r.revision as number,reportAgeMs:r.reportAgeMs as number,microphone,system,notices};
}
function backwards(m:MicrophoneStatus,old:MicrophoneStatus){return m.sourceEpoch<old.sourceEpoch||m.interruptions<old.interruptions||old.state==="stopped"&&JSON.stringify(m)!==JSON.stringify(old)||m.state==="connected"&&(["disconnected","unavailable"].includes(old.state)||m.interruptions>old.interruptions)&&m.sourceEpoch<=old.sourceEpoch;}
export class RecordingInputSession{
  private owner:string|null=null;private snapshot:InputStatus|null=null;private at=0;private age=0;private firstAt=0;private firstAge=0;private lastRead=0;private invalid=false;
  reset(value:string|null){if(value!==null&&!owner(value))throw Error("入力状態の録音IDが不正です。");this.owner=value;this.snapshot=null;this.at=0;this.age=0;this.firstAt=0;this.firstAge=0;this.lastRead=0;this.invalid=false;}
  accept(value:unknown,at:number){
    if(!this.owner||this.invalid||!time(at)||at<this.at)throw Error("入力状態の時計または対象が不正です。");
    const next=checkedInputStatus(value,this.owner),prior=this.snapshot;
    if(prior&&next.revision<prior.revision)return false;
    if(prior){
      if(backwards(next.microphone,prior.microphone)||!!next.system!==!!prior.system||next.system&&prior.system&&backwards(next.system,prior.system))throw Error("入力の取得世代が逆行しました。");
      if(next.revision===prior.revision){
        if(JSON.stringify(next.microphone)!==JSON.stringify(prior.microphone)||JSON.stringify(next.system)!==JSON.stringify(prior.system)||next.reportAgeMs<this.age)throw Error("同じ入力状態の版が変更されました。");
      }else{this.firstAt=at;this.firstAge=next.reportAgeMs;}
    }else{this.firstAt=at;this.firstAge=next.reportAgeMs;}
    this.snapshot=next;this.at=at;this.age=next.reportAgeMs;return true;
  }
  read(at:number):{microphone:VisibleInput;system:VisibleInput;notices:InputNotice[]}{
    if(!time(at)||at<this.lastRead||at<this.at)this.invalid=true;this.lastRead=at;
    if(this.invalid||!this.snapshot)return {microphone:{state:"unknown"},system:{state:"unknown"},notices:[]};
    if(Math.max(this.age+at-this.at,this.firstAge+at-this.firstAt)>1000)return {microphone:{state:"stale"},system:{state:"stale"},notices:[]};
    return {microphone:this.snapshot.microphone,system:this.snapshot.system??{state:"unknown"},notices:this.snapshot.notices};
  }
}
export function microphoneLabel(m:VisibleInput):string{
  return inputLabel(m,"microphone");
}
export function systemLabel(m:VisibleInput):string{return inputLabel(m,"system");}
export function inputLabel(m:VisibleInput,input:InputKind):string{
  const label={unknown:"状態を確認できません",stale:"状態の更新を確認できません",connected:"接続済み",disconnected:"切断されました",reconnecting:"再取得中",unavailable:"再取得できません",stopped:"取得を停止しました"}[m.state];
  if(!("reason" in m)||!m.reason)return label;
  const detail=m.reason==="permission_unavailable"?(input==="microphone"?"マイクの権限を確認してください":"PC音声の権限を確認してください"):
    ({identity_changed:"機器の識別が変更",engine_changed:"音声エンジンが変更",device_disconnected:"選択した機器が未接続",identity_ambiguous:"機器を一意に確認できません",metadata_unavailable:"機器情報を確認できません",restart_failed:"再取得に失敗",format_changed:"入力形式が変更",stream_stopped:"取得処理が停止",application_unavailable:"選択したアプリを確認できません",acquisition_timeout:"取得処理の応答待ち"} as Record<string,string>)[m.reason]??"取得条件を確認してください";
  return `${label}（${detail}）`;
}
