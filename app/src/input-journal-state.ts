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
// Keep one diagnostic occurrence across short recoveries and unavailable reads.
export class InputJournalFailures {
  private confirmedSince:number|null=null;
  private failure:"failed"|"invalid"|null=null;
  private lastAt:number|null=null;
  read(view:JournalView,phase:string,at:number):"failed"|"invalid"|null {
    if(!time(at)||this.lastAt!==null&&(at<this.lastAt||at-this.lastAt>1000))this.confirmedSince=null;
    this.lastAt=at;
    if(phase!=="recording"){
      this.confirmedSince=null;this.failure=null;
      return null;
    }
    if(view.state==="failed"||view.state==="invalid"||view.problem){
      this.confirmedSince=null;
      if(view.state==="invalid")this.failure="invalid";
      else this.failure??="failed";
    }else if(view.state==="confirmed"&&time(at)){
      this.confirmedSince??=at;
      if(at-this.confirmedSince>=2000)this.failure=null;
    }else{
      this.confirmedSince=null;
    }
    return this.failure;
  }
}
