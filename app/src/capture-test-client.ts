import {checkedCaptureTest,checkedCaptureTestRequest,reconcileCaptureTest,type CaptureTestRequest,type CaptureTestSnapshot} from "./capture-test-state.ts";
export interface CaptureTestBridge{
  read():Promise<unknown|null>;start(request:CaptureTestRequest):Promise<unknown>;
  cancel(runId:string):Promise<unknown>;discard(runId:string):Promise<void>;
}
export type CaptureTestView={snapshot:CaptureTestSnapshot|null;pending:CaptureTestRequest|null;connected:boolean;commandBusy:boolean;error:string};
// One start attempt is sent once. Read reconciliation never sends start, and
// delayed reads from before a command cannot overwrite its ownership epoch.
export class CaptureTestClient{
  private view:CaptureTestView={snapshot:null,pending:null,connected:false,commandBusy:false,error:""};
  private epoch=0;private reading:Promise<CaptureTestSnapshot|null>|null=null;
  private listeners=new Set<()=>void>();private used=new Set<string>();private bridge:CaptureTestBridge;
  constructor(bridge:CaptureTestBridge){this.bridge=bridge;}
  getSnapshot=()=>this.view;
  subscribe=(listener:()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
  private change(patch:Partial<CaptureTestView>){this.view={...this.view,...patch};for(const listener of this.listeners)listener();}
  private accept(value:unknown):CaptureTestSnapshot{
    const expected=this.view.pending??this.view.snapshot;
    const next=expected?reconcileCaptureTest(this.view.snapshot,value,expected):checkedCaptureTest(value);
    this.change({snapshot:next,pending:null,connected:true,error:""});return next;
  }
  blocked(){return !this.view.connected||this.view.commandBusy||!!this.view.pending||!!this.view.snapshot?.busy;}
  poll():Promise<CaptureTestSnapshot|null>{
    if(this.view.commandBusy)return Promise.resolve(this.view.snapshot);
    if(this.reading)return this.reading;
    const epoch=this.epoch;
    const job=(async()=>{
      try{
        const value=await this.bridge.read();
        if(epoch!==this.epoch)return this.view.snapshot;
        if(value===null){
          if(this.view.pending)throw Error("テストの受付結果を確認できません。開始は再送していません。");
          if(this.view.snapshot?.busy)throw Error("進行中のテストの停止結果を確認できません。");
          this.change({snapshot:null,connected:true,error:""});return null;
        }
        return this.accept(value);
      }catch(cause){if(epoch===this.epoch)this.change({connected:false,error:String(cause)});throw cause;}
    })();
    this.reading=job;
    void job.finally(()=>{if(this.reading===job)this.reading=null;}).catch(()=>{});
    return job;
  }
  async refresh(){
    if(this.reading)await this.reading.catch(()=>{});
    return this.poll();
  }
  async start(value:CaptureTestRequest){
    const request=checkedCaptureTestRequest(value);
    if(this.blocked()||this.view.snapshot||this.used.has(request.runId))throw Error("以前のテストが未確認または未解放です。開始は再送していません。");
    this.used.add(request.runId);this.epoch++;
    this.change({pending:structuredClone(request),commandBusy:true,error:""});
    try{return this.accept(await this.bridge.start(request));}
    catch(cause){this.change({connected:false,error:`テストの受付結果を再確認してください。開始は自動再送していません: ${String(cause)}`});throw cause;}
    finally{this.change({commandBusy:false});}
  }
  async cancel(runId:string){
    if(this.view.commandBusy)throw Error("テストの受付確認中です。結果を確認してから停止してください。");
    const current=await this.refresh();
    if(this.view.commandBusy)throw Error("別のテスト操作の受付を確認しています。");
    if(!current||current.runId!==runId)throw Error("停止するテストが変更されています。");
    if(!current.busy)return current;
    this.epoch++;this.change({commandBusy:true});
    try{return this.accept(await this.bridge.cancel(runId));}
    catch(cause){this.change({connected:false,error:`取消の結果を再確認してください: ${String(cause)}`});throw cause;}
    finally{this.change({commandBusy:false});}
  }
  async discard(runId:string){
    if(this.view.commandBusy)throw Error("テストの受付確認中です。");
    const current=await this.refresh();
    if(this.view.commandBusy)throw Error("別のテスト操作の受付を確認しています。");
    if(!current||current.runId!==runId||current.busy)throw Error("同じテストの停止・検証が終わるまで一時音声を解放できません。");
    this.epoch++;this.change({commandBusy:true});
    try{
      try{await this.bridge.discard(runId);}
      catch(cause){if(await this.bridge.read()!==null)throw cause;}
      this.change({snapshot:null,pending:null,connected:true,error:""});
    }catch(cause){this.change({connected:false,error:`一時音声の解放を確認できません: ${String(cause)}`});throw cause;}
    finally{this.change({commandBusy:false});}
  }
}
export async function waitForCaptureTestStop(runId:string,read:()=>Promise<CaptureTestSnapshot|null>,timeoutMs=100_000,pause=(ms:number)=>new Promise<void>(resolve=>setTimeout(resolve,ms))){
  const started=Date.now();
  for(;;){
    const value=await read();
    if(!value||value.runId!==runId)throw Error("停止確認するテストが変更されたか、結果を確認できません。");
    if(!value.busy)return value;
    if(Date.now()-started>timeoutMs)throw Error("テストの停止確認が時間上限を超えました。アプリは終了していません。");
    await pause(200);
  }
}
