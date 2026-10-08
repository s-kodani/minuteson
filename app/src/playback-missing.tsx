import { useState } from "react";
import { missingAtFrame, nextSavedFrame, type PlaybackInfo } from "./playback";

function time(us: number) {
  const seconds=Math.floor(us/1_000_000);
  const fraction=String(us%1_000_000).padStart(6,"0").replace(/0+$/,"").padEnd(3,"0");
  return `${Math.floor(seconds/60)}:${String(seconds%60).padStart(2,"0")}.${fraction}`;
}
const kind=(value:string)=>({mic:"マイク",system:"PC音声",imported:"取り込み音声"}[value] ?? "保存音声");
const reason=(value:string)=>({capture_start_gap:"入力開始前",capture_stop_gap:"保存PCM終端から取得終了まで",capture_timestamp_gap:"入力中断"}[value] ?? "取得できなかった音声");

export function PlaybackMissing({info,frame,disabled,onSeek}:{info:PlaybackInfo;frame:number;disabled:boolean;onSeek:(frame:number)=>void}) {
  const [page,setPage]=useState(0);
  const gaps=info.missingRanges ?? [];
  if(!gaps.length)return null;
  const pages=Math.ceil(gaps.length/10),current=Math.min(page,pages-1);
  const active=missingAtFrame(info,frame),next=nextSavedFrame(info,frame);
  return <div className="audio-missing">
    <p role="status">音声欠損があります（{gaps.length}区間）。欠損を無音として補いません。</p>
    {!!active.length && <div className="audio-missing-position">
      <p role="status">現在位置は{Array.from(new Set(active.map(gap=>kind(gap.trackKind)))).join("・")}の欠損区間です。保存音声の再生を停止しています。</p>
      <button type="button" className="small-secondary" disabled={disabled || next===null} onClick={()=>{if(next!==null)onSeek(next);}}>次の保存音声へ</button>
      {next===null && <p>この先に保存音声はありません。</p>}
    </div>}
    <details><summary>欠損区間を確認</summary>
      <p>両トラック再生では、どちらかが欠損している区間で停止します。もう一方の音声は再生トラックを切り替えて確認できます。</p>
      <ol start={current*10+1}>{gaps.slice(current*10,current*10+10).map((gap,index)=>{
        const after=nextSavedFrame(info,gap.endFrame);
        return <li key={current*10+index}><span>{kind(gap.trackKind)}：{time(gap.startUs)}〜{time(gap.endUs)}（{reason(gap.reasonCode)}）</span>
          <button type="button" className="small-secondary" disabled={disabled || after===null} onClick={()=>{if(after!==null)onSeek(after);}}>この欠損の後へ</button></li>;
      })}</ol>
      {pages>1 && <div className="audio-controls"><button type="button" disabled={disabled || current===0} onClick={()=>setPage(current-1)}>前の欠損一覧</button><span>{current+1} / {pages}</span><button type="button" disabled={disabled || current+1===pages} onClick={()=>setPage(current+1)}>次の欠損一覧</button></div>}
    </details>
  </div>;
}
