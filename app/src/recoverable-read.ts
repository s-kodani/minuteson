import {InputLevelClock} from "./input-level-state.ts";

export type RecoverableAnomaly="failed"|"stalled"|"hung"|"clock-reversed"|"recovered";

export type RecoverableRevision={
  revision:number;
  reportAgeMs:number;
};

export type RecoverableSession={
  reset:(owner:string|null)=>void;
  accept:(value:unknown,now:number)=>boolean;
  read:(now:number)=>unknown;
};

export type RecoverableHooks={
  mono:()=>number;
  wall:()=>number;
  invoke:()=>Promise<unknown>;
  revision?:(value:unknown)=>RecoverableRevision|null;
  onAnomaly?:(kind:RecoverableAnomaly,detail:Record<string,unknown>)=>void;
  onAdopted?:()=>void;
  // Every completed read of the current clock generation: its error, or null
  // for a valid reply. Callers keep their existing per-read diagnostics.
  onRead?:(error:unknown)=>void;
  graceMs?:number;
};

export type RecoverableClock={
  read:(mono:number,wall:number)=>number;
  reset:(mono:number,wall:number)=>void;
};

export const sharedClock=(originMono:number,originWall:number):RecoverableClock=>{
  let clock=new InputLevelClock(originMono,originWall);
  return {read:(mono,wall)=>clock.read(mono,wall),reset:(mono,wall)=>{clock=new InputLevelClock(mono,wall);}};
};

type Flight={id:number;at:number;generation:number;started:number;ticks:number};

const limit=3;
const graceDefault=15000;

export class RecoverableRead<T=unknown>{
  failed=false;
  private owner:string|null;
  private session:RecoverableSession;
  private hooks:RecoverableHooks;
  private clock:RecoverableClock;
  private clockGen=0;
  private flights:Flight[]=[];
  private nextId=1;
  private hungMs=3000;
  private hungStreak=0;
  private disposed=false;
  private floor=0;
  private maxRevision=0;
  private baseline=false;
  private lastMono=0;
  private lastWall=0;
  private started=0;
  private lastProgress=0;
  private completions=0;
  private stallReported=false;
  private anomalies=new Set<RecoverableAnomaly>();
  private lastRevision:number|null=null;
  private abandonedActive=0;
  private lastException:string|null=null;
  private readError:unknown=null;

  constructor(owner:string|null,session:RecoverableSession,clock:RecoverableClock,hooks:RecoverableHooks){
    this.owner=owner;this.session=session;this.clock=clock;this.hooks=hooks;
    this.lastMono=hooks.mono();this.lastWall=hooks.wall();this.started=this.lastMono;
    this.lastProgress=this.started;
  }

  dispose(){this.disposed=true;this.flights=[];}

  private now():number{return this.clock.read(this.hooks.mono(),this.hooks.wall());}

  private report(kind:RecoverableAnomaly,detail:Record<string,unknown>){
    if(this.disposed||this.anomalies.has(kind))return;
    this.anomalies.add(kind);
    this.hooks.onAnomaly?.(kind,detail);
  }

  private recoverAnomalies(only?:RecoverableAnomaly[]){
    const targets=only??(["failed","stalled","hung","clock-reversed"] as const);
    const cleared=targets.filter(k=>this.anomalies.has(k));
    for(const k of cleared)this.anomalies.delete(k);
    if(!cleared.length)return;
    this.hooks.onAnomaly?.("recovered",{kinds:cleared});
    if(cleared.some(k=>k==="failed"||k==="stalled")){this.stallReported=false;this.completions=0;}
  }

  private checkStall(){
    if(this.hooks.mono()-this.started<(this.hooks.graceMs??graceDefault))return;
    const idle=this.hooks.mono()-this.lastProgress;
    if(idle<3000||this.completions<3||this.stallReported)return;
    this.stallReported=true;
    if(this.lastException)this.report("failed",{error:this.lastException,completions:this.completions,idleMs:idle});
    else this.report("stalled",{idleMs:idle,completions:this.completions});
  }

  reverseClock(){
    const mono=this.hooks.mono(),wall=this.hooks.wall();
    const detail={deltaMono:mono-this.lastMono,deltaWall:wall-this.lastWall};
    this.report("clock-reversed",detail);
    this.clockGen++;
    this.baseline=true;
    this.failed=false;
    try{this.session.reset(this.owner);}catch{this.failed=true;}
    this.clock.reset(mono,wall);
    this.lastMono=mono;this.lastWall=wall;
    this.stallReported=false;this.completions=0;this.lastProgress=mono;this.lastRevision=null;
    this.lastException=null;
  }

