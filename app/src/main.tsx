import { invoke, isTauri } from "@tauri-apps/api/core";
import { InfoPopover } from "./info-popover";
import { ConfirmationHost } from "./confirmation-host";
import { IssueNotice, ToastHost, ToastNotice } from "./toast";
import { JobNotifications } from "./job-notifications";
import {ArchivedInputNotices} from "./archived-input-notices";
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
import {RecordingWorkspace} from "./recording-workspace";
import {CaptureTestPanel,useCaptureTest} from "./capture-test";
import {CaptureTestRecoveryPanel,useCaptureTestRecovery} from "./capture-test-recovery";
import {RecordingWriteStatus} from "./recording-write-status";
import {RecordingInputProvider,RecordingInputNotice,RecordingCompactInputs} from "./recording-input-status";
import {waitForCaptureTestStop} from "./capture-test-client";
import {stopAllPlayback} from "./playback-barrier";
import {newRecordingPreparation,sameRecordingPreparation,validRecordingTitle,type RecordingPreparation} from "./recording-preparation-state";
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
        {state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)&&<RecordingCompactInputs owner={state.snapshot.sessionId}/>}
        <span className="timer">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</span>
      </div>
      <div className="mini-bottom">
        {state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)?<RecordingWriteStatus compact owner={state.snapshot.sessionId}/>:<span className="mini-note">{idle ? "待機中" : "録音状態はメイン画面と共通"}</span>}
        <button type="button" onClick={() => action.run("open_main_window")} disabled={action.busy || !isTauri()}>メインを開く</button>
        {state.kind === "ready" && (state.snapshot.phase === "recording" || state.snapshot.phase === "preparing") && <button type="button" className="stop-button" onClick={() => void decision.request()} disabled={action.busy || decision.requestBusy || decision.visible || !decision.connected}>{state.snapshot.phase === "preparing" ? "中止" : "停止"}</button>}
        {idle && <button type="button" className="text-button" onClick={() => action.run("hide_recorder_preview")} disabled={action.busy}>閉じる</button>}
      </div>
      <RecordingInputNotice windowLabel="recorder"/>
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
  const [preparation,setPreparation]=useState<RecordingPreparation|null>(null);
  const [screen, setScreen] = useState<"overview" | "meetings" | "settings" | "recording">("overview");
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
  const captureOwner=screen==="recording"&&preparation?preparation.id:selectedMeeting?.id??null;
  const captureEnabled=captureAvailable&&(screen==="recording"&&!!preparation&&!preparation.submitted||selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null)&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase);
  const captureInputs=useCaptureInputs(captureEnabled,captureOwner);
  const captureScope=useCaptureScope(captureEnabled,captureOwner);
  const captureTest=useCaptureTest();
  const captureTestRecovery=useCaptureTestRecovery();
  const [closingBusy, setClosingBusy] = useState(false);
  const closing = useRef(false), starting = useRef(false);
  const [recoveredCount, setRecoveredCount] = useState(0);
  const [navigationBusy, setNavigationBusy] = useState(false);
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
  const deletion = useMeetingDeletion({ operation: dataOperation, locked: captureTest.blocked || navigationBusy || dataPending || recoveryPending || modelDeletionPending || audioDeletionPending, withSavedEdits,
    onDeleted: async receipt => {
      setSelectedMeeting(current => current?.id === receipt.meetingId ? null : current);
      setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins();
    } });
  const modelDeletion = useModelBodyDeletion({ operation: dataOperation, locked: captureTest.blocked || navigationBusy || dataPending || recoveryPending || deletion.protectedNow || audioDeletionPending, withSavedEdits,
    onProtectionChange: setModelDeletionPending, onDeleted: async () => { setModelCatalogEpoch(epoch => epoch + 1); } });
  const audioDeletion = useAudioDeletion({ operation: dataOperation, locked: captureTest.blocked || navigationBusy || dataPending || recoveryPending || deletion.protectedNow || modelDeletion.protectedNow, withSavedEdits,
    onProtectionChange: setAudioDeletionPending, onDeleted: async () => {
      // Audio availability changes, not document heads. Keep mounted editors and
      // their local buffers, including when recovery deliberately skipped writes.
      setMeetings(await invoke<Meeting[]>("list_meetings"));
    } });
  const operationNavigationLocked = recordingDecision.visible || recordingDecision.requestBusy || navigationBusy || dataPending || recoveryPending || deletion.protectedNow || modelDeletion.protectedNow || audioDeletion.protectedNow || !dataOperation.connected || !!dataOperation.snapshot?.busy;
  const baseNavigationLocked=operationNavigationLocked||captureTest.blocked||captureTestRecovery.blocked;
  const deletionBlocked = !!dataOperation.deletionListing?.writeBlocked || !!dataOperation.snapshot?.deletionPreview?.pending || !!dataOperation.modelDeletionListing?.writeBlocked || !!dataOperation.snapshot?.modelDeletionPreview?.pending || !!dataOperation.audioDeletionListing?.writeBlocked || !!dataOperation.snapshot?.audioDeletionPreview?.pending;
  const baseWriteLocked = baseNavigationLocked || deletionBlocked || !!dataOperation.snapshot?.restartRequired;
  const operationWriteLocked=operationNavigationLocked||deletionBlocked||!!dataOperation.snapshot?.restartRequired;
  const startConfirm = useActionConfirmation(baseWriteLocked || closingBusy);
  const closeConfirm = useActionConfirmation(false);
  const navigationLocked = baseNavigationLocked || captureBusy || closingBusy || startConfirm.pending;
  const writeLocked = baseWriteLocked || captureBusy || closingBusy || startConfirm.pending;
  const root = useRef({ selectedMeeting, preparation, screen, baseWriteLocked, captureAvailable, captureInputs, captureScope, captureTest, state });
  root.current = { selectedMeeting, preparation, screen, baseWriteLocked, captureAvailable, captureInputs, captureScope, captureTest, state };
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
      const recovery=await captureTestRecovery.client.refresh();
      if(recovery.phase==="checking")throw Error("テスト音声の起動時確認が進行中です。確認後に終了してください。");
      if(recovery.phase==="attention"){
        const outcome=await closeConfirm({title:"未回収のテスト音声を保持して終了しますか？",target:"以前の録音前テスト",confirmLabel:"音声を保持して終了へ進む",
          message:"テスト音声の回収が未完了です。所有台帳と音声を残し、次の起動時に取得の終了と所有を再確認します。未確認の取得プロセスを推測したPIDで終了させたり、音声を強制削除したりしません。会議データは通常の終了手順で保存します。",
          action:async()=>{const current=await captureTestRecovery.client.refresh();if(current.phase!=="attention"||current.revision!==recovery.revision)throw Error("保持する回復状態が変更されています。終了は再確認してください。");}});
        if(outcome!=="completed")return;
      }
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
    const microphoneUID=captureInputs.uid;
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
      const microphone=usableMicrophone(checkedCaptureInputs(await invoke("get_capture_inputs")),microphoneUID);
      if(!microphone)throw Error("選択したマイクまたは権限を再確認してください。別機器には切り替えていません。");
      const applications=scope.mode==="application"?checkedCaptureApplications(await invoke("get_capture_applications")):null;
      checkedCaptureScope(scope,applications);
      const application=captureApplicationScope(scope,applications);
      const scopeMessage=scope.mode==="system"?"PC音声は全体取得し、通知音や他の動画の音も含まれる場合があります。":`PC音声は選択したアプリ（${application?.name}）を取得します。ブラウザでは他のタブの音も含まれる場合があり、特定タブだけの取得は保証しません。終了・再起動時は全体取得や別アプリに切り替えません。`;
      if (!expected || expected.lifecycle !== "draft" || expected.sourceType != null || !ownerUnchanged()) throw Error("録音対象の会議を再確認してください。");
      await startConfirm({ title: "選択した会議の録音を開始しますか？", target: expected.title, confirmLabel: "録音を開始する", message: "マイクとPCの再生音が端末内に保存されます。参加者の同意と録音権限を確認してください。実行中の解析は安全な区切りで一時停止し、録音終了後に再開します。"+`\n録音するマイク: ${microphone.name}。${scopeMessage}` + (expectedTest ? "\n前のテスト音声の再生を停止し、一時音声を破棄してから録音を開始します。保存済み会議の音声は変更しません。" : "") + (importNeedsConsent(importing) ? `\n取り込み（${importing.fileName ?? "選択・確認中の音声"}）を安全に取り消して録音を優先します。取り込みの確認内容と未確定物だけを整理し、元ファイルと保存済み会議を保持します。` : ""), validate: () => ownerUnchanged() && root.current.captureAvailable && root.current.captureInputs.uid===microphoneUID&&!!root.current.captureInputs.selected && scopeUnchanged() && !root.current.baseWriteLocked && root.current.state.kind === "ready" && ["idle", "error"].includes(root.current.state.snapshot.phase), action: () => withConfirmedEdits(withSavedEdits, async () => {
        const [meeting, recording, currentImport, currentInputs, currentApplications] = await Promise.all([invoke<Meeting | null>("get_meeting", { id: expected.id }), invoke<RecordingSnapshot>("get_recording_snapshot"), invoke<ImportSnapshot>("get_audio_import"),invoke("get_capture_inputs").then(checkedCaptureInputs),scope.mode==="application"?invoke("get_capture_applications").then(checkedCaptureApplications):Promise.resolve(null)]);
        assertCloseRecording(recording);
        assertCloseImport(currentImport);
        if(!usableMicrophone(currentInputs,microphoneUID)||root.current.captureInputs.uid!==microphoneUID)throw Error("確認したマイクまたは権限が変更されました。録音は開始していません。別機器には切り替えていません。");
        checkedCaptureScope(scope,currentApplications);
        if(!scopeUnchanged())throw Error("PC音声の取得範囲が変更されました。録音は開始していません。");
        const meetingUnchanged=local?meeting===null:!!meeting&&meeting.id===expected.id&&meeting.title===expected.title&&meeting.lockVersion===expected.lockVersion&&meeting.lifecycle==="draft"&&meeting.sourceType==null;
        if (!meetingUnchanged || !ownerUnchanged() || recordingActive(recording) || currentImport.token !== importing.token || !importing.busy && currentImport.revision !== importing.revision) throw Error("録音対象が確認後に変更されました。録音は開始していません。");
        await stopAllPlayback();
        const currentTest=await captureTest.client.refresh();
        if((currentTest?.runId??null)!==(expectedTest?.runId??null)||currentTest?.busy)throw Error("録音前テストの対象・状態が変更されました。録音は開始していません。");
        if(currentTest)await captureTest.client.discard(currentTest.runId);
        if(local)setPreparation(current=>current?.id===local.id?{...current,submitted:true}:current);
        await invoke("start_recording", { meetingId: expected.id, microphoneUid:microphoneUID, captureScope:scope, newTitle:local?expected.title:null });
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
  const stopRecording = async () => {
    await recordingDecision.request();
  };
  // The caller has already flushed edits. This only creates local preparation.
  const prepareNewRecording=async()=>{
    const current=await invoke<RecordingSnapshot>("get_recording_snapshot");assertCloseRecording(current);
    if(recordingActive(current))throw Error("録音状態が変更されています。最新のヘッダーから録音画面を開き直してください。");
    setPreparation(newRecordingPreparation(crypto.randomUUID(),new Date()));setSelectedMeeting(null);setCaptureError("");setScreen("recording");
    requestAnimationFrame(()=>document.getElementById("recording-title")?.focus());
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
  const prepareSelectedRecording=async()=>{
    const expected=selectedMeeting;if(!expected)return;
    await withSavedEdits(async()=>{
      const [meeting,recording]=await Promise.all([invoke<Meeting|null>("get_meeting",{id:expected.id}),invoke<RecordingSnapshot>("get_recording_snapshot")]);assertCloseRecording(recording);
      if(!meeting||meeting.id!==expected.id||meeting.lifecycle!=="draft"||meeting.sourceType!=null||root.current.selectedMeeting?.id!==expected.id||recordingActive(recording))throw Error("録音対象または録音状態が変更されています。準備を開き直してください。");
      setPreparation(null);setSelectedMeeting(meeting);setRenameTitle(meeting.title);setCaptureError("");setScreen("recording");
      requestAnimationFrame(()=>document.querySelector<HTMLElement>('[aria-label="録音状態"]')?.focus());
    });
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
        <RecordingInputNotice windowLabel="main"/>
        <ThemeNotifications locked={writeLocked} />
        <ToastNotice scope={state.kind === "ready" ? state.snapshot.sessionId ?? "recording" : "recording"} type="recording-failure" level="error" message={state.kind === "ready" ? state.snapshot.errorMessage : state.kind === "error" ? state.message : ""} />
        <header className="topbar"><span>{detail ? "会議詳細" : screen === "recording" ? "録音" : screen === "overview" ? "概要" : screen === "meetings" ? "会議一覧" : "設定"}</span><ThemeSelection compact locked={writeLocked} /><div className="header-recording" aria-label="共通録音状態"><span className="header-status"><span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} />{phaseLabel(state)}{state.kind === "ready" && recordingActive(state.snapshot) && <span className="header-timer">{formatElapsed(state.snapshot.elapsedMs)}</span>}</span>{state.kind === "ready" && recordingActive(state.snapshot) && <button type="button" className="small-secondary" disabled={navigationLocked} onClick={() => void openRecordingMeeting()}>録音画面へ</button>}{state.kind === "ready" && ["preparing", "recording"].includes(state.snapshot.phase) && <button type="button" className="stop-button" disabled={captureBusy || navigationBusy || recordingDecision.requestBusy || recordingDecision.visible || !recordingDecision.connected} onClick={() => void stopRecording()}>録音を終了</button>}</div></header>
        {detail && <div className="meeting-detail-tabs" role="tablist" aria-label="会議詳細">
          {(["transcript", "minutes"] as const).map(tab => <button key={tab} type="button" role="tab" id={`meeting-tab-${tab}`} aria-controls={`meeting-panel-${tab}`} aria-selected={detailTab === tab} tabIndex={detailTab === tab ? 0 : -1} disabled={navigationLocked} onClick={() => void switchDetailTab(tab)} onKeyDown={event => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return;
            const target = event.key === "Home" ? "transcript" : event.key === "End" ? "minutes" : ["ArrowLeft", "ArrowRight"].includes(event.key) ? tab === "transcript" ? "minutes" : "transcript" : null;
            if (target) { event.preventDefault(); document.getElementById(`meeting-tab-${target}`)?.focus(); }
          }}>{tab === "transcript" ? "文字起こし" : "議事録"}</button>)}
        </div>}
        <main className={`content${detail ? " meeting-detail-content" : ""}`} aria-label="会議ワークスペース" tabIndex={-1}>
          <CaptureTestRecoveryPanel control={captureTestRecovery} locked={operationNavigationLocked||captureBusy||closingBusy||startConfirm.pending} currentTest={!!captureTest.snapshot||!!captureTest.pending}/>
          <JobNotifications locked={navigationLocked} onOpen={openNotifiedMeeting} />
          <ArchivedInputNotices locked={navigationLocked} paused={!dataOperation.connected||!!dataOperation.snapshot?.busy||!!dataOperation.snapshot?.restartRequired||deletionBlocked||closingBusy} onOpen={openNotifiedMeeting}/>
          <AudioImportPanel visible={screen==="overview" && !detail} locked={writeLocked} recordingActive={state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)} withSavedEdits={withSavedEdits} onNewRecording={prepareNewRecording} onSaved={async(id)=>{
            await flushPendingEdits();
            const [meeting,items]=await Promise.all([invoke<Meeting|null>("get_meeting",{id}),invoke<Meeting[]>("list_meetings")]);
            if(!meeting)throw Error("保存した会議が見つかりません。");
            setSelectedMeeting(meeting);setMeetingToolsOpen(false);setRenameTitle(meeting.title);setMeetings(items);setScreen("overview");
          }}/>
          {screen === "settings" && <><h1>設定</h1><p className="lead">会議データのバックアップと追加復元、共通用語とAIモデルの設定を管理します。</p></>}
          {screen === "settings" && <ThemeSettingsPanel locked={writeLocked} />}
          {screen !== "settings" && <ThemeReadNotice />}
          <DataSavePanel operation={dataOperation} visible={screen === "settings"} locked={captureTest.blocked || navigationBusy || recoveryPending || deletion.protectedNow || modelDeletion.protectedNow || audioDeletion.protectedNow || deletionBlocked} withSavedEdits={withSavedEdits} onProtectionChange={setDataPending} onRestored={async () => { setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins(); }} onShowMeetings={async ids => { await withSavedEdits(async () => { setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins(); setRestoredFilter(ids); setSelectedMeeting(null); setScreen("meetings"); }); }} />
          <MeetingDeletionPanel operation={dataOperation} control={deletion} visible={screen === "settings"} />
          <ModelBodyDeletionPanel operation={dataOperation} control={modelDeletion} visible={screen === "settings"} />
          <AudioDeletionPanel operation={dataOperation} control={audioDeletion} visible={screen === "settings"} meetingTitle={meetings.find(m => m.id === audioDeletion.preview?.meetingId)?.title} />
          <RecoveryPanel operation={dataOperation} visible={screen === "settings"} locked={captureTest.blocked || navigationBusy || dataPending || deletion.protectedNow || modelDeletion.protectedNow || audioDeletion.protectedNow || deletionBlocked} withSavedEdits={withSavedEdits} onProtectionChange={setRecoveryPending} />
          {screen === "settings" ? <><VocabularyEditor locked={writeLocked} /><ModelInspectionPanel locked={writeLocked} deletionControl={modelDeletion} refreshEpoch={modelCatalogEpoch} recordingActive={state.kind!=="ready" || !["idle","error"].includes(state.snapshot.phase)} /><IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} /></> : screen==="recording" ? <>
            <RecordingWorkspace preparation={preparation} title={selectedMeeting?.title??preparation?.title??"録音対象を確認してください"} state={state} owner={captureOwner} locked={writeLocked}
              canStart={isTauri()&&captureAvailable&&!!captureInputs.selected&&!!captureScope.scope&&!writeLocked&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase)&&(preparation?!preparation.submitted&&validRecordingTitle(preparation.title):selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null)}
              canStop={!captureBusy&&!navigationBusy&&!recordingDecision.requestBusy&&!recordingDecision.visible&&recordingDecision.connected&&state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)}
              onTitle={title=>setPreparation(current=>current&&!current.submitted?{...current,title}:current)} onStart={()=>void startRecording()} onStop={()=>void stopRecording()}
              onBack={()=>void withSavedEdits(()=>{setSelectedMeeting(null);setPreparation(null);setRestoredFilter(null);setScreen("meetings");})} onSettings={()=>void withSavedEdits(()=>setScreen("settings"))} onConfirmResult={()=>void confirmPreparationResult()}
              inputs={captureAvailable?<CaptureInputSelection control={captureInputs} scopeControl={captureScope} locked={writeLocked||!!preparation?.submitted||state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)}/>:<p>録音ヘルパーが未配置です。署名済みバンドルのビルド後に利用できます。</p>}
              test={captureAvailable?<CaptureTestPanel control={captureTest} currentUid={captureInputs.uid} currentScope={captureScope.scope}
                canStart={!!captureInputs.selected&&!!captureScope.scope&&!captureTest.blocked&&!captureTestRecovery.blocked&&!operationWriteLocked&&!preparation?.submitted&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase)}
                locked={operationWriteLocked||captureBusy||closingBusy||startConfirm.pending} onStart={()=>void startCaptureTest()} onCancel={()=>void cancelCaptureTest()} onDiscard={()=>void discardCaptureTest()}/>:null}
              notices={<><IssueNotice scope="main" type="recording-command" label="録音の操作" message={captureError}/><IssueNotice scope="main" type="recording-confirmation" label="録音終了の確認" message={recordingDecision.error}/><IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError}/></>}/>
          </> : <>
          {!detail && <><div className="eyebrow">DESKTOP FOUNDATION</div>
          <h1>{screen === "meetings" ? "会議一覧" : "会議の記録を、手元で。"}</h1>
          <p className="lead">保存済み音声を端末内で文字起こし・話者分離し、修正後の内容から議事録候補を生成できます。内容と根拠を確認して採用してください。macOSの実録音と推定品質は検証中です。</p></>}
          {screen==="meetings"&&<button type="button" disabled={!isTauri()||writeLocked||state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)} onClick={()=>void withSavedEdits(prepareNewRecording)}>新しい録音</button>}
          {detail && <div className="meeting-detail-status"><strong title={selectedMeeting!.title}>{selectedMeeting!.title}</strong>{navigationBusy && <span role="status">編集の保存を確認中</span>}<IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} /></div>}
          <details className={detail ? "meeting-workspace-tools workspace-tools" : "landing-tools"} open={!detail || meetingToolsOpen} onToggle={event => { if (detail) setMeetingToolsOpen(event.currentTarget.open); }}>
          <summary hidden={!detail}>会議管理・録音・解析操作</summary>
          <div className="workspace-tools-body">
          <section className="status-card" aria-label="録音状態" tabIndex={-1}>
            <div className="card-head"><div className="heading-with-info"><h2>録音セッション</h2><InfoPopover label="録音セッション" details={<p>マイクとPC音声を別々のトラックに保存します。PC音声の対象範囲はOSのキャプチャ機能と権限に依存し、会議アプリだけに限定されるとは限りません。参加者の同意を得て、不要なアプリの音声を止めてください。保存後は準備済みモデルで文字起こし・話者分離を順に進め、結果を候補として保存します。モデル未準備でも音声は保持します。録音品質・長時間と10話者の正式評価は保留中です。</p>}>マイクとPC音声を別々に保存します。PC音声には会議以外の音が含まれる場合があります。同意を確認し、不要な音を止めてください。保存後の解析結果は確認待ちの候補です。</InfoPopover></div><span className="state-pill">{phaseLabel(state)}</span></div>
            <div className="clock">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</div>
            <p>{state.kind === "ready" && state.snapshot.phase === "idle" ? "録音は開始していません。" : state.kind === "ready" && state.snapshot.errorMessage ? state.snapshot.errorMessage : state.kind === "error" ? state.message : "録音状態を確認しています。"}</p>
            <div className="card-actions">
              <button type="button" onClick={() => void prepareSelectedRecording()} disabled={!isTauri() || !selectedMeeting || selectedMeeting.lifecycle !== "draft" || selectedMeeting.sourceType != null || writeLocked || state.kind !== "ready" || !["idle","error"].includes(state.snapshot.phase)}>録音準備を開く</button>
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
        <div ref={setPlaybackHost} className="meeting-playback-dock" role="region" aria-label="会議の共通音声プレーヤー" hidden={!selectedMeeting || screen === "settings" || screen === "recording"} />
      </div>
    </div>
    {selectedMeeting && screen !== "settings" && screen !== "recording" && ["recorded", "imported", "archived"].includes(selectedMeeting.lifecycle) && selectedMeeting.sourceType && <MeetingAudioControls key={selectedMeeting.id} meetingId={selectedMeeting.id} sourceType={selectedMeeting.sourceType} locked={writeLocked} host={playbackHost} />}
    </MeetingPlaybackSurface>
  );
}

createRoot(document.getElementById("root")!).render(<ThemeProvider>{isRecordingConfirmation ? <RecordingConfirmationWindow /> : <RecordingInputProvider windowLabel={isRecorderPanel?"recorder":"main"}>{isRecorderPanel ? <RecorderPanel /> : <><MainWindow /><ConfirmationHost /></>}</RecordingInputProvider>}</ThemeProvider>);
