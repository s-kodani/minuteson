import {checkedInputNoticeReceipt,validInputOwner,type InputKind,type InputNotice} from "./recording-input-state.ts";
export type NoticeReception="none"|"pending"|"unconfirmed"|"received";
export type QueueAction="receive"|"render"|"close";
export type QueueAttempt={sessionId:string;noticeId:string;recipientId:string;generation:number;receiveId:string;target:"main"|"recorder";input:InputKind;action:QueueAction;notice:InputNotice};
type Head={notice:InputNotice;active:boolean;shown:boolean;renderIntent:boolean;confirmed:boolean;dismissed:boolean;closed:boolean;pending:Map<QueueAction,QueueAttempt>;last:Map<QueueAction,number>;failed:Map<QueueAction,QueueAttempt>};
const inputs:InputKind[]=["microphone","system"];
const same=(a:InputNotice,b:InputNotice)=>a.id===b.id&&a.delivery?.generation===b.delivery?.generation;
const leased=(n:InputNotice)=>n.delivery?.leaseAgeMs!==null&&n.delivery?.leaseAgeMs!==undefined&&n.delivery.leaseAgeMs<5000;

/** One WebView lifetime. Authoritative truth is at the recorder; no local seen
 * watermark is set by receipt alone. Pending RPC slots survive failed reads. */
export class RecordingInputQueue{
  private heads=new Map<InputKind,Head>();private lastAt=0;private invalid=false;
  readonly sessionId:string;
  readonly recipientId:string;
  readonly target:"main"|"recorder";
  constructor(sessionId:string,recipientId:string,target:"main"|"recorder"){
    if(!validInputOwner(sessionId)||!validInputOwner(recipientId)||!["main","recorder"].includes(target))throw Error("通知キューの所有が不正です。");
    this.sessionId=sessionId;this.recipientId=recipientId;this.target=target;
  }
  hasPending(){return [...this.heads.values()].some(h=>h.pending.size>0||h.active&&!h.closed&&h.failed.size>0);}
  private at(at:number){if(!Number.isFinite(at)||at<0||at>Number.MAX_SAFE_INTEGER||at<this.lastAt)this.invalid=true;this.lastAt=at;if(this.invalid)this.heads.clear();return !this.invalid;}
  observe(notices:InputNotice[],at:number):QueueAttempt[]{
    if(!this.at(at))return [];const attempts:QueueAttempt[]=[];
    for(const input of inputs){
      const notice=notices.find(n=>n.input===input&&n.target===this.target&&n.delivery),prior=this.heads.get(input);
      if(!notice){if(prior){prior.active=false;prior.shown=false;if(prior.confirmed)prior.dismissed=true;}continue;}
      let head=prior;
      if(!head||!same(head.notice,notice)){
        head={notice,active:true,shown:false,renderIntent:false,confirmed:false,dismissed:false,closed:false,pending:new Map(),last:new Map(),failed:new Map()};this.heads.set(input,head);
      }
      head.active=true;head.notice=notice;head.confirmed ||= notice.delivery!.rendered;
      const pending=head.pending.get("receive")??head.failed.get("receive");if(pending&&notice.delivery!.receiveId===pending.receiveId){head.pending.delete("receive");head.failed.delete("receive");}
      if(notice.delivery!.rendered){head.pending.delete("render");head.failed.delete("render");}
      const own=notice.recipientId===this.recipientId&&leased(notice);
      // Other recipients never renew the winning lease. An abandoned unshown
      // recipient becomes eligible only under a new backend generation.
      head.shown=own&&!head.dismissed&&!head.closed;
      if(head.closed||notice.recipientId!==null&&!own)continue;
      if(notice.recipientId===null||own&&!head.dismissed)this.request(head,"receive",at,attempts);
      if(own&&head.renderIntent&&!head.confirmed)this.request(head,"render",at,attempts);
      if(own&&head.dismissed&&head.confirmed)this.request(head,"close",at,attempts);
    }
    return attempts;
  }
  private request(head:Head,action:QueueAction,at:number,result:QueueAttempt[]){
    if(head.pending.has(action)||at-(head.last.get(action)??-1000)<1000)return;
    const n=head.notice,a:QueueAttempt={sessionId:this.sessionId,noticeId:n.id,recipientId:this.recipientId,generation:n.delivery!.generation,receiveId:crypto.randomUUID(),target:this.target,input:n.input,action,notice:n};
    head.pending.set(action,a);head.last.set(action,at);result.push(a);
  }
  complete(attempt:QueueAttempt,value:unknown,at:number){
    if(!this.at(at))return false;const head=this.heads.get(attempt.input);
    if(!head||!same(head.notice,attempt.notice)||head.pending.get(attempt.action)!==attempt)return false;
    const r=checkedInputNoticeReceipt(value,this.sessionId,attempt.noticeId,this.target,attempt.notice);
    if(attempt.action==="receive"&&r.recipientId===this.recipientId&&r.delivery?.receiveId!==attempt.receiveId)throw Error("通知受付の操作IDが一致しません。");
    if(attempt.action!=="receive"&&(r.recipientId!==this.recipientId||!r.delivery?.rendered))throw Error("通知の表示・終了確認が一致しません。");
    head.pending.delete(attempt.action);head.failed.delete(attempt.action);
    // A lost source read cannot create optimistic display. Reconcile the own
    // receipt, but wait for fresh source readback before presenting its toast.
    if(attempt.action==="receive"&&head.active){head.notice={...head.notice,recipientId:r.recipientId,delivery:r.delivery};head.shown=r.recipientId===this.recipientId&&!head.dismissed;}
    if(attempt.action!=="receive"){head.confirmed=true;if(attempt.action==="close"){head.closed=true;head.shown=false;}}
    return r.recipientId===this.recipientId;
  }
  defer(attempt:QueueAttempt){const h=this.heads.get(attempt.input);if(h?.pending.get(attempt.action)===attempt)h.pending.delete(attempt.action);}
  fail(attempt:QueueAttempt){const h=this.heads.get(attempt.input);if(h?.pending.get(attempt.action)===attempt){h.pending.delete(attempt.action);h.failed.set(attempt.action,attempt);}}
  rendered(id:string,generation:number,at:number){
    if(!this.at(at))return;for(const h of this.heads.values())if(h.active&&h.shown&&h.notice.id===id&&h.notice.delivery?.generation===generation&&h.notice.recipientId===this.recipientId&&leased(h.notice))h.renderIntent=true;
  }
  dismiss(input:InputKind){const h=this.heads.get(input);if(h){h.dismissed=true;h.shown=false;}}
  read(at:number):{notices:InputNotice[];reception:Record<InputKind,NoticeReception>}{
    this.at(at);const reception:Record<InputKind,NoticeReception>={microphone:"none",system:"none"};const notices:InputNotice[]=[];
    for(const [input,h]of this.heads){if(!h.active)continue;
      const waiting=[...h.pending.keys()].some(action=>at-(h.last.get(action)??at)>=1000);
      reception[input]=h.failed.size||waiting?"unconfirmed":h.notice.recipientId&&leased(h.notice)?"received":"pending";
      if(h.shown&&leased(h.notice))notices.push(h.notice);
    }
    return {notices,reception};
  }
}
