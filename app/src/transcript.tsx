import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";

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

interface Analysis {
  id: string;
  meetingId: string;
  contentHash: string;
  speakers: Speaker[];
  utterances: Utterance[];
}

interface DraftPayload {
  schema_version: number;
  speaker_names: { speaker_id: string; display_name: string | null }[];
  utterances: { utterance_id: string; effective_text: string; speaker_ids: string[] }[];
}

interface Draft {
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

function timeLabel(microseconds: number): string {
  const seconds = Math.floor(microseconds / 1_000_000);
  return `${String(Math.floor(seconds / 3600)).padStart(2, "0")}:${String(Math.floor((seconds % 3600) / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

export function TranscriptEditor({ meetingId }: { meetingId: string }) {
  const [analysis, setAnalysis] = useState<Analysis | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [payload, setPayload] = useState<DraftPayload | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveBlocked, setSaveBlocked] = useState(false);
  const [error, setError] = useState("");
  const [page, setPage] = useState(0);
  const savingRef = useRef(false);
  const dirty = Boolean(draft && payload && JSON.stringify(canonicalPayload(payload)) !== JSON.stringify(draft.payload));

  const reload = async () => {
    setLoading(true);
    setError("");
    setSaveBlocked(false);
    try {
      const [current, working] = await Promise.all([
        invoke<Analysis | null>("get_current_analysis", { meetingId }),
        invoke<Draft | null>("get_transcript_draft", { meetingId }),
      ]);
      setAnalysis(current);
      setDraft(working);
      setPayload(working?.payload ?? null);
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
    if (!draft || !payload || !dirty || saveBlocked || draft.baseAnalysisId !== analysis?.id) return;
    const timer = window.setTimeout(() => {
      if (savingRef.current) return;
      savingRef.current = true;
      setSaving(true);
      const toSave = canonicalPayload(payload);
      void invoke<Draft>("save_transcript_draft", {
        draftId: draft.id,
        expectedLockVersion: draft.lockVersion,
        payload: toSave,
      }).then((saved) => {
        setDraft(saved);
        setError("");
      }).catch((cause: unknown) => {
        setError(`自動保存できませんでした。競合または入力内容を確認し、必要なら再読込してください: ${String(cause)}`);
        setSaveBlocked(true);
      }).finally(() => {
        savingRef.current = false;
        setSaving(false);
      });
    }, 700);
    return () => window.clearTimeout(timer);
  }, [analysis?.id, draft, payload, dirty, saveBlocked]);

  const begin = async () => {
    if (!analysis) return;
    setBusy(true);
    setError("");
    try {
      const created = await invoke<Draft>("begin_transcript_draft", { meetingId, analysisId: analysis.id });
      setDraft(created);
      setPayload(created.payload);
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
      setDraft(previous);
      setPayload(previous.payload);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const changeName = (speakerId: string, value: string) => {
    setPayload((current) => {
      if (!current) return current;
      const other = current.speaker_names.filter((item) => item.speaker_id !== speakerId);
      return { ...current, speaker_names: [...other, { speaker_id: speakerId, display_name: value || null }] };
    });
  };

  const changeUtterance = (utterance: Utterance, update: Partial<DraftPayload["utterances"][number]>) => {
    setPayload((current) => {
      if (!current) return current;
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
  const editable = Boolean(draft && payload && draft.baseAnalysisId === analysis?.id);

  return (
    <section className="transcript-card" aria-label="文字起こし編集">
      <div className="card-head">
        <h2>文字起こし</h2>
        <button type="button" className="small-secondary" onClick={() => { if (!dirty || window.confirm("未保存の編集を破棄して再読込しますか？")) void reload(); }} disabled={loading || saving}>再読込</button>
      </div>
      {loading && <p>解析結果を読み込んでいます。</p>}
      {!loading && !analysis && !error && <p>確定済みの解析結果はまだありません。</p>}
      {analysis && <>
        <div className="transcript-actions">
          {!draft && <button type="button" onClick={() => void begin()} disabled={busy}>編集を開始</button>}
          {draft && <span role="status">{saveBlocked ? "保存要確認" : saving ? "保存中" : dirty ? "未保存" : "保存済み"}</span>}
          {saveBlocked && <button type="button" className="small-secondary" onClick={() => { setError(""); setSaveBlocked(false); }} disabled={saving}>保存を再試行</button>}
          {editable && <button type="button" className="small-secondary" onClick={() => void undo()} disabled={busy || saving || dirty || draft!.lockVersion === 0}>直前の編集を取り消す</button>}
        </div>
        {draft && !editable && <p className="action-error">下書きは別の解析版に基づいています。元版の編集を保持したまま、再基準化する機能は未接続です。</p>}
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
              <div className="utterance-meta"><time>{timeLabel(utterance.startUs)}–{timeLabel(utterance.endUs)}</time><span>{utterance.attributionState === "overlap" ? "重複発話" : speakerIds.length === 0 ? "話者不明" : "話者あり"}</span></div>
              <textarea aria-label={`${timeLabel(utterance.startUs)}の発言`} value={edited?.effective_text ?? utterance.effectiveText} onChange={(event) => changeUtterance(utterance, { effective_text: event.target.value })} disabled={!editable} rows={2} />
              {(edited?.effective_text ?? utterance.effectiveText) !== utterance.recognizedText && <small>認識原文: {utterance.recognizedText}</small>}
              {analysis.speakers.length > 0 && <div className="utterance-speakers" aria-label="話者割り当て">{analysis.speakers.map((speaker) => <label key={speaker.id}><input type="checkbox" checked={speakerIds.includes(speaker.id)} disabled={!editable} onChange={(event) => changeUtterance(utterance, { speaker_ids: event.target.checked ? [...speakerIds, speaker.id] : speakerIds.filter((id) => id !== speaker.id) })} />{payload?.speaker_names.find((item) => item.speaker_id === speaker.id)?.display_name || speaker.displayName || speaker.label}</label>)}</div>}
            </article>;
          })}
        </div>
        {analysis.utterances.length > PAGE_SIZE && <div className="transcript-pages"><button type="button" className="small-secondary" onClick={() => setPage((value) => Math.max(0, value - 1))} disabled={page === 0}>前へ</button><span>{page + 1} / {Math.ceil(analysis.utterances.length / PAGE_SIZE)}</span><button type="button" className="small-secondary" onClick={() => setPage((value) => value + 1)} disabled={(page + 1) * PAGE_SIZE >= analysis.utterances.length}>次へ</button></div>}
      </>}
      {error && <p className="action-error" role="alert">{error}</p>}
    </section>
  );
}
