import type { PlaybackInfo } from "./playback";

export interface PreparationBridge {
  register(id:string):Promise<void>;
  prepare(command:string,args:Record<string,unknown>):Promise<PlaybackInfo>;
  cancel(id:string):Promise<void>;
  acknowledge(id:string,token:string):Promise<void>;
  close(token:string):Promise<void>;
}
interface Entry {
  id:string; registration:Promise<void>; job:Promise<PlaybackInfo>|null;
  cancelled:boolean; finished:boolean; released:boolean; releaseJob:Promise<void>|null;
  info:PlaybackInfo|null;
}
// Register before starting expensive verification. A local cancellation marks
// the entry immediately, but its IPC waits for registration to settle, so it
// cannot run before the native reservation exists. Failed cleanup stays owned
// for the global data-operation barrier's explicit retry.
export class PlaybackPreparationPool {
  private entries=new Map<string,Entry>();
  private bridge:PreparationBridge;
  private makeId:()=>string;
  constructor(bridge:PreparationBridge,makeId:()=>string=()=>crypto.randomUUID()) {this.bridge=bridge;this.makeId=makeId;}
  run(command:string,args:Record<string,unknown>):Promise<PlaybackInfo> {
    const id=this.makeId();
    if(this.entries.has(id))return Promise.reject(Error("音声準備IDが重複しています"));
    const entry:Entry={id,registration:Promise.resolve().then(()=>this.bridge.register(id)),job:null,cancelled:false,finished:false,released:false,releaseJob:null,info:null};
    this.entries.set(id,entry);
    entry.job=(async()=>{
      try {
        await entry.registration;
        if(entry.cancelled)throw Error("音声の再生準備を取り消しました");
        const info=await this.bridge.prepare(command,{...args,preparationId:id});
        if(!info||typeof info.token!=="string"||!info.token.length||info.token.length>256)throw Error("音声準備の応答が不正です");
        entry.info=info;
        if(entry.cancelled){entry.released=false;throw Error("音声の再生準備を取り消しました");}
        await this.bridge.acknowledge(id,info.token);
        if(entry.cancelled){entry.released=false;throw Error("音声の再生準備を取り消しました");}
        entry.released=true;
        return info;
      } finally {
        entry.finished=true;
        // Lost preparation replies close by native reservation. Lost receipt
        // acknowledgements also close the known token, never guess success.
        if(entry.released)this.entries.delete(id);
        else void this.release(entry).catch(()=>{});
      }
    })();
    return entry.job;
  }
  cancelAll():void {
    for(const entry of this.entries.values()){
      entry.cancelled=true;
      void this.release(entry).catch(()=>{});
    }
  }
  async stop():Promise<void> {
    this.cancelAll();
    await Promise.allSettled(Array.from(this.entries.values(),entry=>entry.job));
    await Promise.all(Array.from(this.entries.values(),entry=>this.release(entry)));
  }
  private release(entry:Entry):Promise<void> {
    if(entry.released)return Promise.resolve();
    if(entry.releaseJob)return entry.releaseJob;
    entry.releaseJob=entry.registration.catch(()=>{}).then(async()=>{
      await this.bridge.cancel(entry.id);
      // A cancellation receipt can precede the preparation's late reply.
      // Keep this release job alive until the result is terminal, then close
      // its known token. Never mark cleanup complete between those replies.
      await entry.job?.catch(()=>{});
      if(entry.info)await this.bridge.close(entry.info.token);
      entry.released=true;
      if(entry.finished)this.entries.delete(entry.id);
    }).finally(()=>{entry.releaseJob=null;});
    return entry.releaseJob;
  }
}
