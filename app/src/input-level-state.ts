// Live telemetry only. Never infer measurement from device connection or use
// an old publication's value to claim that audio is currently arriving.
export type LevelTrack =
  | {state:"measured";windowFrames:number;ageMs:number;rms:number;peak:number}
  | {state:"waiting"|"unavailable";windowFrames:null;ageMs:null;rms:null;peak:null};
export interface InputLevels {schemaVersion:1;ownerId:string;revision:number;reportAgeMs:number;microphone:LevelTrack;system:LevelTrack}
export type VisibleLevel = {state:"waiting"|"unavailable"|"stale"} | {state:"measured";rms:number;peak:number;rmsDbFS:number|null;peakDbFS:number|null;clipping:boolean};
const record=(value:unknown,keys:string[])=>{
  if(!value||typeof value!=="object"||Array.isArray(value)||Object.keys(value).sort().join("|")!==keys.slice().sort().join("|"))throw Error("入力レベルの形式が不正です。");
  return value as Record<string,unknown>;
};
const integer=(v:unknown,min:number,max=Number.MAX_SAFE_INTEGER)=>typeof v==="number"&&Number.isSafeInteger(v)&&v>=min&&v<=max;
const owner=(v:unknown)=>typeof v==="string"&&/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(v);
const amplitude=(v:unknown)=>typeof v==="number"&&Number.isFinite(v)&&v>=0&&v<=16;
function track(value:unknown):LevelTrack {
  const t=record(value,["state","windowFrames","ageMs","rms","peak"]);
  if(t.state==="waiting"||t.state==="unavailable"){
    if(t.windowFrames!==null||t.ageMs!==null||t.rms!==null||t.peak!==null)throw Error("未測定の入力レベルに測定値があります。");
    return {state:t.state,windowFrames:null,ageMs:null,rms:null,peak:null};
  }
  if(t.state!=="measured"||!integer(t.windowFrames,1,1_000_000)||!integer(t.ageMs,0)||!amplitude(t.rms)||!amplitude(t.peak)||Number(t.rms)>Number(t.peak)||t.peak===0&&t.rms!==0)throw Error("入力レベルの測定値が不正です。");
  return {state:"measured",windowFrames:t.windowFrames as number,ageMs:t.ageMs as number,rms:t.rms as number,peak:t.peak as number};
}
export function checkedInputLevels(value:unknown,expectedOwner:string):InputLevels {
  const r=record(value,["schemaVersion","ownerId","revision","reportAgeMs","microphone","system"]);
  if(r.schemaVersion!==1||!owner(r.ownerId)||r.ownerId!==expectedOwner||!integer(r.revision,1)||!integer(r.reportAgeMs,0))throw Error("入力レベルの対象または版が一致しません。");
  return {schemaVersion:1,ownerId:r.ownerId as string,revision:r.revision as number,reportAgeMs:r.reportAgeMs as number,microphone:track(r.microphone),system:track(r.system)};
}
function monotonic(value:number):boolean {return Number.isFinite(value)&&value>=0&&value<=Number.MAX_SAFE_INTEGER;}
// Some WebViews pause performance.now() during system sleep. Include wall-time
// advance conservatively; a reversing clock invalidates this owner epoch.
export class InputLevelClock {
  private lastMono:number;private lastWall:number;private invalid=false;
  private originMono:number;private originWall:number;
  constructor(originMono:number,originWall:number){
    this.originMono=originMono;this.originWall=originWall;
    this.lastMono=originMono;this.lastWall=originWall;
    this.invalid=!monotonic(originMono)||!monotonic(originWall);
  }
  read(mono:number,wall:number):number {
    if(!monotonic(mono)||!monotonic(wall)||mono<this.lastMono||wall<this.lastWall)this.invalid=true;
    if(this.invalid)return NaN;
    this.lastMono=mono;this.lastWall=wall;
    return Math.max(mono-this.originMono,wall-this.originWall);
  }
}
export function visibleLevel(track:LevelTrack,reportAgeMs:number,elapsedMs:number,maximumAgeMs=1000):VisibleLevel {
  if(!monotonic(reportAgeMs)||!monotonic(elapsedMs)||!monotonic(maximumAgeMs))return {state:"unavailable"};
  if(track.state!=="measured")return {state:track.state};
  if(track.ageMs+reportAgeMs+elapsedMs>maximumAgeMs)return {state:"stale"};
  return {state:"measured",rms:track.rms,peak:track.peak,rmsDbFS:track.rms===0?null:20*Math.log10(track.rms),peakDbFS:track.peak===0?null:20*Math.log10(track.peak),clipping:track.peak>=1};
}

export class InputLevelSession {
  private ownerId:string|null=null;
  private snapshot:InputLevels|null=null;
  private receivedAt:number|null=null;
  private reportedAge=0;
  private reportedAt:number|null=null;
  private lastReadAt:number|null=null;
  private clockFailed=false;
  // Native readers may increase reportAgeMs when reading the same immutable
  // publication again. That must neither reset its local age nor replace PCM.
  reset(ownerId:string|null){if(ownerId!==null&&!owner(ownerId))throw Error("入力確認の対象が不正です。");this.ownerId=ownerId;this.snapshot=null;this.receivedAt=null;this.reportedAge=0;this.reportedAt=null;this.lastReadAt=null;this.clockFailed=false;}
  accept(value:unknown,now:number):boolean {
    if(this.ownerId===null||this.clockFailed||!monotonic(now))throw Error("入力レベルの時刻または対象が不正です。");
    const next=checkedInputLevels(value,this.ownerId),prior=this.snapshot;
    if(prior&&next.revision<prior.revision)return false;
    if(this.reportedAt!==null&&now<this.reportedAt){this.clockFailed=true;throw Error("入力レベルの時計が逆行しました。入力確認を開き直してください。");}
    if(prior&&next.revision===prior.revision){
      if(JSON.stringify({...next,reportAgeMs:0})!==JSON.stringify({...prior,reportAgeMs:0})||next.reportAgeMs<this.reportedAge)throw Error("同じ入力レベルの版に異なる測定値があります。");
      this.reportedAge=next.reportAgeMs;this.reportedAt=now;
      return false;
    }
    this.snapshot=next;this.receivedAt=now;this.reportedAge=next.reportAgeMs;this.reportedAt=now;return true;
  }
  read(now:number):{microphone:VisibleLevel;system:VisibleLevel}{
    if(!monotonic(now)||this.lastReadAt!==null&&now<this.lastReadAt||this.receivedAt!==null&&now<this.receivedAt)this.clockFailed=true;
    if(this.clockFailed)return {microphone:{state:"unavailable"},system:{state:"unavailable"}};
    this.lastReadAt=now;
    if(!this.snapshot||this.receivedAt===null)return {microphone:{state:"unavailable"},system:{state:"unavailable"}};
    const age=Math.max(this.snapshot.reportAgeMs+now-this.receivedAt,this.reportedAge+now-(this.reportedAt??this.receivedAt));
    return {microphone:visibleLevel(this.snapshot.microphone,age,0),system:visibleLevel(this.snapshot.system,age,0)};
  }
}
