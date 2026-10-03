import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { useTranscriptAudio } from "./transcript-audio";

interface Speaker {
  id: string;
  label: string;
  displayName: string | null;
}

interface Utterance {
  id: string;
  startUs: number;
  endUs: number;
  recognizedText: string;
  effectiveText: string;
  attributionState: string;
  speakerIds: string[];
}

export interface Analysis {
  id: string;
  meetingId: string;
  contentHash: string;
  speakers: Speaker[];
  utterances: Utterance[];
}

export interface DraftPayload {
  schema_version: number;
  speaker_names: { speaker_id: string; display_name: string | null }[];
  utterances: { utterance_id: string; effective_text: string; speaker_ids: string[] }[];
}

export interface Draft {
  id: string;
  meetingId: string;
  baseAnalysisId: string;
  payload: DraftPayload;
  lockVersion: number;
  updatedAt: number;
}

const PAGE_SIZE = 50;
const compareId = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;

function canonicalPayload(payload: DraftPayload): DraftPayload {
  return {
    ...payload,
    speaker_names: [...payload.speaker_names].sort((a, b) => compareId(a.speaker_id, b.speaker_id)),
    utterances: payload.utterances.map((item) => ({ ...item, speaker_ids: [...item.speaker_ids].sort(compareId) })).sort((a, b) => compareId(a.utterance_id, b.utterance_id)),
  };
}

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
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
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

  const shown = analysis?.utterances.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE) ?? [];
  const matchingBase = Boolean(draft && draft.baseAnalysisId === analysis?.id);
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
        </div>
        {draft && !matchingBase && <p className="action-error">下書きは別の解析版に基づいています。元版の編集を保持したまま、再基準化する機能は未接続です。</p>}
        {audio.controls}
        <div className="speaker-grid">
          {analysis.speakers.map((speaker) => {
            const edited = payload?.speaker_names.find((item) => item.speaker_id === speaker.id);
            return <label key={speaker.id}>{speaker.label}<input value={edited ? edited.display_name ?? "" : speaker.displayName ?? ""} onChange={(event) => changeName(speaker.id, event.target.value)} disabled={!editable} maxLength={200} placeholder="表示名" /></label>;
          })}
        </div>
        <div className="utterance-list">
          {shown.map((utterance) => {
            const edited = payload?.utterances.find((item) => item.utterance_id === utterance.id);
            const speakerIds = edited?.speaker_ids ?? utterance.speakerIds;
            return <article key={utterance.id} className="utterance-row">
              <div className="utterance-meta"><time>{timeLabel(utterance.startUs)}–{timeLabel(utterance.endUs)}</time><span>{utterance.attributionState === "overlap" ? "重複発話" : speakerIds.length === 0 ? "話者不明" : "話者あり"}</span><button type="button" className="small-secondary" onClick={() => void audio.start(analysis.id, utterance.id)} disabled={locked || loading || audio.preparing}>{audio.selected === utterance.id ? "この発言を再生し直す" : "音声を再生"}</button></div>
              <textarea aria-label={`${timeLabel(utterance.startUs)}の発言`} value={edited?.effective_text ?? utterance.effectiveText} onChange={(event) => changeUtterance(utterance, { effective_text: event.target.value })} disabled={!editable} rows={2} />
              {(edited?.effective_text ?? utterance.effectiveText) !== utterance.recognizedText && <small>認識原文: {utterance.recognizedText}</small>}
              {analysis.speakers.length > 0 && <div className="utterance-speakers" aria-label="話者割り当て">{analysis.speakers.map((speaker) => <label key={speaker.id}><input type="checkbox" checked={speakerIds.includes(speaker.id)} disabled={!editable} onChange={(event) => changeUtterance(utterance, { speaker_ids: event.target.checked ? [...speakerIds, speaker.id] : speakerIds.filter((id) => id !== speaker.id) })} />{payload?.speaker_names.find((item) => item.speaker_id === speaker.id)?.display_name || speaker.displayName || speaker.label}</label>)}</div>}
            </article>;
          })}
        </div>
        {analysis.utterances.length > PAGE_SIZE && <div className="transcript-pages"><button type="button" className="small-secondary" onClick={() => setPage((value) => Math.max(0, value - 1))} disabled={page === 0}>前へ</button><span>{page + 1} / {Math.ceil(analysis.utterances.length / PAGE_SIZE)}</span><button type="button" className="small-secondary" onClick={() => setPage((value) => value + 1)} disabled={(page + 1) * PAGE_SIZE >= analysis.utterances.length}>次へ</button></div>}
      </>}
      {error && <p className="action-error" role="alert">{error}</p>}
      {saveError && <p className="action-error" role="alert">自動保存できませんでした。編集は画面に保持しています。内容を確認して保存を再試行してください: {saveError}</p>}
    </section>
  );
}
