import {checkedInputNoticeReceipt,validInputOwner,type InputKind,type InputNotice} from "./recording-input-state.ts";
export type NoticeAttempt={sessionId:string;noticeId:string;recipientId:string;target:"main"|"recorder";input:InputKind};
export type NoticeReception="none"|"pending"|"unconfirmed"|"received";
const kinds:InputKind[]=["microphone","system"];
// One lifetime per recording and WebView. The backend owns the authoritative
// receipt; these bounded watermarks only suppress redisplay in this lifetime.
export class RecordingInputReceipts{
  private current=new Map<InputKind,InputNotice>();private shown=new Map<InputKind,InputNotice>();private pending=new Map<InputKind,NoticeAttempt>();
  private seen={microphone:{interrupted:0,recovered:0},system:{interrupted:0,recovered:0}};
  private lastAttempt=new Map<InputKind,number>();private waiting=new Map<InputKind,number>();private failed=new Set<InputKind>();private lastAt=0;private invalid=false;
  readonly sessionId:string;readonly recipientId:string;readonly target:"main"|"recorder";
  constructor(sessionId:string,recipientId:string,target:"main"|"recorder"){
    if(!validInputOwner(sessionId)||!validInputOwner(recipientId)||!["main","recorder"].includes(target))throw Error("入力通知の録音または受付先が不正です。");
    this.sessionId=sessionId;this.recipientId=recipientId;this.target=target;
  }
  private at(value:number){if(!Number.isFinite(value)||value<0||value>Number.MAX_SAFE_INTEGER||value<this.lastAt)this.invalid=true;this.lastAt=value;if(this.invalid){this.current.clear();this.shown.clear();this.pending.clear();return false;}return true;}
  observe(notices:InputNotice[],at:number):NoticeAttempt[]{
    if(!this.at(at))return [];const requests:NoticeAttempt[]=[];
    for(const input of kinds){
      const notice=notices.find(n=>n.input===input&&n.target===this.target),old=this.current.get(input);
      // A temporary read failure is not termination of the receipt RPC. Keep
      // its slot until completion or supersession by a different event.
      if(!notice){this.current.delete(input);this.shown.delete(input);this.waiting.delete(input);this.failed.delete(input);continue;}
      if(old?.id!==notice.id){this.shown.delete(input);if(this.pending.get(input)?.noticeId!==notice.id)this.pending.delete(input);this.waiting.set(input,at);this.failed.delete(input);}
      this.current.set(input,notice);
      if(notice.recipientId!==null){this.pending.delete(input);this.failed.delete(input);this.approve(notice,notice.recipientId);continue;}
      if(this.seen[input][notice.kind]>=notice.interruptions||this.pending.has(input))continue;
      const last=this.lastAttempt.get(input);if(last!==undefined&&at-last<1000)continue;
      const request={sessionId:this.sessionId,noticeId:notice.id,recipientId:this.recipientId,target:this.target,input};
      this.pending.set(input,request);this.lastAttempt.set(input,at);requests.push(request);
    }
    return requests;
  }
  private approve(notice:InputNotice,recipient:string){
    if(this.seen[notice.input][notice.kind]>=notice.interruptions)return;
    this.seen[notice.input][notice.kind]=notice.interruptions;
    if(recipient===this.recipientId)this.shown.set(notice.input,{...notice,recipientId:recipient});
    else this.shown.delete(notice.input);
  }
  complete(request:NoticeAttempt,value:unknown,at:number){
    if(!this.at(at)||this.pending.get(request.input)!==request)return false;
    const receipt=checkedInputNoticeReceipt(value,this.sessionId,request.noticeId,this.target);
    this.pending.delete(request.input);this.failed.delete(request.input);
    const notice=this.current.get(request.input);if(notice?.id!==request.noticeId)return false;
    this.approve(notice,receipt.recipientId);return receipt.recipientId===this.recipientId;
  }
  fail(request:NoticeAttempt){if(this.pending.get(request.input)===request){this.pending.delete(request.input);this.failed.add(request.input);}}
  dismiss(input:InputKind){this.shown.delete(input);}
  read(at:number):{notices:InputNotice[];reception:Record<InputKind,NoticeReception>}{
    this.at(at);const reception:Record<InputKind,NoticeReception>={microphone:"none",system:"none"};
    for(const input of kinds){const n=this.current.get(input);if(!n)continue;
      reception[input]=this.seen[input][n.kind]>=n.interruptions?"received":this.failed.has(input)||at-(this.waiting.get(input)??at)>=1000?"unconfirmed":"pending";
    }
    return {notices:[...this.shown.values()],reception};
  }
}
