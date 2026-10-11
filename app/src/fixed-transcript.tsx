import { invoke } from "@tauri-apps/api/core";
import { IconButton } from "./action-icon";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { assertMinutesTranscriptPage, evidenceRequest, minutesTranscriptAudioRequest, type Evidence, type MinutesTranscriptPage, type MinutesTranscriptRequest, type MinutesView } from "./minutes-state";
import { useTranscriptAudio } from "./transcript-audio";
import { timeLabel } from "./transcript";
import { SpeakerLabels } from "./speaker-label";

// A read-only workspace within the document, not another modal or editable
// current transcript. The host document/editor stays mounted around this view.
export function FixedTranscriptWorkspace({ view, evidence, locked, back, active = true, dockPlayback = false }: {
  view: MinutesView; evidence: Evidence; locked: boolean; back: () => void; active?: boolean; dockPlayback?: boolean;
}) {
  const [data, setData] = useState<MinutesTranscriptPage | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [pageNumber, setPageNumber] = useState("");
  const generation = useRef(0);
  const initial = useRef<MinutesTranscriptPage | null>(null);
  const pendingPage = useRef<number | null>(null);
  const busy = useRef(false);
  const scrollToTarget = useRef(true);
  const heading = useRef<HTMLHeadingElement>(null);
  const target = useRef<HTMLElement>(null);
  const audio = useTranscriptAudio(view.meetingId, locked, { inline:!dockPlayback,label:"議事録の固定文字起こし音声" });
  useEffect(() => { if (!active) audio.clear(); }, [active]);
  const load = async (page: number | null) => {
    if (busy.current) return;
    busy.current = true; pendingPage.current = page; audio.clear();
    setLoading(true); setError(""); setData(null);
    const current = ++generation.current;
    try {
      const request: MinutesTranscriptRequest = { reference: evidenceRequest(view, evidence), page };
      const result = await invoke<MinutesTranscriptPage>("get_minutes_transcript_page", { request });
      assertMinutesTranscriptPage(request, result, initial.current ?? undefined);
      if (current !== generation.current) return;
      initial.current ??= result; scrollToTarget.current = page === null;
      setData(result); setPageNumber(String(result.page + 1));
    } catch (cause) { if (current === generation.current) setError(String(cause)); }
    finally { if (current === generation.current) { busy.current = false; setLoading(false); } }
  };
  useEffect(() => {
    // Development StrictMode replays setup/cleanup on the same instance.
    // An abandoned first read must not keep the replayed setup busy forever.
    busy.current = false;
    void load(null);
    return () => { generation.current++; busy.current = false; };
    // Keyed by document and evidence; never reuse a late result for another ref.
  }, []);
  useLayoutEffect(() => {
    if (!data || !active) return;
    const destination = scrollToTarget.current ? target.current : heading.current;
    destination?.focus({ preventScroll: true });
    destination?.scrollIntoView({ block: scrollToTarget.current ? "center" : "start" });
  }, [data]);
  const play = (id: string) => {
    if (!data || !active) return;
    try { void audio.start(data.analysisId, id, minutesTranscriptAudioRequest(view, evidence, data, id)); }
    catch (cause) { setError(String(cause)); }
  };
  const pages = initial.current ? Math.ceil(initial.current.totalUtterances / initial.current.pageSize) : 0;
  return <section className="fixed-transcript-workspace" aria-label="固定文字起こし全文">
    <div className="card-head"><h3 ref={heading} tabIndex={-1}>固定した文字起こし（読取り専用）</h3><IconButton label="議事録の元の位置へ戻る" icon="previous" type="button" className="small-secondary" onClick={back} /></div>
    <p>この議事録の生成入力です。現在の文字起こしや編集には切り替えません。議事録の編集・閲覧位置を保持して戻れます。</p>
    <small>固定解析版: {evidence.analysisRevisionId}<br />固定入力SHA-256: {view.inputContentHash}</small>
    <p>根拠の固定引用: {evidence.quoteText} · {timeLabel(evidence.startUs)}–{timeLabel(evidence.endUs)} · {evidence.verificationState === "user_verified" ? "利用者確認済み" : "未確認"}</p>
    {loading && <p role="status">固定文字起こしを検証して読み込んでいます。</p>}
    {error && <div role="alert"><p className="action-error">固定文字起こしを表示できません: {error}</p><button type="button" className="small-secondary" disabled={loading || locked} onClick={() => void load(pendingPage.current)}>固定文字起こしの読込を再試行</button></div>}
    {data && <>
      <nav className="transcript-pages" aria-label="固定文字起こしのページ">
        <IconButton label="前の50発言" icon="previous" type="button" className="small-secondary" disabled={locked || data.page === 0} onClick={() => void load(data.page - 1)} />
        <span>{data.page + 1} / {pages}ページ · 全{data.totalUtterances}発言</span>
        <IconButton label="次の50発言" icon="next" type="button" className="small-secondary" disabled={locked || data.page + 1 >= pages} onClick={() => void load(data.page + 1)} />
        <button type="button" className="small-secondary" disabled={locked} onClick={() => void load(null)}>根拠の発言へ移動</button>
        <form onSubmit={(event) => { event.preventDefault(); const n = Number(pageNumber); if (!Number.isInteger(n) || n < 1 || n > pages) { setError("ページ番号は1から最終ページまでの整数で指定してください。"); return; } void load(n - 1); }}>
          <label>ページ番号<input type="number" min={1} max={pages} step={1} value={pageNumber} disabled={locked} onChange={(event) => setPageNumber(event.target.value)} /></label><button type="submit" className="small-secondary" disabled={locked}>指定ページへ移動</button>
        </form>
      </nav>
      <p>{data.firstSequence}–{data.firstSequence + data.utterances.length - 1}番目の発言を表示。本文・話者・認識原文は生成時の固定版です。</p>
      <div className="fixed-transcript">
        {data.utterances.map((u, index) => <article key={u.id} ref={u.id === evidence.utteranceId ? target : undefined} tabIndex={-1} className={u.id === evidence.utteranceId ? "evidence-target" : ""} aria-label={`固定発言${data.firstSequence + index}${u.id === evidence.utteranceId ? "・根拠の参照先" : ""}`}>
          <div className="utterance-meta"><span>#{data.firstSequence + index}</span><time>{timeLabel(u.startUs)}–{timeLabel(u.endUs)}</time><SpeakerLabels meetingId={view.meetingId} speakerIds={u.speakerIds} speakers={data.speakers}/><IconButton label={`固定発言${data.firstSequence + index}の音声を再生`} icon="play" disabled={locked || audio.preparing} onClick={() => play(u.id)} /></div>
          <p className="document-text">{u.effectiveText}</p>
          {(u.recognitionIsSource||u.effectiveText !== u.recognizedText) && <small className="document-text">{u.recognitionIsSource?"分割元の認識原文（全文・単語位置未推定）":"認識原文"}: {u.recognizedText}</small>}
          {audio.selected === u.id && audio.controls}
        </article>)}
      </div>
    </>}
  </section>;
}
