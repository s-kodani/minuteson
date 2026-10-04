import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { useTranscriptAudio } from "./transcript-audio";
import { SpeakerReviewBadge,SpeakerReviewDetails,useSpeakerReviews } from "./speaker-review";

import { activeSpeakers, addSpeaker, assignSpeakers, canonicalPayload, mergeSpeaker, projectTranscript, type Analysis, type DraftPayload, type Utterance } from "./transcript-state";
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

export function TranscriptEditor({ meetingId, locked = false }: { meetingId: string; locked?: boolean }) {
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
  const addDialog = useRef<HTMLDialogElement>(null);
  const audio = useTranscriptAudio(meetingId, locked);

  useEffect(() => {
    const unsubscribe = saveQueue.subscribe(() => setSaveSnapshot(saveQueue.snapshot()));
    const unregister = registerPendingEdits(() => saveQueue.flush());
    return () => { unsubscribe(); unregister(); };
  }, [saveQueue]);

  const reload = async () => {
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
      setSelection([]); setMergeSource(""); setMergeTarget(""); setNotice("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (isTauri()) void reload();
    // A keyed component is remounted for each meeting.
  }, [meetingId]);

  useEffect(() => {
    if (!draft || !payload || !dirty || saving || saveError || draft.baseAnalysisId !== analysis?.id) return;
    const timer = window.setTimeout(() => {
      void saveQueue.flush().catch(() => { /* The queue retains the error and local edit. */ });
    }, 700);
    return () => window.clearTimeout(timer);
  }, [analysis?.id, draft, payload, dirty, saving, saveError, saveQueue]);

  const begin = async () => {
    if (!analysis) return;
    setBusy(true);
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
    if (!draft || dirty || saving) return;
    setBusy(true);
    setError("");
    try {
      const previous = await invoke<Draft>("undo_transcript_edit", { draftId: draft.id, expectedLockVersion: draft.lockVersion });
      saveQueue.load(previous);
      setNotice("直前の編集を取り消しました。"); setMergeSource(""); setMergeTarget("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  useEffect(()=>{ if(adding) addDialog.current?.showModal(); else addDialog.current?.close(); },[adding]);
  // Structural operations get their own durable undo entry, after preceding
  // text/name edits have been acknowledged. Failure retains the queued change.
  const structuralEdit = async (edit:(current:DraftPayload)=>DraftPayload,message:string) => {
    if (!analysis || locked || busy) return false;
    setBusy(true); setError(""); setNotice("");
    try { await saveQueue.flush(); saveQueue.edit(edit); await saveQueue.flush(); setNotice(message); return true; }
    catch(cause) { setError(String(cause)); return false; }
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
  const view = analysis ? projectTranscript(analysis,matchingBase?payload:null) : null;
  const speakers = view ? activeSpeakers(view,matchingBase?payload:null) : [];
  const shown = view?.utterances.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE) ?? [];
  const review = useSpeakerReviews(analysis,shown.map(u=>u.id));
  const speakerLabels = new Map(view?.speakers.map(s=>[s.id,(s.displayName||s.label)+(payload?.speaker_merges?.some(m=>m.source_id===s.id)?"（統合前の推定ラベル）":"")])??[]);
  const editable = Boolean(matchingBase && payload && !locked && !loading && !busy);

  return (
    <section className="transcript-card" aria-label="文字起こし編集">
      <div className="card-head">
        <h2>文字起こし</h2>
        <button type="button" className="small-secondary" onClick={() => { if (!dirty || window.confirm("未保存の編集を破棄して再読込しますか？")) void reload(); }} disabled={loading || saving || busy || locked}>再読込</button>
      </div>
      {loading && <p>解析結果を読み込んでいます。</p>}
      {!loading && !analysis && !error && <p>確定済みの解析結果はまだありません。</p>}
      {analysis && <>
        <div className="transcript-actions">
          {!draft && <button type="button" onClick={() => void begin()} disabled={busy || loading || locked}>編集を開始</button>}
          {draft && <span role="status">{saveError ? "保存要確認" : saving ? "保存中" : dirty ? "未保存" : "保存済み"}</span>}
          {saveError && <button type="button" className="small-secondary" onClick={() => { void saveQueue.retry().catch(() => {}); }} disabled={saving || locked}>保存を再試行</button>}
          {matchingBase && <button type="button" className="small-secondary" onClick={() => void undo()} disabled={busy || saving || dirty || locked || draft!.lockVersion === 0}>直前の編集を取り消す</button>}
          <button type="button" disabled={!editable||!view||view.speakers.length>=100} onClick={()=>{setNewName("");setAdding({});}}>話者を追加</button>
        </div>
        {notice&&<p role="status">{notice}</p>}
        {draft && !matchingBase && <p className="action-error">下書きは別の解析版に基づいています。元版の編集を保持したまま、再基準化する機能は未接続です。</p>}
        {audio.controls}
        <div className="speaker-grid">
          {speakers.map((speaker) => {
            return <label key={speaker.id}>{speaker.label}<input value={speaker.displayName ?? ""} onChange={(event) => changeName(speaker.id, event.target.value)} disabled={!editable} maxLength={200} placeholder="表示名" /></label>;
          })}
        </div>
        <div className="transcript-actions" aria-label="話者統合">
          <label>統合元 <select value={mergeSource} disabled={!editable} onChange={e=>setMergeSource(e.target.value)}><option value="">選択</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label>
          <label>統合先 <select value={mergeTarget} disabled={!editable} onChange={e=>setMergeTarget(e.target.value)}><option value="">選択</option>{speakers.filter(s=>s.id!==mergeSource).map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label>
          <button type="button" disabled={!editable||!mergeSource||!mergeTarget||mergeSource===mergeTarget} onClick={()=>{
            if(!view||!payload)return;
            const count=view.utterances.filter(u=>u.speakerIds.includes(mergeSource)).length;
            const message=`${speakerLabels.get(mergeSource)}の${count}発言を${speakerLabels.get(mergeTarget)}にまとめます。本文・時刻・推定元の区間は変更しません。続けますか？`;
            if(window.confirm(message))void structuralEdit(p=>mergeSpeaker(analysis,p,mergeSource,mergeTarget),"話者を統合しました。直前の編集を取り消すことができます。").then(ok=>{if(ok){setMergeSource("");setMergeTarget("");setBulkSpeaker("");}});
          }}>話者を統合</button>
        </div>
        {!!payload?.speaker_merges?.length&&<p className="hint">統合済みラベルは割り当て候補から除外しています。推定区間には統合前のラベルを残します。</p>}
        <div className="transcript-actions" aria-label="複数発言の話者変更"><span>選択 {selection.length} 発言（全ページ）</span><label>割り当て先 <select value={bulkSpeaker} disabled={!editable} onChange={e=>setBulkSpeaker(e.target.value)}><option value="">話者不明</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label><button type="button" disabled={!editable||!selection.length} onClick={()=>{
          if(!payload||!window.confirm(`選択した${selection.length}発言の割り当てを${speakerLabels.get(bulkSpeaker)||"話者不明"}に置き換えます。複数話者の割り当ても置き換えます。本文は変更しません。`))return;
          void structuralEdit(p=>assignSpeakers(analysis,p,selection,bulkSpeaker?[bulkSpeaker]:[]),"選択した発言の話者を変更しました。取り消しが可能です。");
        }}>選択した発言に適用</button><button type="button" disabled={!selection.length||busy} onClick={()=>setSelection([])}>選択解除</button></div>
        <div className="utterance-list">
          {review.error&&<p className="action-error" role="alert">推定区間を照合できません: {review.error} <button type="button" disabled={locked} onClick={review.retry}>推定表示を再読込</button></p>}
          {shown.map((utterance) => {
            const speakerIds = utterance.speakerIds;
            return <article key={utterance.id} className="utterance-row">
              <div className="utterance-meta"><time>{timeLabel(utterance.startUs)}–{timeLabel(utterance.endUs)}</time><span>{speakerIds.length>1 ? "複数話者の割り当て" : speakerIds.length === 0 ? "話者不明" : "話者あり"}</span><SpeakerReviewBadge review={review.reviews.get(utterance.id)}/><button type="button" className="small-secondary" onClick={() => void audio.start(analysis.id, utterance.id)} disabled={locked || loading || audio.preparing}>{audio.selected === utterance.id ? "この発言を再生し直す" : "音声を再生"}</button></div>
              <label><input type="checkbox" aria-label={`${timeLabel(utterance.startUs)}の発言を選択`} disabled={!editable} checked={selection.includes(utterance.id)} onChange={e=>setSelection(ids=>e.target.checked?[...ids,utterance.id]:ids.filter(id=>id!==utterance.id))}/>一括変更の対象に選択</label>
              <textarea aria-label={`${timeLabel(utterance.startUs)}の発言`} value={utterance.effectiveText} onChange={(event) => changeUtterance(utterance, { effective_text: event.target.value })} disabled={!editable} rows={2} />
              {utterance.effectiveText !== utterance.recognizedText && <small>認識原文: {utterance.recognizedText}</small>}
              {speakers.length > 0 && <div className="utterance-speakers" aria-label="話者割り当て">{speakers.map((speaker) => <label key={speaker.id}><input type="checkbox" checked={speakerIds.includes(speaker.id)} disabled={!editable} onChange={(event) => changeUtterance(utterance, { speaker_ids: event.target.checked ? [...speakerIds, speaker.id] : speakerIds.filter((id) => id !== speaker.id) })} />{speaker.displayName || speaker.label}</label>)}</div>}
              <button type="button" className="small-secondary" disabled={!editable||!view||view.speakers.length>=100} onClick={()=>{setNewName("");setAdding({utteranceId:utterance.id});}}>新しい話者を作成してこの発言に割り当て</button>
              <SpeakerReviewDetails analysis={analysis} utteranceId={utterance.id} startUs={utterance.startUs} labels={speakerLabels} locked={locked||audio.preparing} onPlay={at=>audio.start(analysis.id,utterance.id,undefined,at)}/>
            </article>;
          })}
        </div>
        {analysis.utterances.length > PAGE_SIZE && <div className="transcript-pages"><button type="button" className="small-secondary" onClick={() => setPage((value) => Math.max(0, value - 1))} disabled={page === 0}>前へ</button><span>{page + 1} / {Math.ceil(analysis.utterances.length / PAGE_SIZE)}</span><button type="button" className="small-secondary" onClick={() => setPage((value) => value + 1)} disabled={(page + 1) * PAGE_SIZE >= analysis.utterances.length}>次へ</button></div>}
      </>}
      {error && <p className="action-error" role="alert">{error}</p>}
      {saveError && <p className="action-error" role="alert">自動保存できませんでした。編集は画面に保持しています。内容を確認して保存を再試行してください: {saveError}</p>}
      <dialog ref={addDialog} className="analysis-dialog speaker-add-dialog" aria-label="新しい話者" onCancel={e=>{if(busy)e.preventDefault();else setAdding(null);}}><h3>新しい話者</h3><p>{adding?.utteranceId?"この発言の割り当てを新しい話者1名に置き換えます。本文は保持します。":"話者を作成します。作成後に発言へ割り当ててください。"}</p><label>表示名 <input value={newName} maxLength={200} disabled={busy} onChange={e=>setNewName(e.target.value)} autoFocus/></label><div className="transcript-actions"><button type="button" disabled={busy||!editable} onClick={()=>{
        if(!analysis||!adding)return;
        const id=`manual:${crypto.randomUUID()}`;
        void structuralEdit(p=>addSpeaker(analysis,p,id,newName,adding.utteranceId),"新しい話者を保存しました。直前の編集を取り消すことができます。").then(ok=>{if(ok)setAdding(null);});
      }}>作成して保存</button><button type="button" disabled={busy} onClick={()=>setAdding(null)}>キャンセル</button></div>{error&&<p role="alert" className="action-error">{error}</p>}{saveError&&<button type="button" disabled={busy||locked} onClick={()=>{
        setBusy(true);setError("");
        void saveQueue.retry().then(()=>{setAdding(null);setNotice("新しい話者を保存しました。取り消しが可能です。");}).catch(cause=>setError(String(cause))).finally(()=>setBusy(false));
      }}>追加話者の保存を再試行</button>}</dialog>
    </section>
  );
}
