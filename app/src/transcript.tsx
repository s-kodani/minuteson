import { SpeakerStructureDialog, type SpeakerStructureEdit } from "./speaker-structure-dialog";
import { VoicePanel } from "./voice";
import { listen } from "@tauri-apps/api/event";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { createPortal } from "react-dom";
import { CommonModal } from "./common-modal";
import { IconButton } from "./action-icon";
import { IssueNotice } from "./toast";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { SpeakerLabel, SpeakerLabels } from "./speaker-label";
import { IdentificationBadge } from "./speaker-identification";
import { WorkspacePanel } from "./workspace-panel";
import { useMeetingAudio } from "./meeting-audio";
import { playingRows, playbackTimeline } from "./playback-follow";
import { usePlaybackHighlights } from "./playback-highlights";
import { useTranscriptTrackKinds } from "./transcript-track-kinds-hook";
import { appShortcut, overlayOpen } from "./keyboard";
import { canonicalPayload, projectTranscript, filterUtterances, activeSpeakers, addSpeaker, assignSpeakers, mergeSpeaker, splitUtterance, splitPosition, textPieces, type Analysis, type DraftPayload, type Utterance } from "./transcript-state";
import { retainedTextRows } from "./transcript-edit-session";
import { stopAllPlayback } from "./playback-barrier";
import { groupTranscriptRows } from "./transcript-groups";
import "./transcript-groups.css";
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
export function TranscriptEditor({ meetingId, locked = false, visible = true, onAnalysisLoaded, onConsolidated }: {
  meetingId: string; meetingTitle?: string; locked?: boolean; playbackHost?: HTMLElement | null; visible?: boolean; onAnalysisLoaded?: (present: boolean) => void; onConsolidated?: () => Promise<void>;
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
  const [assignmentEditor, setAssignmentEditor] = useState<{ utteranceId:string; analysisId:string; startUs:number; speakerIds:string[] } | null>(null);
  const assignmentRef = useRef(assignmentEditor); assignmentRef.current = assignmentEditor;
  const assignmentOpener = useRef<HTMLElement | null>(null);
  const [structureEditor,setStructureEditor]=useState<SpeakerStructureEdit|null>(null);
  const structureRef=useRef(structureEditor);structureRef.current=structureEditor;
  const structureOpener=useRef<HTMLElement|null>(null);
  const [adding, setAdding] = useState(false), [newName, setNewName] = useState("");
  const addingRef = useRef(false); addingRef.current = adding;
  const nameOpener = useRef<HTMLElement | null>(null), addOpener = useRef<HTMLElement | null>(null);
  const mounted = useRef(true), running = useRef(false), barrier = useRef(async () => {});
  const bodyFocus = useRef<HTMLElement | null>(null);
  const transcriptRoot=useRef<HTMLElement|null>(null);
  const [jumpRequest,setJumpRequest]=useState<{id:string}|null>(null);
  const [speakerMount] = useState(() => { const node = document.createElement("div"); node.className = "workspace-panel-body speaker-panel-body"; return node; });
  const [speakerFilter, setSpeakerFilter] = useState(""), [query, setQuery] = useState("");
  const audio = useMeetingAudio(meetingId);
  const matching = !!analysis && snapshot.draft?.baseAnalysisId === analysis.id;
  const view = useMemo(() => analysis ? projectTranscript(analysis, matching ? snapshot.payload : null) : null, [analysis, matching, snapshot.payload]);
  const speakers = view ? activeSpeakers(view, snapshot.payload) : [];
  const filtered = useMemo(() => view ? filterUtterances(view, speakerFilter, query) : [], [view, speakerFilter, query]);
  const shown = retainedTextRows(view?.utterances ?? [], filtered, Object.keys(texts)).rows;
  const consolidationPossible=useMemo(()=>!!view&&groupTranscriptRows(view.utterances,view.utterances).some(group=>group.length>1),[view]);
  const consolidationAttempt=useRef("");
  const consolidationOperation=useRef<{key:string;id:string}|null>(null);
  const [consolidationRetry,setConsolidationRetry]=useState(0),[consolidationError,setConsolidationError]=useState("");
  const tracks = useTranscriptTrackKinds(meetingId, analysis?.id ?? null);
  const timeline=useSyncExternalStore(playbackTimeline.subscribe,playbackTimeline.snapshot);
  const positionRows=playingRows(view?.utterances??[],timeline?{...timeline,playing:true}:null,meetingId,analysis?.id??null,tracks.kinds);
  const jumpRow=view?.utterances[positionRows[0]];
  const jumpToPlayback=()=>{
    if(!view||!jumpRow)return;
    const hidden=!filtered.some(row=>row.id===jumpRow.id);
    if(hidden){setQuery("");setSpeakerFilter("");}
    setJumpRequest({id:jumpRow.id});
  };
  useLayoutEffect(()=>{
    if(!jumpRequest||!visible)return;
    const row=transcriptRoot.current?.querySelector<HTMLElement>(`[data-utterance-id="${CSS.escape(jumpRequest.id)}"]`);
    row?.scrollIntoView({block:"center",behavior:"auto"});
    setJumpRequest(null);
  },[jumpRequest,visible]);
  const playingIds = usePlaybackHighlights({ meetingId, analysisId: analysis?.id ?? null, rows: shown,
    active: visible && !locked, trackKinds: tracks.kinds });
  const disabled = locked || loading || busy;
  const editable = !disabled && !!analysis;
  // Restore only after React has replaced the editor and unlocked its row.
  // A frame callback can precede that commit (notably in Chromium).
  useLayoutEffect(() => {
    const row = bodyFocus.current;
    if (!row) return;
    if (!row.isConnected || !visible || overlayOpen()) { bodyFocus.current = null; return; }
    if (disabled) return;
    if (document.activeElement !== document.body && !row.contains(document.activeElement)) { bodyFocus.current = null; return; }
    const body = row.querySelector<HTMLElement>(".transcript-body-edit");
    if (body) { bodyFocus.current = null; body.focus({ preventScroll: true }); }
  });
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
    bodyFocus.current = returnFocus ?? null;
    setTexts(current => Object.fromEntries(Object.entries(current).filter(([id, value]) => inputs[id] !== value)));
    if (name && nameRef.current?.id === name.id && nameRef.current.value === name.value) setNameEditor(null);
  };
  barrier.current = async () => {
    if (running.current) throw Error("文字起こしを保存中です。完了後に操作してください。");
    if (structureRef.current) throw Error("分割・統合を保存するか、閉じてから操作してください。");
    if (assignmentRef.current) throw Error("話者の変更を保存するか、閉じてから操作してください。");
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
      setAnalysis(base); queue.load(draft); onAnalysisLoaded?.(!!base);
    }).catch(cause => { if (mounted.current) setError(String(cause)); }).finally(() => { if (mounted.current) setLoading(false); });
    else setLoading(false);
    return () => { mounted.current = false; unsubscribe(); unregister(); speakerMount.remove(); };
  }, [meetingId, queue, speakerMount]);
  useEffect(()=>{
    if(!analysis||!view||!consolidationPossible||loading||busy||locked||!visible||running.current||snapshot.error||snapshot.saving||snapshot.dirty
      ||Object.keys(texts).length||nameEditor||assignmentEditor||structureEditor||adding||overlayOpen())return;
    const key=`${analysis.id}:${snapshot.draft?.contentHash??""}:${consolidationRetry}`;
    if(consolidationAttempt.current===key)return;
    consolidationAttempt.current=key;
    const sourceKey=`${analysis.id}:${snapshot.draft?.contentHash??""}`;
    if(consolidationOperation.current?.key!==sourceKey)consolidationOperation.current={key:sourceKey,id:crypto.randomUUID()};
    const operationId=consolidationOperation.current.id;
    void run(async()=>{
      setConsolidationError("");
      try {
        await stopAllPlayback();
        const result=await invoke<{analysis:Analysis;draft:Draft|null;changed:boolean}>("consolidate_transcript",{request:{meetingId,analysisId:analysis.id,expectedContentHash:analysis.contentHash,expectedDraft:snapshot.draft?{id:snapshot.draft.id,lockVersion:snapshot.draft.lockVersion}:null,operationId}});
        if(!mounted.current)return;
        if(!result||result.analysis.meetingId!==meetingId||!/^[a-f0-9]{64}$/.test(result.analysis.contentHash)
          ||result.changed&&(result.analysis.id===analysis.id||result.draft!==null||result.analysis.utterances.length>=view.utterances.length)
          ||!result.changed&&(result.analysis.id!==analysis.id||result.analysis.contentHash!==analysis.contentHash))throw Error("発言統合の保存結果が一致しません。再読込してください。");
        if(result.changed){setSpeakerFilter("");setAnalysis(result.analysis);queue.load(result.draft);await onConsolidated?.();}
      } catch(cause) {if(mounted.current)setConsolidationError(String(cause));}
    });
  },[analysis,view,consolidationPossible,loading,busy,locked,visible,snapshot.draft,snapshot.error,snapshot.saving,snapshot.dirty,texts,nameEditor,assignmentEditor,structureEditor,adding,consolidationRetry]);
  const openText = (utterance: Utterance) => { if (editable) setTexts(current => Object.hasOwn(current, utterance.id) ? current : { ...current, [utterance.id]: utterance.effectiveText }); };
  const cancelText = (id: string) => { if (!disabled && !snapshot.error && !snapshot.dirty) {
    const row = document.activeElement?.closest<HTMLElement>("[data-utterance-id]");
    bodyFocus.current = row?.dataset.utteranceId === id ? row : null;
    setTexts(current => Object.fromEntries(Object.entries(current).filter(([key]) => key !== id)));
  } };
  const openName = (id: string, opener: HTMLElement) => {
    const speaker = speakers.find(item => item.id === id); if (!speaker || !editable) return;
    nameOpener.current = opener; setNameEditor({ id, value: speaker.displayName ?? "" });
  };
  const saveAssignment = async (retry = false) => {
    const editing = assignmentRef.current;
    if (!editing || !analysis || editing.analysisId !== analysis.id) throw Error("話者の変更対象が変わりました。会議を開き直してください。");
    if (retry) await queue.retry();
    else {
      await ensureDraft();
      queue.edit(payload => assignSpeakers(analysis, payload, [editing.utteranceId], editing.speakerIds));
      await queue.flush();
    }
    if (mounted.current) setAssignmentEditor(null);
  };
  const saveStructure = async (retry=false) => {
    const edit=structureRef.current;
    if(!edit||!analysis||edit.analysisId!==analysis.id)throw Error("編集する解析版が変更されています。");
    if(retry)await queue.retry();
    else {
      await ensureDraft();
      queue.edit(payload=>{
        if(edit.kind==="merge")return mergeSpeaker(analysis,payload,edit.sourceId,edit.targetId);
        const selected=edit.utterance;
        const staged={...payload,utterances:[...payload.utterances.filter(u=>u.utterance_id!==selected.id),{utterance_id:selected.id,effective_text:selected.effectiveText,speaker_ids:selected.speakerIds}]};
        const assignments=[edit.first,edit.second].map(id=>id==="keep"?selected.speakerIds:id?[id]:[]) as [string[],string[]];
        return splitUtterance(analysis,staged,selected.id,splitPosition(selected.startUs,selected.endUs,edit.seconds),edit.offset,[`split:${crypto.randomUUID()}`,`split:${crypto.randomUUID()}`],assignments);
      });
      await queue.flush();
    }
    if(!mounted.current)return;
    if(edit.kind==="split"){
      setTexts(current=>Object.fromEntries(Object.entries(current).filter(([id,value])=>id!==edit.utterance.id||value!==edit.utterance.effectiveText)));
      setAssignmentEditor(null);
    } else if(speakerFilter===edit.sourceId)setSpeakerFilter(edit.targetId);
    setStructureEditor(null);
  };
  const retry = () => run(() => structureRef.current ? saveStructure(true) : assignmentRef.current ? saveAssignment(true) : saveInputs({ ...textRef.current }, nameRef.current && { ...nameRef.current }, true));
  const play = (utterance: Utterance) => {
    if (!analysis || !audio) return;
    void audio.playReference(utterance.sourceUtteranceId && snapshot.draft ? { kind:"draftUtterance", request: {
      meetingId, analysisId: analysis.id, expectedContentHash: analysis.contentHash, expectedDraft: { id: snapshot.draft.id, lockVersion: snapshot.draft.lockVersion },
      utteranceId: utterance.id, expectedStartUs: utterance.startUs, expectedEndUs: utterance.endUs,
    } } : { kind:"utterance", analysisId:analysis.id, utteranceId:utterance.id, expectedContentHash:analysis.contentHash,
      startUs:utterance.startUs, endUs:utterance.endUs });
  };
  const [transcriptRefresh,setTranscriptRefresh]=useState(0);
  const appliedRefresh=useRef(0);
  const reloadVoice=async()=>{setTranscriptRefresh(value=>value+1);};
  useEffect(()=>{
    if(!isTauri())return;
    let disposed=false;
    const subscriptions=["voice-identity-changed","analysis-version-changed"].map(event=>listen<string>(event,message=>{
      if(message.payload===meetingId&&!disposed)setTranscriptRefresh(value=>value+1);
    }));
    return()=>{disposed=true;for(const subscription of subscriptions)void subscription.then(unlisten=>unlisten());};
  },[meetingId]);
  useEffect(()=>{
    // Completion publishes a new immutable revision. Keep its notification
    // pending until the editor is idle; never flush unconfirmed inputs here.
    if(transcriptRefresh===appliedRefresh.current||loading||busy||locked||!visible||running.current
      ||snapshot.dirty||snapshot.saving||snapshot.error||Object.keys(texts).length||nameEditor||adding||assignmentEditor||structureEditor)return;
    let disposed=false;
    const current=queue.snapshot();
    void Promise.all([invoke<Analysis|null>("get_current_analysis",{meetingId}),invoke<Draft|null>("get_transcript_draft",{meetingId})]).then(([base,draft])=>{
      const latest=queue.snapshot();
      // Inputs or a local operation may have started while the reads ran.
      if(disposed||!mounted.current||running.current||analysisRef.current!==analysis||latest.draft!==current.draft||latest.payload!==current.payload
        ||latest.dirty||latest.saving||latest.error||Object.keys(textRef.current).length||nameRef.current||addingRef.current||assignmentRef.current||structureRef.current)return;
      if(!base||base.meetingId!==meetingId||!/^[a-f0-9]{64}$/.test(base.contentHash)
        ||draft&&(draft.meetingId!==meetingId||draft.baseAnalysisId!==base.id))throw Error("保存された文字起こしと編集が一致しません。再読込してください。");
      appliedRefresh.current=transcriptRefresh;
      consolidationAttempt.current="";
      if(base.id!==analysis?.id)setSpeakerFilter("");
      setAnalysis(base);queue.load(draft);
    }).catch(cause=>{if(!disposed&&mounted.current){appliedRefresh.current=transcriptRefresh;setError(String(cause));}});
    return()=>{disposed=true;};
  },[transcriptRefresh,meetingId,analysis,queue,loading,busy,locked,visible,snapshot.draft,snapshot.payload,snapshot.dirty,snapshot.saving,snapshot.error,texts,nameEditor,adding,assignmentEditor,structureEditor]);
  return <section ref={transcriptRoot} className="transcript-card" aria-label="文字起こし編集">
    {loading && <p className="transcript-state" role="status">文字起こしを読み込んでいます。</p>}
    {!loading && !analysis && !error && <p className="transcript-state">文字起こしはまだありません。</p>}
    {analysis && <div className="transcript-split">
      <WorkspacePanel content={speakerMount} label="話者一覧" active={visible} query="(max-width: 760px)" />
      {createPortal(<>
        <div className="speaker-heading"><h3>話者</h3><IconButton label="話者を追加" icon="plus" disabled={!editable || speakers.length >= 100} onClick={event => { addOpener.current = event.currentTarget; setNewName(""); setAdding(true); }} /></div>
        <button type="button" className="small-secondary speaker-merge-trigger" disabled={!editable||!!snapshot.error||speakers.length<2} onClick={event=>{
          structureOpener.current=event.currentTarget;setError("");setStructureEditor({kind:"merge",analysisId:analysis.id,sourceId:speakers[0]?.id??"",targetId:speakers[1]?.id??"",choices:speakers.map(s=>({...s})),affected:view?.utterances.filter(u=>u.speakerIds.includes(speakers[0]?.id)).length??0});
        }}>話者を統合</button>
        <div className="speaker-filters" aria-label="話者一覧から絞り込み">
          <button type="button" className="speaker-filter" aria-pressed={!speakerFilter} onClick={() => setSpeakerFilter("")}>すべての発言 <span>{view?.utterances.length ?? 0}</span></button>
          {view?.utterances.some(item => !item.speakerIds.length) && <button type="button" className="speaker-filter" aria-pressed={speakerFilter === "unknown"} onClick={() => setSpeakerFilter("unknown")}>話者不明の発言 <span>{view.utterances.filter(item => !item.speakerIds.length).length}</span></button>}
        </div>
        <div className="speaker-grid">{speakers.map(speaker => {
          const name = speaker.displayName || speaker.label;
          return <div key={speaker.id} className="speaker-list-row" data-selected={speakerFilter === speaker.id} role="group" aria-label={name}>
            <button type="button" className="speaker-name-button" aria-label={name + "の名前を変更"} aria-haspopup="dialog" disabled={!editable} onClick={event => openName(speaker.id, event.currentTarget)}><SpeakerLabel meetingId={meetingId} speakerId={speaker.id} name={name} /></button>
            <button type="button" className="speaker-count-filter" aria-label={name + "の発言で絞り込み"} aria-pressed={speakerFilter === speaker.id} onClick={() => setSpeakerFilter(speaker.id)}>{view?.utterances.filter(item => item.speakerIds.includes(speaker.id)).length ?? 0}発言</button>
          </div>;
        })}</div>
        <VoicePanel meetingId={meetingId} speakers={speakers} revision={`${analysis.id}:${analysis.contentHash}:${snapshot.draft?.contentHash??""}`} locked={disabled||!!snapshot.error||!visible} beforeOperation={()=>barrier.current()} onChanged={reloadVoice} onPlay={async window=>{
          if(!audio?.canPlay||audio.locked||audio.durationUs===null||!analysis)throw Error("登録元の音声を再生できません。");
          const mode="mixed" as const;
          if(!audio.modes.includes("mixed"))throw Error("登録元の音声トラックを確認できません。");
          await audio.start(analysis.id,`voice-preview:${window.trackId}:${window.startUs}`,{meetingId,mode,startUs:window.startUs,endUs:window.endUs,expectedDurationUs:audio.durationUs});
        }} />
      </>, speakerMount)}
      <div className="transcript-main">
        {!!analysis.reviewRanges?.length && <section className="recognition-review" aria-label="文字起こしの要確認区間">
          <p role="status">完了・要確認あり（{analysis.reviewRanges.length}区間）{!analysis.utterances.length && "。認識できた本文はありません。"}</p>
          <ul>{analysis.reviewRanges.map((range,index)=><li key={`${range.trackId}:${range.startUs}:${index}`}>
            <button type="button" className="utterance-stamp" disabled={disabled||!audio?.canPlay||audio.locked||audio.durationUs===null} onClick={()=>void audio?.playReference({kind:"interval",startUs:range.startUs,endUs:range.endUs,expectedDurationUs:audio.durationUs!})}>{timeLabel(range.startUs)}–{timeLabel(range.endUs)}</button> 認識できませんでした
          </li>)}</ul>
        </section>}

        <div className="transcript-toolbar">
          <div className="transcript-actions transcript-list-heading"><div className="transcript-heading-actions"><h2>発言一覧</h2><button type="button" className="small-secondary" disabled={!jumpRow||disabled||!visible} onClick={jumpToPlayback}>再生位置へ移動</button></div><span role="status">{snapshot.error ? "保存要確認" : snapshot.saving || busy ? "保存中" : Object.keys(texts).length || nameEditor || assignmentEditor || structureEditor || snapshot.dirty ? "編集中" : "保存済み"}</span></div>
          <div className="transcript-actions transcript-search"><label><span className="sr-only">発言を検索</span><input type="search" placeholder="発言を検索" disabled={disabled} value={query} onChange={event => setQuery(event.target.value)} maxLength={1000} /></label><span>{filtered.length} / {view?.utterances.length ?? 0} 発言</span></div>
        </div>
        <div className="utterance-list" role="region" aria-label="発言本文" tabIndex={0}>{shown.map(utterance=><article key={utterance.id} data-utterance-id={utterance.id} className={`utterance-row${playingIds.has(utterance.id)?" utterance-playing":""}`} aria-current={playingIds.has(utterance.id)?"true":undefined} tabIndex={0} aria-label={`${timeLabel(utterance.startUs)}の発言行`} aria-keyshortcuts="Space" onKeyDown={event=>{
          if(event.target!==event.currentTarget||appShortcut(event.nativeEvent)!=="playback"||overlayOpen()||disabled||!audio||!audio.canPlay||audio.locked)return;
          event.preventDefault();if(audio.selected===utterance.id&&audio.available&&!audio.atEnd)audio.toggle();else play(utterance);
        }}>
          <div className="utterance-meta"><SpeakerLabels meetingId={meetingId} speakerIds={utterance.speakerIds} speakers={speakers} /><button type="button" className="utterance-stamp" aria-label="音声を再生" title={timeLabel(utterance.startUs) + "–" + timeLabel(utterance.endUs)} disabled={disabled || !audio || !audio.canPlay || audio.locked} onClick={() => play(utterance)}><time>{timeLabel(utterance.startUs)}</time></button></div>
          <IdentificationBadge value={utterance.speakerIdentification}/>
          {Object.hasOwn(texts, utterance.id) ? <div className="utterance-text-edit">
            <textarea autoFocus aria-label={timeLabel(utterance.startUs) + "の発言"} rows={2} maxLength={20000} disabled={disabled} value={texts[utterance.id]} onChange={event => setTexts(current => ({ ...current, [utterance.id]: event.target.value }))} onKeyDown={event => {
              if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229 || event.repeat) return;
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) { event.preventDefault(); void run(() => saveInputs({ [utterance.id]: textRef.current[utterance.id] }, null)); }
              if (event.key === "Escape") { event.preventDefault(); cancelText(utterance.id); }
            }} />
            <div className="transcript-actions"><button type="button" disabled={disabled || !!snapshot.error} onClick={() => void run(() => saveInputs({ [utterance.id]: textRef.current[utterance.id] }, null))}>保存</button><button type="button" className="small-secondary" disabled={disabled || !!snapshot.error || snapshot.dirty} onClick={() => cancelText(utterance.id)}>キャンセル</button></div>
          </div> : <div className="utterance-text"><span role="button" className="transcript-body-edit" aria-label={timeLabel(utterance.startUs) + "の発言を編集"} aria-disabled={!editable} tabIndex={editable ? 0 : -1} onKeyDown={event => { if (shouldOpenTranscriptBodyFromKey(event.nativeEvent)) { event.preventDefault(); openText(utterance); } }} onClick={event => { const body = event.currentTarget; if (shouldOpenTranscriptBody(event.detail, body.ownerDocument.getSelection(), node => body.contains(node))) openText(utterance); }}>{utterance.effectiveText}</span></div>}
          <div className="utterance-edit-tools options-modal" hidden={!visible}>
            <IconButton className="options-modal-trigger" label={timeLabel(utterance.startUs) + "の話者を変更"} icon="more" aria-haspopup="dialog" disabled={!editable || !!snapshot.error} onClick={event => {
              assignmentOpener.current = event.currentTarget; setError("");
              setAssignmentEditor({ utteranceId:utterance.id, analysisId:analysis.id, startUs:utterance.startUs, speakerIds:[...utterance.speakerIds] });
            }} />
          </div>
        </article>)}</div>
        {!filtered.length && <p>条件に一致する発言はありません。</p>}
      </div>
    </div>}
    <IssueNotice scope={meetingId} type="transcript-consolidation" label="発言の統合" message={consolidationError}/>
    {consolidationError&&<button type="button" className="small-secondary" disabled={disabled} onClick={()=>setConsolidationRetry(value=>value+1)}>発言統合を再試行</button>}
    <IssueNotice scope={meetingId} type="transcript-save" label="文字起こしの保存" message={snapshot.error || error} action={snapshot.error ? { kind: "retry", label: "文字起こし保存を再試行", disabled, run: retry } : undefined} />
    {snapshot.error && <button type="button" className="small-secondary" disabled={disabled} onClick={() => void retry()}>保存を再試行</button>}
    <SpeakerStructureDialog edit={visible?structureEditor:null} speakers={speakers} disabled={disabled} issue={snapshot.error||error} saveFailed={!!snapshot.error} onChange={edit=>setStructureEditor(edit.kind==="merge"?{...edit,affected:view?.utterances.filter(u=>u.speakerIds.includes(edit.sourceId)).length??0}:edit)} onSave={retry=>void run(()=>saveStructure(retry))} onDismiss={()=>setStructureEditor(null)} restoreTarget={()=>structureOpener.current} onPlay={()=>{if(structureRef.current?.kind==="split")play(structureRef.current.utterance);}} playing={!!audio?.playing} onPause={()=>audio?.toggle()} canPlay={!!audio?.canPlay&&!audio.locked&&!audio.preparing}/>
    <CommonModal portal open={visible && !!assignmentEditor} blocked={disabled || !!snapshot.error} onDismiss={() => setAssignmentEditor(null)} restoreTarget={() => assignmentOpener.current} dismissLabel="閉じる" className="speaker-assignment-dialog" aria-label={timeLabel(assignmentEditor?.startUs ?? 0) + "の話者を変更"}>
      <h2>{timeLabel(assignmentEditor?.startUs ?? 0)}の話者を変更</h2>
      <fieldset><legend>この発言の話者</legend>
        {speakers.map(speaker => <label key={speaker.id}><input type="checkbox" disabled={!editable || !!snapshot.error} checked={assignmentEditor?.speakerIds.includes(speaker.id) ?? false} onChange={event => {
          const checked = event.target.checked;
          setAssignmentEditor(current => current && ({ ...current, speakerIds:checked ? [...current.speakerIds, speaker.id] : current.speakerIds.filter(id => id !== speaker.id) }));
        }} />{speaker.displayName || speaker.label}</label>)}
      </fieldset>

      <div className="transcript-actions"><button type="button" className="small-secondary" disabled={disabled||!!snapshot.error||!assignmentEditor||!view?.utterances.some(u=>u.id===assignmentEditor.utteranceId&&textPieces(textRef.current[u.id]??u.effectiveText).length>=2&&u.endUs-u.startUs>=2)} onClick={event=>{
        const selected=view?.utterances.find(u=>u.id===assignmentEditor?.utteranceId);if(!selected||!assignmentEditor||!analysis)return;
        structureOpener.current=event.currentTarget;setError("");setStructureEditor({kind:"split",analysisId:analysis.id,utterance:{...selected,effectiveText:textRef.current[selected.id]??selected.effectiveText,speakerIds:[...assignmentEditor.speakerIds]},seconds:((selected.endUs-selected.startUs)/2_000_000).toFixed(6),offset:Math.floor(textPieces(textRef.current[selected.id]??selected.effectiveText).length/2),first:"keep",second:"keep"});
      }}>発言を手動分割</button><button type="button" disabled={disabled || !!snapshot.error} onClick={() => void run(() => saveAssignment())}>保存</button>{snapshot.error && <button type="button" disabled={disabled} onClick={() => void run(() => saveAssignment(true))}>保存を再試行</button>}</div>
    </CommonModal>
    <CommonModal open={visible && !!nameEditor} blocked={disabled || !!snapshot.error} onDismiss={() => setNameEditor(null)} restoreTarget={() => nameOpener.current} className="speaker-name-editor speaker-name-dialog" aria-label="話者名を変更">
      <h2>話者名を変更</h2><label>表示名<input autoFocus value={nameEditor?.value ?? ""} maxLength={200} disabled={disabled} onChange={event => setNameEditor(current => current && { ...current, value: event.target.value })} /></label>

      <div className="transcript-actions"><button type="button" disabled={disabled || !!snapshot.error} onClick={() => { if (nameRef.current) void run(() => saveInputs({}, { ...nameRef.current! })); }}>保存</button>{snapshot.error && <button type="button" disabled={disabled} onClick={() => void retry()}>保存を再試行</button>}</div>
    </CommonModal>
    <CommonModal open={visible && adding} blocked={disabled || !!snapshot.error} onDismiss={() => setAdding(false)} restoreTarget={() => addOpener.current} className="speaker-add-dialog" aria-label="新しい話者">
      <h3>新しい話者</h3><label>表示名<input autoFocus value={newName} maxLength={200} disabled={disabled} onChange={event => setNewName(event.target.value)} /></label>

      <div className="transcript-actions"><button type="button" disabled={disabled || !!snapshot.error} onClick={() => void run(async () => {
        await ensureDraft(); queue.edit(payload => addSpeaker(analysis!, payload, "manual:" + crypto.randomUUID(), newName)); await queue.flush(); if (mounted.current) setAdding(false);
      })}>作成して保存</button>{snapshot.error && <button type="button" disabled={disabled} onClick={() => void run(async () => { await queue.retry(); if (mounted.current) setAdding(false); })}>保存を再試行</button>}</div>
    </CommonModal>

  </section>;
}
