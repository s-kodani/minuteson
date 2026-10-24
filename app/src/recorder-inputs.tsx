import {useEffect,useRef,useState} from "react";
import {useRecordingLevels} from "./input-levels";
import type {VisibleLevel} from "./input-level-state";
import {RecordingInputStatus,useRecordingInputValue} from "./recording-input-status";
import type {InputKind} from "./recording-input-state";
import {RecorderWaveformHistory,waveformBars} from "./recorder-waveform-state";

function SourceIcon({input}:{input:InputKind}){
  return <svg className="mini-source-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {input==="microphone"?<><rect x="9" y="2" width="6" height="13" rx="3"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3M8 22h8"/></>:<><rect x="2" y="3" width="20" height="14" rx="1.5"/><path d="M12 17v5M7 22h10"/></>}
  </svg>;
}

function Waveform({revision,level,connected,label}:{revision:number|null;level:VisibleLevel;connected:boolean;label:string}){
  const history=useRef(new RecorderWaveformHistory());
  const [samples,setSamples]=useState<readonly number[]>([]);
  useEffect(()=>{setSamples(history.current.read(revision,level,connected));},[revision,level,connected]);
  const state=!connected?"unavailable":level.state;
  const visible=state==="measured"?samples:[];
  const status=state==="measured"?(level.state==="measured"&&level.peak===0?"無音":"入力中"):{waiting:"音声未受信",unavailable:"確認不可",stale:"更新なし"}[state];
  return <span className="mini-waveform" data-level-state={state} role="img" aria-label={`${label}の音声波形：${status}`} title={`${label}：${status}`}>
    <svg viewBox="0 0 256 36" preserveAspectRatio="none" aria-hidden="true">
      <path className="mini-waveform-baseline" d="M2 18H254"/>
      {visible.map((peak,i)=>{
        // Square-root scaling keeps quiet input visible; zero remains flat.
        const half=Math.max(1,Math.sqrt(peak)*16),x=2+(waveformBars-visible.length+i)*4;
        return <line key={i} x1={x} x2={x} y1={18-half} y2={18+half}/>;
      })}
    </svg>
  </span>;
}

function SourceRow({owner,input,level,revision}:{owner:string|null;input:InputKind;level:VisibleLevel;revision:number|null}){
  const value=useRecordingInputValue(owner,input),label=input==="microphone"?"マイク":"PC音声";
  return <div className="mini-source-row" data-source={input}>
    <span className="mini-source-name" role="img" aria-label={label} title={label}><SourceIcon input={input}/></span>
    <RecordingInputStatus owner={owner} input={input} indicator/>
    <Waveform key={owner??"inactive"} revision={revision} level={level} connected={value.state==="connected"} label={label}/>
  </div>;
}

export function RecorderInputs({owner}:{owner:string|null}){
  const levels=useRecordingLevels(owner);
  return <div className="mini-inputs" aria-label="音声入力">
    <SourceRow owner={owner} input="microphone" level={levels.microphone} revision={levels.revision}/>
    <SourceRow owner={owner} input="system" level={levels.system} revision={levels.revision}/>
  </div>;
}
