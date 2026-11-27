import {invoke} from "./ipc.ts";

export type InputReadStage="live-status"|"live-journal"|"live-journal-notice"|"live-levels"|"test-levels"|"archived-page"|"archived-rows"|"archived-validation"|"archived-window"|"archived-subscribe";
export type InputDiagnosticKind="failed"|"stalled"|"hung"|"clock-reversed"|"recovered";

/** Keep polling cheap: record changed failures and recovery, never every poll. */
export class InputDiagnosticLog {
  private failures=new Map<string,string>();
  private active=new Map<string,InputDiagnosticKind>();
  private counts=new Map<string,number>();
  private disposed=false;
  private windowLabel:"main"|"recorder"|null=null;
  private send:(payload:object)=>Promise<unknown>;
  constructor(send:(payload:object)=>Promise<unknown>=payload=>invoke("record_input_diagnostic_log",payload as Record<string,unknown>)){this.send=send;}
  bindWindow(label:"main"|"recorder"){this.windowLabel=label;}
  dispose(){this.disposed=true;}
  private key(stage:InputReadStage,owner:string|null){return stage==="live-levels"||stage==="test-levels"?JSON.stringify([stage,owner,this.windowLabel]):JSON.stringify([stage,owner]);}
  private allow(owner:string|null):boolean{if(!owner)return true;const count=this.counts.get(owner)??0;return count<40;}
  private remember(owner:string|null){if(!owner)return;this.counts.set(owner,(this.counts.get(owner)??0)+1);}
  report(stage:InputReadStage,owner:string|null,error:unknown,context:object={}) {
    if(this.disposed)return;
    const key=this.key(stage,owner);
    const message=error===null?null:JSON.stringify({error:error instanceof Error?error.message:String(error),...context}).slice(0,4096);
    if(message===null&&!this.failures.has(key)||message!==null&&this.failures.get(key)===message)return;
    if(message===null)this.failures.delete(key);else {if(!this.allow(owner))return;this.failures.set(key,message);this.remember(owner);if(this.failures.size>256)this.failures.delete(this.failures.keys().next().value!);}
    void this.send({stage,sessionId:owner,message}).catch(()=>{});
  }
  reportAnomaly(stage:InputReadStage,owner:string|null,kind:InputDiagnosticKind,detail:Record<string,unknown>={}){
    if(this.disposed||!this.allow(owner))return;
    const key=this.key(stage,owner);
    if(kind==="recovered"){if(!this.active.has(key))return;this.active.delete(key);this.report(stage,owner,null,detail);return;}
    if(this.active.get(key)===kind)return;
    this.active.set(key,kind);
    this.report(stage,owner,new Error(kind),{kind,lastRevision:detail.lastRevision??null,...detail});
  }
}
