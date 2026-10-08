import { invoke, isTauri } from "@tauri-apps/api/core";
import { InfoPopover } from "./info-popover";
import { ConfirmationHost } from "./confirmation-host";
import { IssueNotice, ToastHost, ToastNotice } from "./toast";
import { JobNotifications } from "./job-notifications";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type FormEvent } from "react";
import { createRoot } from "react-dom/client";
import { formatElapsed, useRecordingSession, type BridgeState } from "./session";
import { TranscriptEditor } from "./transcript";
import { AnalysisPanel } from "./analysis";
import { MinutesPanel } from "./minutes";
import { VocabularyEditor } from "./vocabulary";
import { flushPendingEdits } from "./draft-save";
import { AudioImportPanel } from "./audio-import";
import { ModelInspectionPanel } from "./model-inspection";
import { useDataOperation } from "./data-operation";
import { DataSavePanel } from "./data-save";
import { RecoveryPanel } from "./recovery-panel";
import { MeetingDeletionPanel, useMeetingDeletion } from "./meeting-deletion";
import { ModelBodyDeletionPanel, useModelBodyDeletion } from "./model-body-deletion";
import { AudioDeletionPanel, useAudioDeletion } from "./audio-deletion";
import type { DataSnapshot } from "./data-operation-state";
import type { ImportSnapshot } from "./import-state";
import type { RecordingSnapshot } from "./session";
import "./style.css";
import { ThemeProvider, ThemeSelection, ThemeSettingsPanel, ThemeReadNotice, ThemeNotifications } from "./theme";
import { RecordingConfirmationWindow, useRecordingConfirmation } from "./recording-confirmation";
import { confirmations, useActionConfirmation } from "./confirmation-service";
import { withConfirmedEdits } from "./confirmed-action";
import { assertRecordingDecision } from "./recording-confirmation-state";
import { waitForDataStop } from "./data-stop";
import { appShortcut, overlayOpen } from "./keyboard";
import { MeetingPlaybackSurface } from "./playback-surface";
import { MeetingAudioControls } from "./meeting-audio";
import {CaptureInputSelection,useCaptureInputs} from "./capture-inputs";
import {checkedCaptureInputs,usableMicrophone} from "./capture-input-state";
import {useCaptureScope} from "./capture-applications";
import {captureApplicationScope,checkedCaptureApplications,checkedCaptureScope} from "./capture-scope-state";
import { assertCloseContext, assertCloseImport, assertCloseRecording, assertCloseTargets, closeMessage, closeTarget, dataNeedsConsent, importNeedsConsent, recordingActive, waitForImportStop, waitForRecordingSave, type CloseActivity, type CloseContext } from "./application-close";

const isRecorderPanel = new URLSearchParams(window.location.search).get("window") === "recorder";
const isRecordingConfirmation = new URLSearchParams(window.location.search).get("window") === "recording-confirmation";

interface Meeting {
  id: string;
  title: string;
  lifecycle: string;
  sourceType?: "recording" | "audio_import" | null;
  createdAt: number;
  updatedAt: number;
  lockVersion: number;
}

function phaseLabel(state: BridgeState): string {
  if (state.kind === "loading") return "接続中";
  if (state.kind === "unavailable") return "デスクトップ接続なし";
  if (state.kind === "error") return "接続エラー";
  return {
    idle: "待機中",
    preparing: "準備中",
    recording: "録音中",
    stopping: "停止中",
    saving: "保存中",
    error: "要対応",
  }[state.snapshot.phase];
}

