import {invoke} from "./ipc.ts";
import {checkedInputLevels,InputLevelSession,type VisibleLevel} from "./input-level-state.ts";
import {RecoverableRead,sharedClock,type RecoverableAnomaly} from "./recoverable-read.ts";
import {InputDiagnosticLog,type InputReadStage} from "./input-diagnostic-log.ts";

const unavailable=():{microphone:VisibleLevel;system:VisibleLevel}=>({microphone:{state:"unavailable"},system:{state:"unavailable"}});

export type LevelDisplay={owner:string|null;revision:number|null;microphone:VisibleLevel;system:VisibleLevel};

export type InputLevelPollerOptions={
  owner:string;
  kind:"recording"|"test";
  windowLabel?:"main"|"recorder";
  mono?:()=>number;
  wall?:()=>number;
  onUpdate:(value:LevelDisplay)=>void;
  diagnostic?:InputDiagnosticLog;
  invoke?:()=>Promise<unknown>;
};

export function createInputLevelPoller(options:InputLevelPollerOptions):{tick:()=>void;dispose:()=>void}{
  const mono=options.mono??(()=>performance.now());
  const wall=options.wall??(()=>Date.now());
  const session=new InputLevelSession();
  const clock=sharedClock(mono(),wall());
  const stage:InputReadStage=options.kind==="recording"?"live-levels":"test-levels";
  const diagnostic=options.diagnostic??new InputDiagnosticLog();
  if(options.windowLabel)diagnostic.bindWindow(options.windowLabel);
  try{session.reset(options.owner);}catch{options.onUpdate({owner:options.owner,revision:null,...unavailable()});return {tick:()=>{},dispose:()=>{}};}
  const report=(kind:RecoverableAnomaly,detail:Record<string,unknown>)=>diagnostic.reportAnomaly(stage,options.owner,kind,detail);
  const paint=()=>{const view=poll.failed?unavailable():poll.read();options.onUpdate({owner:options.owner,revision:poll.revision(),...view});};
  const poll=new RecoverableRead<{microphone:VisibleLevel;system:VisibleLevel}>(options.owner,session,clock,{
    mono,wall,
    invoke:options.invoke??(()=>invoke(options.kind==="recording"?"get_recording_levels":"get_capture_test_levels",options.kind==="recording"?{sessionId:options.owner}:{runId:options.owner})),
    revision:value=>{try{const r=checkedInputLevels(value,options.owner);return {revision:r.revision,reportAgeMs:r.reportAgeMs};}catch{return null;}},
    onAnomaly:report,
    onAdopted:paint,
  });
  const tick=()=>{poll.advance();poll.poll();paint();};
  tick();
  return {tick,dispose:()=>{poll.dispose();diagnostic.dispose();}};
}

export function startInputLevelPoller(options:InputLevelPollerOptions):()=>void{
  let live=true;
  const guarded:InputLevelPollerOptions={...options,onUpdate:value=>{if(live)options.onUpdate(value);}};
  const {tick,dispose}=createInputLevelPoller(guarded);
  const timer=window.setInterval(tick,100);
  return()=>{live=false;window.clearInterval(timer);dispose();};
}
