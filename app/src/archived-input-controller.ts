import {ArchivedInputSession,checkedArchivedInputPage,checkedArchivedInputRows,type ArchivedInputTarget,type ArchivedInputRow} from "./archived-input-state.ts";
import {RecordingInputQueue,type QueueAttempt} from "./recording-input-queue.ts";
import type {InputKind,InputNotice,NoticeLoss} from "./recording-input-state.ts";
import type {NoticeReception} from "./recording-input-queue.ts";

type Entry={row:ArchivedInputRow;session:ArchivedInputSession;queue:RecordingInputQueue;fresh:boolean;seen:boolean;error:boolean;watch:boolean};
export type ArchivedAttempt={entry:object;meetingId:string;attempt:QueueAttempt};
export type ArchivedInputView={meetingId:string;sessionId:string;title:string;notices:InputNotice[];reception:Record<InputKind,NoticeReception>;loss:NoticeLoss;historyIssue:null|"missing"|"invalid";unavailable:boolean;historySaveIssue:ArchivedInputRow["historySaveIssue"];checked:boolean;historySaveConfirmed:boolean};

/** Main WebView lifetime, independent of the selected meeting. One paginated
 * traversal, bounded owners and identity-fenced asynchronous acknowledgements. */
export class ArchivedInputController {
  private entries=new Map<string,Entry>();
  private cursor:string|null=null;
  private paused=false;
  private recipientId:string;
  private readLog:(owner:string,error:unknown)=>void;
  constructor(recipientId:string,readLog:(owner:string,error:unknown)=>void=()=>{}){this.recipientId=recipientId;this.readLog=readLog;}
  after(){return this.cursor;}
  restart(){this.cursor=null;}
  targets():ArchivedInputTarget[]{return [...this.entries.values()].filter(e=>e.watch||e.queue.hasPending()).map(e=>({meetingId:e.row.meetingId,sessionId:e.row.sessionId}));}
  unavailable(id:string){return this.entries.get(id)?.error===true;}
  needsPaint(){return [...this.entries.values()].some(e=>e.fresh&&e.watch||e.queue.hasPending());}
  deferTargets(targets:ArchivedInputTarget[]){for(const t of targets){const e=this.entries.get(t.sessionId);if(e)e.fresh=false;}}
  failedTargets(targets:ArchivedInputTarget[]){for(const t of targets){const e=this.entries.get(t.sessionId);if(e){e.fresh=false;e.error=true;}}}
  acceptRows(value:unknown,targets:ArchivedInputTarget[],requestedAt:number){
    const rows=checkedArchivedInputRows(value,targets);if(this.paused)return;
    for(const id of rows.missingSessionIds)this.entries.delete(id);
    this.acceptRecordings(rows.recordings,requestedAt,false);
    return rows;
  }
  removeMeetings(ids:Set<string>){for(const [id,e]of this.entries)if(ids.has(e.row.meetingId))this.entries.delete(id);}
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
    this.acceptRecordings(page.recordings,requestedAt,true);
    this.cursor=page.nextCursor;
    // Only a complete authoritative traversal may forget absent owners. A
    // failed page keeps receipts and floors for a later traversal/readback.
    if(this.cursor===null)for(const [id,entry]of this.entries)if(!entry.seen)this.entries.delete(id);
  }
  private acceptRecordings(rows:ArchivedInputRow[],requestedAt:number,discovery:boolean){
    for(const row of rows){
      let entry=this.entries.get(row.sessionId);
      if(row.issue==="paused"){if(entry){if(discovery)entry.seen=true;entry.fresh=false;entry.error=false;}continue;}
      if(!entry&&row.batch===null&&row.issue===null&&!row.historySaveIssue)continue;
      if(!entry){
        if(this.entries.size===128)this.entries.delete(this.entries.keys().next().value!);
        entry={row,session:new ArchivedInputSession(row.sessionId),queue:new RecordingInputQueue(row.sessionId,this.recipientId,"main"),fresh:false,seen:true,error:false,watch:false};
      }
      // LRU order follows actual page use, never the UUID's magnitude.
      this.entries.delete(row.sessionId);this.entries.set(row.sessionId,entry);if(discovery)entry.seen=true;
      if(entry.row.meetingId!==row.meetingId){entry.fresh=false;entry.error=true;continue;}
      entry.watch=row.issue==="unavailable"||row.batch!==null&&row.batch.notices.length>0;
      entry.row=row;
      try{const accepted=row.batch!==null&&entry.session.accept(row.batch,requestedAt);entry.fresh=accepted&&row.issue===null;entry.error=row.issue==="unavailable";if(accepted)this.readLog(row.sessionId,null);}
      catch(error){entry.fresh=false;entry.error=true;this.readLog(row.sessionId,error);}
    }
  }
  paint(at:number):{views:ArchivedInputView[];attempts:ArchivedAttempt[]}{
    const views:ArchivedInputView[]=[],attempts:ArchivedAttempt[]=[];
    for(const entry of this.entries.values()){
      const raw=this.paused||!entry.fresh?[]:entry.session.read(at);
      for(const attempt of entry.queue.observe(raw,at))attempts.push({entry,meetingId:entry.row.meetingId,attempt});
      const queued=entry.queue.read(at),diagnostic=entry.session.diagnostics();
      views.push({meetingId:entry.row.meetingId,sessionId:entry.row.sessionId,title:entry.row.title,...queued,
        loss:diagnostic?.loss??{microphone:0,system:0},historyIssue:diagnostic?.issue??null,unavailable:entry.error,historySaveIssue:entry.row.historySaveIssue,
        checked:!this.paused&&entry.fresh&&!entry.error&&entry.row.issue===null,
        // Final persistence is independent of a failed notification queue.
        historySaveConfirmed:!this.paused&&entry.row.issue!=="paused"&&entry.row.historySaveIssue===null});
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
