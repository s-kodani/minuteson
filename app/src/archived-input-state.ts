import {checkedHistoricalInputNotice,validInputOwner,type InputKind,type InputNotice,type NoticeLoss} from "./recording-input-state.ts";

export type ArchivedInputBatch={schemaVersion:1;ownerId:string;revision:number;notices:InputNotice[];noticeLoss:NoticeLoss;historyIssue:null|"missing"|"invalid"};
export type HistorySaveIssue=null|"failed"|"invalid"|"unconfirmed"|"unavailable";
export type ArchivedInputRow={meetingId:string;sessionId:string;title:string;batch:ArchivedInputBatch|null;issue:null|"unavailable"|"deliveryUnavailable";historySaveIssue?:HistorySaveIssue};
export type ArchivedInputPage={schemaVersion:1|2|3;recordings:ArchivedInputRow[];nextCursor:string|null};
const integer=(v:unknown,min=0):v is number=>typeof v==="number"&&Number.isSafeInteger(v)&&v>=min;
const time=(v:number)=>Number.isFinite(v)&&v>=0&&v<=Number.MAX_SAFE_INTEGER;
function object(v:unknown,keys:string[]):Record<string,unknown>{
  if(!v||typeof v!=="object"||Array.isArray(v)||Object.keys(v).sort().join("|")!==keys.slice().sort().join("|"))throw Error("保存後通知の形式が不正です。");
  return v as Record<string,unknown>;
}
const event=(n:InputNotice)=>JSON.stringify([n.id,n.input,n.kind,n.interruptions,n.delivery!.sequence,
  n.delivery!.status.state,n.delivery!.status.sourceEpoch,n.delivery!.status.interruptions,n.delivery!.status.reason]);
export function checkedArchivedInputBatch(value:unknown,owner:string):ArchivedInputBatch {
  const b=object(value,["schemaVersion","ownerId","revision","notices","noticeLoss","historyIssue"]);
  if(b.schemaVersion!==1||!validInputOwner(owner)||b.ownerId!==owner||!integer(b.revision,1)||
    ![null,"missing","invalid"].includes(b.historyIssue as null)||!Array.isArray(b.notices)||b.notices.length>2)throw Error("保存後通知の対象・版または診断が不正です。");
  const notices=b.notices.map(n=>checkedHistoricalInputNotice(n,owner));
  if(new Set(notices.map(n=>n.input)).size!==notices.length)throw Error("保存後通知の入力が重複しています。");
  const loss=object(b.noticeLoss,["microphone","system"]);
  if(!integer(loss.microphone)||!integer(loss.system))throw Error("保存後通知の保持不足が不正です。");
  return {schemaVersion:1,ownerId:owner,revision:b.revision,notices,noticeLoss:{microphone:loss.microphone as number,system:loss.system as number},historyIssue:b.historyIssue as ArchivedInputBatch["historyIssue"]};
}
export function checkedArchivedInputPage(value:unknown,after:string|null):ArchivedInputPage {
  const p=object(value,["schemaVersion","recordings","nextCursor"]);
  if((p.schemaVersion!==1&&p.schemaVersion!==2&&p.schemaVersion!==3)||after!==null&&!validInputOwner(after)||!Array.isArray(p.recordings)||p.recordings.length>32||
    p.nextCursor!==null&&!validInputOwner(p.nextCursor))throw Error("保存後通知一覧の版または続き位置が不正です。");
  let prior=after;
  const recordings=p.recordings.map(value=>{
    const r=object(value,["meetingId","sessionId","title","batch","issue",...(p.schemaVersion!==1?["historySaveIssue"]:[])]);
    if(p.schemaVersion!==1&&![null,"failed","invalid","unconfirmed","unavailable"].includes(r.historySaveIssue as null))throw Error("停止時の履歴保存診断が不正です。");
    if(!validInputOwner(r.meetingId)||!validInputOwner(r.sessionId)||prior!==null&&String(r.sessionId)<=prior||
      typeof r.title!=="string"||Array.from(r.title).length>120||![null,"unavailable",...(p.schemaVersion===3?["deliveryUnavailable"]:[])].includes(r.issue as null)||r.batch!==null&&r.issue==="unavailable")throw Error("保存後通知一覧の会議・録音または診断が不正です。");
    prior=r.sessionId as string;
    const batch=r.batch===null?null:checkedArchivedInputBatch(r.batch,r.sessionId as string);
    // Immutable imported retention can provide diagnosis, but never a notice
    // or a receipt until a writable destination delivery store is available.
    if(r.issue==="deliveryUnavailable"&&(batch===null||batch.notices.length!==0))throw Error("未接続の復元通知に受付可能な通知があります。");
    return {meetingId:r.meetingId as string,sessionId:r.sessionId as string,title:r.title,batch,issue:r.issue as ArchivedInputRow["issue"],...(p.schemaVersion!==1?{historySaveIssue:r.historySaveIssue as HistorySaveIssue}:{})};
  });
  if(p.nextCursor!==null&&(recordings.length!==32||p.nextCursor!==recordings.at(-1)?.sessionId))throw Error("保存後通知一覧の末尾と続き位置が一致しません。");
  return {schemaVersion:p.schemaVersion as 1|2|3,recordings,nextCursor:p.nextCursor as string|null};
}

