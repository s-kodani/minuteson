import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { evidenceRequest, assertEvidenceSource, type Evidence, type MinutesEvidenceSource, type MinutesView } from "./minutes-state";
import { useTranscriptAudio } from "./transcript-audio";
import { timeLabel } from "./transcript";

export function MinutesEvidencePanel({ view, evidence, locked, close, showTranscript }: {
  view: MinutesView; evidence: Evidence; locked: boolean; close: () => void; showTranscript: () => void;
}) {
  const audio = useTranscriptAudio(view.meetingId, locked);
  const [source, setSource] = useState<MinutesEvidenceSource | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const generation = useRef(0);
  const busy = useRef(false);
  useEffect(() => () => { generation.current++; }, []);
  const showSource = async () => {
    if (locked || busy.current) return;
    busy.current = true; setLoading(true); setError(""); setSource(null);
    const current = ++generation.current;
    try {
      const request = evidenceRequest(view, evidence);
      const result = await invoke<MinutesEvidenceSource>("get_minutes_evidence_source", { request });
      assertEvidenceSource(request, result);
      if (current === generation.current) setSource(result);
    } catch (cause) { if (current === generation.current) setError(String(cause)); }
    finally { if (current === generation.current) { busy.current = false; setLoading(false); } }
  };
  const play = async () => {
    try { await audio.start(evidence.analysisRevisionId, evidence.utteranceId, evidenceRequest(view, evidence)); }
    catch (cause) { setError(String(cause)); }
  };
  return <aside className="fixed-evidence" aria-label="固定された根拠発言">
    <div className="card-head"><h4>根拠の発言</h4><button type="button" className="small-secondary" disabled={locked} onClick={close}>閉じる</button></div>
    <p>{timeLabel(evidence.startUs)}–{timeLabel(evidence.endUs)} · {evidence.verificationState === "user_verified" ? "利用者確認済み" : "未確認"}</p>
    <blockquote>{evidence.quoteText}</blockquote>
    <small>固定解析版: {evidence.analysisRevisionId}<br />発言ID: {evidence.utteranceId}</small>
    <p>生成時に固定した引用です。現在の文字起こしや編集内容へ置き換えません。表示・再生だけでは確認済みになりません。</p>
    <div className="audio-controls">
      <button type="button" className="small-secondary" disabled={locked || loading} onClick={() => void showSource()}>{loading ? "固定発言を確認中" : "固定した元発言を表示"}</button>
      <button type="button" className="small-secondary" disabled={locked || audio.preparing} onClick={() => void play()}>根拠の音声を再生</button>
      <button type="button" className="small-secondary" data-fixed-transcript-opener={evidence.id} disabled={locked} onClick={showTranscript}>固定文字起こし全文へ移動</button>
    </div>
    {error && <p className="action-error" role="alert">固定発言を表示できません: {error}</p>}
    {audio.controls}
    {source && <section className="fixed-transcript" aria-label="生成時に固定した文字起こし">
      <h5>固定した元発言と前後の文脈</h5>
      <p>全{source.totalUtterances}発言のうち{source.firstSequence}–{source.firstSequence + source.utterances.length - 1}。読取り専用です。</p>
      {source.utterances.map((u) => <article className={u.id === evidence.utteranceId ? "evidence-target" : ""} key={u.id} aria-label={u.id === evidence.utteranceId ? "参照先の固定発言" : "前後の固定発言"}>
        <p><time>{timeLabel(u.startUs)}–{timeLabel(u.endUs)}</time> · {u.speakerIds.length ? u.speakerIds.map((id) => { const s = source.speakers.find((s) => s.id === id); return s?.displayName || s?.label || "話者不明"; }).join(" / ") : "話者不明"}</p>
        <p className="document-text">{u.effectiveText}</p>
        {u.recognizedText !== u.effectiveText && <small className="document-text">認識原文: {u.recognizedText}</small>}
      </article>)}
      <small>固定入力SHA-256: {source.analysisContentHash}</small>
    </section>}
  </aside>;
}
