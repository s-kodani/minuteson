import { VoicePeopleSettings } from "./voice";
import { MinutesPromptSettings } from "./minutes-prompt-settings";
import { InternalLogs } from "./internal-logs";
import appIconUrl from "../src-tauri/icons/icon.svg";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { InfoPopover } from "./info-popover";
import { OptionsModal } from "./options-modal";
import { SettingsModalScope } from "./settings-modal-scope";
import { useAnalysisPublication } from "./analysis-publication";
import { ActionIcon, IconButton } from "./action-icon";
import { RowMenu } from "./row-menu";
import { NotificationPopover } from "./notification-popover";
import { ScreenHistory } from "./screen-history";
import { CommonModal } from "./common-modal";
import type { ModalReplacement } from "./common-modal";
import { ConfirmationHost } from "./confirmation-host";
import { IssueNotice, NotificationList, ToastHost, ToastNotice, toasts } from "./toast";
import { JobNotifications } from "./job-notifications";
import { RecorderInfoTrigger, RecorderInfoWindow } from "./recorder-info";
import { RecorderOperationProvider, RecorderOperationNotifications, useRecorderOperationPublisher, useRecorderOperations } from "./recorder-operation-notices";
import { StartupAttentionWindow, StartupGate } from "./startup-attention";
import {ArchivedInputNotices} from "./archived-input-notices";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useLayoutEffect, useRef, useState, type FormEvent, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { formatElapsed, useRecordingSession, type BridgeState } from "./session";
import { meetingStatusLabel } from "./meeting-status";
import { TranscriptEditor } from "./transcript";
import { AnalysisPanel } from "./analysis";
import { MinutesPanel } from "./minutes";
import { VocabularyEditor } from "./vocabulary";
import { flushPendingEdits } from "./draft-save";
import { runAfterSaving, type SavedActionPhase } from "./saved-action";
import { AudioImportPanel } from "./audio-import";
import { ModelInspectionPanel } from "./model-inspection";
import { CommonOperationProgress } from "./operation-progress";
import { JobCount, JobList } from "./job-list";
import { useDataOperation } from "./data-operation";
import { DataSavePanel } from "./data-save";
import { MeetingDeletionPanel, useMeetingDeletion } from "./meeting-deletion";
import { AudioDeletionPanel, useAudioDeletion } from "./audio-deletion";
import type { DataSnapshot } from "./data-operation-state";
import type { ImportSnapshot } from "./import-state";
import type { RecordingSnapshot } from "./session";
import "./style.css";
import "./prototype-alignment.css";
import "./recorder-panel.css";
import { ThemeProvider, ThemeSettingsPanel, ThemeReadNotice, ThemeNotifications, useTheme } from "./theme";
import { RecordingConfirmationWindow, useRecordingConfirmation } from "./recording-confirmation";
import { confirmations, useActionConfirmation } from "./confirmation-service";
import { SavedEditsCancelled, withConfirmedEdits } from "./confirmed-action";
import { assertRecordingDecision } from "./recording-confirmation-state";
import { waitForDataStop } from "./data-stop";
import { appShortcut, overlayOpen } from "./keyboard";
import { MeetingPlaybackSurface } from "./playback-surface";
import { MeetingAudioControls, MeetingAudioProvider } from "./meeting-audio";
import {ActiveMicrophoneSelection,CaptureInputSelection,useCaptureInputs} from "./capture-inputs";
import {RecordingWorkspace} from "./recording-workspace";
import {CaptureTestPanel,useCaptureTest} from "./capture-test";
import {RecordingInputProvider,RecordingInputNotice,RecordingCompactInputs,RecordingJournalNotice,RecordingJournalCompactStatus} from "./recording-input-status";
import {waitForCaptureTestStop} from "./capture-test-client";
import {stopAllPlayback} from "./playback-barrier";
import {newRecordingPreparation,sameRecordingPreparation,validRecordingTitle,type RecordingPreparation} from "./recording-preparation-state";
import {checkedCaptureInputs,usableAutomaticMicrophone,usableMicrophone} from "./capture-input-state";
import {useRecordingMicrophoneSelection} from "./microphone-selection-status";
import {useCaptureScope} from "./capture-applications";
import {captureApplicationScope,checkedCaptureApplications,checkedCaptureScope} from "./capture-scope-state";
import { assertCloseContext, assertCloseImport, assertCloseRecording, assertCloseTargets, closeMessage, closeTarget, dataNeedsConsent, importNeedsConsent, recordingActive, waitForImportStop, waitForRecordingSave, type CloseActivity, type CloseContext } from "./application-close";

const isRecorderPanel = new URLSearchParams(window.location.search).get("window") === "recorder";
const isRecordingConfirmation = new URLSearchParams(window.location.search).get("window") === "recording-confirmation";
const isRecorderInfo = new URLSearchParams(window.location.search).get("window") === "recorder-info";
const isStartupAttention = new URLSearchParams(window.location.search).get("window") === "startup-attention";

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
  const [resolved, setResolved] = useState(false);
  const run = async (command: string) => {
    setBusy(true);
    setResolved(false);
    setError("");
    try {
      await invoke(command);
      setResolved(true);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  return { run, error, busy, resolved };
}

function RecorderPanel() {
  const state = useRecordingSession();
  const decision = useRecordingConfirmation("recorder");
  const action = useWindowAction();
  const closeAction = useWindowAction();
  const operations = useRecorderOperations(), theme = useTheme();
  const actionRelay = useRecorderOperationPublisher("window_action", action.error, !action.busy, action.resolved);
  const decisionRelay = useRecorderOperationPublisher("confirmation", decision.error, !decision.requestBusy, decision.requestResolved && decision.connected);
  const themeRelay = useRecorderOperationPublisher("theme_read", theme.state.error, !theme.state.busy, !!theme.state.snapshot && !theme.state.error);
  const sessionError = state.kind === "error" ? state.message : state.kind === "ready" ? state.snapshot.errorMessage ?? "" : "";
  const sessionRelay = useRecorderOperationPublisher("session_read", sessionError, true, state.kind === "ready");
  const localProblem = [action.error, closeAction.error, decision.error, theme.state.error, sessionError, actionRelay, decisionRelay, themeRelay, sessionRelay, operations.error].filter(Boolean).join("\n");
  const idle = state.kind === "ready" && state.snapshot.phase === "idle";
  return (
    <main className="mini-shell" aria-label="録音ミニパネル">
      <div className="mini-row">
      <span className="mini-drag" data-tauri-drag-region title="ドラッグで移動" aria-hidden="true">⠿</span>
      <div className="mini-summary">
      <div className="mini-top">
        <span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} />
        <strong>{phaseLabel(state)}</strong>
        <span className="timer">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</span>
      </div>
        {state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)&&<RecordingCompactInputs owner={state.snapshot.sessionId}/>}
        <div className="mini-save-diagnostics">{state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)?<RecordingJournalCompactStatus owner={state.snapshot.sessionId}/>:<span className="mini-note">{idle ? "待機中" : "録音状態はメイン画面と共通"}</span>}</div>
      </div>
      <div className="mini-bottom">
        {state.kind === "ready" && (state.snapshot.phase === "recording" || state.snapshot.phase === "preparing") && <IconButton label={state.snapshot.phase === "preparing" ? "中止" : "停止"} icon="stop" className="mini-action-icon mini-stop" onClick={event => void decision.request(false,event.currentTarget)} disabled={action.busy || decision.requestBusy || decision.visible || !decision.connected} />}
        <IconButton label="メインを開く" icon="open" className="mini-action-icon mini-open" onClick={() => void action.run("open_main_window")} disabled={action.busy || !isTauri()} />
        <RecorderInfoTrigger disabled={decision.visible} problem={localProblem} />
      </div>
      </div>
      <IconButton label="ミニパネルを閉じる" icon="close" className="text-button mini-close" onClick={() => closeAction.run("hide_recorder_panel")} disabled={closeAction.busy || !isTauri()} />

    </main>
  );
}

