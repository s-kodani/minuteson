import { invoke, isTauri } from "@tauri-apps/api/core";
import { CommonModal } from "./common-modal";
import { IssueNotice, ToastNotice } from "./toast";
import { useActionConfirmation } from "./confirmation-service";
import { useEffect, useMemo, useRef, useState } from "react";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { useTranscriptAudio } from "./transcript-audio";
import { SpeakerReviewBadge,SpeakerReviewDetails,useSpeakerReviews } from "./speaker-review";
import {preciseTime} from "./diarization-state";

import { activeSpeakers, addSpeaker, assignSpeakers, canonicalPayload, filterUtterances, mergeSpeaker, projectTranscript, splitPosition, splitUtterance, textPieces, type Analysis, type DraftPayload, type Utterance } from "./transcript-state";
export type { Analysis, DraftPayload } from "./transcript-state";

export interface Draft {
  id: string;
  meetingId: string;
  baseAnalysisId: string;
  payload: DraftPayload;
  lockVersion: number;
  updatedAt: number;
}

const PAGE_SIZE = 50;

export function timeLabel(microseconds: number): string {
  const seconds = Math.floor(microseconds / 1_000_000);
  return `${String(Math.floor(seconds / 3600)).padStart(2, "0")}:${String(Math.floor((seconds % 3600) / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

export function TranscriptEditor({ meetingId, meetingTitle, locked = false }: { meetingId: string; meetingTitle?: string; locked?: boolean }) {
  const confirm = useActionConfirmation(locked);
  const [analysis, setAnalysis] = useState<Analysis | null>(null);
  const [saveQueue] = useState(() => new DraftSaveQueue<DraftPayload, Draft>(
    (draftId, expectedLockVersion, payload) => invoke<Draft>("save_transcript_draft", { draftId, expectedLockVersion, payload }),
    canonicalPayload,
  ));
  const [saveSnapshot, setSaveSnapshot] = useState(() => saveQueue.snapshot());
  const { draft, payload, dirty, saving, error: saveError } = saveSnapshot;
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [page, setPage] = useState(0);
  const [selection,setSelection] = useState<string[]>([]);
  const [bulkSpeaker,setBulkSpeaker] = useState("");
  const [mergeSource,setMergeSource] = useState("");
  const [mergeTarget,setMergeTarget] = useState("");
  const [adding,setAdding] = useState<{utteranceId?:string}|null>(null);
  const [newName,setNewName] = useState("");
  const [notice,setNotice] = useState("");
  const [noticeVersion, setNoticeVersion] = useState<number | null>(null);
  const addDialog = useRef<HTMLDialogElement>(null);
  const [speakerFilter,setSpeakerFilter]=useState("");
  const [query,setQuery]=useState("");
  const [splitting,setSplitting]=useState<Utterance|null>(null);
  const [pendingSplit,setPendingSplit]=useState(false);
  const [pendingAdd,setPendingAdd]=useState(false);
  const [splitSeconds,setSplitSeconds]=useState("");
  const [splitOffset,setSplitOffset]=useState(1);
  const [firstSpeaker,setFirstSpeaker]=useState("keep");
  const [secondSpeaker,setSecondSpeaker]=useState("keep");
  const splitDialog=useRef<HTMLDialogElement>(null);
  const audio = useTranscriptAudio(meetingId, locked);

  useEffect(() => {
    const unsubscribe = saveQueue.subscribe(() => setSaveSnapshot(saveQueue.snapshot()));
    const unregister = registerPendingEdits(() => saveQueue.flush());
    return () => { unsubscribe(); unregister(); };
  }, [saveQueue]);

  const reload = async (propagateFailure = false) => {
    audio.clear();
    setLoading(true);
    setError("");
    try {
      const [current, working] = await Promise.all([
        invoke<Analysis | null>("get_current_analysis", { meetingId }),
        invoke<Draft | null>("get_transcript_draft", { meetingId }),
      ]);
      setAnalysis(current);
      saveQueue.load(working);
      setPage(0);
      setSelection([]); setMergeSource(""); setMergeTarget(""); setNotice(""); setNoticeVersion(null);
      setSpeakerFilter("");setQuery("");setSplitting(null);
    } catch (cause) {
      setError(String(cause));
      if (propagateFailure) throw cause;
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (isTauri()) void reload();
    // A keyed component is remounted for each meeting.
  }, [meetingId]);

  useEffect(() => {
    if (confirm.pending || !draft || !payload || !dirty || saving || saveError || draft.baseAnalysisId !== analysis?.id) return;
    const timer = window.setTimeout(() => {
      void saveQueue.flush().catch(() => { /* The queue retains the error and local edit. */ });
    }, 700);
    return () => window.clearTimeout(timer);
  }, [analysis?.id, draft, payload, dirty, saving, saveError, saveQueue, confirm.pending]);

  const begin = async () => {
    if (!analysis) return;
    setBusy(true);
    audio.clear();
    setError("");
    try {
      const created = await invoke<Draft>("begin_transcript_draft", { meetingId, analysisId: analysis.id });
      saveQueue.load(created);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const undo = async () => {
    if (!draft || dirty || saving || busy || locked || confirm.pending) return;
    setBusy(true);
    audio.clear();
    setError("");
    try {
      const previous = await invoke<Draft>("undo_transcript_edit", { draftId: draft.id, expectedLockVersion: draft.lockVersion });
      saveQueue.load(previous);
      setNotice("直前の編集を取り消しました。"); setNoticeVersion(null); setMergeSource(""); setMergeTarget("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  // Structural operations get their own durable undo entry, after preceding
  // text/name edits have been acknowledged. Failure retains the queued change.
  const structuralEdit = async (edit:(current:DraftPayload)=>DraftPayload,message:string,propagateFailure=false) => {
    if (!analysis || locked || busy) return false;
    setBusy(true); setError(""); setNotice(""); setNoticeVersion(null);
    audio.clear();
    try { await saveQueue.flush(); saveQueue.edit(edit); await saveQueue.flush(); setNotice(message); setNoticeVersion(saveQueue.snapshot().draft?.lockVersion ?? null); return true; }
    catch(cause) { setError(String(cause)); if (propagateFailure) throw cause; return false; }
    finally { setBusy(false); }
  };

  const changeName = (speakerId: string, value: string) => {
    saveQueue.edit((current) => {
      const other = current.speaker_names.filter((item) => item.speaker_id !== speakerId);
      return { ...current, speaker_names: [...other, { speaker_id: speakerId, display_name: value || null }] };
    });
  };

  const changeUtterance = (utterance: Utterance, update: Partial<DraftPayload["utterances"][number]>) => {
    saveQueue.edit((current) => {
      const previous = current.utterances.find((item) => item.utterance_id === utterance.id);
      const other = current.utterances.filter((item) => item.utterance_id !== utterance.id);
      return {
        ...current,
        utterances: [...other, {
          utterance_id: utterance.id,
          effective_text: previous?.effective_text ?? utterance.effectiveText,
          speaker_ids: previous?.speaker_ids ?? utterance.speakerIds,
          ...update,
        }],
      };
    });
  };

  const matchingBase = Boolean(draft && draft.baseAnalysisId === analysis?.id);
  const view = useMemo(()=>analysis ? projectTranscript(analysis,matchingBase?payload:null) : null,[analysis,matchingBase,payload]);
  const speakers = view ? activeSpeakers(view,matchingBase?payload:null) : [];
  const filtered=useMemo(()=>view?filterUtterances(view,speakerFilter,query):[],[view,speakerFilter,query]);
  const shown = filtered.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);
  const review = useSpeakerReviews(analysis,[...new Set(shown.map(u=>u.sourceUtteranceId??u.id))]);
  const speakerLabels = new Map(view?.speakers.map(s=>[s.id,(s.displayName||s.label)+(payload?.speaker_merges?.some(m=>m.source_id===s.id)?"（統合前の推定ラベル）":"")])??[]);
  const editable = Boolean(matchingBase && payload && !locked && !loading && !busy && !confirm.pending);
  const confirmReload = () => {
    if (!saveQueue.snapshot().dirty) { void reload(); return; }
    const expected = JSON.stringify(saveQueue.snapshot().payload);
    void confirm({ title: "文字起こし編集の破棄を確認", target: `会議「${meetingTitle ?? meetingId}」の文字起こし編集`, confirmLabel: "入力を破棄して再読込する", message: "未保存の文字起こし編集を破棄して再読込します。この入力は復元できません。保存済みの編集と認識原文は保持します。", validate: () => JSON.stringify(saveQueue.snapshot().payload) === expected, action: () => reload(true) });
  };
  useEffect(()=>{setPage(0);},[speakerFilter,query]);
  useEffect(()=>{const valid=new Set(view?.utterances.map(u=>u.id));setSelection(ids=>ids.every(id=>valid.has(id))?ids:ids.filter(id=>valid.has(id)));setPage(p=>Math.min(p,Math.max(0,Math.ceil(filtered.length/PAGE_SIZE)-1)));},[view,filtered.length]);
  useEffect(()=>{if(speakerFilter&&speakerFilter!=="unknown"&&!speakers.some(s=>s.id===speakerFilter))setSpeakerFilter("");},[speakerFilter,speakers]);
  const play=async(u:Utterance,at?:number)=>{
    if(!analysis)return;
    if(u.sourceUtteranceId){
      audio.clear();
      try {await saveQueue.flush();const saved=saveQueue.snapshot().draft;if(!saved)throw Error("分割を保存してから再生してください。");
        await audio.start(analysis.id,u.id,{meetingId,analysisId:analysis.id,expectedContentHash:analysis.contentHash,expectedDraft:{id:saved.id,lockVersion:saved.lockVersion},utteranceId:u.id,expectedStartUs:u.startUs,expectedEndUs:u.endUs},at);
      }catch(cause){setError(String(cause));}
    }else await audio.start(analysis.id,u.id,undefined,at);
  };
  const splitPieces=splitting?textPieces(splitting.effectiveText):[];
  let splitAt:number|null=null,splitError="";
  if(splitting)try{splitAt=splitPosition(splitting.startUs,splitting.endUs,splitSeconds);if(splitOffset<1||splitOffset>=splitPieces.length)throw Error("空の部分を作らない本文の切れ目を指定してください。");}catch(cause){splitError=String(cause);}

  return (
    <section className="transcript-card" aria-label="文字起こし編集">
      <div className="card-head">
        <h2>文字起こし</h2>
        <button type="button" className="small-secondary" onClick={confirmReload} disabled={loading || saving || busy || locked || confirm.pending}>再読込</button>
      </div>
      {loading && <p>解析結果を読み込んでいます。</p>}
      {!loading && !analysis && !error && <p>確定済みの解析結果はまだありません。</p>}
      {analysis && <>
        <div className="transcript-actions">
          {!draft && <button type="button" onClick={() => void begin()} disabled={busy || loading || locked}>編集を開始</button>}
          {draft && <span role="status">{saveError ? "保存要確認" : saving ? "保存中" : dirty ? "未保存" : "保存済み"}</span>}
          {saveError && <button type="button" className="small-secondary" onClick={() => { void saveQueue.retry().catch(() => {}); }} disabled={saving || locked}>保存を再試行</button>}
          {matchingBase && <button type="button" className="small-secondary" onClick={() => void undo()} disabled={busy || saving || dirty || locked || draft!.lockVersion === 0}>直前の編集を取り消す</button>}
          <button type="button" disabled={!editable||!view||view.speakers.length>=100} onClick={()=>{setPendingAdd(false);setError("");setNewName("");setAdding({});}}>話者を追加</button>
        </div>
        <ToastNotice scope={meetingId} type="transcript-edit" eventId={noticeVersion !== null ? `${draft?.id}:${noticeVersion}` : undefined} message={notice} onClose={() => setNotice("")} action={noticeVersion !== null ? { kind: "undo", label: "元に戻す", disabled: busy || saving || dirty || locked || confirm.pending || !matchingBase || draft?.lockVersion !== noticeVersion, run: async () => { if (saveQueue.snapshot().draft?.lockVersion === noticeVersion && !saveQueue.snapshot().dirty) await undo(); } } : undefined} />
        {draft && !matchingBase && <p className="action-error">下書きは別の解析版に基づいています。元版の編集を保持したまま、再基準化する機能は未接続です。</p>}
        {!splitting&&audio.controls}
        <div className="speaker-grid">
          {speakers.map((speaker) => {
            return <label key={speaker.id}>{speaker.label}<input value={speaker.displayName ?? ""} onChange={(event) => changeName(speaker.id, event.target.value)} disabled={!editable} maxLength={200} placeholder="表示名" /></label>;
          })}
        </div>
        <div className="transcript-actions" aria-label="発言の絞り込み"><label>話者で絞り込み <select value={speakerFilter} disabled={loading} onChange={e=>setSpeakerFilter(e.target.value)}><option value="">すべて</option><option value="unknown">話者不明</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label><label>発言を検索 <input type="search" value={query} disabled={loading} onChange={e=>setQuery(e.target.value)} maxLength={1000}/></label><span>{filtered.length} / {view?.utterances.length??0} 発言</span></div>
        <div className="transcript-actions" aria-label="話者統合">
          <label>統合元 <select value={mergeSource} disabled={!editable} onChange={e=>setMergeSource(e.target.value)}><option value="">選択</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label>
          <label>統合先 <select value={mergeTarget} disabled={!editable} onChange={e=>setMergeTarget(e.target.value)}><option value="">選択</option>{speakers.filter(s=>s.id!==mergeSource).map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label>
          <button type="button" disabled={!editable||!mergeSource||!mergeTarget||mergeSource===mergeTarget} onClick={()=>{
            if(!view||!payload)return;
            const count=view.utterances.filter(u=>u.speakerIds.includes(mergeSource)).length;
            const message=`${speakerLabels.get(mergeSource)}の${count}発言を${speakerLabels.get(mergeTarget)}にまとめます。本文・時刻・推定元の区間は変更しません。続けますか？`;
            const expected = JSON.stringify(saveQueue.snapshot().payload), source = mergeSource, target = mergeTarget;
            void confirm({ title: "話者の統合を確認", target: `${speakerLabels.get(source)} → ${speakerLabels.get(target)} · ${count}発言`, confirmLabel: "話者を統合する", message: `${message} 直前の編集を取り消すことで戻せます。`, validate: () => JSON.stringify(saveQueue.snapshot().payload) === expected, action: async () => { if (await structuralEdit(p=>mergeSpeaker(analysis,p,source,target),"話者を統合しました。直前の編集を取り消すことができます。",true)) { setMergeSource("");setMergeTarget("");setBulkSpeaker(""); } } });
          }}>話者を統合</button>
        </div>
        {!!payload?.speaker_merges?.length&&<p className="hint">統合済みラベルは割り当て候補から除外しています。推定区間には統合前のラベルを残します。</p>}
        <div className="transcript-actions" aria-label="複数発言の話者変更"><span>選択 {selection.length} 発言（全ページ）</span><label>割り当て先 <select value={bulkSpeaker} disabled={!editable} onChange={e=>setBulkSpeaker(e.target.value)}><option value="">話者不明</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label><button type="button" disabled={!editable||!selection.length} onClick={()=>{
          if(!payload)return;
          const ids = [...selection], speaker = bulkSpeaker, expected = JSON.stringify(saveQueue.snapshot().payload);
          void confirm({ title: "複数発言の話者変更を確認", target: `選択した${ids.length}発言 → ${speakerLabels.get(speaker)||"話者不明"}`, confirmLabel: "選択した発言に適用する", message: `選択した${ids.length}発言の割り当てを${speakerLabels.get(speaker)||"話者不明"}に置き換えます。複数話者の割り当ても置き換えます。本文は変更しません。直前の編集を取り消すことで戻せます。`, validate: () => JSON.stringify(saveQueue.snapshot().payload) === expected, action: async () => { await structuralEdit(p=>assignSpeakers(analysis,p,ids,speaker?[speaker]:[]),"選択した発言の話者を変更しました。取り消しが可能です。",true); } });
        }}>選択した発言に適用</button><button type="button" disabled={!selection.length||busy} onClick={()=>setSelection([])}>選択解除</button></div>
        <div className="utterance-list">
          {review.error&&<p className="action-error" role="alert">推定区間を照合できません: {review.error} <button type="button" disabled={locked} onClick={review.retry}>推定表示を再読込</button></p>}
          {shown.map((utterance) => {
            const speakerIds = utterance.speakerIds;
            const source=analysis.utterances.find(u=>u.id===(utterance.sourceUtteranceId??utterance.id))!;
            return <article key={utterance.id} className="utterance-row">
              <div className="utterance-meta"><time title={`${utterance.startUs}–${utterance.endUs} µs`}>{utterance.recognitionIsSource?preciseTime(utterance.startUs):timeLabel(utterance.startUs)}–{utterance.recognitionIsSource?preciseTime(utterance.endUs):timeLabel(utterance.endUs)}</time><span>{speakerIds.length>1 ? "複数話者の割り当て" : speakerIds.length === 0 ? "話者不明" : "話者あり"}</span>{utterance.sourceUtteranceId?<span>手動分割 · 単語時刻未推定</span>:<SpeakerReviewBadge review={review.reviews.get(utterance.id)}/>}<button type="button" className="small-secondary" onClick={() => void play(utterance)} disabled={locked || loading || busy || audio.preparing}>{audio.selected === utterance.id ? "この発言を再生し直す" : "音声を再生"}</button></div>
              <label><input type="checkbox" aria-label={`${timeLabel(utterance.startUs)}の発言を選択`} disabled={!editable} checked={selection.includes(utterance.id)} onChange={e=>setSelection(ids=>e.target.checked?[...ids,utterance.id]:ids.filter(id=>id!==utterance.id))}/>一括変更の対象に選択</label>
              <textarea aria-label={`${timeLabel(utterance.startUs)}の発言`} value={utterance.effectiveText} onChange={(event) => changeUtterance(utterance, { effective_text: event.target.value })} disabled={!editable} rows={2} />
              {(utterance.recognitionIsSource||utterance.effectiveText !== utterance.recognizedText) && <small>{utterance.recognitionIsSource?"分割元の認識原文（全文・単語位置未推定）":"認識原文"}: {utterance.recognizedText}</small>}
              {speakers.length > 0 && <div className="utterance-speakers" aria-label="話者割り当て">{speakers.map((speaker) => <label key={speaker.id}><input type="checkbox" checked={speakerIds.includes(speaker.id)} disabled={!editable} onChange={(event) => changeUtterance(utterance, { speaker_ids: event.target.checked ? [...speakerIds, speaker.id] : speakerIds.filter((id) => id !== speaker.id) })} />{speaker.displayName || speaker.label}</label>)}</div>}
              <button type="button" className="small-secondary" disabled={!editable||!view||view.speakers.length>=100} onClick={()=>{setPendingAdd(false);setError("");setNewName("");setAdding({utteranceId:utterance.id});}}>新しい話者を作成してこの発言に割り当て</button>
              <button type="button" className="small-secondary" disabled={!editable||textPieces(utterance.effectiveText).length<2||utterance.endUs-utterance.startUs<2} onClick={()=>{setPendingSplit(false);setError("");setSplitting(utterance);setSplitSeconds(((utterance.endUs-utterance.startUs)/2_000_000).toFixed(6));setSplitOffset(Math.floor(textPieces(utterance.effectiveText).length/2));setFirstSpeaker("keep");setSecondSpeaker("keep");}}>発言を手動分割</button>
              {utterance.sourceUtteranceId&&<small>推定区間と区間再生は分割元全体を参照します。この部分だけを聞くには「音声を再生」を使用してください。</small>}
              <SpeakerReviewDetails analysis={analysis} utteranceId={source.id} startUs={source.startUs} labels={speakerLabels} locked={locked||audio.preparing} onPlay={at=>audio.start(analysis.id,source.id,undefined,at)}/>
            </article>;
          })}
        </div>
        {!filtered.length&&<p>条件に一致する発言はありません。</p>}
        {filtered.length > PAGE_SIZE && <div className="transcript-pages"><button type="button" className="small-secondary" onClick={() => setPage((value) => Math.max(0, value - 1))} disabled={page === 0}>前へ</button><span>{page + 1} / {Math.ceil(filtered.length / PAGE_SIZE)}</span><button type="button" className="small-secondary" onClick={() => setPage((value) => value + 1)} disabled={(page + 1) * PAGE_SIZE >= filtered.length}>次へ</button></div>}
      </>}
      {error && <p className="action-error" role="alert">{error}</p>}
      <IssueNotice scope={meetingId} type="transcript-save" label="文字起こしの保存に失敗" message={saveError ? `編集は画面に保持しています。内容を確認して保存を再試行してください: ${saveError}` : ""} action={{ kind: "retry", label: "文字起こし保存を再試行", disabled: saving || locked || busy || confirm.pending, run: () => saveQueue.retry().catch(() => { /* The queue publishes the retained failure. */ }) }} />
      <CommonModal ref={splitDialog} open={!!splitting} blocked={busy} onDismiss={()=>{audio.clear();setSplitting(null);}} restoreReady={!busy} className="analysis-dialog split-dialog" aria-label="発言の手動分割">
        <h3>発言の手動分割</h3><p>音声を聞き、時刻と本文の切れ目をそれぞれ指定してください。単語時刻を自動推定していません。分割した部分の連結は元の本文と完全一致します。保存後は部分ごとに本文・話者を編集できます。</p>
        {splitting&&<><button type="button" disabled={busy||locked||audio.preparing} onClick={()=>void play(splitting)}>分割対象の音声を聞く</button><label>分割位置（発言先頭からの秒）<input type="text" inputMode="decimal" value={splitSeconds} disabled={busy||pendingSplit} onChange={e=>setSplitSeconds(e.target.value)}/></label><label>本文の切れ目（文字数）<input type="number" min={1} max={splitPieces.length-1} value={splitOffset} disabled={busy||pendingSplit} onChange={e=>setSplitOffset(Number(e.target.value))}/></label><div className="split-preview"><section><h4>前半</h4><p>{splitPieces.slice(0,splitOffset).join("")}</p></section><section><h4>後半</h4><p>{splitPieces.slice(splitOffset).join("")}</p></section></div>
        {[firstSpeaker,secondSpeaker].map((value,index)=><label key={index}>{index===0?"前半の話者":"後半の話者"}<select value={value} disabled={busy||pendingSplit} onChange={e=>(index===0?setFirstSpeaker:setSecondSpeaker)(e.target.value)}><option value="keep">現在の割り当てを維持</option><option value="">話者不明</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label>)}</>}
        {splitError&&<p role="alert" className="action-error">{splitError}</p>}{error&&<p role="alert" className="action-error">{error}</p>}
        {splitting&&audio.controls}
        <div className="transcript-actions"><button type="button" disabled={!editable||pendingSplit||!!splitError||splitAt===null} onClick={()=>{
          if(!analysis||!splitting||splitAt===null)return;
          const selected=splitting,at=splitAt,ids:[string,string]=[`split:${crypto.randomUUID()}`,`split:${crypto.randomUUID()}`];
          const assigned:[string[],string[]]=[firstSpeaker,secondSpeaker].map(id=>id==="keep"?selected.speakerIds:id?[id]:[]) as [string[],string[]];
          void structuralEdit(p=>{const result=splitUtterance(analysis,p,selected.id,at,splitOffset,ids,assigned);setPendingSplit(true);return result;},"発言を分割して保存しました。本文・話者は部分ごとに編集できます。取り消しも可能です。").then(ok=>{if(ok){setSplitting(null);setSelection([]);}});
        }}>分割して保存</button><button type="button" disabled={busy} data-dialog-cancel onClick={()=>{audio.clear();setSplitting(null);}}>{pendingSplit?"閉じる（未保存の分割を保持）":"キャンセル"}</button></div>
        {saveError&&<button type="button" disabled={busy||locked} onClick={()=>{setBusy(true);setError("");void saveQueue.retry().then(()=>{if(pendingSplit){setSplitting(null);setSelection([]);setNotice("分割を保存しました。取り消しが可能です。");setNoticeVersion(saveQueue.snapshot().draft?.lockVersion ?? null);}else{setError("");setNoticeVersion(null);setNotice("先行する編集を保存しました。分割内容を確認して保存してください。");}}).catch(cause=>setError(String(cause))).finally(()=>setBusy(false));}}>分割の保存を再試行</button>}
      </CommonModal>
      <CommonModal ref={addDialog} open={!!adding} blocked={busy} onDismiss={()=>setAdding(null)} restoreReady={!busy} className="analysis-dialog speaker-add-dialog" aria-label="新しい話者"><h3>新しい話者</h3><p>{adding?.utteranceId?"この発言の割り当てを新しい話者1名に置き換えます。本文は保持します。":"話者を作成します。作成後に発言へ割り当ててください。"}</p><label>表示名 <input value={newName} maxLength={200} disabled={busy||pendingAdd} onChange={e=>setNewName(e.target.value)}/></label><div className="transcript-actions"><button type="button" disabled={busy||pendingAdd||!editable} onClick={()=>{
        if(!analysis||!adding)return;
        const id=`manual:${crypto.randomUUID()}`;
        void structuralEdit(p=>{const result=addSpeaker(analysis,p,id,newName,adding.utteranceId);setPendingAdd(true);return result;},"新しい話者を保存しました。直前の編集を取り消すことができます。").then(ok=>{if(ok)setAdding(null);});
      }}>作成して保存</button><button type="button" disabled={busy} data-dialog-cancel onClick={()=>setAdding(null)}>キャンセル</button></div>{error&&<p role="alert" className="action-error">{error}</p>}{saveError&&<button type="button" disabled={busy||locked} onClick={()=>{
        setBusy(true);setError("");
        void saveQueue.retry().then(()=>{if(pendingAdd){setAdding(null);setNotice("新しい話者を保存しました。取り消しが可能です。");setNoticeVersion(saveQueue.snapshot().draft?.lockVersion ?? null);}else{setError("");setNoticeVersion(null);setNotice("先行する編集を保存しました。作成内容を確認して保存してください。");}}).catch(cause=>setError(String(cause))).finally(()=>setBusy(false));
      }}>追加話者の保存を再試行</button>}</CommonModal>
    </section>
  );
}