function useWindowAction() {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const run = async (command: string) => {
    setBusy(true);
    setError("");
    try {
      await invoke(command);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  return { run, error, busy };
}

function RecorderPanel() {
  const state = useRecordingSession();
  const decision = useRecordingConfirmation("recorder");
  const action = useWindowAction();
  const idle = state.kind === "ready" && state.snapshot.phase === "idle";
  return (
    <main className="mini-shell" aria-label="録音ミニパネル">
      <div className="mini-top">
        <span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} />
        <strong>{phaseLabel(state)}</strong>
        <span className="timer">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</span>
      </div>
      <div className="mini-bottom">
        <span className="mini-note">{idle ? "待機中" : "録音状態はメイン画面と共通"}</span>
        <button type="button" onClick={() => action.run("open_main_window")} disabled={action.busy || !isTauri()}>メインを開く</button>
        {state.kind === "ready" && (state.snapshot.phase === "recording" || state.snapshot.phase === "preparing") && <button type="button" className="stop-button" onClick={() => void decision.request()} disabled={action.busy || decision.requestBusy || decision.visible || !decision.connected}>{state.snapshot.phase === "preparing" ? "中止" : "停止"}</button>}
        {idle && <button type="button" className="text-button" onClick={() => action.run("hide_recorder_preview")} disabled={action.busy}>閉じる</button>}
      </div>
      {action.error && <p className="mini-error" role="alert">{action.error}</p>}
      {decision.error && <p className="mini-error" role="alert">{decision.error}</p>}
      <ThemeReadNotice compact />
    </main>
  );
}

function MainWindow() {
  const state = useRecordingSession();
  const recordingDecision = useRecordingConfirmation("main");
  const dataOperation = useDataOperation();
  const [dataPending, setDataPending] = useState(false);
  const [recoveryPending, setRecoveryPending] = useState(false);
  const [modelDeletionPending, setModelDeletionPending] = useState(false);
  const [audioDeletionPending, setAudioDeletionPending] = useState(false);
  const [modelCatalogEpoch, setModelCatalogEpoch] = useState(0);
  const [restoredIds, setRestoredIds] = useState<string[]>([]);
  const [restoredFilter, setRestoredFilter] = useState<string[] | null>(null);
  const [originError, setOriginError] = useState("");
  const action = useWindowAction();
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [meetingTitle, setMeetingTitle] = useState("");
  const [meetingError, setMeetingError] = useState("");
  const [meetingBusy, setMeetingBusy] = useState(false);
  const [selectedMeeting, setSelectedMeeting] = useState<Meeting | null>(null);
  const [detailTab, setDetailTab] = useState<"transcript" | "minutes">("transcript");
  const [diarizationHost,setDiarizationHost] = useState<{ meetingId: string; node: HTMLDivElement } | null>(null);
  const bindDiarizationHost = useCallback((meetingId: string,node: HTMLDivElement | null) => {
    setDiarizationHost(current => node ? { meetingId,node } : current?.meetingId === meetingId ? null : current);
  }, []);
  const [meetingToolsOpen, setMeetingToolsOpen] = useState(false);
  useLayoutEffect(() => { setDetailTab("transcript"); }, [selectedMeeting?.id]);
  const [playbackHost,setPlaybackHost] = useState<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    const column = playbackHost?.parentElement,header = column?.querySelector<HTMLElement>(".topbar");
    if(!playbackHost || !column || !header) return;
    const measure = () => {
      column.style.setProperty("--playback-dock-height",`${Math.ceil(playbackHost.getBoundingClientRect().height)}px`);
      const tabs = column.querySelector<HTMLElement>(".meeting-detail-tabs");
      column.style.setProperty("--workspace-header-height",`${Math.ceil(header.getBoundingClientRect().height + (tabs?.getBoundingClientRect().height ?? 0))}px`);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(playbackHost); observer.observe(header); measure();
    return () => { observer.disconnect(); column.style.removeProperty("--playback-dock-height"); column.style.removeProperty("--workspace-header-height"); };
  },[playbackHost,selectedMeeting?.id]);
  const [renameTitle, setRenameTitle] = useState("");
  const [captureAvailable, setCaptureAvailable] = useState(false);
  const [captureBusy, setCaptureBusy] = useState(false);
  const [captureError, setCaptureError] = useState("");
  const captureInputs=useCaptureInputs(captureAvailable&&selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType!=="audio_import"&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase),selectedMeeting?.id??null);
  const captureScope=useCaptureScope(captureAvailable&&selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType!=="audio_import"&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase),selectedMeeting?.id??null);
  const [closingBusy, setClosingBusy] = useState(false);
  const closing = useRef(false), starting = useRef(false);
  const [recoveredCount, setRecoveredCount] = useState(0);
  const [navigationBusy, setNavigationBusy] = useState(false);
  const [screen, setScreen] = useState<"overview" | "meetings" | "settings">("overview");
  const loadOrigins = async () => {
    try { setRestoredIds(await invoke<string[]>("list_restored_meeting_ids")); setOriginError(""); }
    catch (cause) { setOriginError(`復元由来を確認できません: ${String(cause)}`); }
  };
  const [analysisEpoch, setAnalysisEpoch] = useState(0);
  const navigating = useRef(false);
  const priorPhase = useRef<string | null>(null);
  const withSavedEdits = async (action: () => void | Promise<void>) => {
    if (navigating.current) return;
    navigating.current = true;
    setNavigationBusy(true);
    setMeetingError("");
    try {
      await flushPendingEdits();
      await action();
    } catch (cause) {
      setMeetingError(`操作を完了できませんでした。編集内容はこの画面に保持しています: ${String(cause)}`);
    } finally {
      navigating.current = false;
      setNavigationBusy(false);
    }
  };
  const deletion = useMeetingDeletion({ operation: dataOperation, locked: navigationBusy || dataPending || recoveryPending || modelDeletionPending || audioDeletionPending, withSavedEdits,
    onDeleted: async receipt => {
      setSelectedMeeting(current => current?.id === receipt.meetingId ? null : current);
      setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins();
    } });
  const modelDeletion = useModelBodyDeletion({ operation: dataOperation, locked: navigationBusy || dataPending || recoveryPending || deletion.protectedNow || audioDeletionPending, withSavedEdits,
    onProtectionChange: setModelDeletionPending, onDeleted: async () => { setModelCatalogEpoch(epoch => epoch + 1); } });
  const audioDeletion = useAudioDeletion({ operation: dataOperation, locked: navigationBusy || dataPending || recoveryPending || deletion.protectedNow || modelDeletion.protectedNow, withSavedEdits,
    onProtectionChange: setAudioDeletionPending, onDeleted: async () => {
      // Audio availability changes, not document heads. Keep mounted editors and
      // their local buffers, including when recovery deliberately skipped writes.
      setMeetings(await invoke<Meeting[]>("list_meetings"));
    } });
  const baseNavigationLocked = recordingDecision.visible || recordingDecision.requestBusy || navigationBusy || dataPending || recoveryPending || deletion.protectedNow || modelDeletion.protectedNow || audioDeletion.protectedNow || !dataOperation.connected || !!dataOperation.snapshot?.busy;
  const deletionBlocked = !!dataOperation.deletionListing?.writeBlocked || !!dataOperation.snapshot?.deletionPreview?.pending || !!dataOperation.modelDeletionListing?.writeBlocked || !!dataOperation.snapshot?.modelDeletionPreview?.pending || !!dataOperation.audioDeletionListing?.writeBlocked || !!dataOperation.snapshot?.audioDeletionPreview?.pending;
  const baseWriteLocked = baseNavigationLocked || deletionBlocked || !!dataOperation.snapshot?.restartRequired;
  const startConfirm = useActionConfirmation(baseWriteLocked || closingBusy);
  const closeConfirm = useActionConfirmation(false);
  const navigationLocked = baseNavigationLocked || captureBusy || closingBusy || startConfirm.pending;
  const writeLocked = baseWriteLocked || captureBusy || closingBusy || startConfirm.pending;
  const root = useRef({ selectedMeeting, baseWriteLocked, captureAvailable, captureInputs, captureScope, state });
  root.current = { selectedMeeting, baseWriteLocked, captureAvailable, captureInputs, captureScope, state };
  const readCloseContext = async (): Promise<CloseContext> => {
    const [recording, importing, data, activity] = await Promise.all([
      invoke<RecordingSnapshot>("get_recording_snapshot"), invoke<ImportSnapshot>("get_audio_import"),
      invoke<DataSnapshot>("get_data_operation"), invoke<CloseActivity>("get_application_close_activity"),
    ]);
    const result = { recording, importing, data, activity }; assertCloseContext(result); return result;
  };
  const closeHandler = useRef<(request: { quit: boolean; afterRecording?: string }) => Promise<void>>(async () => {});
  closeHandler.current = async ({ quit, afterRecording }) => {
    if (closing.current || starting.current || navigating.current || confirmations.getSnapshot()) return;
    closing.current = true; setClosingBusy(true); setMeetingError("");
    try {
      const recording = await invoke<RecordingSnapshot>("get_recording_snapshot"); assertCloseRecording(recording);
      if (!quit && recordingActive(recording)) {
        await withConfirmedEdits(withSavedEdits, () => invoke("finish_main_close", { quit: false }));
        return;
      }
      let coveredAnalysis = false;
      if (afterRecording) {
        const decision = await invoke<import("./recording-confirmation-state").RecordingDecision>("get_recording_confirmation");
        assertRecordingDecision(decision);
        if (decision.id !== afterRecording || decision.phase !== "completed" || !decision.quitAfterSave || recordingActive(recording)) throw Error("録音終了の対象が変更されています。アプリは終了していません。");
        coveredAnalysis = true;
      }
      if (quit && ["preparing", "recording"].includes(recording.phase)) {
        await withConfirmedEdits(withSavedEdits, async () => {
          const current = await invoke<RecordingSnapshot>("get_recording_snapshot"); assertCloseRecording(current);
          if (current.sessionId !== recording.sessionId || current.meetingId !== recording.meetingId || !["preparing", "recording"].includes(current.phase)) throw Error("録音の終了対象が変更されました。最新の状態を再確認してください。");
          await recordingDecision.request(true);
        });
        return; // The native confirmation continues closing only after saved.
      }
      if (quit && recordingActive(recording)) {
        const outcome = await closeConfirm({ title: "録音の保存完了後の終了を確認", target: recording.meetingId ?? "保存中の録音", confirmLabel: "保存完了を待って終了へ進む", message: "録音の停止・保存が進行中です。保存成功を確認してから終了へ進みます。同じ録音は継続できません。停止は再送せず、保存失敗・結果未確認では終了しません。", action: () => waitForRecordingSave(recording, () => invoke("get_recording_snapshot")) });
        if (outcome !== "completed") return;
      }
      const expected = await readCloseContext();
      const finish = () => withConfirmedEdits(withSavedEdits, async () => {
        const current = await readCloseContext(); assertCloseTargets(expected, current);
        let importing = current.importing, data = current.data;
        if (importNeedsConsent(expected.importing) && importNeedsConsent(importing)) {
          await invoke("cancel_audio_import", { token: importing.token });
          importing = await waitForImportStop(importing.token!, () => invoke("get_audio_import"));
        }
        if (dataNeedsConsent(expected.data) && dataNeedsConsent(data)) {
          await invoke("cancel_data_operation", { token: data.token });
          data = await waitForDataStop(data.token!, data.kind!, () => invoke("get_data_operation"));
        }
        const final = await readCloseContext(); assertCloseTargets({ ...current, importing, data }, final);
        await invoke("finish_main_close", { quit, expected: { importToken: final.importing.token, dataToken: final.data.token } });
      });
      if (importNeedsConsent(expected.importing) || dataNeedsConsent(expected.data) || !coveredAnalysis && (expected.activity.analysisBusy || expected.activity.modelPreparationBusy)) {
        await closeConfirm({ title: "進行中の処理を停止して終了しますか？", target: closeTarget(expected), confirmLabel: "安全な停止を待って終了する", message: closeMessage(expected), action: finish });
      } else await finish();
    } catch (cause) { setMeetingError(`終了できませんでした。アプリと保存結果は保持しています: ${String(cause)}`); }
    finally { closing.current = false; setClosingBusy(false); }
  };
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<{ quit: boolean; afterRecording?: string }>("main-close-requested", (event) => {
      if (disposed) return;
      void closeHandler.current(event.payload);
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch((cause: unknown) => setMeetingError(String(cause)));
    return () => { disposed = true; unlisten?.(); };
  }, []);
  useEffect(() => {
    if (!isTauri()) return;
    void invoke<Meeting[]>("list_meetings").then(setMeetings).catch((error: unknown) => setMeetingError(String(error)));
    void loadOrigins();
    void invoke<boolean>("capture_available").then(setCaptureAvailable).catch(() => setCaptureAvailable(false));
  }, []);
  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<string[]>("recordings-recovered", (event) => {
      if (!active) return;
      setRecoveredCount(event.payload.length);
      void invoke<Meeting[]>("list_meetings").then((items) => { if (active) setMeetings(items); }).catch((error: unknown) => { if (active) setMeetingError(String(error)); });
      if (selectedMeeting && event.payload.some((id) => id === selectedMeeting.id)) {
        void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then((item) => { if (active) setSelectedMeeting(item); }).catch((error: unknown) => { if (active) setMeetingError(String(error)); });
      }
    }).then((stop) => {
      if (!active) { stop(); return; }
      unlisten = stop;
      // Also refresh after subscribing in case recovery finished before the listener existed.
      void invoke<Meeting[]>("list_meetings").then((items) => { if (active) setMeetings(items); }).catch((error: unknown) => { if (active) setMeetingError(String(error)); });
      if (selectedMeeting) void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then((item) => { if (active) setSelectedMeeting(item); }).catch((error: unknown) => { if (active) setMeetingError(String(error)); });
    }).catch((error: unknown) => { if (active) setMeetingError(String(error)); });
    return () => { active = false; unlisten?.(); };
  }, [selectedMeeting?.id]);
  useEffect(() => {
    const phase = state.kind === "ready" ? state.snapshot.phase : null;
    if (priorPhase.current && priorPhase.current !== phase && (phase === "idle" || phase === "error") && isTauri()) {
      void invoke<Meeting[]>("list_meetings").then(setMeetings).catch((error: unknown) => setMeetingError(String(error)));
      if (selectedMeeting) void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then(setSelectedMeeting).catch((error: unknown) => setMeetingError(String(error)));
    }
    priorPhase.current = phase;
  }, [state, selectedMeeting]);
  const startRecording = async () => {
    if (!selectedMeeting || !captureInputs.selected || !captureScope.scope || starting.current || closing.current || root.current.baseWriteLocked) return;
    const microphoneUID=captureInputs.uid;
    const scope=checkedCaptureScope(captureScope.scope,captureScope.applications);
    const scopeKey=JSON.stringify(scope);
    const scopeUnchanged=()=>JSON.stringify(root.current.captureScope.scope)===scopeKey;
    starting.current = true; setCaptureBusy(true); setCaptureError("");
    try {
      const expected = await invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id });
      const importing = await invoke<ImportSnapshot>("get_audio_import"); assertCloseImport(importing);
      const microphone=usableMicrophone(checkedCaptureInputs(await invoke("get_capture_inputs")),microphoneUID);
      if(!microphone)throw Error("選択したマイクまたは権限を再確認してください。別機器には切り替えていません。");
      const applications=scope.mode==="application"?checkedCaptureApplications(await invoke("get_capture_applications")):null;
      checkedCaptureScope(scope,applications);
      const application=captureApplicationScope(scope,applications);
      const scopeMessage=scope.mode==="system"?"PC音声は全体取得し、通知音や他の動画の音も含まれる場合があります。":`PC音声は選択したアプリ（${application?.name}）を取得します。ブラウザでは他のタブの音も含まれる場合があり、特定タブだけの取得は保証しません。終了・再起動時は全体取得や別アプリに切り替えません。`;
      if (!expected || expected.lifecycle !== "draft" || expected.sourceType === "audio_import") throw Error("録音対象の会議を再確認してください。");
      await startConfirm({ title: "選択した会議の録音を開始しますか？", target: expected.title, confirmLabel: "録音を開始する", message: "マイクとPCの再生音が端末内に保存されます。参加者の同意と録音権限を確認してください。実行中の解析は安全な区切りで一時停止し、録音終了後に再開します。"+`\n録音するマイク: ${microphone.name}。${scopeMessage}` + (importNeedsConsent(importing) ? `\n取り込み（${importing.fileName ?? "選択・確認中の音声"}）を安全に取り消して録音を優先します。取り込みの確認内容と未確定物だけを整理し、元ファイルと保存済み会議を保持します。` : ""), validate: () => root.current.selectedMeeting?.id === expected.id && root.current.captureAvailable && root.current.captureInputs.uid===microphoneUID&&!!root.current.captureInputs.selected && scopeUnchanged() && !root.current.baseWriteLocked && root.current.state.kind === "ready" && ["idle", "error"].includes(root.current.state.snapshot.phase), action: () => withConfirmedEdits(withSavedEdits, async () => {
        const [meeting, recording, currentImport, currentInputs, currentApplications] = await Promise.all([invoke<Meeting | null>("get_meeting", { id: expected.id }), invoke<RecordingSnapshot>("get_recording_snapshot"), invoke<ImportSnapshot>("get_audio_import"),invoke("get_capture_inputs").then(checkedCaptureInputs),scope.mode==="application"?invoke("get_capture_applications").then(checkedCaptureApplications):Promise.resolve(null)]);
        assertCloseRecording(recording);
        assertCloseImport(currentImport);
        if(!usableMicrophone(currentInputs,microphoneUID)||root.current.captureInputs.uid!==microphoneUID)throw Error("確認したマイクまたは権限が変更されました。録音は開始していません。別機器には切り替えていません。");
        checkedCaptureScope(scope,currentApplications);
        if(!scopeUnchanged())throw Error("PC音声の取得範囲が変更されました。録音は開始していません。");
        if (!meeting || meeting.id !== expected.id || meeting.title !== expected.title || meeting.lockVersion !== expected.lockVersion || meeting.lifecycle !== "draft" || meeting.sourceType === "audio_import" || root.current.selectedMeeting?.id !== expected.id || recordingActive(recording) || currentImport.token !== importing.token || !importing.busy && currentImport.revision !== importing.revision) throw Error("録音対象が確認後に変更されました。録音は開始していません。");
        await invoke("start_recording", { meetingId: expected.id, microphoneUid:microphoneUID, captureScope:scope });
        setSelectedMeeting(await invoke<Meeting | null>("get_meeting", { id: expected.id }));
        setMeetings(await invoke<Meeting[]>("list_meetings"));
      }), onError: cause => setCaptureError(`録音の受付状態を確認してください。開始は自動再送していません: ${String(cause)}`) });
    } catch (error) {
      setCaptureError(String(error));
    } finally {
      starting.current = false;
      setCaptureBusy(false);
    }
  };
  const stopRecording = async () => {
    await recordingDecision.request();
  };
  const createMeeting = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    await withSavedEdits(async () => {
      setMeetingBusy(true);
      try {
        const created = await invoke<Meeting>("create_meeting", { title: meetingTitle });
        setMeetingTitle("");
        setMeetings(await invoke<Meeting[]>("list_meetings"));
        setSelectedMeeting(created);
        setMeetingToolsOpen(false);
        setScreen("overview");
        setRenameTitle(created.title);
      } finally {
        setMeetingBusy(false);
      }
    });
  };
  const selectMeeting = async (id: string) => {
    await withSavedEdits(async () => {
      const meeting = await invoke<Meeting | null>("get_meeting", { id });
      if (!meeting) throw new Error("会議が見つかりません");
      setSelectedMeeting(meeting);
      setMeetingToolsOpen(false);
      setRenameTitle(meeting.title);
      setScreen("overview");
    });
  };
  const openNotifiedMeeting = async (id: string) => {
    if (navigationLocked) return;
    await withSavedEdits(async () => {
      const meeting = await invoke<Meeting | null>("get_meeting", { id });
      if (!meeting || meeting.id !== id) throw Error("通知対象の会議が見つかりません。別の会議には切り替えていません。");
      setSelectedMeeting(meeting); setRenameTitle(meeting.title); setScreen("overview");
      setMeetingToolsOpen(false);
      requestAnimationFrame(() => document.querySelector<HTMLElement>('[aria-label="会議ワークスペース"]')?.focus({ preventScroll: false }));
    });
  };
  const openRecordingMeeting = async () => {
    const expected = state.kind === "ready" ? state.snapshot : null;
    if (!expected?.meetingId || !expected.sessionId || !recordingActive(expected) || navigationLocked) return;
    await withSavedEdits(async () => {
      const current = await invoke<RecordingSnapshot>("get_recording_snapshot");
      assertCloseRecording(current);
      if (!recordingActive(current) || current.sessionId !== expected.sessionId || current.meetingId !== expected.meetingId || current.revision < expected.revision) throw Error("録音の対象が変更されています。最新のヘッダーから開き直してください。");
      const meeting = await invoke<Meeting | null>("get_meeting", { id: current.meetingId });
      if (!meeting || meeting.id !== current.meetingId) throw Error("録音中の会議が見つかりません。録音は停止していません。");
      setSelectedMeeting(meeting); setRenameTitle(meeting.title); setScreen("overview");
      setMeetingToolsOpen(true);
      requestAnimationFrame(() => {
        const tools = document.querySelector<HTMLDetailsElement>(".meeting-workspace-tools");
        if (tools) tools.open = true;
        document.querySelector<HTMLElement>('[aria-label="録音状態"]')?.focus({ preventScroll: false });
      });
    });
  };
  const switchDetailTab = async (tab: "transcript" | "minutes") => {
    if (tab === detailTab || navigationLocked || overlayOpen()) return;
    const owner = selectedMeeting?.id;
    await withSavedEdits(() => {
      if (!owner || root.current.selectedMeeting?.id !== owner) throw Error("会議が変更されています。元のタブ切替は実行していません。");
      setDetailTab(tab);
      requestAnimationFrame(() => document.getElementById(`meeting-tab-${tab}`)?.focus({ preventScroll: true }));
    });
  };
  const keyboardAction = useRef<(event: KeyboardEvent) => void>(() => {});
  keyboardAction.current = event => {
    if (overlayOpen() || navigationLocked || confirmations.getSnapshot()) return;
    const shortcut = appShortcut(event);
    if (shortcut === "search") {
      const search = Array.from(document.querySelectorAll<HTMLInputElement>('.content input[type="search"]'))
        .find(input => !input.disabled && !input.closest('[hidden], [inert]') && input.getClientRects().length > 0);
      if (search) { event.preventDefault(); search.focus(); search.select(); }
      else if (selectedMeeting && screen === "overview" && detailTab === "minutes") {
        event.preventDefault();
        const owner = selectedMeeting.id;
        void withSavedEdits(() => {
          if (root.current.selectedMeeting?.id !== owner) throw Error("検索対象の会議が変更されています。");
          setDetailTab("transcript");
          requestAnimationFrame(() => { const field = document.querySelector<HTMLInputElement>(".transcript-search input[type=search]"); field?.focus(); field?.select(); });
        });
      }
    } else if (shortcut === "recording" && isTauri() && state.kind === "ready") {
      event.preventDefault();
      if (recordingActive(state.snapshot)) { void openRecordingMeeting(); return; }
      if (writeLocked || meetingBusy || navigating.current || starting.current || closing.current) return;
      void withSavedEdits(async () => {
        const current = await invoke<RecordingSnapshot>("get_recording_snapshot"); assertCloseRecording(current);
        if (recordingActive(current)) throw Error("録音状態が変更されています。最新の状態から録音画面を開き直してください。");
        setSelectedMeeting(null);
        setScreen("overview");
        requestAnimationFrame(() => document.getElementById("meeting-title")?.focus());
      });
    }
  };
  useEffect(() => {
    const handler = (event: KeyboardEvent) => keyboardAction.current(event);
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, []);
  const renameMeeting = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!selectedMeeting) return;
    setMeetingBusy(true);
    setMeetingError("");
    try {
      const updated = await invoke<Meeting>("rename_meeting", { id: selectedMeeting.id, title: renameTitle, expectedLockVersion: selectedMeeting.lockVersion });
      setSelectedMeeting(updated);
      setRenameTitle(updated.title);
      setMeetings(await invoke<Meeting[]>("list_meetings"));
    } catch (error) {
      setMeetingError(String(error));
    } finally {
      setMeetingBusy(false);
    }
  };
  const reloadMeetings = async () => {
    await withSavedEdits(async () => {
      setMeetings(await invoke<Meeting[]>("list_meetings"));
      await loadOrigins();
      if (selectedMeeting) {
        const current = await invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id });
        setSelectedMeeting(current);
        setRenameTitle(current?.title ?? "");
      }
    });
  };
  const canPreview = state.kind === "ready" && state.snapshot.phase === "idle";
  const shownMeetings = restoredFilter ? meetings.filter(meeting => restoredFilter.includes(meeting.id)) : meetings;
  const detail = screen === "overview" && !!selectedMeeting;
  return (
    <MeetingPlaybackSurface host={playbackHost} meetingId={selectedMeeting?.id ?? null} title={selectedMeeting?.title}>
    <div className="app-layout">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark" aria-hidden="true">m</span><span className="brand-name">mon</span></div>
        <nav aria-label="メインナビゲーション">
          <button type="button" aria-label="概要" title="概要" aria-current={screen === "overview" ? "page" : undefined} className={`nav-item ${screen === "overview" ? "selected" : ""}`} disabled={navigationLocked} onClick={() => void withSavedEdits(() => setScreen("overview"))}><span className="nav-glyph" aria-hidden="true">⌂</span><span className="nav-label">概要</span></button>
          <button type="button" aria-label="会議一覧" title="会議一覧" aria-current={screen === "meetings" ? "page" : undefined} className={`nav-item ${screen === "meetings" ? "selected" : ""}`} disabled={navigationLocked} onClick={() => void withSavedEdits(() => { setSelectedMeeting(null); setRestoredFilter(null); setScreen("meetings"); })}><span className="nav-glyph" aria-hidden="true">☷</span><span className="nav-label">会議一覧</span></button>
          <button type="button" aria-label="設定" title="設定" aria-current={screen === "settings" ? "page" : undefined} className={`nav-item ${screen === "settings" ? "selected" : ""}`} disabled={navigationLocked} onClick={() => void withSavedEdits(() => setScreen("settings"))}><span className="nav-glyph" aria-hidden="true">⚙</span><span className="nav-label">設定</span></button>
        </nav>
        <div className="sidebar-foot">端末内で会議を記録</div>
      </aside>
      <div className="main-column">
        <ToastHost />
        <ThemeNotifications locked={writeLocked} />
        <ToastNotice scope={state.kind === "ready" ? state.snapshot.sessionId ?? "recording" : "recording"} type="recording-failure" level="error" message={state.kind === "ready" ? state.snapshot.errorMessage : state.kind === "error" ? state.message : ""} />
        <header className="topbar"><span>{detail ? "会議詳細" : screen === "overview" ? "概要" : screen === "meetings" ? "会議一覧" : "設定"}</span><ThemeSelection compact locked={writeLocked} /><div className="header-recording" aria-label="共通録音状態"><span className="header-status"><span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} />{phaseLabel(state)}{state.kind === "ready" && recordingActive(state.snapshot) && <span className="header-timer">{formatElapsed(state.snapshot.elapsedMs)}</span>}</span>{state.kind === "ready" && recordingActive(state.snapshot) && <button type="button" className="small-secondary" disabled={navigationLocked} onClick={() => void openRecordingMeeting()}>録音画面へ</button>}{state.kind === "ready" && ["preparing", "recording"].includes(state.snapshot.phase) && <button type="button" className="stop-button" disabled={captureBusy || navigationBusy || recordingDecision.requestBusy || recordingDecision.visible || !recordingDecision.connected} onClick={() => void stopRecording()}>録音を終了</button>}</div></header>
        {detail && <div className="meeting-detail-tabs" role="tablist" aria-label="会議詳細">
          {(["transcript", "minutes"] as const).map(tab => <button key={tab} type="button" role="tab" id={`meeting-tab-${tab}`} aria-controls={`meeting-panel-${tab}`} aria-selected={detailTab === tab} tabIndex={detailTab === tab ? 0 : -1} disabled={navigationLocked} onClick={() => void switchDetailTab(tab)} onKeyDown={event => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return;
            const target = event.key === "Home" ? "transcript" : event.key === "End" ? "minutes" : ["ArrowLeft", "ArrowRight"].includes(event.key) ? tab === "transcript" ? "minutes" : "transcript" : null;
            if (target) { event.preventDefault(); document.getElementById(`meeting-tab-${target}`)?.focus(); }
          }}>{tab === "transcript" ? "文字起こし" : "議事録"}</button>)}
        </div>}
        <main className={`content${detail ? " meeting-detail-content" : ""}`} aria-label="会議ワークスペース" tabIndex={-1}>
          <JobNotifications locked={navigationLocked} onOpen={openNotifiedMeeting} />
          <AudioImportPanel visible={screen==="overview" && !detail} locked={writeLocked} recordingActive={state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)} withSavedEdits={withSavedEdits} onNewRecording={()=>{document.getElementById("meeting-title")?.focus();}} onSaved={async(id)=>{
            await flushPendingEdits();
            const [meeting,items]=await Promise.all([invoke<Meeting|null>("get_meeting",{id}),invoke<Meeting[]>("list_meetings")]);
            if(!meeting)throw Error("保存した会議が見つかりません。");
            setSelectedMeeting(meeting);setMeetingToolsOpen(false);setRenameTitle(meeting.title);setMeetings(items);setScreen("overview");
          }}/>
          {screen === "settings" && <><h1>設定</h1><p className="lead">会議データのバックアップと追加復元、共通用語とAIモデルの設定を管理します。</p></>}
          {screen === "settings" && <ThemeSettingsPanel locked={writeLocked} />}
          {screen !== "settings" && <ThemeReadNotice />}
          <DataSavePanel operation={dataOperation} visible={screen === "settings"} locked={navigationBusy || recoveryPending || deletion.protectedNow || modelDeletion.protectedNow || audioDeletion.protectedNow || deletionBlocked} withSavedEdits={withSavedEdits} onProtectionChange={setDataPending} onRestored={async () => { setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins(); }} onShowMeetings={async ids => { await withSavedEdits(async () => { setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins(); setRestoredFilter(ids); setSelectedMeeting(null); setScreen("meetings"); }); }} />
          <MeetingDeletionPanel operation={dataOperation} control={deletion} visible={screen === "settings"} />
          <ModelBodyDeletionPanel operation={dataOperation} control={modelDeletion} visible={screen === "settings"} />
          <AudioDeletionPanel operation={dataOperation} control={audioDeletion} visible={screen === "settings"} meetingTitle={meetings.find(m => m.id === audioDeletion.preview?.meetingId)?.title} />
          <RecoveryPanel operation={dataOperation} visible={screen === "settings"} locked={navigationBusy || dataPending || deletion.protectedNow || modelDeletion.protectedNow || audioDeletion.protectedNow || deletionBlocked} withSavedEdits={withSavedEdits} onProtectionChange={setRecoveryPending} />
          {screen === "settings" ? <><VocabularyEditor locked={writeLocked} /><ModelInspectionPanel locked={writeLocked} deletionControl={modelDeletion} refreshEpoch={modelCatalogEpoch} recordingActive={state.kind!=="ready" || !["idle","error"].includes(state.snapshot.phase)} /><IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} /></> : <>
          {!detail && <><div className="eyebrow">DESKTOP FOUNDATION</div>
          <h1>{screen === "meetings" ? "会議一覧" : "会議の記録を、手元で。"}</h1>
          <p className="lead">保存済み音声を端末内で文字起こし・話者分離し、修正後の内容から議事録候補を生成できます。内容と根拠を確認して採用してください。macOSの実録音と推定品質は検証中です。</p></>}
          {detail && <div className="meeting-detail-status"><strong title={selectedMeeting!.title}>{selectedMeeting!.title}</strong>{navigationBusy && <span role="status">編集の保存を確認中</span>}<IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} /></div>}
          <details className={detail ? "meeting-workspace-tools workspace-tools" : "landing-tools"} open={!detail || meetingToolsOpen} onToggle={event => { if (detail) setMeetingToolsOpen(event.currentTarget.open); }}>
          <summary hidden={!detail}>会議管理・録音・解析操作</summary>
          <div className="workspace-tools-body">
          <section className="status-card" aria-label="録音状態" tabIndex={-1}>
            <div className="card-head"><div className="heading-with-info"><h2>録音セッション</h2><InfoPopover label="録音セッション" details={<p>マイクとPC音声を別々のトラックに保存します。PC音声の対象範囲はOSのキャプチャ機能と権限に依存し、会議アプリだけに限定されるとは限りません。参加者の同意を得て、不要なアプリの音声を止めてください。保存後は準備済みモデルで文字起こし・話者分離を順に進め、結果を候補として保存します。モデル未準備でも音声は保持します。録音品質・長時間と10話者の正式評価は保留中です。</p>}>マイクとPC音声を別々に保存します。PC音声には会議以外の音が含まれる場合があります。同意を確認し、不要な音を止めてください。保存後の解析結果は確認待ちの候補です。</InfoPopover></div><span className="state-pill">{phaseLabel(state)}</span></div>
            <div className="clock">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</div>
            <p>{state.kind === "ready" && state.snapshot.phase === "idle" ? "録音は開始していません。" : state.kind === "ready" && state.snapshot.errorMessage ? state.snapshot.errorMessage : state.kind === "error" ? state.message : "録音状態を確認しています。"}</p>
            {captureAvailable&&selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType!=="audio_import"&&<CaptureInputSelection control={captureInputs} scopeControl={captureScope} locked={writeLocked||state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)}/>}
            <div className="card-actions">
              <button type="button" onClick={() => void startRecording()} disabled={!isTauri() || !captureAvailable || !captureInputs.selected || !captureScope.scope || !selectedMeeting || selectedMeeting.lifecycle !== "draft" || selectedMeeting.sourceType === "audio_import" || captureBusy || writeLocked || state.kind !== "ready" || (state.snapshot.phase !== "idle" && state.snapshot.phase !== "error")}>録音を開始</button>
              <button type="button" className="stop-button" onClick={() => void stopRecording()} disabled={captureBusy || navigationBusy || recordingDecision.requestBusy || recordingDecision.visible || !recordingDecision.connected || state.kind !== "ready" || (state.snapshot.phase !== "recording" && state.snapshot.phase !== "preparing")}>録音を停止</button>
              <button type="button" className="secondary" onClick={() => action.run("show_recorder_preview")} disabled={!canPreview || action.busy}>ミニパネルをプレビュー</button>
            </div>
            {!captureAvailable && <p className="capture-note">macOS録音プローブが未配置です。署名済みバンドルのビルド後に利用できます。</p>}
            <IssueNotice scope="main" type="recording-command" label="録音の操作" message={captureError} />
            <IssueNotice scope="main" type="recording-confirmation" label="録音終了の確認" message={recordingDecision.error} />
            <IssueNotice scope="main" type="window-command" label="ウィンドウの操作" message={action.error} />
          </section>
          <section className="meetings-card" aria-label="保存済み会議">
            <div className="card-head"><h2>会議</h2><button type="button" className="small-secondary" onClick={reloadMeetings} disabled={!isTauri() || writeLocked}>再読込</button></div>
            {navigationBusy && <p role="status">編集内容を保存して操作を完了しています。</p>}
            <ToastNotice scope="main" type="recording-recovered" message={recoveredCount > 0 ? `前回の音声保存 ${recoveredCount} 件を検証して復旧しました。` : ""} onClose={() => setRecoveredCount(0)} />
            <form className="meeting-form" onSubmit={createMeeting}>
              <label htmlFor="meeting-title">会議名</label>
              <div className="meeting-form-row"><input id="meeting-title" value={meetingTitle} onChange={(event) => setMeetingTitle(event.target.value)} maxLength={200} placeholder="例：週次定例" disabled={!isTauri() || meetingBusy || writeLocked} /><button type="submit" disabled={!isTauri() || meetingBusy || writeLocked || !meetingTitle.trim()}>会議を作成</button></div>
            </form>
            {!detail && <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} />}
            {restoredFilter && <p>今回復元した会議を表示しています。<button type="button" className="small-secondary" disabled={writeLocked} onClick={() => void withSavedEdits(() => setRestoredFilter(null))}>すべての会議を表示</button></p>}
            <IssueNotice scope="main" type="restore-origin" label="復元由来の確認" message={originError} />
            {shownMeetings.length === 0 ? <p className="empty-meetings">{restoredFilter ? "対象の復元会議は現在の一覧にありません。" : "保存済み会議はありません。"}</p> : <ul className="meeting-list">{shownMeetings.map((meeting) => <li key={meeting.id}><div><strong>{meeting.title}</strong>{restoredIds.includes(meeting.id) && <small className="restore-badge">バックアップから復元</small>}<span>{meeting.lifecycle === "imported" ? "取り込み済み" : meeting.lifecycle === "recorded" ? "録音済み" : meeting.lifecycle === "recording" ? "録音処理中" : "下書き"} · {new Date(meeting.createdAt).toLocaleString("ja-JP")}</span></div><div className="analysis-actions"><button type="button" className="small-secondary" onClick={() => selectMeeting(meeting.id)} disabled={writeLocked}>開く</button>{["recorded", "imported"].includes(meeting.lifecycle) && <button type="button" className="small-secondary deletion-trigger" aria-label={`${meeting.title}の音声だけを削除`} onClick={() => void audioDeletion.reviewNew(meeting.id)} disabled={!audioDeletion.canReviewNew}>音声だけを削除</button>}<button type="button" className="small-secondary deletion-trigger" aria-label={`${meeting.title}を削除`} onClick={() => void deletion.reviewMeeting(meeting.id)} disabled={!deletion.canReviewMeeting || meeting.lifecycle === "recording"}>削除</button>{dataOperation.deletionListing?.deletingMeetingIds.includes(meeting.id) && <span role="status">削除中・要確認</span>}</div></li>)}</ul>}
            {selectedMeeting && <form className="rename-form" onSubmit={renameMeeting}>
              <h3>会議情報</h3><label htmlFor="rename-title">会議名を変更</label>
              <div className="meeting-form-row"><input id="rename-title" value={renameTitle} maxLength={200} onChange={(event) => setRenameTitle(event.target.value)} disabled={meetingBusy || writeLocked} /><button type="submit" disabled={meetingBusy || writeLocked || !renameTitle.trim()}>保存</button><button type="button" className="small-secondary" onClick={() => void withSavedEdits(() => setSelectedMeeting(null))} disabled={writeLocked}>閉じる</button></div>
              <p>{selectedMeeting.sourceType === "audio_import" && selectedMeeting.lifecycle === "draft" ? "音声取り込みが未完了です。保存状況を確認してから取り込み直してください。" : selectedMeeting.lifecycle === "draft" ? "この会議を選択して録音を開始できます。" : ["recorded","imported"].includes(selectedMeeting.lifecycle) ? "録音・取り込みは完了しています。音声削除後は再生・解析・再解析できません。採用した文字起こしから議事録を生成できます。" : "録音状態を確認してください。"}</p>
            </form>}
          </section>
          {selectedMeeting && <AnalysisPanel key={selectedMeeting.id} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} recorded={["recorded","imported"].includes(selectedMeeting.lifecycle)} recordingActive={state.kind !== "ready" || state.snapshot.phase !== "idle"} locked={writeLocked} withSavedEdits={withSavedEdits} diarizationHost={diarizationHost?.meetingId === selectedMeeting.id ? diarizationHost.node : null} onHeadChanged={async () => {
            setAnalysisEpoch((value) => value + 1);
            const [meeting, items] = await Promise.all([invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }), invoke<Meeting[]>("list_meetings")]);
            setSelectedMeeting(meeting); setMeetings(items);
          }} />}
          </div></details>
          {selectedMeeting && <div className="workspace-tab-panel" id="meeting-panel-transcript" role="tabpanel" aria-labelledby="meeting-tab-transcript" hidden={detailTab !== "transcript"}>
            <TranscriptEditor key={`${selectedMeeting.id}:${analysisEpoch}`} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} locked={writeLocked} playbackHost={playbackHost} visible={detailTab === "transcript"} onDiarizationHost={bindDiarizationHost} />
          </div>}
          {selectedMeeting && <div className="workspace-tab-panel" id="meeting-panel-minutes" role="tabpanel" aria-labelledby="meeting-tab-minutes" hidden={detailTab !== "minutes"}><MinutesPanel key={`minutes:${selectedMeeting.id}`} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} sourceEpoch={analysisEpoch} recordingActive={state.kind !== "ready" || state.snapshot.phase !== "idle"} locked={writeLocked} withSavedEdits={withSavedEdits} visible={detailTab === "minutes"} onHeadChanged={async () => {
            const [meeting, items] = await Promise.all([invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }), invoke<Meeting[]>("list_meetings")]);
            setSelectedMeeting(meeting); setMeetings(items);
          }} /></div>}
          {!detail && <div className="next-grid">
            <section><span className="step-number">01</span><h2>保存基盤</h2><p>会議・録音・候補版と、版ごとの編集を端末内に保持します。</p><span className="soon">一部実装</span></section>
            <section><span className="step-number">02</span><h2>録音をつなぐ</h2><p>macOSでマイクとシステム音声を別々に保存します。</p><span className="soon">実機検証待ち</span></section>
            <section><span className="step-number">03</span><h2>内容を整える</h2><p>文字起こし・話者分離・議事録の候補を確認・採用し、本文・話者を修正できます。macOSのMarkdown出力にも対応します。</p><span className="soon">一部実装</span></section>
          </div>}
          </>}
        </main>
        <div ref={setPlaybackHost} className="meeting-playback-dock" role="region" aria-label="会議の共通音声プレーヤー" hidden={!selectedMeeting || screen === "settings"} />
      </div>
    </div>
    {selectedMeeting && screen !== "settings" && ["recorded", "imported", "archived"].includes(selectedMeeting.lifecycle) && selectedMeeting.sourceType && <MeetingAudioControls key={selectedMeeting.id} meetingId={selectedMeeting.id} sourceType={selectedMeeting.sourceType} locked={writeLocked} host={playbackHost} />}
    </MeetingPlaybackSurface>
  );
}

createRoot(document.getElementById("root")!).render(<ThemeProvider>{isRecordingConfirmation ? <RecordingConfirmationWindow /> : isRecorderPanel ? <RecorderPanel /> : <><MainWindow /><ConfirmationHost /></>}</ThemeProvider>);
