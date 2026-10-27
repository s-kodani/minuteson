import {ArchivedInputController} from "./archived-input-controller.ts";
import {unpackNoticeReply,type ArchivedInputTarget} from "./archived-input-state.ts";

type Clock={now:()=>number;start:(run:()=>void,delay:number)=>unknown;stop:(timer:unknown)=>void};
type Watch={target:ArchivedInputTarget;due:number;failures:number};
type Request={kind:"page";after:string|null}|{kind:"rows";targets:ArchivedInputTarget[]};
const delay=(failures:number)=>Math.min(60_000,1000*2**Math.min(6,failures-1));

/** One read slot for both discovery and delivery readback. Events invalidate
 * snapshots; they never fabricate receipts. Timers sleep completely at idle. */
export class ArchivedInputScheduler {
  private watches=new Map<string,Watch>();
  private timer:unknown=null;
  private running=false;private started=false;private disposed=false;private blocked=true;
  private epoch=0;private scanning=false;private pageDue=Infinity;private dirtyDue:number|null=null;
  private scanFailures=0;private last:"page"|"rows"="rows";
  private clockError=false;private scanError=false;private rowErrors=new Set<string>();
  private controller:ArchivedInputController;private clock:Clock;
  private read:(request:Request)=>Promise<unknown>;private changed:(reading:boolean,error:string)=>void;
  constructor(controller:ArchivedInputController,clock:Clock,read:(request:Request)=>Promise<unknown>,changed:(reading:boolean,error:string)=>void){
    this.controller=controller;this.clock=clock;this.read=read;this.changed=changed;
  }
  start(blocked:boolean){this.started=true;this.setBlocked(blocked);this.invalidate(true);}
  setBlocked(blocked:boolean){
    if(this.disposed||this.blocked===blocked)return;
    this.blocked=blocked;this.epoch++;this.controller.pause(blocked);
    if(!blocked){this.scanning=false;this.controller.restart();this.pageDue=this.clock.now();this.dirtyDue=null;}
    this.changed(this.running,this.error());this.schedule();
  }
  invalidate(immediate=false){
    if(this.disposed)return;
    const at=this.clock.now();
    if(immediate){this.scanFailures=0;this.pageDue=at;for(const w of this.watches.values()){w.failures=0;w.due=at;}}
    const due=at+(immediate?0:250);
    this.dirtyDue=Math.min(this.dirtyDue??Infinity,due);this.schedule();
  }
  sync(){
    const targets=this.controller.targets(),ids=new Set(targets.map(t=>t.sessionId)),at=this.clock.now();
    for(const id of this.watches.keys())if(!ids.has(id)){this.watches.delete(id);this.rowErrors.delete(id);}
    for(const target of targets)if(!this.watches.has(target.sessionId))this.watches.set(target.sessionId,{target,due:at+500,failures:0});
    this.schedule();
  }
  dispose(){this.disposed=true;this.epoch++;if(this.timer!==null)this.clock.stop(this.timer);this.timer=null;}
  private error(){if(this.clockError)return "時計の変更により通知の受付期限を確認できません。アプリを再起動して通知履歴を確認してください。";return this.scanError||this.rowErrors.size?"終了済み録音の通知履歴を確認できません。保存された音声の成否を示す診断ではありません。":"";}
  private schedule(){
    if(this.timer!==null){this.clock.stop(this.timer);this.timer=null;}
    if(!this.started||this.disposed||this.blocked||this.running||this.clockError)return;
    if(!Number.isFinite(this.clock.now())){this.clockError=true;this.controller.failedRead();this.changed(false,this.error());return;}
    const full=this.scanning?this.pageDue:Math.min(this.pageDue,this.dirtyDue??Infinity);
    const next=Math.min(full,...[...this.watches.values()].map(w=>w.due));
    if(Number.isFinite(next))this.timer=this.clock.start(()=>{this.timer=null;void this.poll();},Math.max(0,next-this.clock.now()));
  }
  private async poll(){
    if(this.disposed||this.blocked||this.running)return;
    const at=this.clock.now();
    if(!Number.isFinite(at)){this.clockError=true;this.controller.failedRead();this.changed(false,this.error());return;}
    const pageReady=this.pageDue<=at||!this.scanning&&this.dirtyDue!==null&&this.dirtyDue<=at;
    const due=[...this.watches.values()].filter(w=>w.due<=at).sort((a,b)=>a.due-b.due);
    if(!pageReady&&!due.length){this.schedule();return;}
    const kind=pageReady&&(!due.length||this.last==="rows")?"page":"rows";
    if(kind==="page"&&!this.scanning){this.scanning=true;this.controller.restart();this.dirtyDue=null;}
    const request:Request=kind==="page"?{kind,after:this.controller.after()}:{kind,targets:due.slice(0,32).map(w=>w.target)};
    const epoch=this.epoch;this.running=true;this.last=kind;this.changed(true,this.error());
    try{
      const reply=unpackNoticeReply(await this.read(request));
      if(this.disposed||this.blocked||epoch!==this.epoch)return;
      const now=this.clock.now();
      if(reply.paused){
        if(request.kind==="page"){this.controller.defer();this.pageDue=now+1000;}
        else {this.controller.deferTargets(request.targets);for(const t of request.targets){const w=this.watches.get(t.sessionId);if(w)w.due=now+1000;}}
        return;
      }
      if(request.kind==="page"){
        this.controller.acceptPage(reply.value,request.after,at);this.scanFailures=0;this.scanError=false;
        this.scanning=this.controller.after()!==null;this.pageDue=now+(this.scanning?50:60_000);
      }else{
        const rows=this.controller.acceptRows(reply.value,request.targets,at)!;
        for(const t of request.targets){
          const w=this.watches.get(t.sessionId);if(!w)continue;
          const row=rows.recordings.find(r=>r.sessionId===t.sessionId);
          if(this.controller.unavailable(t.sessionId)){w.failures++;w.due=now+delay(w.failures);this.rowErrors.add(t.sessionId);}
          else {w.failures=0;w.due=now+(row?.issue==="paused"?1000:500);this.rowErrors.delete(t.sessionId);}
        }
      }
    }catch{
      if(this.disposed||this.blocked||epoch!==this.epoch)return;
      const now=this.clock.now();
      if(request.kind==="page"){
        this.controller.failedRead();this.scanning=false;this.scanError=true;this.scanFailures++;
        this.pageDue=now+delay(this.scanFailures);this.dirtyDue=null;
      }else{
        this.controller.failedTargets(request.targets);
        for(const t of request.targets){const w=this.watches.get(t.sessionId);if(w){w.failures++;w.due=now+delay(w.failures);this.rowErrors.add(t.sessionId);}}
      }
    }finally{
      this.running=false;
      if(!this.disposed){this.sync();this.changed(false,this.error());this.schedule();}
    }
  }
}