function MainWindow() {
  const state = useRecordingSession();
  const recordingDecision = useRecordingConfirmation("main");
  const dataOperation = useDataOperation();
  const [dataPending, setDataPending] = useState(false);
  const [audioDeletionPending, setAudioDeletionPending] = useState(false);
  const [restoredIds, setRestoredIds] = useState<string[]>([]);
  const [restoredFilter, setRestoredFilter] = useState<string[] | null>(null);
  const [originError, setOriginError] = useState("");
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [meetingQuery, setMeetingQuery] = useState("");
  const [meetingError, setMeetingError] = useState("");
  const [meetingBusy, setMeetingBusy] = useState(false);
  const [selectedMeeting, setSelectedMeeting] = useState<Meeting | null>(null);
  const [preparation,setPreparation]=useState<RecordingPreparation|null>(null);
  const [screen, setScreen] = useState<"overview" | "meetings" | "settings" | "recording" | "jobs" | "logs" | "voices">("meetings");
  const [detailTab, setDetailTab] = useState<"transcript" | "minutes">("transcript");
  const [history] = useState(() => new ScreenHistory<{ screen: typeof screen; meeting: Meeting | null; preparation: RecordingPreparation | null; filter: string[] | null }>());
  const [, updateHistory] = useState(0);
  const routeKey = `${screen}:${screen === "overview" || screen === "recording" ? selectedMeeting?.id ?? preparation?.id ?? "" : ""}`;
  useLayoutEffect(() => {
    if (history.visit(routeKey, { screen, meeting: selectedMeeting, preparation, filter: restoredFilter })) updateHistory(value => value + 1);
  });
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  const [meetingToolsOpen, setMeetingToolsOpen] = useState(false);
  const [meetingActionsHost, setMeetingActionsHost] = useState<HTMLDivElement | null>(null);
  useLayoutEffect(() => { setDetailTab("transcript"); }, [selectedMeeting?.id]);
  const [playbackHost,setPlaybackHost] = useState<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    const column = playbackHost?.parentElement,header = column?.querySelector<HTMLElement>(".topbar");
    if(!playbackHost || !column || !header) return;
    const measure = () => {
      column.style.setProperty("--playback-dock-height",`${Math.ceil(playbackHost.getBoundingClientRect().height)}px`);
      const navigation = column.querySelector<HTMLElement>(".meeting-detail-navigation");
      const heading = column.querySelector<HTMLElement>(".meeting-detail-status");
      column.style.setProperty("--workspace-header-height",`${Math.ceil(header.getBoundingClientRect().height + (heading?.getBoundingClientRect().height ?? 0) + (navigation?.getBoundingClientRect().height ?? 0))}px`);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(playbackHost); observer.observe(header);
    for (const element of column.querySelectorAll<HTMLElement>(".meeting-detail-status,.meeting-detail-navigation")) observer.observe(element);
    measure();
    return () => { observer.disconnect(); column.style.removeProperty("--playback-dock-height"); column.style.removeProperty("--workspace-header-height"); };
  },[playbackHost,selectedMeeting?.id]);
  const [renameTitle, setRenameTitle] = useState("");
  const [renameOpen, setRenameOpen] = useState(false);
  const renameOpener = useRef<HTMLButtonElement>(null);
  const [captureAvailable, setCaptureAvailable] = useState(false);
  const [captureBusy, setCaptureBusy] = useState(false);
  const [microphoneRequest,setMicrophoneRequest]=useState<{owner:string;revision:number}|null>(null);
  const [captureError, setCaptureError] = useState("");
  const captureOwner=screen==="recording"&&preparation?preparation.id:selectedMeeting?.id??null;
  const captureEnabled=captureAvailable&&(screen==="recording"&&!!preparation&&!preparation.submitted||selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null)&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase);
  const activeMicrophoneInput=screen==="recording"&&state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)&&state.snapshot.meetingId===selectedMeeting?.id;
  const captureInputs=useCaptureInputs(captureEnabled||activeMicrophoneInput,captureOwner);
  const microphoneSession=state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)?state.snapshot.sessionId:null;
  const microphoneSelection=useRecordingMicrophoneSelection(microphoneSession);
  useEffect(()=>{
    if(microphoneRequest&&(microphoneRequest.owner!==microphoneSession||microphoneSelection?.requestRevision!==undefined&&microphoneSelection.requestRevision>=microphoneRequest.revision))setMicrophoneRequest(null);
  },[microphoneSession,microphoneRequest,microphoneSelection?.requestRevision]);
  useEffect(()=>{
    if(!activeMicrophoneInput||!microphoneSelection||!captureInputs.inputs||microphoneRequest?.owner===microphoneSession&&microphoneSelection.requestRevision<microphoneRequest.revision)return;
    if(microphoneSelection.mode==="automatic"&&captureInputs.mode!=="automatic")captureInputs.select("");
    if(microphoneSelection.mode==="fixed"&&microphoneSelection.source!=="pending"&&captureInputs.inputs.devices.some(device=>device.uid===microphoneSelection.deviceUID)&&(captureInputs.mode!=="fixed"||captureInputs.uid!==microphoneSelection.deviceUID))captureInputs.select(microphoneSelection.deviceUID);
  },[activeMicrophoneInput,microphoneSelection?.revision,captureInputs.inputs,captureInputs.mode,captureInputs.uid,microphoneRequest,microphoneSession]);
  const captureScope=useCaptureScope(captureEnabled,captureOwner);
  const captureTest=useCaptureTest();
  const [closingBusy, setClosingBusy] = useState(false);
  const closing = useRef(false), starting = useRef(false);
  const [recoveredCount, setRecoveredCount] = useState(0);
  const [navigationPhase, setNavigationPhase] = useState<SavedActionPhase>("idle");
  const navigationBusy = navigationPhase !== "idle";
  const loadOrigins = async () => {
    try { setRestoredIds(await invoke<string[]>("list_restored_meeting_ids")); setOriginError(""); }
    catch (cause) { setOriginError(`復元由来を確認できません: ${String(cause)}`); }
  };
  const [analysisEpoch, setAnalysisEpoch] = useState(0);
  const [transcriptMeeting, setTranscriptMeeting] = useState<string | null>(null);
  const navigating = useRef(false);
  const priorPhase = useRef<string | null>(null);
  const withSavedEdits = async (action: () => void | Promise<void>, reason: "leave" | "tab" | "transcription" = "leave") => {
    if (navigating.current) return;
    navigating.current = true;
    setMeetingError("");
    try {
      const reportPhase = (phase: SavedActionPhase) => {
        // Publication must render the brief edit lock before taking the final
        // input snapshot, not one paint after the native head has changed.
        if (reason === "transcription") flushSync(() => setNavigationPhase(phase));
        else setNavigationPhase(phase);
      };
      await runAfterSaving(() => flushPendingEdits(reason), action, reportPhase);
    } catch (cause) {
      if (cause instanceof SavedEditsCancelled) return "cancelled" as const;
      setMeetingError(`操作を完了できませんでした。編集内容はこの画面に保持しています: ${String(cause)}`);
    } finally {
      navigating.current = false;
    }
  };
  const deletion = useMeetingDeletion({ operation: dataOperation, locked: captureTest.blocked || navigationBusy || dataPending || audioDeletionPending, withSavedEdits,
    onDeleted: async receipt => {
      setSelectedMeeting(current => current?.id === receipt.meetingId ? null : current);
      setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins();
    } });
  const audioDeletion = useAudioDeletion({ operation: dataOperation, locked: captureTest.blocked || navigationBusy || dataPending || deletion.protectedNow, withSavedEdits,
    onProtectionChange: setAudioDeletionPending, onDeleted: async () => {
      // Audio availability changes, not document heads. Keep mounted editors and
      // their local buffers, including when recovery deliberately skipped writes.
      setMeetings(await invoke<Meeting[]>("list_meetings"));
    } });
  const operationNavigationLocked = recordingDecision.visible || recordingDecision.requestBusy || navigationBusy || dataPending || deletion.protectedNow || audioDeletion.protectedNow || !dataOperation.connected || !!dataOperation.snapshot?.busy;
  const baseNavigationLocked=operationNavigationLocked||captureTest.blocked;
  const deletionBlocked = !!dataOperation.deletionListing?.writeBlocked || !!dataOperation.snapshot?.deletionPreview?.pending || !!dataOperation.modelDeletionListing?.writeBlocked || !!dataOperation.audioDeletionListing?.writeBlocked || !!dataOperation.snapshot?.audioDeletionPreview?.pending;
  const baseWriteLocked = baseNavigationLocked || deletionBlocked || !!dataOperation.snapshot?.restartRequired;
  const operationWriteLocked=operationNavigationLocked||deletionBlocked||!!dataOperation.snapshot?.restartRequired;
  const startConfirm = useActionConfirmation(baseWriteLocked || closingBusy);
  const closeConfirm = useActionConfirmation(false);
  const navigationLocked = baseNavigationLocked || captureBusy || closingBusy || startConfirm.pending;
  const writeLocked = baseWriteLocked || captureBusy || closingBusy || startConfirm.pending;
  // Navigation saves temporarily lock writes, but must not cancel background
  // model preparation. Recording, maintenance and shutdown still stop it.
  const modelPreparationStopRequested = recordingDecision.visible || recordingDecision.requestBusy || dataPending || deletion.protectedNow || audioDeletion.protectedNow || !dataOperation.connected || !!dataOperation.snapshot?.busy || captureTest.blocked || deletionBlocked || !!dataOperation.snapshot?.restartRequired || captureBusy || closingBusy || startConfirm.pending;
  const publication = useAnalysisPublication(writeLocked, action => withSavedEdits(action, "transcription"));
  const root = useRef({ selectedMeeting, preparation, screen, baseWriteLocked, captureAvailable, captureInputs, captureScope, captureTest, state });
  root.current = { selectedMeeting, preparation, screen, baseWriteLocked, captureAvailable, captureInputs, captureScope, captureTest, state };
  const goBack = async () => {
    const target = history.previous, from = history.current;
    if (!target || navigationLocked) return;
    await withSavedEdits(async () => {
      const route = target.value;
      const owner = route.meeting?.id ?? (route.preparation?.submitted ? route.preparation.id : null);
      const meeting = owner ? await invoke<Meeting | null>("get_meeting", { id: owner }) : route.meeting;
      if (owner && meeting?.id !== owner) throw Error("戻る先の会議がありません。会議一覧から選び直してください。");
      if (history.current?.key !== from?.key || !history.restore(target)) throw Error("画面が変更されています。戻る操作をやり直してください。");
      // Refresh persisted data, but retain local preparation fields. Returning
      // never creates a meeting, starts capture or replays settings operations.
      setSelectedMeeting(meeting); setRenameTitle(meeting?.title ?? "");
      setPreparation(owner ? null : route.preparation); setRestoredFilter(route.filter);
      setMeetingToolsOpen(false);
      setScreen(route.screen);
      updateHistory(value => value + 1);
    });
  };
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
      const test=await captureTest.client.refresh();
      if(test){
        const outcome=await closeConfirm({title:"録音前テストを閉じて終了しますか？",target:"録音前の5秒テスト",confirmLabel:"テストを閉じて終了へ進む",
          message:"進行中なら取得プロセスの停止完了を待ち、再生を止めて一時テスト音声を破棄します。会議の保存音声は変更しません。停止や解放が未確認の場合は終了しません。",
          action:async()=>{
            await stopAllPlayback();
            const current=await captureTest.client.refresh();
            if(!current||current.runId!==test.runId)throw Error("終了するテストが変更されています。");
            if(current.busy){await captureTest.client.cancel(test.runId);await waitForCaptureTestStop(test.runId,()=>captureTest.client.refresh());}
            await captureTest.client.discard(test.runId);
          }});
        if(outcome!=="completed")return;
      }
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
    } catch (cause) { if (!(cause instanceof SavedEditsCancelled)) setMeetingError(`終了できませんでした。アプリと保存結果は保持しています: ${String(cause)}`); }
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
    let stopCleanup: (() => void) | undefined;
    void listen("background-cleanup-finished", () => {
      if (!active) return;
      void invoke<Meeting[]>("list_meetings").then(items => { if (active) setMeetings(items); }).catch(error => { if (active) setMeetingError(String(error)); });
      if (selectedMeeting) void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then(item => { if (active) setSelectedMeeting(item); }).catch(error => { if (active) setMeetingError(String(error)); });
    }).then(stop => { if (active) stopCleanup = stop; else stop(); }).catch(error => { if (active) setMeetingError(String(error)); });
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
    return () => { active = false; unlisten?.(); stopCleanup?.(); };
  }, [selectedMeeting?.id]);
  useEffect(() => {
    const phase = state.kind === "ready" ? state.snapshot.phase : null;
    if (priorPhase.current && priorPhase.current !== phase && (phase === "idle" || phase === "error") && isTauri()) {
      void invoke<Meeting[]>("list_meetings").then(setMeetings).catch((error: unknown) => setMeetingError(String(error)));
      if (selectedMeeting) {
        const owner=selectedMeeting.id;
        void invoke<Meeting | null>("get_meeting", { id: owner }).then(meeting=>{
          if(root.current.selectedMeeting?.id!==owner)return;
          setSelectedMeeting(meeting);
          if(phase==="idle"&&meeting?.lifecycle==="recorded"&&root.current.screen==="recording")setScreen("overview");
        }).catch((error: unknown) => setMeetingError(String(error)));
      }
    }
    priorPhase.current = phase;
  }, [state, selectedMeeting]);
  const startRecording = async () => {
    const local=screen==="recording"?preparation:null;
    if ((!selectedMeeting&&!local) || local&&(local.submitted||!validRecordingTitle(local.title)) || !captureInputs.selected || !captureScope.scope || starting.current || closing.current || root.current.baseWriteLocked) return;
    const owner=local?.id??selectedMeeting!.id;
    const microphoneMode=captureInputs.mode;
    const microphoneUID=microphoneMode==="fixed"?captureInputs.uid:null;
    const scope=checkedCaptureScope(captureScope.scope,captureScope.applications);
    const scopeKey=JSON.stringify(scope);
    const scopeUnchanged=()=>JSON.stringify(root.current.captureScope.scope)===scopeKey;
    starting.current = true; setCaptureBusy(true); setCaptureError("");
    try {
      const saved = await invoke<Meeting | null>("get_meeting", { id: owner });
      if(local&&saved)throw Error("録音準備の会議は既に保存されています。受付結果を確認してください。");
      const expected:Meeting|null=local?{id:local.id,title:local.title.trim(),lifecycle:"draft",sourceType:null,lockVersion:0,createdAt:0,updatedAt:0}:saved;
      const ownerUnchanged=()=>local?sameRecordingPreparation(root.current.preparation,local)&&root.current.screen==="recording":root.current.selectedMeeting?.id===owner;
      const importing = await invoke<ImportSnapshot>("get_audio_import"); assertCloseImport(importing);
      const expectedTest=await captureTest.client.refresh();
      if(expectedTest?.busy)throw Error("録音前テストの停止・検証が終わってから録音を開始してください。");
      const currentInputList=checkedCaptureInputs(await invoke("get_capture_inputs"));
      const microphone=microphoneUID===null?usableAutomaticMicrophone(currentInputList):usableMicrophone(currentInputList,microphoneUID);
      if(!microphone)throw Error("マイクまたは権限を再確認してください。録音は開始していません。");
      const applications=scope.mode==="application"?checkedCaptureApplications(await invoke("get_capture_applications")):null;
      checkedCaptureScope(scope,applications);
      const application=captureApplicationScope(scope,applications);
      const countsMessage=local?`\n録音後の話者数: マイク ${local.speakerCounts.mic===null?"自動":local.speakerCounts.mic+"人"}、PC音声 ${local.speakerCounts.system===null?"自動":local.speakerCounts.system+"人"}。`:"";
      const scopeMessage=scope.mode==="system"?"PC音声は全体取得し、通知音や他の動画の音も含まれる場合があります。":`PC音声は選択したアプリ（${application?.name}）を取得します。ブラウザでは他のタブの音も含まれる場合があり、特定タブだけの取得は保証しません。終了・再起動時は全体取得や別アプリに切り替えません。`;
      if (!expected || expected.lifecycle !== "draft" || expected.sourceType != null || !ownerUnchanged()) throw Error("録音対象の会議を再確認してください。");
      await startConfirm({ title: "選択した会議の録音を開始しますか？", target: expected.title, confirmLabel: "録音を開始する", message: "マイクとPCの再生音が端末内に保存されます。参加者の同意と録音権限を確認してください。実行中の解析は安全な区切りで一時停止し、録音終了後に再開します。"+`\n録音するマイク: ${microphoneUID===null?"自動（会議アプリの使用マイクを優先し、不明なときはOSの既定入力）":microphone.name}。${scopeMessage}` + countsMessage + (expectedTest ? "\n前のテスト音声の再生を停止し、一時音声を破棄してから録音を開始します。保存済み会議の音声は変更しません。" : "") + (importNeedsConsent(importing) ? `\n取り込み（${importing.fileName ?? "選択・確認中の音声"}）を安全に取り消して録音を優先します。取り込みの確認内容と未確定物だけを整理し、元ファイルと保存済み会議を保持します。` : ""), validate: () => ownerUnchanged() && root.current.captureAvailable && root.current.captureInputs.mode===microphoneMode&&(microphoneUID===null||root.current.captureInputs.uid===microphoneUID)&&!!root.current.captureInputs.selected && scopeUnchanged() && !root.current.baseWriteLocked && root.current.state.kind === "ready" && ["idle", "error"].includes(root.current.state.snapshot.phase), action: () => withConfirmedEdits(withSavedEdits, async () => {
        const [meeting, recording, currentImport, currentInputs, currentApplications] = await Promise.all([invoke<Meeting | null>("get_meeting", { id: expected.id }), invoke<RecordingSnapshot>("get_recording_snapshot"), invoke<ImportSnapshot>("get_audio_import"),invoke("get_capture_inputs").then(checkedCaptureInputs),scope.mode==="application"?invoke("get_capture_applications").then(checkedCaptureApplications):Promise.resolve(null)]);
        assertCloseRecording(recording);
        assertCloseImport(currentImport);
        if(!(microphoneUID===null?usableAutomaticMicrophone(currentInputs):usableMicrophone(currentInputs,microphoneUID))||root.current.captureInputs.mode!==microphoneMode||microphoneUID!==null&&root.current.captureInputs.uid!==microphoneUID)throw Error("確認したマイクまたは権限が変更されました。録音は開始していません。");
        checkedCaptureScope(scope,currentApplications);
        if(!scopeUnchanged())throw Error("PC音声の取得範囲が変更されました。録音は開始していません。");
        const meetingUnchanged=local?meeting===null:!!meeting&&meeting.id===expected.id&&meeting.title===expected.title&&meeting.lockVersion===expected.lockVersion&&meeting.lifecycle==="draft"&&meeting.sourceType==null;
        if (!meetingUnchanged || !ownerUnchanged() || recordingActive(recording) || currentImport.token !== importing.token || !importing.busy && currentImport.revision !== importing.revision) throw Error("録音対象が確認後に変更されました。録音は開始していません。");
        await stopAllPlayback();
        const currentTest=await captureTest.client.refresh();
        if((currentTest?.runId??null)!==(expectedTest?.runId??null)||currentTest?.busy)throw Error("録音前テストの対象・状態が変更されました。録音は開始していません。");
        if(currentTest)await captureTest.client.discard(currentTest.runId);
        if(local)setPreparation(current=>current?.id===local.id?{...current,submitted:true}:current);
        await invoke("start_recording", { meetingId: expected.id, microphoneUid:microphoneUID, captureScope:scope, newTitle:local?expected.title:null, speakerCounts:local?.speakerCounts??null });
        const accepted=await invoke<Meeting | null>("get_meeting", { id: expected.id });
        if(!accepted||accepted.id!==expected.id)throw Error("録音の会議保存結果を確認できません。開始は再送していません。");
        setSelectedMeeting(accepted);setRenameTitle(accepted.title);setPreparation(null);setScreen("recording");
        setMeetings(await invoke<Meeting[]>("list_meetings"));
      }), onError: cause => setCaptureError(`録音の受付状態を確認してください。開始は自動再送していません: ${String(cause)}`) });
    } catch (error) {
      setCaptureError(String(error));
    } finally {
      starting.current = false;
      setCaptureBusy(false);
    }
  };
  const selectRecordingMicrophone=async(value:string)=>{
    const snapshot=state.kind==="ready"?state.snapshot:null;
    const sessionId=snapshot?.sessionId;
    if(!sessionId||!["preparing","recording"].includes(snapshot.phase)||captureBusy||!captureInputs.inputs)return;
    if(value!==""&&!captureInputs.inputs.devices.some(device=>device.uid===value))return;
    setCaptureBusy(true);
    try{
      const current=await invoke<RecordingSnapshot>("get_recording_snapshot");
      if(current.sessionId!==sessionId||!["preparing","recording"].includes(current.phase))throw Error("録音が切り替わったか、停止しました。");
      const revision=await invoke<number>("select_recording_microphone",{sessionId,microphoneUid:value===""?null:value});
      setMicrophoneRequest({owner:sessionId,revision});
      if(root.current.state.kind==="ready"&&root.current.state.snapshot.sessionId===sessionId)captureInputs.select(value);
    }catch(error){
      toasts.show({key:`recording-microphone-selection-${sessionId}`,level:"error",message:`マイクを切り替えられませんでした: ${String(error)}`});
    }finally{setCaptureBusy(false);}
  };
  const startCaptureTest=async()=>{
    if(!captureOwner||!captureInputs.selected||!captureScope.scope||root.current.baseWriteLocked||starting.current||closing.current)return;
    const owner=captureOwner,uid=captureInputs.uid,scope=checkedCaptureScope(captureScope.scope,captureScope.applications),scopeKey=JSON.stringify(scope);
    const sameInputs=()=>root.current.screen==="recording"&&(root.current.preparation?.id??root.current.selectedMeeting?.id)===owner&&root.current.captureInputs.uid===uid&&JSON.stringify(root.current.captureScope.scope)===scopeKey;
    starting.current=true;setCaptureBusy(true);setCaptureError("");
    try{
      const previous=await captureTest.client.refresh();
      if(previous?.busy)throw Error("前のテストの停止・検証を待ってください。");
      const importing=await invoke<ImportSnapshot>("get_audio_import");assertCloseImport(importing);
      const microphone=usableMicrophone(checkedCaptureInputs(await invoke("get_capture_inputs")),uid);
      if(!microphone)throw Error("選択したマイクまたは権限を再確認してください。");
      const apps=scope.mode==="application"?checkedCaptureApplications(await invoke("get_capture_applications")):null;
      checkedCaptureScope(scope,apps);
      const app=captureApplicationScope(scope,apps);
      await startConfirm({title:"5秒の録音前テストを開始しますか？",target:`${microphone.name} ／ ${scope.mode==="system"?"PCの再生音全体":app?.name}`,confirmLabel:"5秒テストを開始する",
        message:"マイクとPCの音を一時保存します。参加者の同意と録音権限を確認してください。入力の初期化が終わってから5秒録音し、停止後に各音声を再生できます。会議・解析結果は作りません。解析とモデル準備は安全な停止を待ちます。"+
          (captureInputs.mode==="automatic"?"この5秒テストはOSの既定マイクを確認します。通常録音で会議アプリの別のマイクが選ばれた場合、その入力の検証にはなりません。":"")+
          (scope.mode==="system"?"PC音声には通知音や他の動画の音も含まれます。":"選択したアプリを取得します。ブラウザの他のタブの音も含まれる場合があります。終了・再起動時に別アプリへ切り替えません。")+
          (previous?"前のテスト音声は再生を停止して破棄します。":"")+(importNeedsConsent(importing)?"確認した音声取り込みを安全に取り消し、元ファイルと保存済み会議を保持します。":""),
        validate:()=>sameInputs()&&!root.current.baseWriteLocked&&root.current.state.kind==="ready"&&["idle","error"].includes(root.current.state.snapshot.phase),
        action:()=>withConfirmedEdits(withSavedEdits,async()=>{
          const [recording,currentImport,inputs,applications]=await Promise.all([
            invoke<RecordingSnapshot>("get_recording_snapshot"),invoke<ImportSnapshot>("get_audio_import"),
            invoke("get_capture_inputs").then(checkedCaptureInputs),scope.mode==="application"?invoke("get_capture_applications").then(checkedCaptureApplications):Promise.resolve(null),
          ]);
          assertCloseRecording(recording);assertCloseImport(currentImport);checkedCaptureScope(scope,applications);
          if(!sameInputs()||recordingActive(recording)||!usableMicrophone(inputs,uid)||currentImport.token!==importing.token||!importing.busy&&(currentImport.busy||currentImport.revision!==importing.revision))throw Error("確認した入力または取り込み状態が変更されました。テストは開始していません。");
          await stopAllPlayback();
          const current=await captureTest.client.refresh();
          if((current?.runId??null)!==(previous?.runId??null)||current?.busy)throw Error("前のテストの状態が変更されています。");
          if(current)await captureTest.client.discard(current.runId);
          await captureTest.client.start({runId:crypto.randomUUID(),microphoneUid:uid,captureScope:scope,expectedImport:{token:importing.token,revision:importing.revision,busy:importing.busy}});
        }),onError:cause=>setCaptureError(String(cause))});
    }catch(cause){setCaptureError(String(cause));}
    finally{starting.current=false;setCaptureBusy(false);}
  };
  const cancelCaptureTest=async()=>{
    const id=captureTest.snapshot?.runId??captureTest.pending?.runId;if(!id)return;
    try{await captureTest.client.cancel(id);}catch(cause){setCaptureError(String(cause));}
  };
  const discardCaptureTest=async()=>{
    const id=captureTest.snapshot?.runId;if(!id)return;
    try{await stopAllPlayback();await captureTest.client.discard(id);}catch(cause){setCaptureError(String(cause));}
  };
  const stopRecording = async (source?:HTMLElement) => {
    await recordingDecision.request(false,source);
  };
  // The caller has already flushed edits. This only creates local preparation.
  const prepareNewRecording=async()=>{
    const current=await invoke<RecordingSnapshot>("get_recording_snapshot");assertCloseRecording(current);
    if(recordingActive(current))throw Error("録音状態が変更されています。最新のヘッダーから録音画面を開き直してください。");
    setPreparation(newRecordingPreparation(crypto.randomUUID(),new Date()));setSelectedMeeting(null);setCaptureError("");setScreen("recording");
  };
  const confirmPreparationResult=async()=>{
    const expected=preparation;if(!expected?.submitted||navigationLocked)return;
    await withSavedEdits(async()=>{
      const [meeting,recording]=await Promise.all([invoke<Meeting|null>("get_meeting",{id:expected.id}),invoke<RecordingSnapshot>("get_recording_snapshot")]);
      assertCloseRecording(recording);
      if(root.current.preparation?.id!==expected.id||root.current.screen!=="recording")throw Error("録音準備が変更されています。別の会議には切り替えていません。");
      if(!meeting)throw Error("保存済みの会議は確認できません。開始は再送していません。録音状態を確認し、会議一覧から新しく準備してください。");
      if(meeting.id!==expected.id||meeting.title!==expected.title.trim()||meeting.sourceType==="audio_import"||!["draft","recording","recorded"].includes(meeting.lifecycle)||recordingActive(recording)&&recording.meetingId!==expected.id)throw Error("録音の受付内容が一致しません。別の会議や録音は採用していません。");
      setSelectedMeeting(meeting);setRenameTitle(meeting.title);setPreparation(null);setMeetings(await invoke<Meeting[]>("list_meetings"));setCaptureError("");
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
      setNotificationsOpen(false);
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
      setSelectedMeeting(meeting); setRenameTitle(meeting.title); setPreparation(null); setScreen("recording");
      requestAnimationFrame(() => {
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
    }, "tab");
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
      void withSavedEdits(prepareNewRecording);
    }
  };
  useEffect(() => {
    const handler = (event: KeyboardEvent) => keyboardAction.current(event);
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, []);
  const renameMeeting = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!selectedMeeting || !renameOpen || meetingBusy || writeLocked || !renameTitle.trim()) return;
    setMeetingBusy(true);
    setMeetingError("");
    try {
      const updated = await invoke<Meeting>("rename_meeting", { id: selectedMeeting.id, title: renameTitle, expectedLockVersion: selectedMeeting.lockVersion });
      setSelectedMeeting(updated);
      setRenameTitle(updated.title);
      setMeetings(await invoke<Meeting[]>("list_meetings"));
      setRenameOpen(false);
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
  const shownMeetings = meetings.filter(meeting => (!restoredFilter || restoredFilter.includes(meeting.id)) && meeting.title.toLocaleLowerCase("ja-JP").includes(meetingQuery.toLocaleLowerCase("ja-JP")));
  const detail = screen === "overview" && !!selectedMeeting;
  const showJobs = () => { if (!navigationLocked) void withSavedEdits(() => { setMeetingToolsOpen(false); setScreen("jobs"); }); };
  const renderMeetingTools = (analysisTools: ReactNode, review: ModalReplacement | null, persistentViews?: ReactNode) => !detail ? null : <OptionsModal className="meeting-workspace-tools workspace-tools" triggerHost={meetingActionsHost}
    open={meetingToolsOpen || !!review} onOpenChange={setMeetingToolsOpen} label="会議管理・録音・解析操作" triggerLabel="会議操作" icon="more" replacement={review} persistentReplacement={persistentViews}>
    <div className="workspace-tools-body">

      <div className="meeting-analysis-tools">{analysisTools}</div>
    </div>
  </OptionsModal>;
  return (
    <MeetingPlaybackSurface host={playbackHost} meetingId={selectedMeeting?.id ?? null} title={selectedMeeting?.title}>
    <MeetingAudioProvider meetingId={selectedMeeting?.id ?? null} sourceType={selectedMeeting?.sourceType} locked={writeLocked}
      active={!!selectedMeeting && !["settings", "recording", "jobs", "logs", "voices"].includes(screen) && ["recorded", "imported", "archived"].includes(selectedMeeting.lifecycle) && !!selectedMeeting.sourceType}>
    <div className="app-layout">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark" aria-hidden="true"><img src={appIconUrl} alt="" /></span><span className="brand-name">mon</span></div>
        <nav aria-label="メインナビゲーション">
          <button type="button" className="new-recording-nav" aria-label="新しい録音" title="新しい録音" disabled={!isTauri() || writeLocked || state.kind !== "ready" || !["idle","error"].includes(state.snapshot.phase)} onClick={() => void withSavedEdits(prepareNewRecording)}><span className="nav-glyph" aria-hidden="true">＋</span><span className="nav-label">新しい録音</span></button>
          <button type="button" aria-label="会議一覧" title="会議一覧" aria-current={screen === "meetings" || screen === "overview" ? "page" : undefined} className={`nav-item ${screen === "meetings" || screen === "overview" ? "selected" : ""}`} disabled={navigationLocked} onClick={() => void withSavedEdits(() => { setSelectedMeeting(null); setMeetingToolsOpen(false); setRestoredFilter(null); setScreen("meetings"); })}><span className="nav-glyph" aria-hidden="true">▤</span><span className="nav-label">会議一覧</span></button>
          <button type="button" aria-label="ジョブ一覧" title="ジョブ一覧" aria-current={screen === "jobs" ? "page" : undefined} className={`nav-item ${screen === "jobs" ? "selected" : ""}`} disabled={navigationLocked} onClick={showJobs}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="list" /></span><span className="nav-label">ジョブ一覧</span><JobCount /></button>
          <button type="button" aria-label="ボイス一覧" title="ボイス一覧" aria-current={screen === "voices" ? "page" : undefined} className={`nav-item ${screen === "voices" ? "selected" : ""}`} disabled={navigationLocked} onClick={() => void withSavedEdits(() => { setMeetingToolsOpen(false); setScreen("voices"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="voice" /></span><span className="nav-label">ボイス一覧</span></button>
          <button type="button" aria-label="内部ログ" title="内部ログ" aria-current={screen === "logs" ? "page" : undefined} className={`nav-item ${screen === "logs" ? "selected" : ""}`} disabled={navigationLocked} onClick={() => void withSavedEdits(() => { setMeetingToolsOpen(false); setScreen("logs"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="logs" /></span><span className="nav-label">内部ログ</span></button>
          <button type="button" aria-label="設定" title="設定" aria-current={screen === "settings" ? "page" : undefined} className={`nav-item ${screen === "settings" ? "selected" : ""}`} disabled={navigationLocked} onClick={() => void withSavedEdits(() => setScreen("settings"))}><span className="nav-glyph settings-nav-glyph" aria-hidden="true"><ActionIcon name="gear" /></span><span className="nav-label">設定</span></button>
        </nav>
        <div className="sidebar-foot">端末内で会議を記録</div>
      </aside>
      <div className="main-column">
        <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError}/>
        <IssueNotice scope="main" type="recording-command" label="録音の操作" message={captureError}/>
        <IssueNotice scope="main" type="recording-confirmation" label="録音終了の確認" message={recordingDecision.error}/>
        <RecorderOperationNotifications />
        <RecordingInputNotice windowLabel="main"/>
        <RecordingJournalNotice/>
        <ThemeNotifications locked={writeLocked} />
        <IssueNotice scope="main" type="transcription-publication" label="文字起こしの保存" message={publication.error} />
        <ToastNotice scope={state.kind === "ready" ? state.snapshot.sessionId ?? "recording" : "recording"} type="recording-failure" level="error" message={state.kind === "ready" ? state.snapshot.errorMessage : state.kind === "error" ? state.message : ""} />
        <header className="topbar"><nav className="screen-navigation" aria-label="画面の移動">{history.previous && <IconButton label="前の画面へ戻る" icon="previous" disabled={navigationLocked} onClick={() => void goBack()} />}{screen !== "meetings" && <><button type="button" className="breadcrumb-button" disabled={navigationLocked} onClick={() => void withSavedEdits(() => { setSelectedMeeting(null); setMeetingToolsOpen(false); setRestoredFilter(null); setScreen("meetings"); })}>会議一覧</button><span aria-hidden="true">/</span></>}<span aria-current="page">{detail ? "会議詳細" : screen === "recording" ? "録音" : screen === "overview" || screen === "meetings" ? "会議一覧" : screen === "jobs" ? "ジョブ一覧" : screen === "logs" ? "内部ログ" : screen === "voices" ? "ボイス一覧" : "設定"}</span></nav><div className="topbar-actions"><div className="header-recording" aria-label="共通録音状態"><span className="header-status"><span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} />{phaseLabel(state)}{state.kind === "ready" && recordingActive(state.snapshot) && <span className="header-timer">{formatElapsed(state.snapshot.elapsedMs)}</span>}</span>{state.kind === "ready" && recordingActive(state.snapshot) && <button type="button" className="small-secondary" disabled={navigationLocked} onClick={() => void openRecordingMeeting()}>録音画面へ</button>}{state.kind === "ready" && ["preparing", "recording"].includes(state.snapshot.phase) && <button type="button" className="stop-button" disabled={captureBusy || navigationBusy || recordingDecision.requestBusy || recordingDecision.visible || !recordingDecision.connected} onClick={event => void stopRecording(event.currentTarget)}>録音を終了</button>}</div><NotificationPopover open={notificationsOpen} onOpenChange={setNotificationsOpen}><NotificationList visible={notificationsOpen} onOpen={openNotifiedMeeting} locked={navigationLocked}/></NotificationPopover></div><ToastHost docked={detail}/></header>
        <CommonOperationProgress />
        {detail && <div className="meeting-detail-status"><div><div className="meeting-title-row"><h1 title={selectedMeeting!.title}>{selectedMeeting!.title}</h1><IconButton ref={renameOpener} label="会議名を変更" icon="edit" disabled={writeLocked || meetingBusy} onClick={() => { setRenameTitle(selectedMeeting!.title); setRenameOpen(true); }} /></div><p className="lead">作成 {new Date(selectedMeeting!.createdAt).toLocaleString("ja-JP")}</p></div><IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} /></div>}
        {detail && <div className="meeting-detail-navigation"><div className="meeting-detail-tabs" role="tablist" aria-label="会議詳細">
          {(["transcript", "minutes"] as const).map(tab => <button key={tab} type="button" role="tab" id={`meeting-tab-${tab}`} aria-controls={`meeting-panel-${tab}`} aria-selected={detailTab === tab} tabIndex={detailTab === tab ? 0 : -1} disabled={navigationLocked} onClick={() => void switchDetailTab(tab)} onKeyDown={event => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return;
            const target = event.key === "Home" ? "transcript" : event.key === "End" ? "minutes" : ["ArrowLeft", "ArrowRight"].includes(event.key) ? tab === "transcript" ? "minutes" : "transcript" : null;
            if (target) { event.preventDefault(); document.getElementById(`meeting-tab-${target}`)?.focus(); }
          }}>{tab === "transcript" ? "文字起こし" : "議事録"}</button>)}
        </div><div className="meeting-detail-actions" ref={setMeetingActionsHost} /></div>}
        <main className={`content${detail ? " meeting-detail-content" : ""}`} aria-label="会議ワークスペース" tabIndex={-1}>
          <JobNotifications locked={navigationLocked} onOpen={openNotifiedMeeting} />
          <ArchivedInputNotices locked={navigationLocked} paused={!dataOperation.connected||dataPending||!!dataOperation.snapshot?.busy||dataOperation.snapshot?.phase==="ready"||!!dataOperation.snapshot?.restartRequired||deletionBlocked||closingBusy} onOpen={openNotifiedMeeting}/>
          {!detail && (screen === "overview" || screen === "meetings") && <div className="page-heading"><div className="eyebrow">YOUR WORKSPACE</div><h1>会議一覧</h1><p className="lead">会話を記録して、次のアクションにつなげる。</p></div>}
          <AudioImportPanel visible={(screen==="overview" || screen==="meetings") && !detail} showNewRecording={false} locked={writeLocked} recordingActive={state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)} withSavedEdits={withSavedEdits} onNewRecording={prepareNewRecording} onSaved={async(id)=>{
            await flushPendingEdits();
            const [meeting,items]=await Promise.all([invoke<Meeting|null>("get_meeting",{id}),invoke<Meeting[]>("list_meetings")]);
            if(!meeting)throw Error("保存した会議が見つかりません。");
            setSelectedMeeting(meeting);setMeetingToolsOpen(false);setRenameTitle(meeting.title);setMeetings(items);setScreen("overview");
          }}/>
          {screen === "settings" && <><h1>設定</h1><p className="lead">録音環境、ローカルAI、表示を管理します。</p></>}
          {screen === "settings" && <ThemeSettingsPanel compact locked={writeLocked} />}
          <ModelInspectionPanel compact visible={screen === "settings"} locked={writeLocked} stopRequested={modelPreparationStopRequested} recordingActive={state.kind!=="ready" || !["idle","error"].includes(state.snapshot.phase)} />
          <MinutesPromptSettings visible={screen === "settings"} locked={writeLocked} />
          {screen !== "settings" && <ThemeReadNotice />}
          <DataSavePanel operation={dataOperation} visible={screen === "settings"} locked={captureTest.blocked || navigationBusy || deletion.protectedNow || audioDeletion.protectedNow || deletionBlocked} withSavedEdits={withSavedEdits} onProtectionChange={setDataPending} onRestored={async () => { setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins(); }} onShowMeetings={async ids => { await withSavedEdits(async () => { setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins(); setRestoredFilter(ids); setSelectedMeeting(null); setScreen("meetings"); }); }} />
          <MeetingDeletionPanel operation={dataOperation} control={deletion} />
          <AudioDeletionPanel operation={dataOperation} control={audioDeletion} meetingTitle={meetings.find(m => m.id === audioDeletion.preview?.meetingId)?.title} />
          {screen === "settings" ? <><VocabularyEditor locked={writeLocked} /><VoicePeopleSettings locked={writeLocked} /><IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} /></> : screen === "voices" ? <VoicePeopleSettings standalone locked={writeLocked} /> : screen === "logs" ? <InternalLogs /> : screen === "jobs" ? <JobList locked={navigationLocked} onOpenMeeting={openNotifiedMeeting} onOpenModels={() => { if (!navigationLocked) void withSavedEdits(() => setScreen("settings")); }} /> : screen==="recording" ? <>
            <RecordingWorkspace preparation={preparation} title={selectedMeeting?.title??preparation?.title??"録音対象を確認してください"} state={state} owner={captureOwner} locked={writeLocked}
              canStart={isTauri()&&captureAvailable&&!!captureInputs.selected&&!!captureScope.scope&&!writeLocked&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase)&&(preparation?!preparation.submitted&&validRecordingTitle(preparation.title):selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null)}
              canStop={!captureBusy&&!navigationBusy&&!recordingDecision.requestBusy&&!recordingDecision.visible&&recordingDecision.connected&&state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)}
              onTitle={title=>setPreparation(current=>current&&!current.submitted?{...current,title}:current)} onStart={()=>void startRecording()} onStop={source=>void stopRecording(source)}
              onSpeakerCounts={speakerCounts=>setPreparation(current=>current&&!current.submitted?{...current,speakerCounts}:current)}
              onBack={()=>void withSavedEdits(()=>{setSelectedMeeting(null);setPreparation(null);setRestoredFilter(null);setScreen("meetings");})} onSettings={()=>void withSavedEdits(()=>setScreen("settings"))} onConfirmResult={()=>void confirmPreparationResult()}
              inputs={captureAvailable?<CaptureInputSelection control={captureInputs} scopeControl={captureScope} locked={writeLocked||!!preparation?.submitted||state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)}/>:<p>録音ヘルパーが未配置です。署名済みバンドルのビルド後に利用できます。</p>}
              activeMicrophone={captureAvailable&&activeMicrophoneInput?<ActiveMicrophoneSelection control={captureInputs} locked={writeLocked||!captureInputs.inputs} onSelect={value=>void selectRecordingMicrophone(value)} status={microphoneSelection} pending={microphoneSelection?.source==="pending"||microphoneRequest?.owner===microphoneSession&&(!microphoneSelection||microphoneSelection.requestRevision<microphoneRequest.revision)}/>:null}
              test={captureAvailable?<CaptureTestPanel control={captureTest} currentUid={captureInputs.uid} currentScope={captureScope.scope}
                canStart={!!captureInputs.selected&&!!captureScope.scope&&!captureTest.blocked&&!operationWriteLocked&&!preparation?.submitted&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase)}
                locked={operationWriteLocked||captureBusy||closingBusy||startConfirm.pending} onStart={()=>void startCaptureTest()} onCancel={()=>void cancelCaptureTest()} onDiscard={()=>void discardCaptureTest()}/>:null}
              notices={<><IssueNotice scope="main" type="recording-command" label="録音の操作" message={captureError}/><IssueNotice scope="main" type="recording-confirmation" label="録音終了の確認" message={recordingDecision.error}/><IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError}/></>}/>
          </> : <>
          {selectedMeeting ? <AnalysisPanel key={selectedMeeting.id} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} hasTranscript={transcriptMeeting === selectedMeeting.id} recorded={["recorded","imported"].includes(selectedMeeting.lifecycle)} recordingActive={state.kind !== "ready" || state.snapshot.phase !== "idle"} locked={writeLocked} withSavedEdits={action => withSavedEdits(action, "transcription")} onReplaceGenerated={publication.replace} renderTools={renderMeetingTools} onHeadChanged={async () => {
            setAnalysisEpoch((value) => value + 1);
            const [meeting, items] = await Promise.all([invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }), invoke<Meeting[]>("list_meetings")]);
            setSelectedMeeting(meeting); setMeetings(items);
          }} /> : null}
          <section className="meetings-card" aria-label="保存済み会議" hidden={detail}>
            <div className="card-head"><input type="search" className="meeting-search" aria-label="会議名を検索" placeholder="会議名を検索" value={meetingQuery} onChange={event => setMeetingQuery(event.target.value)} /><IconButton label="再読込" icon="refresh" onClick={reloadMeetings} disabled={!isTauri() || writeLocked} /></div>
            <ToastNotice scope="main" type="recording-recovered" message={recoveredCount > 0 ? `前回の音声保存 ${recoveredCount} 件を検証して復旧しました。` : ""} onClose={() => setRecoveredCount(0)} />
            {!detail && <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} />}
            {restoredFilter && <p className="meeting-list-notice">今回復元した会議を表示しています。<button type="button" className="small-secondary" disabled={writeLocked} onClick={() => void withSavedEdits(() => setRestoredFilter(null))}>すべての会議を表示</button></p>}
            <IssueNotice scope="main" type="restore-origin" label="復元由来の確認" message={originError} />
            {shownMeetings.length === 0 ? <p className="empty-meetings">{meetingQuery ? "条件に合う会議はありません。" : restoredFilter ? "対象の復元会議は現在の一覧にありません。" : "保存済み会議はありません。"}</p> : <div className="meeting-table-frame"><table className="meeting-table"><thead><tr><th scope="col">会議名</th><th scope="col">作成日時</th><th scope="col">状態</th><th scope="col"><span className="sr-only">操作</span></th></tr></thead><tbody>{shownMeetings.map((meeting) => <tr key={meeting.id}>
              <td><button type="button" className="meeting-name" onClick={() => selectMeeting(meeting.id)} disabled={writeLocked}>{meeting.title}</button>{restoredIds.includes(meeting.id) && <small className="restore-badge">バックアップから復元</small>}</td>
              <td className="meeting-date">{new Date(meeting.createdAt).toLocaleString("ja-JP")}</td>
              <td><span className="state-pill">{meetingStatusLabel(meeting, state)}</span></td>
              <td><RowMenu label={`${meeting.title}の操作`} disabled={navigationLocked}><button type="button" role="menuitem" onClick={() => selectMeeting(meeting.id)} disabled={writeLocked}>開く</button>{["recorded", "imported"].includes(meeting.lifecycle) && <button type="button" role="menuitem" className="deletion-trigger" aria-label={`${meeting.title}の音声だけを削除`} onClick={() => void audioDeletion.reviewNew(meeting.id)} disabled={!audioDeletion.canReviewNew}>音声だけを削除</button>}<button type="button" role="menuitem" className="deletion-trigger" aria-label={`${meeting.title}を削除`} onClick={() => void deletion.reviewMeeting(meeting.id)} disabled={!deletion.canReviewMeeting || meeting.lifecycle === "recording"}>削除</button>{dataOperation.deletionListing?.deletingMeetingIds.includes(meeting.id) && <span role="status">削除中・要確認</span>}</RowMenu></td>
            </tr>)}</tbody></table></div>}
          </section>
          {selectedMeeting && <div className="workspace-tab-panel" id="meeting-panel-transcript" role="tabpanel" aria-labelledby="meeting-tab-transcript" hidden={detailTab !== "transcript"}>
            <TranscriptEditor key={`${selectedMeeting.id}:${analysisEpoch}`} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} locked={writeLocked} playbackHost={playbackHost} visible={detailTab === "transcript"} onAnalysisLoaded={present => setTranscriptMeeting(present ? selectedMeeting.id : null)} onConsolidated={async()=>{
              const [meeting,items]=await Promise.all([invoke<Meeting|null>("get_meeting",{id:selectedMeeting.id}),invoke<Meeting[]>("list_meetings")]);
              setSelectedMeeting(meeting);setMeetings(items);setAnalysisEpoch(value=>value+1);
            }} />
          </div>}
          {selectedMeeting && <div className="workspace-tab-panel" id="meeting-panel-minutes" role="tabpanel" aria-labelledby="meeting-tab-minutes" hidden={detailTab !== "minutes"}><MinutesPanel key={`minutes:${selectedMeeting.id}`} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} sourceEpoch={analysisEpoch} recordingActive={state.kind !== "ready" || state.snapshot.phase !== "idle"} locked={writeLocked} withSavedEdits={withSavedEdits} visible={detailTab === "minutes"} onHeadChanged={async () => {
            const [meeting, items] = await Promise.all([invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }), invoke<Meeting[]>("list_meetings")]);
            setSelectedMeeting(meeting); setMeetings(items);
          }} /></div>}
          </>}
        </main>
        <div ref={setPlaybackHost} className="meeting-playback-dock" role="region" aria-label="会議の共通音声プレーヤー" hidden={!selectedMeeting || screen === "settings" || screen === "recording" || screen === "jobs" || screen === "logs" || screen === "voices"} />
      </div>
    </div>
    {selectedMeeting && screen !== "settings" && screen !== "recording" && screen !== "jobs" && screen !== "logs" && screen !== "voices" && ["recorded", "imported", "archived"].includes(selectedMeeting.lifecycle) && selectedMeeting.sourceType && <MeetingAudioControls key={selectedMeeting.id} meetingId={selectedMeeting.id} sourceType={selectedMeeting.sourceType} locked={writeLocked} host={playbackHost} />}
    <CommonModal open={renameOpen && !!selectedMeeting} blocked={meetingBusy} onDismiss={() => setRenameOpen(false)} restoreTarget={() => renameOpener.current} className="meeting-rename-dialog" aria-labelledby="meeting-rename-title" dismissLabel="閉じる">
      <h2 id="meeting-rename-title">会議名を変更</h2>
      <form onSubmit={renameMeeting}>
        <label htmlFor="rename-title">会議名<input id="rename-title" value={renameTitle} maxLength={200} onChange={event => setRenameTitle(event.target.value)} disabled={meetingBusy || writeLocked} /></label>
        <div className="dialog-actions"><button type="button" className="secondary" disabled={meetingBusy} onClick={() => setRenameOpen(false)}>キャンセル</button><button type="submit" disabled={meetingBusy || writeLocked || !renameTitle.trim()}>保存</button></div>
      </form>
    </CommonModal>
    </MeetingAudioProvider>
    </MeetingPlaybackSurface>
  );
}

const desktopWindow = <ThemeProvider>{isRecordingConfirmation ? <RecordingConfirmationWindow /> : <RecorderOperationProvider>{isRecorderInfo ? <RecorderInfoWindow /> : <RecordingInputProvider windowLabel={isRecorderPanel?"recorder":"main"}>{isRecorderPanel ? <RecorderPanel /> : <><SettingsModalScope><MainWindow /></SettingsModalScope><ConfirmationHost /></>}</RecordingInputProvider>}</RecorderOperationProvider>}</ThemeProvider>;
createRoot(document.getElementById("root")!).render(isStartupAttention ? <StartupAttentionWindow /> : isRecorderPanel || isRecordingConfirmation || isRecorderInfo ? desktopWindow : <StartupGate>{desktopWindow}</StartupGate>);
