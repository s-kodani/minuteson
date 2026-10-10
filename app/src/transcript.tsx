import { invoke, isTauri } from "@tauri-apps/api/core";
import { CommonModal } from "./common-modal";
import { IssueNotice, ToastNotice } from "./toast";
import { useActionConfirmation } from "./confirmation-service";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { usePlaybackFollowing } from "./playback-following";
import { useTranscriptTrackKinds } from "./transcript-track-kinds-hook";
import { SpeakerLabel,SpeakerLabels } from "./speaker-label";
import { WorkspacePanel } from "./workspace-panel";
import { createPortal } from "react-dom";
import { confirmations } from "./confirmation-service";
import { TranscriptRecoverySaveQueue, type TranscriptRecovery } from "./transcript-recovery-save";
import { TranscriptRecoveryEditor } from "./transcript-recovery-editor";
import { TranscriptRecoveryPanel } from "./transcript-recovery-panel";
import { TranscriptConflictPanel, type ConflictPreview } from "./transcript-conflict-panel";
import { planConflictInput, resolveConflictInput, type InputChoice } from "./transcript-conflict-input";
import type { ConflictChoice } from "./transcript-conflict";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { useTranscriptAudio } from "./transcript-audio";
import { SpeakerReviewBadge,SpeakerReviewDetails,useSpeakerReviews } from "./speaker-review";
import {preciseTime} from "./diarization-state";
import { appShortcut, nativeTextTarget, overlayOpen } from "./keyboard";
import { cancelTextEdit, retainedTextRows, stageTextEdit, textBuffersDirty, textEditBaseline, type TextEditSession } from "./transcript-edit-session";
import { cancelName, nameBaseline, namesDirty, stageName, type SpeakerNameSession } from "./speaker-name-edit";

import { activeSpeakers, addSpeaker, assignSpeakers, canonicalPayload, filterUtterances, mergeSpeaker, projectTranscript, splitPosition, splitUtterance, textPieces, type Analysis, type DraftPayload, type Utterance } from "./transcript-state";
export type { Analysis, DraftPayload } from "./transcript-state";

export interface Draft {
  id: string;
  meetingId: string;
  baseAnalysisId: string;
  payload: DraftPayload;
  lockVersion: number;
  contentHash: string;
  updatedAt: number;
}

const PAGE_SIZE = 50;

