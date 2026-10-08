// Binding status, PCM levels and CAF writes are deliberately separate evidence.
export type MicrophoneStatus={state:"connected"|"disconnected"|"reconnecting"|"unavailable"|"stopped";sourceEpoch:number;interruptions:number;reason:string|null};
export type InputNotice={id:string;target:"main"|"recorder";kind:"interrupted"|"recovered";interruptions:number};
export type InputStatus={schemaVersion:1;ownerId:string;revision:number;reportAgeMs:number;microphone:MicrophoneStatus;notice:InputNotice|null};
export type VisibleInput={state:"unknown"|"stale"}|MicrophoneStatus;
const owner=(v:unknown)=>typeof v==="string"&&/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(v);
const integer=(v:unknown,min=0)=>typeof v==="number"&&Number.isSafeInteger(v)&&v>=min;
const time=(v:number)=>Number.isFinite(v)&&v>=0&&v<=Number.MAX_SAFE_INTEGER;
const reasons=new Set(["identity_changed","engine_changed","device_disconnected","identity_ambiguous","metadata_unavailable","permission_unavailable","restart_failed","format_changed"]);
function record(v:unknown,keys:string[]){if(!v||typeof v!=="object"||Array.isArray(v)||Object.keys(v).sort().join("|")!==keys.slice().sort().join("|"))throw Error("マイク状態の形式が不正です。");return v as Record<string,unknown>;}
export function checkedInputStatus(v:unknown,expectedOwner:string):InputStatus{
  const r=record(v,["schemaVersion","ownerId","revision","reportAgeMs","microphone","notice"]),m=record(r.microphone,["state","sourceEpoch","interruptions","reason"]);
  if(r.schemaVersion!==1||!owner(r.ownerId)||r.ownerId!==expectedOwner||!integer(r.revision,1)||!integer(r.reportAgeMs)||!integer(m.sourceEpoch)||!integer(m.interruptions)||
    typeof m.state!=="string"||!["connected","disconnected","reconnecting","unavailable","stopped"].includes(m.state)||!(m.reason===null||typeof m.reason==="string"&&reasons.has(m.reason))||
    m.state==="connected"&&(m.sourceEpoch===0||Number(m.interruptions)>=Number(m.sourceEpoch)||m.reason!==null)||["disconnected","unavailable"].includes(m.state)&&(m.interruptions===0||m.reason===null))throw Error("マイク状態の対象・版・取得状態が不正です。");
  let notice:InputNotice|null=null;
  if(r.notice!==null){const n=record(r.notice,["id","target","kind","interruptions"]);
    if(typeof n.target!=="string"||!["main","recorder"].includes(n.target)||typeof n.kind!=="string"||!["interrupted","recovered"].includes(n.kind)||!integer(n.interruptions,1)||n.interruptions!==m.interruptions||
      n.id!==`${r.ownerId}:microphone:${n.interruptions}:${n.kind}`||m.state==="stopped"||(n.kind==="recovered")!==(m.state==="connected"))throw Error("マイク通知と現在の取得状態が一致しません。");
    notice=n as InputNotice;
  }
  return {schemaVersion:1,ownerId:r.ownerId as string,revision:r.revision as number,reportAgeMs:r.reportAgeMs as number,microphone:m as MicrophoneStatus,notice};
}
export class RecordingInputSession{
  private owner:string|null=null;private snapshot:InputStatus|null=null;private at=0;private age=0;private firstAt=0;private firstAge=0;private lastRead=0;private invalid=false;
  reset(value:string|null){if(value!==null&&!owner(value))throw Error("マイク状態の録音IDが不正です。");this.owner=value;this.snapshot=null;this.at=0;this.age=0;this.firstAt=0;this.firstAge=0;this.lastRead=0;this.invalid=false;}
  accept(value:unknown,at:number){
    if(!this.owner||this.invalid||!time(at)||at<this.at)throw Error("マイク状態の時計または対象が不正です。");
    const next=checkedInputStatus(value,this.owner),prior=this.snapshot;
    if(prior&&next.revision<prior.revision)return false;
    if(prior){const m=next.microphone,old=prior.microphone;
      if(m.sourceEpoch<old.sourceEpoch||m.interruptions<old.interruptions||old.state==="stopped"&&JSON.stringify(m)!==JSON.stringify(old)||
        m.state==="connected"&&(["disconnected","unavailable"].includes(old.state)||m.interruptions>old.interruptions)&&m.sourceEpoch<=old.sourceEpoch)throw Error("マイクの取得世代が逆行しました。");
      if(next.revision===prior.revision){
        if(JSON.stringify(m)!==JSON.stringify(old)||next.reportAgeMs<this.age)throw Error("同じマイク状態の版が変更されました。");
      }else{this.firstAt=at;this.firstAge=next.reportAgeMs;}
    }else{this.firstAt=at;this.firstAge=next.reportAgeMs;}
    this.snapshot=next;this.at=at;this.age=next.reportAgeMs;return true;
  }
  read(at:number):{microphone:VisibleInput;notice:InputNotice|null}{
    if(!time(at)||at<this.lastRead||at<this.at)this.invalid=true;this.lastRead=at;
    if(this.invalid||!this.snapshot)return {microphone:{state:"unknown"},notice:null};
    if(Math.max(this.age+at-this.at,this.firstAge+at-this.firstAt)>1000)return {microphone:{state:"stale"},notice:null};
    return {microphone:this.snapshot.microphone,notice:this.snapshot.notice};
  }
}
export function microphoneLabel(m:VisibleInput):string{
  const label={unknown:"状態を確認できません",stale:"状態の更新を確認できません",connected:"接続済み",disconnected:"切断されました",reconnecting:"再取得中",unavailable:"再取得できません",stopped:"取得を停止しました"}[m.state];
  if(!("reason" in m)||!m.reason)return label;
  return `${label}（${{identity_changed:"機器の識別が変更",engine_changed:"音声エンジンが変更",device_disconnected:"選択した機器が未接続",identity_ambiguous:"機器を一意に確認できません",metadata_unavailable:"機器情報を確認できません",permission_unavailable:"マイクの権限を確認してください",restart_failed:"再取得に失敗",format_changed:"入力形式が変更"}[m.reason]??"取得条件を確認してください"}）`;
}
