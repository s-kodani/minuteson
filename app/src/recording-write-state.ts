export type WriteTrack={state:"written";frames:number;ageMs:number}|{state:"waiting"|"unavailable";frames:null;ageMs:null};
export type WriteSnapshot={schemaVersion:1;ownerId:string;revision:number;microphone:WriteTrack;system:WriteTrack};
export type VisibleWrite={state:"written";frames:number;ageMs:number}|{state:"waiting"|"unavailable"|"stale"};
const integer=(value:unknown,min=0)=>typeof value==="number"&&Number.isSafeInteger(value)&&value>=min;
const owner=(value:unknown)=>typeof value==="string"&&/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(value);
function record(value:unknown,keys:string[]){
  if(!value||typeof value!=="object"||Array.isArray(value)||Object.keys(value).sort().join("|")!==keys.slice().sort().join("|"))throw Error("書込み確認の形式が不正です。");
  return value as Record<string,unknown>;
}
function track(value:unknown):WriteTrack{
  const t=record(value,["state","frames","ageMs"]);
  if(t.state==="waiting"||t.state==="unavailable"){
    if(t.frames!==null||t.ageMs!==null)throw Error("未確認の書込みに保存済み値があります。");
    return {state:t.state,frames:null,ageMs:null};
  }
  if(t.state!=="written"||!integer(t.frames,1)||!integer(t.ageMs))throw Error("書込み確認の値が不正です。");
  return {state:"written",frames:t.frames as number,ageMs:t.ageMs as number};
}
export function checkedWrites(value:unknown,expectedOwner:string):WriteSnapshot{
  const r=record(value,["schemaVersion","ownerId","revision","microphone","system"]);
  if(r.schemaVersion!==1||!owner(r.ownerId)||r.ownerId!==expectedOwner||!integer(r.revision,1))throw Error("書込み確認する録音が一致しません。");
  return {schemaVersion:1,ownerId:r.ownerId as string,revision:r.revision as number,microphone:track(r.microphone),system:track(r.system)};
}
const clock=(value:number)=>Number.isFinite(value)&&value>=0&&value<=Number.MAX_SAFE_INTEGER;
type Evidence={frames:number;age:number;at:number};
export class WriteSession{
  private owner:string|null=null;private snapshot:WriteSnapshot|null=null;private lastAccept:number|null=null;private lastRead:number|null=null;private invalid=false;
  private evidence:{microphone:Evidence|null;system:Evidence|null}={microphone:null,system:null};
  reset(value:string|null){if(value!==null&&!owner(value))throw Error("書込み確認の録音IDが不正です。");this.owner=value;this.snapshot=null;this.lastAccept=null;this.lastRead=null;this.invalid=false;this.evidence={microphone:null,system:null};}
  accept(value:unknown,at:number){
    if(!this.owner||this.invalid||!clock(at)||this.lastAccept!==null&&at<this.lastAccept)throw Error("書込み確認の時計または対象が不正です。");
    const next=checkedWrites(value,this.owner),prior=this.snapshot;
    if(prior&&next.revision<prior.revision)return false;
    if(prior&&next.revision===prior.revision){if(JSON.stringify(next)!==JSON.stringify(prior))throw Error("同じ書込み確認の版に異なる状態があります。");return false;}
    const evidence={...this.evidence};
    for(const name of ["microphone","system"] as const){
      const t=next[name],old=evidence[name];if(t.state!=="written")continue;
      if(old&&t.frames<old.frames)throw Error("保存済みフレーム数が逆行しました。");
      // A fresh RPC revision or a metadata touch alone is not another write.
      evidence[name]={frames:t.frames,age:old&&t.frames===old.frames?Math.max(t.ageMs,old.age+at-old.at):t.ageMs,at};
    }
    this.evidence=evidence;this.snapshot=next;this.lastAccept=at;return true;
  }
  read(at:number):{microphone:VisibleWrite;system:VisibleWrite}{
    if(!clock(at)||this.lastRead!==null&&at<this.lastRead||this.lastAccept!==null&&at<this.lastAccept)this.invalid=true;
    this.lastRead=at;
    const visible=(name:"microphone"|"system"):VisibleWrite=>{
      if(this.invalid||!this.snapshot)return {state:"unavailable"};
      const t=this.snapshot[name],e=this.evidence[name];if(t.state!=="written")return {state:t.state};
      if(!e)return {state:"unavailable"};const age=e.age+at-e.at;
      return age>1000?{state:"stale"}:{state:"written",frames:t.frames,ageMs:age};
    };
    return {microphone:visible("microphone"),system:visible("system")};
  }
}
