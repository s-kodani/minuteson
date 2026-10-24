import {ArchivedInputSession,checkedArchivedInputPage,type ArchivedInputRow} from "./archived-input-state.ts";
import {RecordingInputQueue,type QueueAttempt} from "./recording-input-queue.ts";
import type {InputKind,InputNotice,NoticeLoss} from "./recording-input-state.ts";
import type {NoticeReception} from "./recording-input-queue.ts";

type Entry={row:ArchivedInputRow;session:ArchivedInputSession;queue:RecordingInputQueue;fresh:boolean;seen:boolean;error:boolean};
export type ArchivedAttempt={entry:object;meetingId:string;attempt:QueueAttempt};
export type ArchivedInputView={meetingId:string;sessionId:string;title:string;notices:InputNotice[];reception:Record<InputKind,NoticeReception>;loss:NoticeLoss;historyIssue:null|"missing"|"invalid";unavailable:boolean;deliveryUnavailable:boolean;historySaveIssue:ArchivedInputRow["historySaveIssue"]};

/** Main WebView lifetime, independent of the selected meeting. One paginated
 * traversal, bounded owners and identity-fenced asynchronous acknowledgements. */
export class ArchivedInputController {
  private entries=new Map<string,Entry>();
  private cursor:string|null=null;
  private paused=false;
  private recipientId:string;
  constructor(recipientId:string){this.recipientId=recipientId;}
  after(){return this.cursor;}
  retainMeetings(ids:Set<string>){for(const [id,entry]of this.entries)if(!ids.has(entry.row.meetingId))this.entries.delete(id);}
  defer(request?:ArchivedAttempt){if(request){const entry=this.entries.get(request.attempt.sessionId);if(entry===request.entry){entry.queue.defer(request.attempt);entry.fresh=false;}}else for(const entry of this.entries.values())entry.fresh=false;}
  pause(paused:boolean){
    if(paused&&!this.paused)for(const entry of this.entries.values())entry.fresh=false;
    this.paused=paused;
  }
  failedRead(){for(const entry of this.entries.values()){entry.fresh=false;entry.error=true;}this.cursor=null;}
  acceptPage(value:unknown,after:string|null,requestedAt:number){
    if(after!==this.cursor)throw Error("保存後通知の続き位置が変更されています。");
    const page=checkedArchivedInputPage(value,after);
    if(this.paused)return;
    if(after===null)for(const entry of this.entries.values())entry.seen=false;
    for(const row of page.recordings){
      let entry=this.entries.get(row.sessionId);
      if(row.issue==="paused"){if(entry){entry.seen=true;entry.fresh=false;entry.error=false;}continue;}
      if(!entry&&row.batch===null&&row.issue===null&&!row.historySaveIssue)continue; // Legacy: no new local/disk ledger.
      if(!entry){
        if(this.entries.size===128)this.entries.delete(this.entries.keys().next().value!);
        entry={row,session:new ArchivedInputSession(row.sessionId),queue:new RecordingInputQueue(row.sessionId,this.recipientId,"main"),fresh:false,seen:true,error:false};
      }
      // LRU order follows actual page use, never the UUID's magnitude.
      this.entries.delete(row.sessionId);this.entries.set(row.sessionId,entry);entry.seen=true;
      if(entry.row.meetingId!==row.meetingId){entry.fresh=false;entry.error=true;continue;}
      // An old API's absent field is unknown, not evidence that a previously
      // reported final writer failure has been resolved.
      entry.row={...row,...(row.historySaveIssue===undefined&&entry.row.historySaveIssue!==undefined?{historySaveIssue:entry.row.historySaveIssue}:{})};
      try{const accepted=row.batch!==null&&entry.session.accept(row.batch,requestedAt);entry.fresh=accepted&&row.issue===null;entry.error=row.issue==="unavailable";}
      catch{entry.fresh=false;entry.error=true;}
    }
    this.cursor=page.nextCursor;
    // Only a complete authoritative traversal may forget absent owners. A
    // failed page keeps receipts and floors for a later traversal/readback.
    if(this.cursor===null)for(const [id,entry]of this.entries)if(!entry.seen)this.entries.delete(id);
  }
  paint(at:number):{views:ArchivedInputView[];attempts:ArchivedAttempt[]}{
    const views:ArchivedInputView[]=[],attempts:ArchivedAttempt[]=[];
    for(const entry of this.entries.values()){
      const raw=this.paused||!entry.fresh?[]:entry.session.read(at);
      for(const attempt of entry.queue.observe(raw,at))attempts.push({entry,meetingId:entry.row.meetingId,attempt});
      const queued=entry.queue.read(at),diagnostic=entry.session.diagnostics();
      views.push({meetingId:entry.row.meetingId,sessionId:entry.row.sessionId,title:entry.row.title,...queued,
        loss:diagnostic?.loss??{microphone:0,system:0},historyIssue:diagnostic?.issue??null,unavailable:entry.error,deliveryUnavailable:entry.row.issue==="deliveryUnavailable",historySaveIssue:entry.row.historySaveIssue??null});
    }
    return {views,attempts};
  }
  complete(request:ArchivedAttempt,value:unknown,at:number){
    const entry=this.entries.get(request.attempt.sessionId);
    if(entry!==request.entry)return false;
    // A failed/paused/stale read must not be resurrected by a late receipt.
    if(this.paused||!entry.fresh||entry.session.read(at).length===0)entry.queue.observe([],at);
    return entry.queue.complete(request.attempt,value,at);
  }
  fail(request:ArchivedAttempt){const entry=this.entries.get(request.attempt.sessionId);if(entry===request.entry)entry.queue.fail(request.attempt);}
  rendered(owner:string,id:string,generation:number,at:number){
    const entry=this.entries.get(owner);if(!entry||this.paused||!entry.fresh)return;
    // Recheck age before accepting a callback captured by an old render.
    const raw=entry.session.read(at);
    if(!raw.some(n=>n.id===id&&n.delivery?.generation===generation&&n.delivery.leaseAgeMs!==null&&n.delivery.leaseAgeMs<5000))return;
    entry.queue.rendered(id,generation,at);
  }
  dismiss(owner:string,input:InputKind){this.entries.get(owner)?.queue.dismiss(input);}
}