export function timeLabel(microseconds: number): string {
  const seconds = Math.floor(microseconds / 1_000_000);
  return `${String(Math.floor(seconds / 3600)).padStart(2, "0")}:${String(Math.floor((seconds % 3600) / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

export function TranscriptEditor({ meetingId, meetingTitle, locked = false,playbackHost = null, visible = true, onDiarizationHost }: { meetingId: string; meetingTitle?: string; locked?: boolean; playbackHost?: HTMLElement | null; visible?: boolean; onDiarizationHost?: (meetingId: string,node: HTMLDivElement | null) => void }) {
  const confirm = useActionConfirmation(locked);
  const activeConfirmation = useSyncExternalStore(confirmations.subscribe,confirmations.getSnapshot);
  const [analysis, setAnalysis] = useState<Analysis | null>(null);
  const [recovery] = useState(() => new TranscriptRecoveryEditor(
    new TranscriptRecoverySaveQueue<DraftPayload>({
      persist: request => invoke("save_transcript_edit_recovery",{ request }),
      read: context => invoke("get_transcript_edit_recovery",{ meetingId: context.meetingId,draftId: context.draftId,baseAnalysisId: context.baseAnalysisId }),
      operationId: () => crypto.randomUUID(),normalizePayload: canonicalPayload,
    }),
    (draftId,expectedLockVersion,payload) => invoke("save_transcript_draft",{ draftId,expectedLockVersion,payload }),
    owner => invoke("get_transcript_draft",{ meetingId: owner }),
  ));
  const [recoverySnapshot,setRecoverySnapshot] = useState(() => recovery.snapshot());
  const [recoveryReady,setRecoveryReady] = useState(false);
  const [recoveryManual,setRecoveryManual] = useState(false);
  const [bodyManual,setBodyManual] = useState(false);
  const [conflictPreview,setConflictPreview] = useState<ConflictPreview | null>(null);
  const conflictRef = useRef<ConflictPreview | null>(null);
  const conflictBusy = useRef(false), conflictGeneration = useRef(0), mounted = useRef(true);
  const conflictEnvironment = useRef({ locked,analysis }); conflictEnvironment.current = { locked,analysis };
  const [discardPending,setDiscardPending] = useState(false);
  const discardPendingRef = useRef(false);
  const [saveQueue] = useState(() => new DraftSaveQueue<DraftPayload, Draft>(
    (draftId, expectedLockVersion, payload) => recovery.saveDraft(draftId,expectedLockVersion,payload),
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
  const [speakerMount] = useState(() => { const node = document.createElement("div"); node.className = "workspace-panel-body speaker-panel-body"; return node; });
  const bindDiarizationHost = useCallback((node: HTMLDivElement | null) => onDiarizationHost?.(meetingId,node), [meetingId,onDiarizationHost]);
  const [query,setQuery]=useState("");
  const [splitting,setSplitting]=useState<Utterance|null>(null);
  const [pendingSplit,setPendingSplit]=useState(false);
  const [pendingAdd,setPendingAdd]=useState(false);
  const [splitSeconds,setSplitSeconds]=useState("");
  const [splitOffset,setSplitOffset]=useState(1);
  const [firstSpeaker,setFirstSpeaker]=useState("keep");
  const [secondSpeaker,setSecondSpeaker]=useState("keep");
  const splitDialog=useRef<HTMLDialogElement>(null);
  const audio = useTranscriptAudio(meetingId, locked, { inline:!!splitting });
  const [textSessions, setTextSessions] = useState<Record<string, TextEditSession>>({});
  const textBuffers = useRef(textSessions);
  const [textPending, setTextPending] = useState<Record<string, "commit" | "cancel">>({});
  const textOperations = useRef(new Map<string, { kind: "commit" | "cancel" }>());
  const textInputs = useRef(new Map<string, HTMLTextAreaElement>());
  const transcriptRoot = useRef<HTMLElement>(null);
  const followPort=useRef<HTMLDivElement>(null);
  const [focusedRow, setFocusedRow] = useState<string | null>(null);
  const [nameSessions,setNameSessions] = useState<Record<string,SpeakerNameSession>>({});
  const nameBuffers = useRef<Record<string,SpeakerNameSession>>({});
  const nameInputs = useRef(new Map<string,HTMLInputElement>());
  const focusNameAfterRemoval = useRef<{ id: string; input: HTMLInputElement } | null>(null);
  const nameOperations = useRef(new Map<string,{ kind: "commit" | "cancel" }>());
  const [namePending,setNamePending] = useState<Record<string,"commit" | "cancel">>({});
  const focusAfterRemoval = useRef<{ id: string; row: HTMLElement | null } | null>(null);
  const undoPending = useRef(false);
  const removingBuffers = useRef(new Map<string,{ session: TextEditSession; kind: "commit" | "cancel" | "barrier" }>());
  const removingNames = useRef(new Map<string,{ session: SpeakerNameSession; kind: "commit" | "cancel" | "barrier" }>());
  const durableBuffers = () => Object.values(textBuffers.current).filter(session => removingBuffers.current.get(session.utteranceId)?.session !== session);
  const durableNames = () => Object.values(nameBuffers.current).filter(session => removingNames.current.get(session.speakerId)?.session !== session);
  const syncRecoveryBuffers = () => recovery.journal.update(durableBuffers(),durableNames());
  const updateNames = (update: (current: Record<string,SpeakerNameSession>) => Record<string,SpeakerNameSession>,sync = true) => {
    nameBuffers.current = update(nameBuffers.current); setNameSessions(nameBuffers.current);
    for(const [id,pending] of removingNames.current) if(nameBuffers.current[id] !== pending.session) removingNames.current.delete(id);
    if(sync) syncRecoveryBuffers();
  };
  const updateTextSessions = (update: (current: Record<string, TextEditSession>) => Record<string, TextEditSession>) => {
    textBuffers.current = update(textBuffers.current); setTextSessions(textBuffers.current);
    for(const [id,pending] of removingBuffers.current) if(textBuffers.current[id] !== pending.session) removingBuffers.current.delete(id);
    syncRecoveryBuffers();
  };
  const settleRemovedBuffers = () => {
    if(!removingBuffers.current.size && !removingNames.current.size) return;
    const journal = recovery.snapshot();
    if(journal.saving || journal.pendingRequest || journal.error) return;
    const next = { ...textBuffers.current }, nextNames = { ...nameBuffers.current };
    let textChanged = false, namesChanged = false;
    for(const [id,pending] of removingBuffers.current) if(pending.kind !== "barrier") {
      // A read-back can prove this removal while newer input elsewhere still
      // awaits explicit saving. Do not require that unrelated input to be clean.
      if(journal.saved?.payload.buffers.some(session => session.utteranceId === id)) continue;
      if(next[id] === pending.session) { delete next[id]; textChanged = true; } removingBuffers.current.delete(id);
    }
    for(const [id,pending] of removingNames.current) if(pending.kind !== "barrier") {
      if(journal.saved?.payload.nameBuffers?.some(session => session.speakerId === id)) continue;
      if(nextNames[id] === pending.session) { delete nextNames[id]; namesChanged = true; } removingNames.current.delete(id);
    }
    // Keep barrier-only maps identical: a navigation flush uses that identity
    // to distinguish new typing from already acknowledged editing fields.
    if(namesChanged) updateNames(() => nextNames,false);
    if(textChanged) updateTextSessions(() => next);
    else if(namesChanged) syncRecoveryBuffers();
  };
  const stageTextBuffers = () => {
    saveQueue.edit(current => durableBuffers().reduce(stageTextEdit, current));
  };
  const flushTextBuffers = async () => {
    if(conflictRef.current || conflictBusy.current) throw Error("保存競合の比較を閉じるか、選択した編集の再開を完了してから移動してください");
    if(discardPendingRef.current) throw Error("回復入力の破棄結果を確認してから移動してください");
    if(!recoveryReadyRef.current) { if(saveQueue.snapshot().draft) throw Error("未確定入力の回復記録を先に読み込んでください"); return; }
    if(!saveQueue.snapshot().draft) return;
    if(recovery.snapshot().review && !saveQueue.snapshot().dirty && !Object.keys(textBuffers.current).length && !Object.keys(nameBuffers.current).length) return;
    for (;;) {
      if(removingBuffers.current.size || removingNames.current.size) { await recovery.flushRecovery(); settleRemovedBuffers(); }
      const expected = textBuffers.current;
      const expectedNames = nameBuffers.current;
      saveQueue.edit(current => durableNames().reduce(stageName,current));
      stageTextBuffers(); await saveQueue.flush();
      // A barrier can save while this editor remains mounted (e.g. a candidate
      // review). Keep its field/baseline so Esc can still record a restoration.
      for(const session of Object.values(expected)) if(textBuffers.current[session.utteranceId] === session) removingBuffers.current.set(session.utteranceId,{ session,kind: "barrier" });
      for(const session of Object.values(expectedNames)) if(nameBuffers.current[session.speakerId] === session) removingNames.current.set(session.speakerId,{ session,kind: "barrier" });
      syncRecoveryBuffers(); await recovery.flushRecovery(); settleRemovedBuffers();
      if(!durableBuffers().length && !durableNames().length && !saveQueue.snapshot().dirty && expectedNames === nameBuffers.current) return;
    }
  };
  const recoveryReadyRef = useRef(recoveryReady); recoveryReadyRef.current = recoveryReady;
  const retryTextBuffers = async () => {
    if(conflictRef.current || conflictBusy.current) throw Error("保存競合の比較中は古い保存要求を再送しません。比較を閉じるか編集再開を完了してください");
    // Resolve only already-confirmed payload. New names and text typed after
    // a failure must not be published just because another operation is retried.
    await saveQueue.retry(); await recovery.flushRecovery(); settleRemovedBuffers(); setBodyManual(false);
  };

  useEffect(() => {
    const unsubscribe = saveQueue.subscribe(() => setSaveSnapshot(saveQueue.snapshot()));
    const unsubscribeRecovery = recovery.subscribe(() => setRecoverySnapshot(recovery.snapshot()));
    const unregister = registerPendingEdits(flushTextBuffers);
    return () => { mounted.current = false; unsubscribe(); unsubscribeRecovery(); unregister(); recovery.dispose(); };
  }, [saveQueue,recovery]);

  useEffect(() => {
    // Removing a focused textarea does not reliably emit blur. Clear retained
    // focus on the next real focus move, including moves outside this section.
    const focus = (event: FocusEvent) => {
      const target = event.target instanceof Element ? event.target : null;
      const row = target?.closest(".utterance-row");
      setFocusedRow(row && transcriptRoot.current?.contains(row) ? row.getAttribute("data-utterance-id") : null);
    };
    document.addEventListener("focusin", focus);
    return () => document.removeEventListener("focusin", focus);
  }, []);
  useLayoutEffect(() => {
    const pending = focusAfterRemoval.current;
    if(!pending || textSessions[pending.id]) return;
    focusAfterRemoval.current = null;
    // Restore in the DOM commit that removes the input, before the next paint.
    // A later RAF would leave a filtered retained row briefly unfocused.
    if(document.activeElement !== document.body && !pending.row?.contains(document.activeElement)) return;
    if(pending.row?.isConnected) pending.row.focus();
    else transcriptRoot.current?.querySelector<HTMLInputElement>('input[type="search"]')?.focus();
  },[textSessions]);
  useLayoutEffect(() => {
    const pending = focusNameAfterRemoval.current;
    if(!pending || nameSessions[pending.id]) return;
    focusNameAfterRemoval.current = null;
    if(pending.input.isConnected && (document.activeElement === document.body || pending.input.closest(".speaker-name-editor")?.contains(document.activeElement))) pending.input.focus();
  },[nameSessions]);

  const reload = async (propagateFailure = false) => {
    audio.clear();
    setLoading(true);
    setError("");
    try {
      const [current, working] = await Promise.all([
        invoke<Analysis | null>("get_current_analysis", { meetingId }),
        invoke<Draft | null>("get_transcript_draft", { meetingId }),
      ]);
      const record = working ? await invoke<TranscriptRecovery<DraftPayload> | null>("get_transcript_edit_recovery",{ meetingId,draftId: working.id,baseAnalysisId: working.baseAnalysisId }) : null;
      recovery.load(working,record);
      setRecoveryManual(false);
      setBodyManual(false);
      setAnalysis(current);
      saveQueue.load(working);
      textBuffers.current = {}; setTextSessions({}); removingBuffers.current.clear(); setFocusedRow(null);
      removingNames.current.clear(); updateNames(() => ({}),false);
      recoveryReadyRef.current = true; setRecoveryReady(true);
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
    if (conflictPreview || activeConfirmation || locked || loading || busy || discardPending || bodyManual || !recoveryReady || recoverySnapshot.review || recoverySnapshot.bodyBusy || recoverySnapshot.bodyPending || !draft || !payload || !dirty || saving || saveError || draft.baseAnalysisId !== analysis?.id) return;
    const timer = window.setTimeout(() => {
      if(conflictRef.current || conflictBusy.current) return;
      void saveQueue.flush().catch(() => { /* The queue retains the error and local edit. */ });
    }, 700);
    return () => window.clearTimeout(timer);
  }, [analysis?.id, draft, payload, dirty, saving, saveError, saveQueue, activeConfirmation,locked,loading,busy,discardPending,bodyManual,recoveryReady,recoverySnapshot.review,recoverySnapshot.bodyBusy,recoverySnapshot.bodyPending,conflictPreview]);

  useEffect(() => {
    if(recoverySnapshot.error) setRecoveryManual(true);
  },[recoverySnapshot.error]);
  useEffect(() => { if(saveError) setBodyManual(true); },[saveError]);
  useEffect(() => {
    if(conflictPreview || !recoveryReady || recoveryManual || !draft || locked || loading || busy || discardPending || activeConfirmation || saving || recoverySnapshot.bodyBusy || recoverySnapshot.bodyPending || recoverySnapshot.review || recoverySnapshot.saving || recoverySnapshot.error || !recoverySnapshot.dirty) return;
    const timer = window.setTimeout(() => {
      if(conflictRef.current || conflictBusy.current || discardPendingRef.current || saveQueue.snapshot().saving || confirmations.getSnapshot() || recovery.snapshot().bodyBusy || recovery.snapshot().bodyPending) return;
      void recovery.flushRecovery().catch(() => { /* Keep the exact request and all local input. */ });
    },300);
    return () => window.clearTimeout(timer);
  },[recovery,recoveryReady,recoveryManual,draft,locked,loading,busy,discardPending,activeConfirmation,saving,recoverySnapshot,conflictPreview]);

  const begin = async () => {
    if (!analysis) return;
    setBusy(true);
    audio.clear();
    setError("");
    try {
      const created = await invoke<Draft>("begin_transcript_draft", { meetingId, analysisId: analysis.id });
      const record = await invoke<TranscriptRecovery<DraftPayload> | null>("get_transcript_edit_recovery",{ meetingId,draftId: created.id,baseAnalysisId: created.baseAnalysisId });
      recovery.load(created,record); recoveryReadyRef.current = true; setRecoveryReady(true);
      setRecoveryManual(false);
      setBodyManual(false);
      saveQueue.load(created);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const undo = async (flushCurrent = false) => {
    if (!draft || !matchingBase || (!flushCurrent && (draft.lockVersion === 0 || dirty || saving || textDirty || nameDirty)) || busy || locked || confirm.pending || undoPending.current) return;
    undoPending.current = true;
    setBusy(true);
    audio.clear();
    setError("");
    try {
      if (flushCurrent) await flushTextBuffers();
      await recovery.flushRecovery();
      const current = saveQueue.snapshot().draft;
      if (!current || current.id !== draft.id || current.baseAnalysisId !== analysis?.id || current.lockVersion === 0) return;
      const previous = await invoke<Draft>("undo_transcript_edit", { draftId: current.id, expectedLockVersion: current.lockVersion });
      recovery.advance(previous);
      saveQueue.load(previous);
      removingNames.current.clear(); updateNames(() => ({}),false);
      updateTextSessions(() => ({}));
      setNotice("直前の編集を取り消しました。"); setNoticeVersion(null); setMergeSource(""); setMergeTarget("");
    } catch (cause) {
      setError(String(cause));
    } finally {
      undoPending.current = false;
      setBusy(false);
    }
  };

  // Structural operations get their own durable undo entry, after preceding
  // text/name edits have been acknowledged. Failure retains the queued change.
  const structuralEdit = async (edit:(current:DraftPayload)=>DraftPayload,message:string,propagateFailure=false) => {
    if (!analysis || locked || busy) return false;
    setBusy(true); setError(""); setNotice(""); setNoticeVersion(null);
    audio.clear();
    try { await flushTextBuffers(); saveQueue.edit(edit); await saveQueue.flush(); setNotice(message); setNoticeVersion(saveQueue.snapshot().draft?.lockVersion ?? null); return true; }
    catch(cause) { setError(String(cause)); if (propagateFailure) throw cause; return false; }
    finally { setBusy(false); }
  };

  const changeName = (speaker: typeof speakers[number], value: string) => {
    const current = saveQueue.snapshot().payload;
    if(!editable || !current) return;
    updateNames(sessions => ({ ...sessions,[speaker.id]: { ...(sessions[speaker.id] ?? nameBaseline(current,speaker)),value } }));
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
  const tracks = useTranscriptTrackKinds(meetingId,analysis?.id??null);
  const speakerCounts = useMemo(() => {
    const counts = new Map<string,number>();
    for (const utterance of view?.utterances ?? []) for (const id of new Set(utterance.speakerIds)) counts.set(id,(counts.get(id) ?? 0) + 1);
    return counts;
  }, [view]);
  const filtered=useMemo(()=>view?filterUtterances(view,speakerFilter,query):[],[view,speakerFilter,query]);
  const retained = retainedTextRows(view?.utterances ?? [], filtered, page, PAGE_SIZE, [...Object.keys(textSessions), ...(focusedRow ? [focusedRow] : [])]);
  const shown = retained.rows;
  const textDirty = textBuffersDirty(textSessions, view?.utterances ?? []);
  const nameDirty = namesDirty(nameSessions,view?.speakers ?? []);
  const review = useSpeakerReviews(analysis,[...new Set(shown.map(u=>u.sourceUtteranceId??u.id))]);
  const speakerLabels = new Map(view?.speakers.map(s=>[s.id,(s.displayName||s.label)+(payload?.speaker_merges?.some(m=>m.source_id===s.id)?"（統合前の推定ラベル）":"")])??[]);
  const unconfirmedSpeakerIds = new Set(view?.speakers.filter(s=>!s.displayName).map(s=>s.id)??[]);
  const editable = Boolean(matchingBase && payload && recoveryReady && !recoverySnapshot.review && !discardPending && !locked && !loading && !busy && !activeConfirmation && !conflictPreview);
  const following=usePlaybackFollowing({meetingId,analysisId:analysis?.id??null,rows:filtered,page,pageSize:PAGE_SIZE,setPage,
    editing:!!Object.keys(textSessions).length||!!Object.keys(nameSessions).length||saving||dirty||!!conflictPreview||!!activeConfirmation||busy||loading,
    active:visible&&!locked,port:followPort,trackKinds:tracks.kinds,highlightRows:shown});
  const finishName = async (id: string,kind: "commit" | "cancel") => {
    const session = nameBuffers.current[id];
    if(!editable || !session || nameOperations.current.has(id)) return;
    const startedFocused = !!nameInputs.current.get(id)?.closest(".speaker-name-editor")?.contains(document.activeElement);
    const operation = { kind }; nameOperations.current.set(id,operation);
    setNamePending(current => ({ ...current,[id]: kind })); setError("");
    try {
      const failed = saveQueue.snapshot();
      if(failed.error && kind === "cancel") {
        const saved = recovery.snapshot().bodyPending ? (await recovery.recheckBody(false)).draft : await invoke<Draft | null>("get_transcript_draft",{ meetingId });
        if(!saved || !failed.draft || saved.id !== failed.draft.id || saved.baseAnalysisId !== failed.draft.baseAnalysisId
          || saved.lockVersion !== failed.draft.lockVersion || saved.contentHash !== failed.draft.contentHash
          || JSON.stringify(canonicalPayload(saved.payload)) !== JSON.stringify(canonicalPayload(failed.draft.payload))) throw Error("先行する保存結果を確認できません。話者名の入力は保持しています。");
        saveQueue.load(saved); if(failed.payload) saveQueue.edit(() => failed.payload!);
      }
      if(nameBuffers.current[id] !== session) return;
      const removal = removingNames.current.get(id);
      if(removal?.session !== session || removal.kind !== kind) {
        removingNames.current.delete(id); syncRecoveryBuffers();
        saveQueue.edit(current => kind === "commit" ? stageName(current,session) : cancelName(current,session));
      }
      await saveQueue.flush();
      setBodyManual(false);
      if(nameBuffers.current[id] !== session) { await recovery.flushRecovery(); return; }
      removingNames.current.set(id,{ session,kind }); syncRecoveryBuffers();
      await recovery.flushRecovery();
      if(nameBuffers.current[id] === session) {
        const input = nameInputs.current.get(id);
        if(input && (input.closest(".speaker-name-editor")?.contains(document.activeElement) || startedFocused && document.activeElement === document.body)) focusNameAfterRemoval.current = { id,input };
      }
      settleRemovedBuffers(); setRecoveryManual(false);
    } catch(cause) { setError(String(cause)); }
    finally {
      if(nameOperations.current.get(id) === operation) { nameOperations.current.delete(id); setNamePending(current => { const next = { ...current }; delete next[id]; return next; }); }
    }
  };
  const beginTextEdit = (utterance: Utterance) => {
    const current = saveQueue.snapshot().payload;
    if (!editable || !current || textOperations.current.has(utterance.id)) return;
    if(!textBuffers.current[utterance.id] && Object.keys(textBuffers.current).length >= 128) { setError("同時に編集できる発言は128件までです。開いている入力を確定またはキャンセルしてください。"); return; }
    const opener = document.activeElement;
    if (!textBuffers.current[utterance.id]) updateTextSessions(sessions => ({ ...sessions, [utterance.id]: { ...textEditBaseline(current, utterance), value: utterance.effectiveText } }));
    requestAnimationFrame(() => {
      const target = textInputs.current.get(utterance.id);
      // Rendering can outlive a user's next focus move. Opening one editor
      // must not steal focus back from search or a different input field.
      if(document.activeElement === document.body || document.activeElement === opener || document.activeElement === target) target?.focus();
    });
  };
  const finishTextEdit = async (id: string, kind: "commit" | "cancel", source: HTMLElement) => {
    let session = textBuffers.current[id];
    const pending = textOperations.current.get(id);
    if (!editable || !session || pending?.kind === "cancel" || pending && kind === "commit") return;
    const operation = { kind }; textOperations.current.set(id, operation);
    setTextPending(current => ({ ...current, [id]: kind })); setError("");
    try {
      // A failed response is not proof that nothing was written. Cancel may
      // clear a queued error only after reading the same unchanged native draft.
      const failed = saveQueue.snapshot();
      if (failed.error && kind === "cancel") {
        const saved = recovery.snapshot().bodyPending ? (await recovery.recheckBody(false)).draft : await invoke<Draft | null>("get_transcript_draft", { meetingId });
        if (!saved || !failed.draft || saved.id !== failed.draft.id || saved.baseAnalysisId !== failed.draft.baseAnalysisId || saved.lockVersion !== failed.draft.lockVersion || saved.contentHash !== failed.draft.contentHash || JSON.stringify(canonicalPayload(saved.payload)) !== JSON.stringify(canonicalPayload(failed.draft.payload))) throw Error("先行する保存結果を確認できません。本文と編集欄は保持しています。保存結果を再確認してください。");
        saveQueue.load(saved); if (failed.payload) saveQueue.edit(() => failed.payload!);
      }
      if (textOperations.current.get(id) !== operation || textBuffers.current[id] !== session) return;
      const removal = removingBuffers.current.get(id);
      if(removal?.session !== session || removal.kind !== kind) {
        removingBuffers.current.delete(id); syncRecoveryBuffers();
        if (kind === "cancel") saveQueue.edit(current => cancelTextEdit(current,session));
        else saveQueue.edit(current => stageTextEdit(current,session));
      }
      await saveQueue.flush();
      setBodyManual(false);
      if (textOperations.current.get(id) !== operation || textBuffers.current[id] !== session) { await recovery.flushRecovery(); return; }
      const row = source.closest<HTMLElement>(".utterance-row"), restoreFocus = !!row?.contains(document.activeElement);
      removingBuffers.current.set(id,{ session,kind }); syncRecoveryBuffers();
      await recovery.flushRecovery();
      if(restoreFocus && (document.activeElement === document.body || row?.contains(document.activeElement))) {
        setFocusedRow(id); focusAfterRemoval.current = { id,row };
      }
      settleRemovedBuffers();
      setRecoveryManual(false);
    } catch (cause) { setError(String(cause)); }
    finally {
      if (textOperations.current.get(id) === operation) {
        textOperations.current.delete(id); setTextPending(current => { const next = { ...current }; delete next[id]; return next; });
      }
    }
  };
  const confirmReload = () => {
    if(discardPendingRef.current || recovery.snapshot().pendingRequest || recovery.snapshot().bodyPending || recovery.snapshot().saving) { setError("未確認の保存結果を確認してから再読込してください。入力を保持しています。"); return; }
    if (!saveQueue.snapshot().dirty && !Object.keys(textBuffers.current).length && !Object.keys(nameBuffers.current).length) { void reload(); return; }
    const stamp = () => JSON.stringify([saveQueue.snapshot().payload, textBuffers.current,nameBuffers.current]);
    const expected = stamp();
    void confirm({ title: "文字起こし編集の破棄を確認", target: `会議「${meetingTitle ?? meetingId}」の文字起こし編集`, confirmLabel: "入力を破棄して再読込する", message: "未保存の文字起こし編集とその回復入力を破棄して再読込します。この入力は復元できません。保存済みの編集と認識原文は保持します。", validate: () => stamp() === expected, action: async () => {
      // Complete fallible reads before consenting input is cleared. The empty
      // journal CAS then proves no older save can resurrect the discarded text.
      const [current,working] = await Promise.all([invoke<Analysis | null>("get_current_analysis",{ meetingId }),invoke<Draft | null>("get_transcript_draft",{ meetingId })]);
      const acknowledged = saveQueue.snapshot().draft;
      if(stamp() !== expected || !working || !acknowledged || working.id !== acknowledged.id
        || working.lockVersion !== acknowledged.lockVersion || working.contentHash !== acknowledged.contentHash
        || working.baseAnalysisId !== current?.id) throw Error("再読込前に本文が変わりました。入力を破棄せず保持しています。");
      for(const session of Object.values(textBuffers.current)) removingBuffers.current.set(session.utteranceId,{ session,kind: "cancel" });
      for(const session of Object.values(nameBuffers.current)) removingNames.current.set(session.speakerId,{ session,kind: "cancel" });
      syncRecoveryBuffers(); await recovery.flushRecovery();
      recovery.load(working,recovery.snapshot().saved); saveQueue.load(working); setAnalysis(current);
      setRecoveryManual(false); setBodyManual(false);
      textBuffers.current = {}; setTextSessions({}); removingBuffers.current.clear(); setFocusedRow(null); setError("");
      removingNames.current.clear(); updateNames(() => ({}),false);
      setNotice("未保存の編集と回復入力を破棄し、保存済み本文を再読込しました。"); setNoticeVersion(null);
    } });
  };
  const runRecoveryAction = async (action: () => Promise<unknown>) => {
    setBusy(true); setError("");
    try { await action(); settleRemovedBuffers(); } catch(cause) { setError(String(cause)); } finally { setBusy(false); }
  };
  const recheckBody = async () => {
    const local = saveQueue.snapshot().payload;
    const result = await recovery.recheckBody();
    setBodyManual(true); setRecoveryManual(true);
    saveQueue.load(result.draft); if(local) saveQueue.edit(() => local);
    setNotice(result.status === "saved" ? "先行する本文保存を確認しました。新しい入力と確定前の本文は保持しています。" : "本文は確認した版のままです。入力を保持しています。"); setNoticeVersion(null);
  };
  const conflictStamp = () => JSON.stringify([saveQueue.snapshot().draft,saveQueue.snapshot().payload,textBuffers.current,nameBuffers.current,[...removingBuffers.current],[...removingNames.current]]);
  const inspectConflict = async () => {
    if(conflictBusy.current || locked || busy || loading || saving || discardPendingRef.current || recovery.snapshot().saving || recovery.snapshot().bodyBusy || textOperations.current.size || nameOperations.current.size || confirmations.getSnapshot()) return;
    conflictBusy.current = true; setBusy(true); setError(""); setBodyManual(true); setRecoveryManual(true);
    audio.clear();
    try {
      const expected = conflictStamp(), local = saveQueue.snapshot(), base = conflictEnvironment.current.analysis;
      if(!base || !local.draft || !local.payload || local.draft.baseAnalysisId !== base.id) throw Error("同じ解析版の下書きを確認してから比較してください");
      const current = await invoke<Analysis | null>("get_current_analysis",{ meetingId });
      if(!current || current.id !== base.id || current.contentHash !== base.contentHash) throw Error("解析版が変わりました。旧解析版を採用せず入力を保持しています");
      const proof = await recovery.inspectAdvancedDraft();
      if(!mounted.current || conflictEnvironment.current.locked || conflictStamp() !== expected) throw Error("比較中に操作条件または入力が変わりました。入力を保持しています");
      if(proof.base.id !== local.draft.id || proof.base.lockVersion !== local.draft.lockVersion || proof.base.contentHash !== local.draft.contentHash) throw Error("本文の比較基底が編集画面と一致しません");
      const plan = planConflictInput(base,proof.base.payload,local.payload,proof.latest.payload,Object.values(textBuffers.current),Object.values(nameBuffers.current),proof.recovery.record);
      const preview = { proof,plan,stamp: expected }; conflictGeneration.current++;
      conflictRef.current = preview; setConflictPreview(preview);
    } catch(cause) { if(mounted.current) setError(String(cause)); throw cause; }
    finally { conflictBusy.current = false; if(mounted.current) setBusy(false); }
  };
  const acceptConflict = async (choices: Record<string,ConflictChoice>,inputs: Record<string,InputChoice>) => {
    const preview = conflictRef.current, base = conflictEnvironment.current.analysis;
    if(!preview || !base || conflictBusy.current) throw Error("比較をやり直してください");
    const unchanged = () => mounted.current && !conflictEnvironment.current.locked && conflictRef.current === preview && conflictStamp() === preview.stamp && conflictEnvironment.current.analysis?.id === base.id && conflictEnvironment.current.analysis.contentHash === base.contentHash;
    if(!unchanged()) throw Error("比較後に入力または操作条件が変わりました。元の入力と要求を保持しています");
    const selected = resolveConflictInput(base,preview.plan,choices,inputs);
    conflictBusy.current = true; setBusy(true);
    try {
      const current = await invoke<Analysis | null>("get_current_analysis",{ meetingId });
      if(!current || current.id !== base.id || current.contentHash !== base.contentHash || !unchanged()) throw Error("解析版または手元の入力が変わりました。比較をやり直してください");
      const latest = await recovery.acceptAdvancedDraftReview(preview.proof,selected.texts,selected.names,unchanged);
      saveQueue.load(latest); saveQueue.edit(() => selected.payload);
      removingBuffers.current.clear(); removingNames.current.clear();
      textBuffers.current = Object.fromEntries(selected.texts.map(session => [session.utteranceId,session])); setTextSessions(textBuffers.current);
      updateNames(() => Object.fromEntries(selected.names.map(session => [session.speakerId,session])),false);
      setBodyManual(true); setRecoveryManual(true); setSelection([]); setFocusedRow(null); setMergeSource(""); setMergeTarget("");
      setSplitting(null); setAdding(null); setPendingSplit(false); setPendingAdd(false);
      conflictRef.current = null; setConflictPreview(null); setError(""); setNoticeVersion(null);
      setNotice("選択した内容で編集を再開しました。本文と回復記録は保存していません。自動保存は停止中です。確定・保存再試行・通常の画面移動で保存します。");
    } finally { conflictBusy.current = false; if(mounted.current) setBusy(false); }
  };
  useEffect(()=>{setPage(0);},[speakerFilter,query]);
  useEffect(()=>{const valid=new Set(view?.utterances.map(u=>u.id));setSelection(ids=>ids.every(id=>valid.has(id))?ids:ids.filter(id=>valid.has(id)));setPage(p=>Math.min(p,Math.max(0,Math.ceil(filtered.length/PAGE_SIZE)-1)));},[view,filtered.length]);
  useEffect(()=>{if(speakerFilter&&speakerFilter!=="unknown"&&!speakers.some(s=>s.id===speakerFilter))setSpeakerFilter("");},[speakerFilter,speakers]);
  useEffect(() => {
    if (!view || loading) return;
    const valid = new Set(view.utterances.map(item => item.id));
    if (Object.keys(textBuffers.current).some(id => !valid.has(id))) updateTextSessions(current => Object.fromEntries(Object.entries(current).filter(([id]) => valid.has(id))));
  }, [view, loading]);
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
    <section ref={transcriptRoot} className="transcript-card" aria-label="文字起こし編集" onKeyDown={event => {
      if (appShortcut(event.nativeEvent) !== "undo" || nativeTextTarget(event.target) || overlayOpen() || !editable || !draft || (draft.lockVersion === 0 && !dirty && !saving && !textDirty && !nameDirty) || undoPending.current) return;
      event.preventDefault(); void undo(true);
    }}>
      {loading && <p>解析結果を読み込んでいます。</p>}
      {!loading && !analysis && !error && <p>確定済みの解析結果はまだありません。</p>}
      {analysis && <div className="transcript-split">
        <WorkspacePanel content={speakerMount} label="話者一覧" active={visible} />
        <div className="transcript-main">
        <div className="transcript-actions">
          <h2>発言一覧</h2>
          {!draft && <button type="button" onClick={() => void begin()} disabled={busy || loading || locked}>編集を開始</button>}
          {draft && <span role="status">{saveError ? "保存要確認" : saving ? "保存中" : dirty || textDirty || nameDirty ? "未保存" : "保存済み"}</span>}
          {(saveError || bodyManual && dirty) && <button type="button" className="small-secondary" onClick={() => void runRecoveryAction(retryTextBuffers)} disabled={saving || busy || locked || discardPending || !!activeConfirmation}>保存を再試行</button>}
          {editable && <span className="hint">本文をクリックして編集</span>}
        </div>
        <details className="workspace-tools transcript-workspace-tools"><summary>回復入力・話者・一括編集操作</summary><div className="workspace-tools-body">
          <button type="button" className="small-secondary" onClick={confirmReload} disabled={loading || saving || busy || locked || confirm.pending}>再読込</button>
          <label>話者で絞り込み <select value={speakerFilter} disabled={loading} onChange={e=>setSpeakerFilter(e.target.value)}><option value="">すべて</option><option value="unknown">話者不明</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label>
          {matchingBase && <button type="button" className="small-secondary" disabled={saving || busy || loading || locked || discardPending || !!activeConfirmation || recoverySnapshot.saving || recoverySnapshot.bodyBusy} onClick={() => void inspectConflict().catch(() => {})}>最新保存版と入力を比較</button>}
          {matchingBase && <button type="button" className="small-secondary" onClick={() => void undo()} disabled={!editable || saving || dirty || textDirty || nameDirty || recoverySnapshot.pendingRequest !== null || recoverySnapshot.bodyPending || draft!.lockVersion === 0}>直前の編集を取り消す</button>}
        <TranscriptRecoveryPanel meetingId={meetingId} review={recoverySnapshot.review} currentDraftId={draft?.id}
          blocked={locked || loading || busy || saving || !!conflictPreview}
          canChangeCurrent={!dirty && !saving && !Object.keys(textSessions).length && !Object.keys(nameSessions).length && !recoverySnapshot.pendingRequest && !recoverySnapshot.bodyPending && !recoverySnapshot.saving}
          onPending={pending => { discardPendingRef.current = pending; setDiscardPending(pending); }}
          onResume={async () => {
            const pending = recovery.snapshot().review;
            const active = new Set(speakers.map(speaker => speaker.id));
            const rows = new Set(view?.utterances.map(utterance => utterance.id));
            if(pending?.payload.nameBuffers?.some(session => !active.has(session.speakerId)) || pending?.payload.buffers.some(session => !rows.has(session.utteranceId))) throw Error("現在の編集画面にない発言・話者の入力を含みます。部分的に再開せず、回復記録を保持しています。プレビューから必要な入力をコピーしてください。");
            await recovery.resume();
            removingBuffers.current.clear(); removingNames.current.clear();
            textBuffers.current = Object.fromEntries(recovery.snapshot().buffers.map(session => [session.utteranceId,session]));
            updateNames(() => Object.fromEntries(recovery.snapshot().nameBuffers.map(session => [session.speakerId,session])),false);
            setTextSessions(textBuffers.current); setError(""); setRecoveryManual(false);
          }}
          onDiscarded={record => {
            const current = saveQueue.snapshot().draft;
            if(current?.id === record.draftId && record.currentContext && record.matchesDraft && record.currentDraftLockVersion === current.lockVersion && record.currentDraftContentHash === current.contentHash
              && !saveQueue.snapshot().dirty && !Object.keys(textBuffers.current).length && !Object.keys(nameBuffers.current).length && !recovery.snapshot().pendingRequest && !recovery.snapshot().bodyPending) {
              recovery.load(current,record); setError("");
            }
          }} />
        {draft && <p className="hint" role="status">未確定入力の回復保存：{recoverySnapshot.error || recoverySnapshot.pendingRequest ? "結果要確認" : recoverySnapshot.saving ? "保存中" : recoverySnapshot.review ? "確認待ち" : recoverySnapshot.dirty ? "未保存" : "保存済み"}（本文の確定とは別です）</p>}
        {recoverySnapshot.bodyPending && <button type="button" disabled={busy || saving || locked || !!activeConfirmation} onClick={() => void runRecoveryAction(recheckBody)}>本文の保存結果を確認</button>}
        {recoverySnapshot.pendingRequest && <button type="button" disabled={busy || recoverySnapshot.saving || recoverySnapshot.bodyPending || locked || !!activeConfirmation} onClick={() => void runRecoveryAction(async () => {
          const result = await recovery.recheckRecovery();
          setNotice(result === "saved" ? "未確定入力の保存結果を確認しました。本文は確定していません。" : "回復記録は未更新です。入力と同じ保存要求を保持しています。再試行は別操作です。"); setNoticeVersion(null);
        })}>回復保存の結果を確認</button>}
        {(recoverySnapshot.error || recoveryManual && recoverySnapshot.dirty) && <div className="action-error" role="alert">未確定入力を保持しています：{recoverySnapshot.error || "保存結果の確認後も、新しい入力の自動再送は停止しています。"}
          <button type="button" disabled={busy || recoverySnapshot.saving || recoverySnapshot.bodyPending || locked || !!activeConfirmation} onClick={() => void runRecoveryAction(async () => { await recovery.retryRecovery(); setRecoveryManual(false); })}>未確定入力の保存を再試行</button>
        </div>}
        <ToastNotice scope={meetingId} type="transcript-edit" eventId={noticeVersion !== null ? `${draft?.id}:${noticeVersion}` : undefined} message={notice} onClose={() => setNotice("")} action={noticeVersion !== null ? { kind: "undo", label: "元に戻す", disabled: busy || saving || dirty || textDirty || nameDirty || locked || confirm.pending || !matchingBase || draft?.lockVersion !== noticeVersion, run: async () => { if (saveQueue.snapshot().draft?.lockVersion === noticeVersion && !saveQueue.snapshot().dirty && !textBuffersDirty(textBuffers.current, view?.utterances ?? []) && !namesDirty(nameBuffers.current,view?.speakers ?? [])) await undo(); } } : undefined} />
        {draft && !matchingBase && <p className="action-error">下書きは別の解析版に基づいています。元版の編集を保持したまま、再基準化する機能は未接続です。</p>}
        {!splitting && !playbackHost && !audio.sharedSurface && audio.controls}
        {createPortal(<>
        <div className="speaker-heading"><h3>話者</h3><button type="button" className="small-secondary" aria-label="話者を追加" disabled={!editable||!view||view.speakers.length>=100} onClick={()=>{setPendingAdd(false);setError("");setNewName("");setAdding({});}}>＋</button></div>
        {draft && (saveError || saving || nameDirty) && <p role="status" className="hint">{saveError ? "話者名の保存結果を確認してください。入力は保持しています。" : saving ? "保存を確認中" : "名前は未確定"}</p>}
        {saveError && <button type="button" className="small-secondary" disabled={saving || busy || locked || discardPending || !!activeConfirmation} onClick={() => void runRecoveryAction(retryTextBuffers)}>話者名の保存を再試行</button>}
        <div className="speaker-filters" aria-label="話者一覧から絞り込み">
          <button type="button" className="speaker-filter" aria-pressed={speakerFilter === ""} disabled={loading} onClick={() => setSpeakerFilter("")}>すべての発言 <span>{view?.utterances.length ?? 0}</span></button>
          {(speakerFilter === "unknown" || view?.utterances.some(u => u.speakerIds.length === 0)) && <button type="button" className="speaker-filter" aria-pressed={speakerFilter === "unknown"} disabled={loading} onClick={() => setSpeakerFilter("unknown")}>話者不明の発言</button>}
        </div>
        <div className="speaker-grid">
          {speakers.map((speaker) => {
            const session = nameSessions[speaker.id], pending = namePending[speaker.id];
            return <div key={speaker.id} className="speaker-name-editor" role="group" aria-label={`${speaker.label}の名前編集`}>
              <button type="button" className="speaker-filter" aria-label={`${speaker.displayName || speaker.label}の発言で絞り込み`} aria-pressed={speakerFilter === speaker.id} disabled={loading} onClick={() => setSpeakerFilter(speaker.id)}>
                <strong><SpeakerLabel meetingId={meetingId} speakerId={speaker.id} name={speaker.displayName || speaker.label}/></strong><span>{speakerCounts.get(speaker.id) ?? 0}発言</span>
              </button>
              <details className="speaker-name-tools"><summary>名前を変更</summary>
              <label>{speaker.label}<input ref={node => { if(node) nameInputs.current.set(speaker.id,node); else nameInputs.current.delete(speaker.id); }} value={session?.value ?? speaker.displayName ?? ""} onChange={event => changeName(speaker,event.target.value)} disabled={!editable} maxLength={200} placeholder="表示名"
                onKeyDown={event => {
                  if(!session || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229 || event.repeat || event.altKey || event.shiftKey || overlayOpen(event.currentTarget)) return;
                  if(event.key === "Enter" || event.key === "Escape") { event.preventDefault(); event.stopPropagation(); void finishName(speaker.id,event.key === "Enter" ? "commit" : "cancel"); }
                }} /></label>
              {session && <div className="transcript-actions"><button type="button" disabled={!editable || !!pending} onClick={() => void finishName(speaker.id,"commit")}>確定</button><button type="button" className="small-secondary" disabled={!editable || !!pending} onClick={() => void finishName(speaker.id,"cancel")}>キャンセル</button><span>{pending ? "保存を確認中" : "Enterで確定 · Escでキャンセル"}</span></div>}
              </details>
            </div>;
          })}
        </div>
        {!!Object.keys(nameSessions).length && <p className="hint">未確定の話者名は本文と一緒に回復記録へ保存します。編集再開だけでは確定せず、確定・通常の画面移動で反映します。最後の入力は回復保存の確認前に終了すると失われる可能性があります。</p>}
        <details className="speaker-merge-tools"><summary>話者を統合</summary><div className="transcript-actions" aria-label="話者統合">
          <label>統合元 <select value={mergeSource} disabled={!editable} onChange={e=>setMergeSource(e.target.value)}><option value="">選択</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label>
          <label>統合先 <select value={mergeTarget} disabled={!editable} onChange={e=>setMergeTarget(e.target.value)}><option value="">選択</option>{speakers.filter(s=>s.id!==mergeSource).map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label>
          <button type="button" disabled={!editable||!mergeSource||!mergeTarget||mergeSource===mergeTarget} onClick={()=>{
            if(!view||!payload)return;
            const count=view.utterances.filter(u=>u.speakerIds.includes(mergeSource)).length;
            const message=`${speakerLabels.get(mergeSource)}の${count}発言を${speakerLabels.get(mergeTarget)}にまとめます。本文・時刻・推定元の区間は変更しません。続けますか？`;
            const expected = JSON.stringify(saveQueue.snapshot().payload), source = mergeSource, target = mergeTarget;
            void confirm({ title: "話者の統合を確認", target: `${speakerLabels.get(source)} → ${speakerLabels.get(target)} · ${count}発言`, confirmLabel: "話者を統合する", message: `${message} 直前の編集を取り消すことで戻せます。`, validate: () => JSON.stringify(saveQueue.snapshot().payload) === expected, action: async () => { if (await structuralEdit(p=>mergeSpeaker(analysis,p,source,target),"話者を統合しました。直前の編集を取り消すことができます。",true)) { setMergeSource("");setMergeTarget("");setBulkSpeaker(""); } } });
          }}>話者を統合</button>
        </div></details>
        {!!payload?.speaker_merges?.length&&<p className="hint">統合済みラベルは割り当て候補から除外しています。推定区間には統合前のラベルを残します。</p>}
        <details className="speaker-diarization-tools"><summary>人数指定・話者分離</summary><div ref={bindDiarizationHost} /></details>
        </>,speakerMount)}
        <div className="transcript-actions" aria-label="複数発言の話者変更"><span>選択 {selection.length} 発言（全ページ）</span><label>割り当て先 <select value={bulkSpeaker} disabled={!editable} onChange={e=>setBulkSpeaker(e.target.value)}><option value="">話者不明</option>{speakers.map(s=><option key={s.id} value={s.id}>{s.displayName||s.label}</option>)}</select></label><button type="button" disabled={!editable||!selection.length} onClick={()=>{
          if(!payload)return;
          const ids = [...selection], speaker = bulkSpeaker, expected = JSON.stringify(saveQueue.snapshot().payload);
          void confirm({ title: "複数発言の話者変更を確認", target: `選択した${ids.length}発言 → ${speakerLabels.get(speaker)||"話者不明"}`, confirmLabel: "選択した発言に適用する", message: `選択した${ids.length}発言の割り当てを${speakerLabels.get(speaker)||"話者不明"}に置き換えます。複数話者の割り当ても置き換えます。本文は変更しません。直前の編集を取り消すことで戻せます。`, validate: () => JSON.stringify(saveQueue.snapshot().payload) === expected, action: async () => { await structuralEdit(p=>assignSpeakers(analysis,p,ids,speaker?[speaker]:[]),"選択した発言の話者を変更しました。取り消しが可能です。",true); } });
        }}>選択した発言に適用</button><button type="button" disabled={!selection.length||busy} onClick={()=>setSelection([])}>選択解除</button></div>
        </div></details>
        <div className="transcript-actions transcript-search" aria-label="発言の絞り込み"><label><span className="sr-only">発言を検索</span><input type="search" placeholder="発言を検索" value={query} disabled={loading} onChange={e=>setQuery(e.target.value)} maxLength={1000}/></label><span>{filtered.length} / {view?.utterances.length??0} 発言</span></div>
        {following.enabled&&<div className="playback-follow-status" role="status">{following.suspended?<>手動操作のため再生追従を一時停止しています。<button type="button" className="small-secondary" onClick={following.resume}>再生位置への追従を再開</button></>:"再生位置に追従します。編集中・確認画面・文字選択中はスクロールしません。"}</div>}
        <div ref={followPort} {...following.handlers} className="utterance-list" role="region" aria-label="発言本文" tabIndex={0}>
          {following.meetingPlaying&&!tracks.kinds&&<p className="hint">{tracks.error?<>再生トラックと発言の対応を確認できないため、強調・追従を行いません。<button type="button" className="small-secondary" disabled={loading||locked} onClick={tracks.retry}>発言のトラック対応を再読込</button></>:"再生トラックと発言の対応を確認中です。"}</p>}
          {retained.extra > 0 && <p className="hint" role="status">検索・ページ条件外の編集中またはフォーカス中の発言 {retained.extra} 件を保持しています。</p>}
          {review.error&&<p className="action-error" role="alert">推定区間を照合できません: {review.error} <button type="button" disabled={locked} onClick={review.retry}>推定表示を再読込</button></p>}
          {shown.map((utterance) => {
            const speakerIds = utterance.speakerIds;
            const source=analysis.utterances.find(u=>u.id===(utterance.sourceUtteranceId??utterance.id))!;
            const session = textSessions[utterance.id], pending = textPending[utterance.id];
            const splitText = session?.value ?? utterance.effectiveText;
            const playing = following.playingIds.has(utterance.id);
            return <article key={utterance.id} data-utterance-id={utterance.id} className={`utterance-row${playing ? " utterance-playing" : ""}`} aria-current={playing ? "true" : undefined} tabIndex={0} aria-label={`${timeLabel(utterance.startUs)}の発言行`} aria-keyshortcuts="Space Control+Z Meta+Z"
              onFocus={() => setFocusedRow(utterance.id)} onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setFocusedRow(current => current === utterance.id ? null : current); }} onKeyDown={event => {
              if (event.target !== event.currentTarget || appShortcut(event.nativeEvent) !== "playback" || overlayOpen() || locked || loading || busy || audio.preparing) return;
              event.preventDefault();
              if (audio.selected === utterance.id && audio.available) audio.toggle(); else void play(utterance);
            }}>
              <div className="utterance-meta"><SpeakerLabels meetingId={meetingId} speakerIds={speakerIds} speakers={view!.speakers}/><button type="button" className="utterance-stamp" aria-label={audio.selected === utterance.id ? "この発言を再生し直す" : "音声を再生"} title={`${timeLabel(utterance.startUs)}–${timeLabel(utterance.endUs)}を再生`} onClick={() => void play(utterance)} disabled={locked || loading || busy || audio.preparing}><time>{utterance.recognitionIsSource?preciseTime(utterance.startUs):timeLabel(utterance.startUs)}</time></button>{speakerIds.length>1 && <span>複数話者</span>}{utterance.sourceUtteranceId?<span>手動分割 · 単語時刻未推定</span>:<SpeakerReviewBadge review={review.reviews.get(utterance.id)}/>}</div>
              {playing && <span className="utterance-playing-label">再生中</span>}
              {session ? <div className="utterance-text-edit">
              <textarea ref={node => { if (node) textInputs.current.set(utterance.id, node); else textInputs.current.delete(utterance.id); }} aria-label={`${timeLabel(utterance.startUs)}の発言`} aria-keyshortcuts="Control+Enter Meta+Enter Escape" value={session.value}
                onKeyDown={event => {
                  if (!editable || overlayOpen()) return;
                  const shortcut = appShortcut(event.nativeEvent);
                  if (shortcut === "cancel" || shortcut === "commit") { event.preventDefault(); void finishTextEdit(utterance.id, shortcut === "cancel" ? "cancel" : "commit", event.currentTarget); }
                }}
                onChange={event => { const value = event.target.value; updateTextSessions(current => ({ ...current, [utterance.id]: { ...current[utterance.id], value } })); }} disabled={!editable} rows={2} maxLength={20000} />
                <div className="transcript-actions"><button type="button" disabled={!editable || !!pending} onClick={event => void finishTextEdit(utterance.id, "commit", event.currentTarget)}>確定</button><button type="button" className="small-secondary" disabled={!editable || pending === "cancel"} onClick={event => void finishTextEdit(utterance.id, "cancel", event.currentTarget)}>キャンセル</button><span>{pending === "cancel" ? "元の本文の保存を確認中" : pending ? "確定した本文を保存中" : "Ctrl／Cmd＋Enterで確定 · Escでキャンセル"}</span></div>
              </div> : <p className="utterance-text" aria-label={`${timeLabel(utterance.startUs)}の発言本文`}>{editable ? <button type="button" className="transcript-body-edit" aria-label={`${timeLabel(utterance.startUs)}の発言を編集`} aria-describedby={`utterance-body-${utterance.id}`} disabled={!editable} onClick={() => beginTextEdit(utterance)}><span id={`utterance-body-${utterance.id}`}>{utterance.effectiveText}</span></button> : utterance.effectiveText}</p>}
              <details className="utterance-edit-tools"><summary>話者・分割・認識原文</summary>
              <label><input type="checkbox" aria-label={`${timeLabel(utterance.startUs)}の発言を選択`} disabled={!editable} checked={selection.includes(utterance.id)} onChange={e=>setSelection(ids=>e.target.checked?[...ids,utterance.id]:ids.filter(id=>id!==utterance.id))}/>一括変更の対象に選択</label>
              {(utterance.recognitionIsSource||utterance.effectiveText !== utterance.recognizedText) && <small>{utterance.recognitionIsSource?"分割元の認識原文（全文・単語位置未推定）":"認識原文"}: {utterance.recognizedText}</small>}
              {speakers.length > 0 && <div className="utterance-speakers" aria-label="話者割り当て">{speakers.map((speaker) => <label key={speaker.id}><input type="checkbox" checked={speakerIds.includes(speaker.id)} disabled={!editable} onChange={(event) => changeUtterance(utterance, { speaker_ids: event.target.checked ? [...speakerIds, speaker.id] : speakerIds.filter((id) => id !== speaker.id) })} /><SpeakerLabel meetingId={meetingId} speakerId={speaker.id} name={speaker.displayName || speaker.label}/></label>)}</div>}
              <button type="button" className="small-secondary" disabled={!editable||!view||view.speakers.length>=100} onClick={()=>{setPendingAdd(false);setError("");setNewName("");setAdding({utteranceId:utterance.id});}}>新しい話者を作成してこの発言に割り当て</button>
              <button type="button" className="small-secondary" disabled={!editable||!!pending||textPieces(splitText).length<2||utterance.endUs-utterance.startUs<2} onClick={()=>{setPendingSplit(false);setError("");setSplitting({...utterance,effectiveText:splitText});setSplitSeconds(((utterance.endUs-utterance.startUs)/2_000_000).toFixed(6));setSplitOffset(Math.floor(textPieces(splitText).length/2));setFirstSpeaker("keep");setSecondSpeaker("keep");}}>発言を手動分割</button>
              {utterance.sourceUtteranceId&&<small>推定区間と区間再生は分割元全体を参照します。この部分だけを聞くには「音声を再生」を使用してください。</small>}
              <SpeakerReviewDetails analysis={analysis} utteranceId={source.id} startUs={source.startUs} labels={speakerLabels} unconfirmedIds={unconfirmedSpeakerIds} locked={locked||audio.preparing} onPlay={at=>audio.start(analysis.id,source.id,undefined,at)}/>
              </details>
            </article>;
          })}
        </div>
        {!filtered.length&&<p>条件に一致する発言はありません。{retained.extra > 0 && "編集中・フォーカス中の発言は保持しています。"}</p>}
        {filtered.length > PAGE_SIZE && <div className="transcript-pages"><button type="button" className="small-secondary" onClick={() => {following.suspend();setPage((value) => Math.max(0, value - 1));}} disabled={page === 0}>前へ</button><span>{page + 1} / {Math.ceil(filtered.length / PAGE_SIZE)}</span><button type="button" className="small-secondary" onClick={() => {following.suspend();setPage((value) => value + 1);}} disabled={(page + 1) * PAGE_SIZE >= filtered.length}>次へ</button></div>}
        </div>
      </div>}
      {error && <p className="action-error" role="alert">{error}</p>}
      <IssueNotice scope={meetingId} type="transcript-save" label="文字起こしの保存に失敗" message={saveError ? `編集は画面に保持しています。内容を確認して保存を再試行してください: ${saveError}` : ""} action={{ kind: "retry", label: "文字起こし保存を再試行", disabled: saving || locked || busy || confirm.pending || !!conflictPreview, run: () => retryTextBuffers().catch(() => { /* The queue publishes the retained failure. */ }) }} />
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
      {conflictPreview && <TranscriptConflictPanel key={conflictGeneration.current} preview={conflictPreview} blocked={busy || locked || !!activeConfirmation}
        onClose={() => { if(conflictBusy.current) return; conflictRef.current = null; setConflictPreview(null); }} onRefresh={inspectConflict} onAccept={acceptConflict}/>}
      {!splitting && audio.sharedSurface && audio.controls}
      {playbackHost && !audio.sharedSurface && createPortal(<div className="meeting-playback-content" data-meeting-id={meetingId}>
        <span className="playback-owner">{meetingTitle || "選択会議"} · 発言区間の音声</span>
        {splitting ? <p className="playback-empty">手動分割の確認画面で音声を操作できます。</p>
          : audio.controls || <p className="playback-empty">発言の時刻か「音声を再生」を選ぶと、保存音声を検証して再生します。</p>}
      </div>,playbackHost)}
    </section>
  );
}