  private finishFlight(id:number,generation:number){
    const index=this.flights.findIndex(f=>f.id===id&&f.generation===generation);
    if(index<0)return null;
    const flight=this.flights[index];
    this.flights=this.flights.filter((_,i)=>i!==index);
    return flight;
  }

  private adopt(value:unknown,requestedAt:number,roundtrip:number):boolean{
    const meta=this.hooks.revision?.(value);
    if(this.baseline){
      if(!meta)return false;
      this.floor=Math.max(this.maxRevision,meta.revision);
      this.maxRevision=Math.max(this.maxRevision,meta.revision);
      this.baseline=false;
      return false;
    }
    if(!meta){
      // The session's own validator names what was wrong with the reply.
      try{this.session.accept(value,requestedAt);this.readError=Error("revision rejected");}catch(error){this.readError=error;}
      this.lastException="revision rejected";
      this.completions++;
      this.failed=true;
      this.checkStall();
      return false;
    }
    if(meta.revision<=this.floor)return false;
    let accepted=false;
    try{accepted=this.session.accept(value,requestedAt);}catch(error){
      this.readError=error;
      this.failed=true;
      this.lastException="accept rejected";
      this.completions++;
      this.checkStall();
      return false;
    }
    if(accepted){
      this.failed=false;
      this.lastException=null;
      this.lastRevision=meta.revision;
      this.maxRevision=Math.max(this.maxRevision,meta.revision);
    }
    // Progress is a newly accepted publication that is still fresh after the
    // round trip. Only progress records recovery, so a slow but advancing
    // reader cannot alternate between stalled and recovered.
    if(accepted&&meta.reportAgeMs+roundtrip<=1000){this.lastProgress=this.hooks.mono();this.completions=0;this.recoverAnomalies();}
    else this.completions++;
    this.checkStall();
    this.hungMs=3000;this.hungStreak=0;
    return accepted;
  }

  private abandon(flight:Flight){
    this.finishFlight(flight.id,flight.generation);
    this.hungStreak++;
    this.hungMs=this.hungStreak===1?6000:12000;
    this.abandonedActive++;
    this.report("hung",{abandons:this.hungStreak,timeoutMs:this.hungMs});
  }

  advance(){
    if(this.disposed)return;
    const mono=this.hooks.mono(),wall=this.hooks.wall();
    const elapsed=this.clock.read(mono,wall);
    if(Number.isNaN(elapsed)){this.reverseClock();return;}
    this.lastMono=mono;this.lastWall=wall;
    for(const flight of [...this.flights]){
      if(mono-flight.started>=this.hungMs){
        flight.ticks++;
        if(flight.ticks>=3)this.abandon(flight);
      }
    }
  }
  tick(){this.advance();this.poll();}
  poll(){
    if(this.disposed)return;
    if(this.flights.some(f=>f.generation===this.clockGen))return;
    if(this.flights.length+this.abandonedActive>=limit+1)return;
    this.issue();
  }

  private issue(){
    if(this.disposed||this.flights.length+this.abandonedActive>=limit+1)return;
    if(this.flights.some(f=>f.generation===this.clockGen))return;
    const generation=this.clockGen,id=this.nextId++,requestedAt=this.now();
    if(Number.isNaN(requestedAt))return;
    const started=this.hooks.mono();
    const flight:Flight={id,at:requestedAt,generation,started,ticks:0};
    this.flights=[...this.flights,flight];
    // Separate handlers: an error thrown by a paint hook must not be counted as
    // a second completion of the same (already finished) read.
    const current=()=>{
      const active=this.finishFlight(id,generation);
      if(!active&&this.abandonedActive>0)this.abandonedActive--;
      return !!active&&!this.disposed&&generation===this.clockGen;
    };
    void this.hooks.invoke().then(value=>{
      const ended=this.hooks.mono();
      if(!current())return;
      this.readError=null;
      const adopted=this.adopt(value,requestedAt,ended-started);
      this.hooks.onRead?.(this.readError);
      if(adopted)this.hooks.onAdopted?.();
    },error=>{
      if(!current())return;
      this.failed=true;
      this.lastException=error instanceof Error?error.message:String(error);
      this.completions++;
      this.checkStall();
      this.hooks.onRead?.(error);
      this.hooks.onAdopted?.();
    });
  }

  read():T{return this.session.read(this.now()) as T;}
  revision():number|null{return this.lastRevision;}
  inFlightCount():number{return this.flights.length;}
  abandonedCount():number{return this.abandonedActive;}
}
