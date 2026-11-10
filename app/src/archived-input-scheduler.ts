import {ArchivedInputController} from "./archived-input-controller.ts";
import {unpackNoticeReply,type ArchivedInputTarget} from "./archived-input-state.ts";

type Clock={now:()=>number;reset:()=>boolean;start:(run:()=>void,delay:number)=>unknown;stop:(timer:unknown)=>void};
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
  private clockUnavailable=false;private clockOffset=0;private lastTime=0;
  private scanError=false;private rowErrors=new Set<string>();
  private controller:ArchivedInputController;private clock:Clock;
  private read:(request:Request)=>Promise<unknown>;private changed:(reading:boolean,error:string)=>void;
  private readLog:(request:Request,error:unknown)=>void;
  constructor(controller:ArchivedInputController,clock:Clock,read:(request:Request)=>Promise<unknown>,changed:(reading:boolean,error:string)=>void,readLog:(request:Request,error:unknown)=>void=()=>{}){
    this.controller=controller;this.clock=clock;this.read=read;this.changed=changed;
    this.readLog=readLog;
  }
  now():number{
    const at=this.clockOffset+this.clock.now();
    if(at>=this.lastTime&&at<=Number.MAX_SAFE_INTEGER){
      this.lastTime=at;this.clockUnavailable=false;return at;
    }
    if(!this.clockUnavailable){
      // A new clock origin cannot make an old read fresh. Keep delivery
      // receipts and revision checks, but require a new native snapshot.
      this.clockUnavailable=true;this.epoch++;this.cancelTimer();
      this.controller.defer();this.restartScan(this.lastTime);
    }
    this.clockOffset=this.lastTime;this.clockUnavailable=!this.clock.reset();
    return this.lastTime;
  }
  private restartScan(at:number){
    this.scanning=false;this.controller.restart();this.pageDue=at;this.dirtyDue=null;
  }
  start(blocked:boolean){this.started=true;this.setBlocked(blocked);this.invalidate(true);}
  setBlocked(blocked:boolean){
    if(this.disposed||this.blocked===blocked)return;
    this.blocked=blocked;this.epoch++;this.controller.pause(blocked);
    if(!blocked)this.restartScan(this.now());
    this.changed(this.running,this.error());this.schedule();
  }
  invalidate(immediate=false){
    if(this.disposed)return;
    const at=this.now();
    if(immediate){this.scanFailures=0;this.pageDue=at;for(const w of this.watches.values()){w.failures=0;w.due=at;}}
    const due=at+(immediate?0:250);
    this.dirtyDue=Math.min(this.dirtyDue??Infinity,due);this.schedule();
  }
  sync(){
    const targets=this.controller.targets(),ids=new Set(targets.map(t=>t.sessionId)),at=this.now();
    for(const id of this.watches.keys())if(!ids.has(id)){this.watches.delete(id);this.rowErrors.delete(id);}
    for(const target of targets)if(!this.watches.has(target.sessionId))this.watches.set(target.sessionId,{target,due:at+500,failures:0});
    this.schedule();
  }
  dispose(){this.disposed=true;this.epoch++;this.cancelTimer();}
  private error(){return this.scanError||this.rowErrors.size?"終了した録音のマイク・PC音声の中断・再開履歴を確認できません。自動で再確認します。":"";}
  private cancelTimer(){
    if(this.timer!==null){this.clock.stop(this.timer);this.timer=null;}
  }
  private schedule(){
    if(!this.started||this.disposed||this.blocked||this.running){this.cancelTimer();return;}
    const at=this.now();
    if(this.clockUnavailable){
      this.timer??=this.clock.start(()=>{this.timer=null;this.schedule();},1000);
      return;
    }
    this.cancelTimer();
    const full=this.scanning?this.pageDue:Math.min(this.pageDue,this.dirtyDue??Infinity);
    const next=Math.min(full,...[...this.watches.values()].map(w=>w.due));
    if(Number.isFinite(next))this.timer=this.clock.start(()=>{this.timer=null;void this.poll();},Math.max(0,next-at));
  }
  private async poll(){
    if(this.disposed||this.blocked||this.running)return;
    const at=this.now();
    if(this.clockUnavailable){this.schedule();return;}
    const pageReady=this.pageDue<=at||!this.scanning&&this.dirtyDue!==null&&this.dirtyDue<=at;
    const due=[...this.watches.values()].filter(w=>w.due<=at).sort((a,b)=>a.due-b.due);
    if(!pageReady&&!due.length){this.schedule();return;}
    const kind=pageReady&&(!due.length||this.last==="rows")?"page":"rows";
    if(kind==="page"&&!this.scanning){this.scanning=true;this.controller.restart();this.dirtyDue=null;}
    const request:Request=kind==="page"?{kind,after:this.controller.after()}:{kind,targets:due.slice(0,32).map(w=>w.target)};
    const epoch=this.epoch;this.running=true;this.last=kind;this.changed(true,this.error());
    try{
      const value=await this.read(request);
      if(this.disposed||this.blocked||epoch!==this.epoch)return;
      const now=this.now();
      if(epoch!==this.epoch)return;
      const reply=unpackNoticeReply(value);
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
      this.readLog(request,null);
    }catch(error){
      if(this.disposed||this.blocked||epoch!==this.epoch)return;
      const now=this.now();
      if(epoch!==this.epoch)return;
      this.readLog(request,error);
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