/** Independent native delivery revision; never infer live connection from this.
 * Keep per-input event/claim floors even across absent/hidden projections. */
export class ArchivedInputSession {
  private batch:ArchivedInputBatch|null=null;
  private at=0;private readAt=0;private invalid=false;
  private floors=new Map<InputKind,InputNotice>();
  readonly owner:string;
  constructor(owner:string){if(!validInputOwner(owner))throw Error("保存後通知の録音IDが不正です。");this.owner=owner;}
  accept(value:unknown,requestedAt:number):boolean {
    if(this.invalid||!time(requestedAt)||requestedAt<this.at)throw Error("保存後通知の時計が不正です。");
    const next=checkedArchivedInputBatch(value,this.owner),old=this.batch;
    if(old&&next.revision<old.revision)return false;
    if(old&&(next.noticeLoss.microphone<old.noticeLoss.microphone||next.noticeLoss.system<old.noticeLoss.system))throw Error("保存後通知の保持不足が逆行しました。");
    for(const n of next.notices){const o=this.floors.get(n.input);if(!o)continue;
      const d=n.delivery!,before=o.delivery!;
      if(d.sequence<before.sequence||d.sequence===before.sequence&&(event(n)!==event(o)||d.generation<before.generation||before.rendered&&!d.rendered||
        d.generation===before.generation&&(o.recipientId!==null&&n.recipientId!==o.recipientId||before.rendered&&!d.rendered)))throw Error("保存後通知の固定履歴・受付または表示確認が逆行しました。");
      if(old&&next.revision===old.revision&&(d.sequence!==before.sequence||d.generation!==before.generation||n.recipientId!==o.recipientId||d.rendered!==before.rendered||d.receiveId!==before.receiveId))throw Error("同じ保存後通知の版で受付が変更されました。");
    }
    if(old&&next.revision===old.revision&&(next.historyIssue!==old.historyIssue||JSON.stringify(next.noticeLoss)!==JSON.stringify(old.noticeLoss)))throw Error("同じ保存後通知の版で診断が変更されました。");
    this.batch=next;this.at=requestedAt;for(const n of next.notices)this.floors.set(n.input,n);return true;
  }
  read(at:number):InputNotice[] {
    if(!time(at)||at<this.readAt||at<this.at)this.invalid=true;this.readAt=at;
    if(this.invalid||!this.batch||at-this.at>1500)return [];
    return this.batch.notices.map(n=>({...n,delivery:{...n.delivery!,status:{...n.delivery!.status},leaseAgeMs:n.delivery!.leaseAgeMs===null?null:Math.min(Number.MAX_SAFE_INTEGER,n.delivery!.leaseAgeMs+at-this.at)}}));
  }
  diagnostics(){return this.batch?{loss:{...this.batch.noticeLoss},issue:this.batch.historyIssue}:null;}
}
