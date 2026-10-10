// Binding status, PCM levels and CAF writes are deliberately separate evidence.
export type MicrophoneStatus={state:"connected"|"disconnected"|"reconnecting"|"unavailable"|"stopped";sourceEpoch:number;interruptions:number;reason:string|null};
export type InputKind="microphone"|"system";
export type NoticeDelivery={sequence:number;generation:number;leaseAgeMs:number|null;rendered:boolean;status:MicrophoneStatus;receiveId:string|null};
export type InputNotice={id:string;input:InputKind;target:"main"|"recorder";kind:"interrupted"|"recovered";interruptions:number;recipientId:string|null;delivery?:NoticeDelivery};
export type NoticeLoss={microphone:number;system:number};
export type InputStatus={schemaVersion:3|4;ownerId:string;revision:number;reportAgeMs:number;microphone:MicrophoneStatus;system:MicrophoneStatus|null;notices:InputNotice[];noticeLoss?:NoticeLoss};
export type InputNoticeReceipt={schemaVersion:1|2;ownerId:string;noticeId:string;target:"main"|"recorder";recipientId:string;delivery?:NoticeDelivery};
export type VisibleInput={state:"unknown"|"stale"}|MicrophoneStatus;
const owner=(v:unknown)=>typeof v==="string"&&/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(v);
export const validInputOwner=owner;
const integer=(v:unknown,min=0)=>typeof v==="number"&&Number.isSafeInteger(v)&&v>=min;
const time=(v:number)=>Number.isFinite(v)&&v>=0&&v<=Number.MAX_SAFE_INTEGER;
const sameInput=(a:MicrophoneStatus|null,b:MicrophoneStatus|null)=>a===null||b===null?a===b:a.state===b.state&&a.sourceEpoch===b.sourceEpoch&&a.interruptions===b.interruptions&&a.reason===b.reason;
const reasons=new Set(["identity_changed","engine_changed","device_disconnected","identity_ambiguous","metadata_unavailable","permission_unavailable","restart_failed","format_changed","clock_changed"]);
const systemReasons=new Set(["stream_stopped","application_unavailable","acquisition_timeout","permission_unavailable","restart_failed","format_changed","clock_changed"]);
function record(v:unknown,keys:string[]){if(!v||typeof v!=="object"||Array.isArray(v)||Object.keys(v).sort().join("|")!==keys.slice().sort().join("|"))throw Error("入力状態の形式が不正です。");return v as Record<string,unknown>;}
function input(value:unknown,kind:InputKind):MicrophoneStatus{
    const m=record(value,["state","sourceEpoch","interruptions","reason"]);
    if(!integer(m.sourceEpoch)||!integer(m.interruptions)||typeof m.state!=="string"||!["connected","disconnected","reconnecting","unavailable","stopped"].includes(m.state)||
      !(m.reason===null||typeof m.reason==="string"&&(kind==="microphone"?reasons:systemReasons).has(m.reason))||
      m.state==="connected"&&(m.sourceEpoch===0||Number(m.interruptions)>=Number(m.sourceEpoch)||m.reason!==null)||["disconnected","unavailable"].includes(m.state)&&(m.interruptions===0||m.reason===null))throw Error("入力の取得状態が不正です。");
    return m as MicrophoneStatus;
}
function checkedDelivery(value:unknown,notice:InputNotice):NoticeDelivery{
  const d=record(value,["sequence","generation","leaseAgeMs","rendered","status","receiveId"]),status=input(d.status,notice.input);
  if(!integer(d.sequence,1)||!integer(d.generation,1)||typeof d.rendered!=="boolean"||!(d.leaseAgeMs===null||integer(d.leaseAgeMs))||
    (notice.recipientId===null)!==(d.leaseAgeMs===null)||(notice.recipientId===null?d.receiveId!==null:!owner(d.receiveId))||d.rendered&&notice.recipientId===null||
    notice.interruptions!==Math.floor(Number(d.sequence)/2)+Number(d.sequence)%2||notice.kind!==(Number(d.sequence)%2?"interrupted":"recovered")||
    status.interruptions!==notice.interruptions||(notice.kind==="recovered"?status.state!=="connected":!["disconnected","unavailable"].includes(status.state)))throw Error("通知の履歴・受付世代または表示確認が不正です。");
  return {...d,status} as NoticeDelivery;
}
/** An actual past event, without a current-input projection or report age. */
export function checkedHistoricalInputNotice(value:unknown,expectedOwner:string):InputNotice {
  const n=record(value,["id","input","target","kind","interruptions","recipientId","delivery"]);
  if(!owner(expectedOwner)||(n.input!=="microphone"&&n.input!=="system")||n.target!=="main"||
    (n.kind!=="interrupted"&&n.kind!=="recovered")||!integer(n.interruptions,1)||
    n.id!==`${expectedOwner}:${n.input}:${n.interruptions}:${n.kind}`||!(n.recipientId===null||owner(n.recipientId)))throw Error("保存後通知のイベントまたは所有が不正です。");
  const delivery=checkedDelivery(n.delivery,n as InputNotice);
  return {...n,delivery:{...delivery,status:{...delivery.status}}} as InputNotice;
}
export function checkedInputStatus(v:unknown,expectedOwner:string):InputStatus{
  const queued=!!v&&typeof v==="object"&&(v as Record<string,unknown>).schemaVersion===4;
  const r=record(v,["schemaVersion","ownerId","revision","reportAgeMs","microphone","system","notices",...(queued?["noticeLoss"]:[])]);
  if(![3,4].includes(Number(r.schemaVersion))||!owner(r.ownerId)||r.ownerId!==expectedOwner||!integer(r.revision,1)||!integer(r.reportAgeMs))throw Error("入力状態の対象または版が不正です。");
  const microphone=input(r.microphone,"microphone"),system=r.system===null?null:input(r.system,"system");
  if(!Array.isArray(r.notices)||r.notices.length>2)throw Error("入力通知の形式が不正です。");
  const seen=new Set<InputKind>(),notices=r.notices.map(value=>{
    const n=record(value,["id","input","target","kind","interruptions","recipientId",...(queued?["delivery"]:[])]);
    if(n.input!=="microphone"&&n.input!=="system"||seen.has(n.input as InputKind))throw Error("入力通知の対象が不正です。");
    const m=n.input==="microphone"?microphone:system;seen.add(n.input as InputKind);
    if(!m||typeof n.target!=="string"||!["main","recorder"].includes(n.target)||typeof n.kind!=="string"||!["interrupted","recovered"].includes(n.kind)||!integer(n.interruptions,1)||
      n.id!==`${r.ownerId}:${n.input}:${n.interruptions}:${n.kind}`||!(n.recipientId===null||owner(n.recipientId))||
      (queued?Number(n.interruptions)>m.interruptions:n.interruptions!==m.interruptions||m.state==="stopped"||(n.kind==="recovered")!==(m.state==="connected")))throw Error("入力通知と取得状態が一致しません。");
    if(queued){const d=checkedDelivery(n.delivery,n as InputNotice);if(d.status.sourceEpoch>m.sourceEpoch)throw Error("通知の取得世代が範囲外です。");n.delivery=d;}
    return n as InputNotice;
  });
  let noticeLoss:NoticeLoss|undefined;
  if(queued){const loss=record(r.noticeLoss,["microphone","system"]);if(!integer(loss.microphone)||!integer(loss.system)||Number(loss.microphone)>microphone.interruptions*2||Number(loss.system)>(system?.interruptions??0)*2)throw Error("通知履歴の保持範囲が不正です。");noticeLoss=loss as NoticeLoss;}
  return {schemaVersion:queued?4:3,ownerId:r.ownerId as string,revision:r.revision as number,reportAgeMs:r.reportAgeMs as number,microphone,system,notices,...(queued?{noticeLoss}:{})};
}
export function checkedInputNoticeReceipt(value:unknown,expectedOwner:string,noticeId:string,target:"main"|"recorder",expected?:InputNotice):InputNoticeReceipt{
  const r=record(value,["schemaVersion","ownerId","noticeId","target","recipientId",...(expected?.delivery?["delivery"]:[])]);
  if(r.schemaVersion!==(expected?.delivery?2:1)||!owner(r.ownerId)||r.ownerId!==expectedOwner||r.noticeId!==noticeId||r.target!==target||!owner(r.recipientId))throw Error("入力通知の受付結果が一致しません。");
  if(expected?.delivery){const d=checkedDelivery(r.delivery,{...expected,recipientId:r.recipientId as string});if(d.sequence!==expected.delivery.sequence||d.generation!==expected.delivery.generation||!sameInput(d.status,expected.delivery.status))throw Error("通知受付の固定履歴が一致しません。");r.delivery=d;}
  return r as InputNoticeReceipt;
}
function backwards(m:MicrophoneStatus,old:MicrophoneStatus){return m.sourceEpoch<old.sourceEpoch||m.interruptions<old.interruptions||old.state==="stopped"&&!sameInput(m,old)||m.state==="connected"&&(["disconnected","unavailable"].includes(old.state)||m.interruptions>old.interruptions)&&m.sourceEpoch<=old.sourceEpoch;}
export class RecordingInputSession{
  private owner:string|null=null;private snapshot:InputStatus|null=null;private at=0;private age=0;private firstAt=0;private firstAge=0;private lastRead=0;private invalid=false;
  private floors=new Map<InputKind,InputNotice>();
  reset(value:string|null){if(value!==null&&!owner(value))throw Error("入力状態の録音IDが不正です。");this.owner=value;this.snapshot=null;this.at=0;this.age=0;this.firstAt=0;this.firstAge=0;this.lastRead=0;this.invalid=false;this.floors.clear();}
  queued(){return this.snapshot?.schemaVersion===4;}
  loss():NoticeLoss{return this.snapshot?.noticeLoss??{microphone:0,system:0};}
  accept(value:unknown,at:number){
    if(!this.owner||this.invalid||!time(at)||at<this.at)throw Error("入力状態の時計または対象が不正です。");
    const next=checkedInputStatus(value,this.owner),prior=this.snapshot;
    if(prior&&next.revision<prior.revision)return false;
    if(prior){
      if(next.schemaVersion!==prior.schemaVersion||next.noticeLoss&&(next.noticeLoss.microphone<(prior.noticeLoss?.microphone??0)||next.noticeLoss.system<(prior.noticeLoss?.system??0)))throw Error("通知の形式または保持範囲が逆行しました。");
      if(backwards(next.microphone,prior.microphone)||!!next.system!==!!prior.system||next.system&&prior.system&&backwards(next.system,prior.system))throw Error("入力の取得世代が逆行しました。");
      for(const notice of next.notices){const old=notice.delivery?this.floors.get(notice.input):prior.notices.find(n=>n.id===notice.id);
        if(notice.delivery&&old?.delivery){const d=notice.delivery,o=old.delivery;
          if(d.sequence<o.sequence||d.sequence===o.sequence&&(d.generation<o.generation||!sameInput(d.status,o.status)||
            o.rendered&&!d.rendered||d.generation===o.generation&&(notice.target!==old.target||old.recipientId!==null&&notice.recipientId!==old.recipientId)))throw Error("通知の履歴・受付先または表示確認が逆行しました。");
        }else if(old?.recipientId&&notice.recipientId!==old.recipientId)throw Error("入力通知の受付先が変更されました。");}
      if(next.revision===prior.revision){
        if(!sameInput(next.microphone,prior.microphone)||!sameInput(next.system,prior.system)||next.reportAgeMs<this.age)throw Error("同じ入力状態の版が変更されました。");
      }else{this.firstAt=at;this.firstAge=next.reportAgeMs;}
    }else{this.firstAt=at;this.firstAge=next.reportAgeMs;}
    this.snapshot=next;for(const n of next.notices)if(n.delivery)this.floors.set(n.input,n);this.at=at;this.age=next.reportAgeMs;return true;
  }
  read(at:number):{microphone:VisibleInput;system:VisibleInput;notices:InputNotice[]}{
    if(!time(at)||at<this.lastRead||at<this.at)this.invalid=true;this.lastRead=at;
    if(this.invalid||!this.snapshot)return {microphone:{state:"unknown"},system:{state:"unknown"},notices:[]};
    if(Math.max(this.age+at-this.at,this.firstAge+at-this.firstAt)>1000)return {microphone:{state:"stale"},system:{state:"stale"},notices:[]};
    return {microphone:this.snapshot.microphone,system:this.snapshot.system??{state:"unknown"},notices:this.snapshot.notices.map(n=>n.delivery?{...n,delivery:{...n.delivery,leaseAgeMs:n.delivery.leaseAgeMs===null?null:Math.min(Number.MAX_SAFE_INTEGER,n.delivery.leaseAgeMs+at-this.at)}}:n)};
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
    ({identity_changed:"機器の識別が変更",engine_changed:"音声エンジンが変更",device_disconnected:"選択した機器が未接続",identity_ambiguous:"機器を一意に確認できません",metadata_unavailable:"機器情報を確認できません",restart_failed:"再取得に失敗",format_changed:"入力形式が変更",clock_changed:"スリープ等により取得時計が変更",stream_stopped:"取得処理が停止",application_unavailable:"選択したアプリを確認できません",acquisition_timeout:"取得処理の応答待ち"} as Record<string,string>)[m.reason]??"取得条件を確認してください";
  return `${label}（${detail}）`;
}
