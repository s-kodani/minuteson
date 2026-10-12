import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CommonModal } from "./common-modal";
import { OptionsModal } from "./options-modal";
import { IconButton } from "./action-icon";
import { IssueNotice } from "./toast";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { SpeakerLabel, SpeakerLabels } from "./speaker-label";
import { WorkspacePanel } from "./workspace-panel";
import { useTranscriptAudio } from "./transcript-audio";
import { usePlaybackFollowing } from "./playback-following";
import { useTranscriptTrackKinds } from "./transcript-track-kinds-hook";
import { appShortcut, overlayOpen } from "./keyboard";
import { canonicalPayload, projectTranscript, filterUtterances, activeSpeakers, addSpeaker, assignSpeakers, type Analysis, type DraftPayload, type Utterance } from "./transcript-state";
import { retainedTextRows } from "./transcript-edit-session";
import { shouldOpenTranscriptBody, shouldOpenTranscriptBodyFromKey } from "./transcript-body-activation";
export type { Analysis, DraftPayload } from "./transcript-state";

export interface Draft {
  id: string; meetingId: string; baseAnalysisId: string; payload: DraftPayload;
  lockVersion: number; contentHash: string; updatedAt: number;
}
export function timeLabel(microseconds: number): string {
  const seconds = Math.floor(microseconds / 1_000_000);
  return [Math.floor(seconds / 3600), Math.floor(seconds % 3600 / 60), seconds % 60].map(value => String(value).padStart(2, "0")).join(":");
}
const PAGE_SIZE = 50;
export function TranscriptEditor({ meetingId, meetingTitle, locked = false, playbackHost = null, visible = true }: {
  meetingId: string; meetingTitle?: string; locked?: boolean; playbackHost?: HTMLElement | null; visible?: boolean;
}) {
  const [analysis, setAnalysis] = useState<Analysis | null>(null);
  const analysisRef = useRef(analysis); analysisRef.current = analysis;
  const [queue] = useState(() => new DraftSaveQueue<DraftPayload, Draft>(
    async (draftId, expectedLockVersion, payload) => {
      const result = await invoke<Draft>("save_transcript_draft", { draftId, expectedLockVersion, payload });
      if (result.id !== draftId || result.meetingId !== meetingId || result.baseAnalysisId !== analysisRef.current?.id
        || result.lockVersion !== expectedLockVersion + 1 || !/^[a-f0-9]{64}$/.test(result.contentHash)
        || JSON.stringify(canonicalPayload(result.payload)) !== JSON.stringify(canonicalPayload(payload))) throw Error("文字起こしの保存結果が一致しません。入力を保持しています。");
      return result;
    }, canonicalPayload,
    async acknowledged => {
      const [base, saved] = await Promise.all([invoke<Analysis | null>("get_current_analysis", { meetingId }), invoke<Draft | null>("get_transcript_draft", { meetingId })]);
      if (base?.id !== acknowledged.baseAnalysisId || saved?.meetingId !== meetingId || saved.baseAnalysisId !== base.id) throw Error("文字起こしが変更されています。入力を保持しています。");
      return saved;
    },
  ));
  const [snapshot, setSnapshot] = useState(() => queue.snapshot());
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [texts, setTexts] = useState<Record<string, string>>({});
  const textRef = useRef(texts); textRef.current = texts;
  const [nameEditor, setNameEditor] = useState<{ id: string; value: string } | null>(null);
  const nameRef = useRef(nameEditor); nameRef.current = nameEditor;
  const [adding, setAdding] = useState(false), [newName, setNewName] = useState("");
  const addingRef = useRef(false); addingRef.current = adding;
  const nameOpener = useRef<HTMLElement | null>(null), addOpener = useRef<HTMLElement | null>(null);
  const mounted = useRef(true), running = useRef(false), barrier = useRef(async () => {});
  const [speakerMount] = useState(() => { const node = document.createElement("div"); node.className = "workspace-panel-body speaker-panel-body"; return node; });
  const [speakerFilter, setSpeakerFilter] = useState(""), [query, setQuery] = useState(""), [page, setPage] = useState(0);
  const audio = useTranscriptAudio(meetingId, locked);
  const matching = !!analysis && snapshot.draft?.baseAnalysisId === analysis.id;
  const view = useMemo(() => analysis ? projectTranscript(analysis, matching ? snapshot.payload : null) : null, [analysis, matching, snapshot.payload]);
  const speakers = view ? activeSpeakers(view, snapshot.payload) : [];
  const filtered = useMemo(() => view ? filterUtterances(view, speakerFilter, query) : [], [view, speakerFilter, query]);
  const shown = retainedTextRows(view?.utterances ?? [], filtered, page, PAGE_SIZE, Object.keys(texts)).rows;
  const followPort = useRef<HTMLDivElement>(null);
  const tracks = useTranscriptTrackKinds(meetingId, analysis?.id ?? null);
  const following = usePlaybackFollowing({ meetingId, analysisId: analysis?.id ?? null, rows: filtered, page, pageSize: PAGE_SIZE, setPage,
    editing: !!Object.keys(texts).length || !!nameEditor || adding || snapshot.saving || snapshot.dirty || busy || loading,
    active: visible && !locked, port: followPort, trackKinds: tracks.kinds, highlightRows: shown });
  const disabled = locked || loading || busy;
  const editable = !disabled && !!analysis;
  const run = async (action: () => Promise<void>) => {
    if (running.current || locked) return;
    running.current = true; setBusy(true); setError("");
    try { await action(); } catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { running.current = false; if (mounted.current) setBusy(false); }
  };
  const ensureDraft = async () => {
    const saved = queue.snapshot().draft;
    if (saved) {
      if (saved.meetingId !== meetingId || saved.baseAnalysisId !== analysis?.id) throw Error("編集対象が変わりました。入力を保持しています。");
      return;
    }
    if (!analysis) throw Error("文字起こしがありません。");
    const result = await invoke<Draft>("begin_transcript_draft", { meetingId, analysisId: analysis.id });
    if (result.meetingId !== meetingId || result.baseAnalysisId !== analysis.id) throw Error("編集対象が変わりました。");
    queue.load(result);
  };
  const stage = (inputs: Record<string, string>, name: typeof nameEditor) => queue.edit(payload => ({
    ...payload,
    utterances: [...payload.utterances.filter(item => !Object.hasOwn(inputs, item.utterance_id)), ...Object.entries(inputs).map(([id, text]) => {
      const row = projectTranscript(analysis!, payload).utterances.find(item => item.id === id);
      if (!row) throw Error("編集した発言が見つかりません。入力を保持しています。");
      return { utterance_id: id, effective_text: text, speaker_ids: row.speakerIds };
    })],
    speaker_names: name ? [...payload.speaker_names.filter(item => item.speaker_id !== name.id), { speaker_id: name.id, display_name: name.value.trim() || null }] : payload.speaker_names,
  }));
  const saveInputs = async (inputs: Record<string, string>, name: typeof nameEditor, retry = false) => {
    const focusedRow = document.activeElement?.closest<HTMLElement>("[data-utterance-id]");
    const returnFocus = focusedRow && Object.hasOwn(inputs, focusedRow.dataset.utteranceId ?? "") ? focusedRow : null;
    await ensureDraft(); stage(inputs, name); await (retry ? queue.retry() : queue.flush());
    if (!mounted.current) return;
    setTexts(current => Object.fromEntries(Object.entries(current).filter(([id, value]) => inputs[id] !== value)));
    if (name && nameRef.current?.id === name.id && nameRef.current.value === name.value) setNameEditor(null);
    requestAnimationFrame(() => {
      if (returnFocus?.isConnected && visible && !overlayOpen() && (document.activeElement === document.body || returnFocus.contains(document.activeElement))) {
        returnFocus.querySelector<HTMLElement>(".transcript-body-edit")?.focus({ preventScroll: true });
      }
    });
  };
  barrier.current = async () => {
    if (running.current) throw Error("文字起こしを保存中です。完了後に操作してください。");
    if (addingRef.current) throw Error("話者の作成を保存するか、閉じてから操作してください。");
    const inputs = { ...textRef.current }, name = nameRef.current && { ...nameRef.current };
    if (!Object.keys(inputs).length && !name) { await queue.flush(); return; }
    running.current = true; setBusy(true);
    try { await saveInputs(inputs, name); } finally { running.current = false; if (mounted.current) setBusy(false); }
  };
  useEffect(() => {
    mounted.current = true;
    const unsubscribe = queue.subscribe(() => { if (mounted.current) setSnapshot(queue.snapshot()); });
    const unregister = registerPendingEdits(() => barrier.current());
    if (isTauri()) void Promise.all([invoke<Analysis | null>("get_current_analysis", { meetingId }), invoke<Draft | null>("get_transcript_draft", { meetingId })]).then(([base, draft]) => {
      if (!mounted.current) return;
      if (base && base.meetingId !== meetingId || draft && (!base || draft.meetingId !== meetingId || draft.baseAnalysisId !== base.id)) throw Error("保存された文字起こしと編集が一致しません。");
      setAnalysis(base); queue.load(draft);
    }).catch(cause => { if (mounted.current) setError(String(cause)); }).finally(() => { if (mounted.current) setLoading(false); });
    else setLoading(false);
    return () => { mounted.current = false; unsubscribe(); unregister(); speakerMount.remove(); };
  }, [meetingId, queue, speakerMount]);
  useEffect(() => { setPage(0); }, [speakerFilter, query]);
  const openText = (utterance: Utterance) => { if (editable) setTexts(current => Object.hasOwn(current, utterance.id) ? current : { ...current, [utterance.id]: utterance.effectiveText }); };
  const cancelText = (id: string) => { if (!disabled && !snapshot.error && !snapshot.dirty) {
    const row = document.activeElement?.closest<HTMLElement>("[data-utterance-id]");
    setTexts(current => Object.fromEntries(Object.entries(current).filter(([key]) => key !== id)));
    requestAnimationFrame(() => { if (row?.isConnected && row.dataset.utteranceId === id && !overlayOpen()) row.querySelector<HTMLElement>(".transcript-body-edit")?.focus({ preventScroll: true }); });
  } };
  const openName = (id: string, opener: HTMLElement) => {
    const speaker = speakers.find(item => item.id === id); if (!speaker || !editable) return;
    nameOpener.current = opener; setNameEditor({ id, value: speaker.displayName ?? "" });
  };
  const retry = () => run(() => saveInputs({ ...textRef.current }, nameRef.current && { ...nameRef.current }, true));
  const play = (utterance: Utterance) => {
    if (!analysis) return;
    void audio.start(analysis.id, utterance.id, utterance.sourceUtteranceId && snapshot.draft ? {
      meetingId, analysisId: analysis.id, expectedContentHash: analysis.contentHash, expectedDraft: { id: snapshot.draft.id, lockVersion: snapshot.draft.lockVersion },
      utteranceId: utterance.id, expectedStartUs: utterance.startUs, expectedEndUs: utterance.endUs,
    } : undefined);
  };
  return <section className="transcript-card" aria-label="文字起こし編集">
    {loading && <p className="transcript-state" role="status">文字起こしを読み込んでいます。</p>}
    {!loading && !analysis && !error && <p className="transcript-state">文字起こしはまだありません。</p>}
    {analysis && <div className="transcript-split">
      <WorkspacePanel content={speakerMount} label="話者一覧" active={visible} query="(max-width: 760px)" />
      {createPortal(<>
        <div className="speaker-heading"><h3>話者</h3><IconButton label="話者を追加" icon="plus" disabled={!editable || speakers.length >= 100} onClick={event => { addOpener.current = event.currentTarget; setNewName(""); setAdding(true); }} /></div>
        <div className="speaker-filters" aria-label="話者一覧から絞り込み">
          <button type="button" className="speaker-filter" aria-pressed={!speakerFilter} onClick={() => setSpeakerFilter("")}>すべての発言 <span>{view?.utterances.length ?? 0}</span></button>
          {view?.utterances.some(item => !item.speakerIds.length) && <button type="button" className="speaker-filter" aria-pressed={speakerFilter === "unknown"} onClick={() => setSpeakerFilter("unknown")}>話者不明の発言</button>}
        </div>
        <div className="speaker-grid">{speakers.map(speaker => {
          const name = speaker.displayName || speaker.label;
          return <div key={speaker.id} className="speaker-list-row" data-selected={speakerFilter === speaker.id} role="group" aria-label={name}>
            <button type="button" className="speaker-name-button" aria-label={name + "の名前を変更"} aria-haspopup="dialog" disabled={!editable} onClick={event => openName(speaker.id, event.currentTarget)}><SpeakerLabel meetingId={meetingId} speakerId={speaker.id} name={name} /></button>
            <button type="button" className="speaker-count-filter" aria-label={name + "の発言で絞り込み"} aria-pressed={speakerFilter === speaker.id} onClick={() => setSpeakerFilter(speaker.id)}>{view?.utterances.filter(item => item.speakerIds.includes(speaker.id)).length ?? 0}発言</button>
          </div>;
        })}</div>
      </>, speakerMount)}
      <div className="transcript-main">
        <div className="transcript-actions"><h2>発言一覧</h2><span role="status">{snapshot.error ? "保存要確認" : snapshot.saving || busy ? "保存中" : Object.keys(texts).length || nameEditor || snapshot.dirty ? "編集中" : "保存済み"}</span></div>
        <div className="transcript-actions transcript-search"><label><span className="sr-only">発言を検索</span><input type="search" placeholder="発言を検索" value={query} onChange={event => setQuery(event.target.value)} maxLength={1000} /></label><span>{filtered.length} / {view?.utterances.length ?? 0} 発言</span></div>
        <div ref={followPort} {...following.handlers} className="utterance-list" role="region" aria-label="発言本文" tabIndex={0}>{shown.map(utterance => <article key={utterance.id} data-utterance-id={utterance.id} className={`utterance-row${following.playingIds.has(utterance.id) ? " utterance-playing" : ""}`} aria-current={following.playingIds.has(utterance.id) ? "true" : undefined} tabIndex={0} aria-label={`${timeLabel(utterance.startUs)}の発言行`} aria-keyshortcuts="Space" onKeyDown={event => {
          if (event.target !== event.currentTarget || appShortcut(event.nativeEvent) !== "playback" || overlayOpen() || disabled || audio.preparing) return;
          event.preventDefault(); if (audio.selected === utterance.id && audio.available) audio.toggle(); else play(utterance);
        }}>
          <div className="utterance-meta"><SpeakerLabels meetingId={meetingId} speakerIds={utterance.speakerIds} speakers={speakers} /><button type="button" className="utterance-stamp" aria-label="音声を再生" title={timeLabel(utterance.startUs) + "–" + timeLabel(utterance.endUs)} disabled={disabled || audio.preparing} onClick={() => play(utterance)}><time>{timeLabel(utterance.startUs)}</time></button></div>
          {Object.hasOwn(texts, utterance.id) ? <div className="utterance-text-edit">
            <textarea autoFocus aria-label={timeLabel(utterance.startUs) + "の発言"} rows={2} maxLength={20000} disabled={disabled} value={texts[utterance.id]} onChange={event => setTexts(current => ({ ...current, [utterance.id]: event.target.value }))} onKeyDown={event => {
              if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229 || event.repeat) return;
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); void run(() => saveInputs({ [utterance.id]: textRef.current[utterance.id] }, null)); }
              if (event.key === "Escape") { event.preventDefault(); cancelText(utterance.id); }
            }} />
            <div className="transcript-actions"><button type="button" disabled={disabled || !!snapshot.error} onClick={() => void run(() => saveInputs({ [utterance.id]: textRef.current[utterance.id] }, null))}>保存</button><button type="button" className="small-secondary" disabled={disabled || !!snapshot.error || snapshot.dirty} onClick={() => cancelText(utterance.id)}>キャンセル</button></div>
          </div> : <p className="utterance-text"><span role="button" className="transcript-body-edit" aria-label={timeLabel(utterance.startUs) + "の発言を編集"} aria-disabled={!editable} tabIndex={editable ? 0 : -1} onKeyDown={event => { if (shouldOpenTranscriptBodyFromKey(event.nativeEvent)) { event.preventDefault(); openText(utterance); } }} onClick={event => { const body = event.currentTarget; if (shouldOpenTranscriptBody(event.detail, body.ownerDocument.getSelection(), node => body.contains(node))) openText(utterance); }}>{utterance.effectiveText}</span></p>}
          <OptionsModal className="utterance-edit-tools" label={timeLabel(utterance.startUs) + "の話者を変更"} triggerLabel="⋯" hidden={!visible}>
            <h3>この発言の話者</h3>
            {speakers.map(speaker => <label key={speaker.id}><input type="checkbox" disabled={!editable || !!snapshot.error} checked={utterance.speakerIds.includes(speaker.id)} onChange={event => { const checked = event.target.checked; void run(async () => {
              await ensureDraft(); queue.edit(payload => assignSpeakers(analysis, payload, [utterance.id], checked ? [...utterance.speakerIds, speaker.id] : utterance.speakerIds.filter(id => id !== speaker.id))); await queue.flush();
            }); }} />{speaker.displayName || speaker.label}</label>)}
          </OptionsModal>
        </article>)}</div>
        {!filtered.length && <p>条件に一致する発言はありません。</p>}
        {filtered.length > PAGE_SIZE && <div className="transcript-pages"><IconButton label="前へ" icon="previous" disabled={!page} onClick={() => { following.suspend(); setPage(value => value - 1); }} /><span>{page + 1} / {Math.ceil(filtered.length / PAGE_SIZE)}</span><IconButton label="次へ" icon="next" disabled={(page + 1) * PAGE_SIZE >= filtered.length} onClick={() => { following.suspend(); setPage(value => value + 1); }} /></div>}
      </div>
    </div>}
    <IssueNotice scope={meetingId} type="transcript-save" label="文字起こしの保存" message={snapshot.error || error} toastMessage="文字起こしを保存できません。入力は保持しています" action={snapshot.error ? { kind: "retry", label: "文字起こし保存を再試行", disabled, run: retry } : undefined} />
    {snapshot.error && <button type="button" className="small-secondary" disabled={disabled} onClick={() => void retry()}>保存を再試行</button>}
    <CommonModal open={visible && !!nameEditor} blocked={disabled || !!snapshot.error} onDismiss={() => setNameEditor(null)} restoreTarget={() => nameOpener.current} className="speaker-name-editor speaker-name-dialog" aria-label="話者名を変更">
      <h2>話者名を変更</h2><label>表示名<input autoFocus value={nameEditor?.value ?? ""} maxLength={200} disabled={disabled} onChange={event => setNameEditor(current => current && { ...current, value: event.target.value })} /></label>
      {snapshot.error && <p className="action-error" role="alert">入力を保持しています。{snapshot.error}</p>}
      <div className="transcript-actions"><button type="button" disabled={disabled || !!snapshot.error} onClick={() => { if (nameRef.current) void run(() => saveInputs({}, { ...nameRef.current! })); }}>保存</button>{snapshot.error && <button type="button" disabled={disabled} onClick={() => void retry()}>保存を再試行</button>}</div>
    </CommonModal>
    <CommonModal open={visible && adding} blocked={disabled || !!snapshot.error} onDismiss={() => setAdding(false)} restoreTarget={() => addOpener.current} className="speaker-add-dialog" aria-label="新しい話者">
      <h3>新しい話者</h3><label>表示名<input autoFocus value={newName} maxLength={200} disabled={disabled} onChange={event => setNewName(event.target.value)} /></label>
      {snapshot.error && <p className="action-error" role="alert">入力を保持しています。{snapshot.error}</p>}
      <div className="transcript-actions"><button type="button" disabled={disabled || !!snapshot.error} onClick={() => void run(async () => {
        await ensureDraft(); queue.edit(payload => addSpeaker(analysis!, payload, "manual:" + crypto.randomUUID(), newName)); await queue.flush(); if (mounted.current) setAdding(false);
      })}>作成して保存</button>{snapshot.error && <button type="button" disabled={disabled} onClick={() => void run(async () => { await queue.retry(); if (mounted.current) setAdding(false); })}>保存を再試行</button>}</div>
    </CommonModal>
    {audio.sharedSurface && audio.controls}
    {!audio.sharedSurface && (playbackHost ? createPortal(<div className="meeting-playback-content" data-meeting-id={meetingId}><span className="playback-owner">{meetingTitle || "選択会議"}</span>{audio.controls || <p className="playback-empty">発言の時刻を選ぶと音声を再生します。</p>}</div>, playbackHost) : audio.controls)}
  </section>;
}
