import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { evidenceRequest, assertEvidenceSource, itemEvidencePage, type Evidence, type MinutesContent, type MinutesEvidenceSource, type MinutesView } from "./minutes-state";
import { useTranscriptAudio } from "./transcript-audio";
import { timeLabel } from "./transcript";
import { SpeakerLabels } from "./speaker-label";

export function MinutesEvidenceGroup({ view, content, selectedId, page, changePage, suspended, locked, close, showTranscript, verify, dockPlayback = false }: {
  view: MinutesView; content: MinutesContent; selectedId: string; page: number | null; changePage: (page: number) => void;
  suspended: boolean; locked: boolean; close: () => void; showTranscript: (id: string) => void; verify?: (id: string, verified: boolean) => void; dockPlayback?: boolean;
}) {
  const group = itemEvidencePage(content, selectedId, page);
  if (!group) return null;
  return <aside className="fixed-evidence-group" aria-label="項目の固定根拠一覧">
    <div className="card-head"><h4>根拠の発言 · {group.total}件</h4><button type="button" className="small-secondary" disabled={locked} onClick={close}>根拠一覧を閉じる</button></div>
    <p className="document-text">対象項目: {group.item.body}</p>
    <p>同じ項目の根拠をまとめて表示します。各引用・話者・元発言は生成時の固定版です。表示・再生だけでは確認済みになりません。</p>
    <p>{verify ? "項目と個々の根拠の対応を確認した場合だけ、確認済みを指定してください。本文・担当者・期限を修正すると、その項目の根拠は未確認へ戻ります。" : "確認状態の変更は、現在版の「議事録を編集」から行えます。候補・直前版は読取り専用です。"}</p>
    <p>{group.first + 1}–{group.first + group.evidence.length} / {group.total}件</p>
    {group.pages > 1 && <nav className="transcript-pages" aria-label="根拠一覧のページ"><button type="button" className="small-secondary" disabled={locked || group.page === 0} onClick={() => changePage(group.page - 1)}>前の10根拠</button><span>{group.page + 1} / {group.pages}</span><button type="button" className="small-secondary" disabled={locked || group.page + 1 === group.pages} onClick={() => changePage(group.page + 1)}>次の10根拠</button></nav>}
    {group.evidence.map((e, i) => <MinutesEvidencePanel key={`${view.id}:${e.id}`} view={view} evidence={e} index={group.first + i + 1} selected={e.id === selectedId} suspended={suspended} locked={locked} dockPlayback={dockPlayback} showTranscript={() => showTranscript(e.id)} verify={verify ? (verified) => verify(e.id, verified) : undefined} />)}
  </aside>;
}

function MinutesEvidencePanel({ view, evidence, index, selected, suspended, locked, showTranscript, verify, dockPlayback }: {
  view: MinutesView; evidence: Evidence; index: number; selected: boolean; suspended: boolean; locked: boolean; showTranscript: () => void; verify?: (verified: boolean) => void; dockPlayback: boolean;
}) {
  const audio = useTranscriptAudio(view.meetingId, locked, { inline:!dockPlayback,label:`議事録の固定根拠${index} · ${timeLabel(evidence.startUs)}–${timeLabel(evidence.endUs)}` });
  const [source, setSource] = useState<MinutesEvidenceSource | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const generation = useRef(0);
  const busy = useRef(false);
  useEffect(() => { if (suspended) audio.clear(); }, [suspended]);
  useEffect(() => {
    busy.current = false;
    void showSource();
    return () => { generation.current++; busy.current = false; };
  }, []);
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
  return <article className={`fixed-evidence${selected ? " evidence-selected" : ""}`} aria-label={`固定根拠${index}`}>
    <h5>根拠{index}{selected ? " · 選択した参照" : ""}</h5>
    <p>{timeLabel(evidence.startUs)}–{timeLabel(evidence.endUs)} · {evidence.verificationState === "user_verified" ? "利用者確認済み" : "未確認"}</p>
    <blockquote>{evidence.quoteText}</blockquote>
    <small>固定解析版: {evidence.analysisRevisionId}<br />発言ID: {evidence.utteranceId}</small>
    {verify && <label className="evidence-verification"><input type="checkbox" checked={evidence.verificationState === "user_verified"} disabled={locked} onChange={(event) => verify(event.target.checked)} />この根拠を確認済みにする</label>}
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
        <p><time>{timeLabel(u.startUs)}–{timeLabel(u.endUs)}</time> · <SpeakerLabels meetingId={view.meetingId} speakerIds={u.speakerIds} speakers={source.speakers}/></p>
        <p className="document-text">{u.effectiveText}</p>
        {(u.recognitionIsSource||u.recognizedText !== u.effectiveText) && <small className="document-text">{u.recognitionIsSource?"分割元の認識原文（全文・単語位置未推定）":"認識原文"}: {u.recognizedText}</small>}
      </article>)}
      <small>固定入力SHA-256: {source.analysisContentHash}</small>
    </section>}
  </article>;
}
