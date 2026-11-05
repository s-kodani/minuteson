export interface AnalysisPreparation {
  meetingId:string;operationId:string;phase:string;completed:number|null;total:number|null;
  startedAtMs:number;acceptedAtMs?:number;sequence:number;meetingTitle?:string;
}
export class Admissions {
  private items=new Map<string,AnalysisPreparation>();
  private listeners=new Set<()=>void>();
  private snapshot:AnalysisPreparation[]=[];
  readonly starts=new Map<string,number>();
  readonly flowStarts=new Map<string,number>();
  subscribe=(listener:()=>void)=>{this.listeners.add(listener);return()=>{this.listeners.delete(listener);};};
  getSnapshot=()=>this.snapshot;
  private publish(){this.snapshot=[...this.items.values()];this.listeners.forEach(f=>f());}
  begin(meetingId:string,meetingTitle:string,operationId:string=crypto.randomUUID()){
    const startedAtMs=Date.now();this.starts.set(meetingId,startedAtMs);this.flowStarts.set(operationId,startedAtMs);
    this.items.set(meetingId,{meetingId,meetingTitle,operationId,startedAtMs,phase:"accepting",completed:null,total:null,sequence:0});this.publish();return operationId;
  }
  end(meetingId:string,operationId?:string){const p=this.items.get(meetingId);if(p && (!operationId || p.operationId===operationId)){p.phase="waiting";p.acceptedAtMs=Date.now();this.publish();}}
  clear(meetingId:string,operationId?:string){if(operationId && this.items.get(meetingId)?.operationId!==operationId)return;this.items.delete(meetingId);this.publish();}
  observePreparations(rows:AnalysisPreparation[],readStartedAt:number){
    let changed=false;
    for(const p of this.items.values()) {
      if(p.acceptedAtMs!==undefined && p.acceptedAtMs<=readStartedAt && !rows.some(row=>row.meetingId===p.meetingId)) {this.items.delete(p.meetingId);changed=true;}
    }
    if(changed)this.publish();
  }
  observe(rows:{meetingId:string;createdAt?:number;kind:string;operationId?:string;analysisOperation?:{id:string}|null}[]){
    for(const p of this.items.values())if(rows.some(j=>j.meetingId===p.meetingId&&j.kind==="transcription"&&(j.analysisOperation?.id===p.operationId || j.operationId===`flow:${p.operationId}:transcription`)))this.items.delete(p.meetingId);
    this.publish();
  }
}
export const transcriptionAdmissions=new Admissions();
