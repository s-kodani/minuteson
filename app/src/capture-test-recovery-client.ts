import {validCaptureTestId} from "./capture-test-state.ts";
export type CaptureRecovery={schemaVersion:1;revision:number;phase:"checking"|"clear"|"attention";runId:string|null;errorMessage:string|null};
export function checkedCaptureRecovery(value:unknown):CaptureRecovery{
  if(!value||typeof value!=="object"||Array.isArray(value)||Object.keys(value).sort().join("|")!==["errorMessage","phase","revision","runId","schemaVersion"].join("|"))throw Error("テスト音声の回復状態の形式が不正です。");
  const v=value as CaptureRecovery;
  if(v.schemaVersion!==1||!Number.isSafeInteger(v.revision)||v.revision<1||!["checking","clear","attention"].includes(v.phase)||
    v.runId!==null&&!validCaptureTestId(v.runId)||v.errorMessage!==null&&(typeof v.errorMessage!=="string"||!v.errorMessage.trim()||v.errorMessage.includes("\0")||new TextEncoder().encode(v.errorMessage).length>16384)||
    (v.phase==="attention"?v.errorMessage===null:v.errorMessage!==null||v.runId!==null))throw Error("テスト音声の回復状態が一致しません。");
  return {schemaVersion:1,revision:v.revision,phase:v.phase,runId:v.runId,errorMessage:v.errorMessage};
}
type View={snapshot:CaptureRecovery|null;connected:boolean;commandBusy:boolean;error:string};
export class CaptureRecoveryClient{
  private view:View={snapshot:null,connected:false,commandBusy:false,error:""};
  private epoch=0;private reading:Promise<CaptureRecovery>|null=null;private listeners=new Set<()=>void>();
  private pendingRevision:number|null=null;
  private bridge:{read():Promise<unknown>;retry(expectedRevision:number):Promise<unknown>};
  constructor(bridge:{read():Promise<unknown>;retry(expectedRevision:number):Promise<unknown>}){this.bridge=bridge;}
  getSnapshot=()=>this.view;
  subscribe=(listener:()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
  private change(patch:Partial<View>){this.view={...this.view,...patch};for(const listener of this.listeners)listener();}
  private accept(value:unknown){
    const next=checkedCaptureRecovery(value),prior=this.view.snapshot;
    if(prior&&next.revision<prior.revision){this.change({connected:true,error:""});return prior;}
    if(prior&&next.revision===prior.revision&&JSON.stringify(next)!==JSON.stringify(prior))throw Error("同じ回復状態の版に異なる結果があります。");
    if(prior&&prior.phase!=="checking"&&next.phase==="checking")throw Error("回復状態が初期確認へ逆行しました。");
    this.change({snapshot:next,connected:true,error:""});return next;
  }
  blocked(){return !this.view.connected||this.view.commandBusy||this.view.snapshot?.phase!=="clear";}
  poll():Promise<CaptureRecovery>{
    if(this.reading)return this.reading;
    const epoch=this.epoch;
    const job=(async()=>{try{const value=await this.bridge.read();if(epoch!==this.epoch)return this.view.snapshot??checkedCaptureRecovery(value);const next=this.accept(value);
      // Native reads share the ownership mutex. A newer terminal revision is
      // proof that the bounded collector finished, even if its reply is lost
      // forever. Reading never resends that collection or starts another one.
      if(this.pendingRevision!==null&&next.revision>this.pendingRevision&&next.phase!=="checking"){
        this.pendingRevision=null;this.change({commandBusy:false});
      }
      return next;}
      catch(cause){if(epoch===this.epoch)this.change({connected:false,error:String(cause)});throw cause;}})();
    this.reading=job;void job.finally(()=>{if(this.reading===job)this.reading=null;}).catch(()=>{});return job;
  }
  async refresh(){if(this.reading)await this.reading.catch(()=>{});return this.poll();}
  async retry(expectedRevision:number){
    if(this.view.commandBusy)throw Error("回復確認の受付中です。");
    const current=await this.refresh();
    if(this.view.commandBusy||current.phase!=="attention"||current.revision!==expectedRevision)throw Error("回復の確認対象が変更されています。回収は開始していません。");
    const epoch=++this.epoch;this.pendingRevision=expectedRevision;this.change({commandBusy:true,error:""});
    try{const value=await this.bridge.retry(expectedRevision);
      if(epoch!==this.epoch||this.pendingRevision!==expectedRevision)return this.view.snapshot??checkedCaptureRecovery(value);
      const next=checkedCaptureRecovery(value);if(next.revision<=expectedRevision||next.phase==="checking")throw Error("回収の完了版を確認できません。");
      return this.accept(next);}
    catch(cause){if(epoch===this.epoch&&this.pendingRevision===expectedRevision)this.change({connected:false,error:`回収結果を再確認してください。回収は自動再送していません: ${String(cause)}`});throw cause;}
    finally{if(epoch===this.epoch){this.pendingRevision=null;this.change({commandBusy:false});}}
  }
}
