import { invoke,isTauri } from "@tauri-apps/api/core";
import { useEffect,useState } from "react";
import { checkedSpeakerReviews,preciseTime,type SpeakerReview,type SpeakerReviews } from "./diarization-state";
import type { Analysis } from "./transcript";

export function useSpeakerReviews(analysis:Analysis|null,ids:string[]) {
  const [result,setResult]=useState<{key:string;reviews:SpeakerReview[]}|null>(null);
  const [error,setError]=useState("");
  const [retry,setRetry]=useState(0);
  const key=JSON.stringify([analysis?.id,analysis?.contentHash,ids]);
  useEffect(()=>{
    let disposed=false;setError("");setResult(null);
    if(analysis&&ids.length&&isTauri()) void invoke<SpeakerReviews>("get_speaker_reviews",{request:{meetingId:analysis.meetingId,analysisId:analysis.id,expectedContentHash:analysis.contentHash,utteranceIds:ids,includeIntervals:false,page:0}})
      .then(value=>{const reviews=checkedSpeakerReviews(value,analysis.id,analysis.contentHash,ids);if(!disposed)setResult({key,reviews});})
      .catch(e=>{if(!disposed)setError(String(e));});
    return()=>{disposed=true;};
  },[key,retry]);
  return {reviews:new Map((result?.key===key?result.reviews:[]).map(r=>[r.utteranceId,r])),error,retry:()=>setRetry(n=>n+1)};
}
export function SpeakerReviewBadge({review}:{review:SpeakerReview|undefined}) {
  return review?<span className="speaker-review-badges">
    {review.overlapCount>0&&<span className="speaker-overlap">発言重複（推定区間）</span>}
    {review.estimatedState==="needsSplit"&&<span className="speaker-split">話者交代あり · 分割を確認</span>}
    {review.manualAssignment&&<span>手動割り当てを保持</span>}
  </span>:null;
}
export function SpeakerReviewDetails({analysis,utteranceId,startUs,labels,locked,onPlay}: {
  analysis:Analysis;utteranceId:string;startUs:number;labels:Map<string,string>;locked:boolean;onPlay:(atUs:number)=>Promise<void>;
}) {
  const [open,setOpen]=useState(false),[page,setPage]=useState(0),[retry,setRetry]=useState(0);
  const [review,setReview]=useState<SpeakerReview|null>(null),[loading,setLoading]=useState(false),[error,setError]=useState("");
  useEffect(()=>{
    let disposed=false;
    if(!open)return;
    setReview(null);setLoading(true);setError("");
    void invoke<SpeakerReviews>("get_speaker_reviews",{request:{meetingId:analysis.meetingId,analysisId:analysis.id,expectedContentHash:analysis.contentHash,utteranceIds:[utteranceId],includeIntervals:true,page}})
      .then(value=>{const r=checkedSpeakerReviews(value,analysis.id,analysis.contentHash,[utteranceId])[0];if(r.page!==page)throw Error("推定区間のページが一致しません。");if(!disposed)setReview(r);})
      .catch(e=>{if(!disposed)setError(String(e));}).finally(()=>{if(!disposed)setLoading(false);});
    return()=>{disposed=true;};
  },[open,analysis.id,analysis.contentHash,utteranceId,page,retry]);
  return <details className="speaker-review" onToggle={e=>setOpen(e.currentTarget.open)}>
    <summary>推定された話者区間を確認</summary>
    <p>現在の話者割り当てとは別に、保存された推定区間を表示します。重複は同じトラック内の区間交差です。音源分離やトラック間の同一人物照合は行いません。</p>
    {loading&&<p role="status">推定区間を照合しています。</p>}
    {error&&<p className="action-error" role="alert">{error} <button type="button" disabled={locked||loading} onClick={()=>setRetry(n=>n+1)}>推定区間を再読込</button></p>}
    {review&&<>
      <SpeakerReviewBadge review={review}/>
      {review.estimatedState==="needsSplit"&&<p>発言内で話者が交代しています。単語時刻を推測して本文を切り分けていません。音声と原区間を確認してください。</p>}
      <h4>推定原区間 · {review.segmentCount} 件</h4>
      {review.segmentCount===0&&<p>この発言に対応する推定区間はありません。</p>}
      <ul>{review.segments.map((s,i)=><li key={i}><span>{preciseTime(s.startUs)}–{preciseTime(s.endUs)} · {s.speakerId?(labels.get(s.speakerId)??"名前未設定"):"話者不明"}{s.confidence===null?" · 信頼度なし":` · エンジン出力値 ${s.confidence}`}</span><button type="button" className="small-secondary" disabled={locked} onClick={()=>void onPlay(Math.max(startUs,s.startUs))}>この区間から聞く</button></li>)}</ul>
      <h4>発言内の重複区間 · {review.overlapCount} 件</h4>
      <ul>{review.overlaps.map((s,i)=><li key={i}><span>{preciseTime(s.startUs)}–{preciseTime(s.endUs)} · {s.speakerIds.map(id=>labels.get(id)??"名前未設定").join(" / ")}</span><button type="button" className="small-secondary" disabled={locked} onClick={()=>void onPlay(s.startUs)}>重複区間から聞く</button></li>)}</ul>
      {review.pageCount>1&&<div className="transcript-pages"><button type="button" disabled={locked||loading||page===0} onClick={()=>setPage(n=>n-1)}>前の推定区間</button><span>{page+1} / {review.pageCount} · 各一覧50件ずつ</span><button type="button" disabled={locked||loading||page+1>=review.pageCount} onClick={()=>setPage(n=>n+1)}>次の推定区間</button></div>}
    </>}
  </details>;
}
