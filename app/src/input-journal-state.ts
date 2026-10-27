// Writer results are neither input connectivity nor audio durability. Keep
// producer heartbeat and journal revision domains independent and bounded.
import {validInputOwner} from "./recording-input-state.ts";
export type JournalPersistence={requestedRevision:number;committedRevision:number;state:"confirmed"|"pending"|"failed"|"invalid";closed:boolean};
type Report={schemaVersion:1;ownerId:string;revision:number;reportAgeMs:number;persistence:JournalPersistence};
export type JournalView={state:"unknown"|"stale"|JournalPersistence["state"];problem:boolean};
const integer=(v:unknown,min=0):v is number=>typeof v==="number"&&Number.isSafeInteger(v)&&v>=min;
const time=(v:number)=>Number.isFinite(v)&&v>=0&&v<=Number.MAX_SAFE_INTEGER;
const exact=(value:unknown,keys:string[])=>{if(!value||typeof value!=="object"||Array.isArray(value)||Object.keys(value).sort().join("|")!==keys.slice().sort().join("|"))throw Error("入力履歴の保存診断の形式が不正です。");return value as Record<string,unknown>;};
export function checkedJournalReport(value:unknown,owner:string):Report {
  const r=exact(value,["schemaVersion","ownerId","revision","reportAgeMs","persistence"]),p=exact(r.persistence,["requestedRevision","committedRevision","state","closed"]);
  if(!validInputOwner(owner)||r.schemaVersion!==1||r.ownerId!==owner||!integer(r.revision,1)||!integer(r.reportAgeMs)||
    !integer(p.requestedRevision,1)||!integer(p.committedRevision,1)||p.committedRevision>p.requestedRevision||
    typeof p.state!=="string"||!["confirmed","pending","failed","invalid"].includes(p.state)||typeof p.closed!=="boolean"||p.state==="confirmed"&&p.requestedRevision!==p.committedRevision)throw Error("入力履歴の保存診断の対象・版または結果が不正です。");
  return {schemaVersion:1,ownerId:owner,revision:r.revision,reportAgeMs:r.reportAgeMs,persistence:{requestedRevision:p.requestedRevision,committedRevision:p.committedRevision,state:p.state as JournalPersistence["state"],closed:p.closed}};
}
export class InputJournalSession {
  private owner:string;
  private report:Report|null=null;private at=0;private firstAt=0;private firstAge=0;private readAt=0;private invalid=false;private failed=false;private problem=false;
  constructor(owner:string){if(!validInputOwner(owner))throw Error("入力履歴診断の録音IDが不正です。");this.owner=owner;}
  accept(value:unknown,at:number){
    if(this.invalid||!time(at)||at<this.at)throw Error("入力履歴診断の時計が不正です。");
    const next=checkedJournalReport(value,this.owner),old=this.report;
    if(old){const p=next.persistence,o=old.persistence;
      if(next.revision<old.revision||p.requestedRevision<o.requestedRevision||p.committedRevision<o.committedRevision||o.closed&&!p.closed||o.state==="invalid"&&p.state!=="invalid")throw Error("入力履歴の保存診断が逆行しました。");
      if(next.revision===old.revision&&(JSON.stringify(p)!==JSON.stringify(o)||next.reportAgeMs<old.reportAgeMs))throw Error("同じ入力履歴診断の版が変更されました。");
    }
    if(!old||next.revision!==old.revision){this.firstAt=at;this.firstAge=next.reportAgeMs;}
    this.report=next;this.at=at;this.failed=false;
    if(["failed","invalid"].includes(next.persistence.state))this.problem=true;
  }
  fail(){this.failed=true;}
  read(at:number):JournalView {
    if(!time(at)||at<this.at||at<this.readAt)this.invalid=true;this.readAt=at;
    if(this.invalid||this.failed||!this.report)return {state:"unknown",problem:this.problem};
    if(Math.max(this.report.reportAgeMs+at-this.at,this.firstAge+at-this.firstAt)>1000)return {state:"stale",problem:this.problem};
    // Only a fresh, explicit confirmed result clears a prior write failure.
    if(this.report.persistence.state==="confirmed")this.problem=false;
    return {state:this.report.persistence.state,problem:this.problem};
  }
}
export function journalMessage(view:JournalView):string {
  const detail={unknown:"マイク・PC音声の接続記録について、保存結果を読み取れません",stale:"マイク・PC音声の接続記録について、新しい保存結果が届いていません",pending:"マイク・PC音声の接続記録を保存しています",failed:"マイク・PC音声の接続記録を保存できませんでした。保存処理でエラーが発生したため表示しています",invalid:"マイク・PC音声の接続記録に不整合が見つかり、記録の保存を停止しました",confirmed:""}[view.state];
  return detail?`${detail}。${view.problem&&!["failed","invalid"].includes(view.state)?"以前の接続記録の保存エラーは、まだ解消を確認できていません。":""}録音ファイルの保存結果とは別の確認です。録音終了後に再確認します。警告が続く場合は会議を開き、音声を再生して確認してください。`:"";
}
export type JournalNotice={message:string;level:"warning"|"error";eventId:string};
// Presentation hysteresis is independent of the one-second evidence expiry.
// Hold one occurrence through short recoveries and changing diagnostic states.
export class InputJournalNotices {
  private unavailableSince:number|null=null;
  private confirmedSince:number|null=null;
  private active:JournalNotice|null=null;
  private failure:"failed"|"invalid"|null=null;
  private sequence=0;
  private lastAt:number|null=null;
  read(view:JournalView,phase:string,at:number):JournalNotice|null {
    if(!time(at)||this.lastAt!==null&&(at<this.lastAt||at-this.lastAt>1000))this.confirmedSince=null;
    this.lastAt=at;
    if(phase!=="recording"){
      this.unavailableSince=null;this.confirmedSince=null;this.active=null;this.failure=null;
      return null;
    }
    if(view.state==="failed"||view.state==="invalid"||view.problem){
      this.confirmedSince=null;
      const failure=view.state==="invalid"?"invalid":"failed";
      if(this.failure===null||failure==="invalid"&&this.failure!=="invalid"){
        this.failure=failure;
        this.active={message:journalMessage({state:failure,problem:true}),level:"error",eventId:String(++this.sequence)};
      }
    }else if(view.state==="confirmed"&&time(at)){
      this.unavailableSince=null;
      this.confirmedSince??=at;
      if(at-this.confirmedSince>=2000){this.active=null;this.failure=null;}
    }else{
      this.confirmedSince=null;
      if(time(at)){
        this.unavailableSince??=at;
        if(!this.active&&at-this.unavailableSince>=5000){
          this.active={message:"マイクやPCの音が途切れたかを調べるための記録について、保存結果を5秒以上確認できていないため表示しています。保存の完了待ち、または確認用の情報を読み取れない・更新されない状態です。自動で再確認します。この通知だけで録音ファイルの保存失敗を意味するものではありません。",level:"warning",eventId:String(++this.sequence)};
        }
      }
    }
    return this.active;
  }
}
