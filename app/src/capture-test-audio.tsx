import { InfoPopover } from "./info-popover";
import {invoke} from "@tauri-apps/api/core";
import {useEffect,useRef,useState} from "react";
import {BrowserAudioClock} from "./transcript-audio";
import {ChunkPlayer,type PlaybackInfo} from "./playback";
import {PlaybackPreparationPool} from "./playback-preparation";
import {registerPlaybackStop} from "./playback-barrier";
import {checkedCaptureTestPlayback,type CaptureTestSnapshot} from "./capture-test-state";

export function CaptureTestAudio({run,locked}:{run:CaptureTestSnapshot;locked:boolean}){
  const [,refresh]=useState(0),[selected,setSelected]=useState<"mic"|"system"|null>(null),[preparing,setPreparing]=useState(false),[error,setError]=useState("");
  const [clock]=useState(()=>new BrowserAudioClock());
  const [player]=useState(()=>new ChunkPlayer(clock,(token,offsetFrames)=>invoke<ArrayBuffer|number[]>("read_playback_chunk",{token,offsetFrames}),()=>refresh(n=>n+1)));
  const [owner]=useState(()=>crypto.randomUUID());
  const [pool]=useState(()=>new PlaybackPreparationPool({
    register:preparationId=>invoke("register_playback_preparation",{preparationId}),
    prepare:(command,args)=>invoke<PlaybackInfo>(command,args),cancel:preparationId=>invoke("cancel_playback_preparation",{preparationId}),
    acknowledge:(preparationId,playbackToken)=>invoke("finish_playback_preparation",{preparationId,playbackToken}),close:token=>invoke("close_playback",{token}),
  }));
  const generation=useRef(0),token=useRef<string|null>(null),jobs=useRef(new Set<Promise<void>>());
  const closing=useRef(new Map<string,{job:Promise<void>;active:boolean}>());
  const closeToken=(id:string):Promise<void>=>{
    const prior=closing.current.get(id);if(prior?.active)return prior.job;
    const entry={job:Promise.resolve(),active:true};
    entry.job=invoke<void>("close_playback",{token:id}).then(()=>{closing.current.delete(id);},cause=>{entry.active=false;throw cause;});
    closing.current.set(id,entry);return entry.job;
  };
  const clear=()=>{
    generation.current++;pool.cancelAll();player.clear();
    if(token.current)void closeToken(token.current).catch(()=>{});token.current=null;
    setSelected(null);setPreparing(false);setError("");
  };
  useEffect(()=>{
    let detached=false;
    const stop=async()=>{
      if(detached){generation.current++;pool.cancelAll();player.clear();if(token.current)void closeToken(token.current).catch(()=>{});token.current=null;}
      else clear();
      await Promise.all(jobs.current);await pool.stop();await Promise.all(Array.from(closing.current.keys(),closeToken));
      if(detached)unregister();
    };
    const unregister=registerPlaybackStop(stop);
    const other=(event:Event)=>{if((event as CustomEvent<string>).detail!==owner)clear();};
    window.addEventListener("mon-audio-start",other);
    const timer=window.setInterval(()=>{if(player.snapshot().playing)refresh(n=>n+1);},100);
    return()=>{detached=true;window.clearInterval(timer);window.removeEventListener("mon-audio-start",other);generation.current++;player.clear();void stop().catch(()=>{});clock.dispose();};
  },[clock,player,pool,owner]);
  useEffect(()=>{if(locked)clear();},[locked]);
  const startInner=async(kind:"mic"|"system")=>{
    const track=kind==="mic"?run.microphone:run.system;
    if(locked||run.phase!=="ready"||!track)return;
    clear();window.dispatchEvent(new CustomEvent("mon-audio-start",{detail:owner}));
    const current=generation.current;setSelected(kind);setPreparing(true);
    try{
      await clock.resume();if(current!==generation.current)return;
      const info=await pool.run("prepare_capture_test_playback",{runId:run.runId,trackKind:kind});
      if(current!==generation.current){await closeToken(info.token);return;}
      token.current=info.token;
      player.load(checkedCaptureTestPlayback(info,kind,track));await player.play();
    }catch(cause){if(current===generation.current){player.clear();if(token.current)void closeToken(token.current).catch(()=>{});token.current=null;setError(String(cause));}}
    finally{if(current===generation.current)setPreparing(false);}
  };
  const start=(kind:"mic"|"system")=>{const job=startInner(kind);jobs.current.add(job);void job.finally(()=>jobs.current.delete(job)).catch(()=>{});};
  const snapshot=player.snapshot();
  return <div className="capture-test-audio" role="group" aria-label="テスト音声の個別再生">
    <div className="heading-with-info"><h4>テスト音声の個別再生</h4><InfoPopover label="テスト音声の個別再生">取得できた音声だけを別々に聴きます。受信していない時間を無音で埋めず、同期精度・エコー除去の確認済み結果とは扱いません。</InfoPopover></div>
    <div className="card-actions">{(["mic","system"] as const).map(kind=>{
      const track=kind==="mic"?run.microphone:run.system,label=kind==="mic"?"マイク":"PC";
      return <div key={kind}><button type="button" className="small-secondary" disabled={locked||!track} onClick={()=>start(kind)}>{label}のテスト音声を再生</button>
        {track?<p>受信して保存した音声: {(track.durationMs/1000).toFixed(3)}秒{track.hasGaps?"（取得の遅れ・欠損あり）":""}</p>:<p role="status">{kind==="mic"?"マイク音声は未受信です。マイクの接続を確認して話しかけ、再テストしてください。":"PC音声は未受信です。確認する場合はPCで音声を再生して再テストしてください。"}</p>}</div>;
    })}</div>
    {selected&&<div className="audio-player">
      <strong>{selected==="mic"?"マイク":"PC"}のテスト音声</strong>
      {preparing&&<p role="status">保存音声を再検証して再生を準備しています。</p>}
      <div className="card-actions">
        {snapshot.info&&<button type="button" className="small-secondary" disabled={locked||preparing} onClick={()=>{if(player.snapshot().playing)player.pause();else void player.play();}}>{snapshot.playing?"一時停止":"再生"}</button>}
        <button type="button" className="small-secondary" disabled={locked} onClick={clear}>{preparing?"テスト音声の再生準備を取り消す":"テスト音声の再生を閉じる"}</button>
        {snapshot.info&&<span>{(snapshot.frame/snapshot.info.sampleRate).toFixed(1)} / {(snapshot.info.totalFrames/snapshot.info.sampleRate).toFixed(1)}秒</span>}
      </div>
      {snapshot.buffering&&<p role="status">音声を読み込んでいます。</p>}
      {(error||snapshot.error)&&<p role="alert" className="capture-test-error">音声を再生できません: {error||snapshot.error}</p>}
    </div>}
  </div>;
}
