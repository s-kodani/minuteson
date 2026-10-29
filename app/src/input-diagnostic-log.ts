import {invoke} from "./ipc.ts";

export type InputReadStage="live-journal"|"live-journal-notice"|"archived-page"|"archived-rows"|"archived-validation"|"archived-window"|"archived-subscribe";
/** Keep polling cheap: record changed failures and recovery, never every poll. */
export class InputDiagnosticLog {
  private failures=new Map<string,string>();
  private send:(payload:object)=>Promise<unknown>;
  constructor(send:(payload:object)=>Promise<unknown>=payload=>invoke("record_input_diagnostic_log",payload as Record<string,unknown>)){this.send=send;}
  report(stage:InputReadStage,owner:string|null,error:unknown,context:object={}) {
    const key=JSON.stringify([stage,owner]);
    const message=error===null?null:JSON.stringify({error:error instanceof Error?error.message:String(error),...context}).slice(0,4096);
    if(message===null&&!this.failures.has(key)||message!==null&&this.failures.get(key)===message)return;
    if(message===null)this.failures.delete(key);else {this.failures.set(key,message);if(this.failures.size>256)this.failures.delete(this.failures.keys().next().value!);}
    void this.send({stage,sessionId:owner,message}).catch(()=>{});
  }
}
