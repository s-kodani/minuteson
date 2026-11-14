import { useCaptureActions, useCaptureActionState, type CaptureMeeting as Meeting } from "./capture-actions";
import { useWorkspaceClose } from "./workspace-close";
import { InfoPopover, closeInfoPopovers } from "./info-popover";
import { ModelDeletionDialog, useModelDeletion } from "./model-deletion";
import { VoicePeopleSettings } from "./voice";
import { MinutesPromptSettings } from "./minutes-prompt-settings";
import { InternalLogs } from "./internal-logs";
import { SettingsFooter } from "./settings-footer";
import { AppUpdateBanner, AppUpdateProvider } from "./app-updates";
import appIconUrl from "../src-tauri/icons/icon.svg";
import { usePlatform } from "./platform";
import { ResidencyNotifications } from "./residency-notifications";
import { invoke, isTauri } from "./ipc.ts";
import { SettingsModalScope } from "./settings-modal-scope";
import { useAnalysisPublication } from "./analysis-publication";
import { ActionIcon, IconButton } from "./action-icon";
import { RowMenu } from "./row-menu";
import { MeetingDeletionMenu } from "./meeting-deletion-menu";
import { NotificationPopover } from "./notification-popover";
import { ScreenHistory } from "./screen-history";
import { CommonModal } from "./common-modal";
import { ConfirmationHost } from "./confirmation-host";
import { IssueNotice, NotificationList, ToastHost, ToastNotice } from "./toast";
import { JobNotifications } from "./job-notifications";
import { RecorderOperationProvider, RecorderOperationNotifications } from "./recorder-operation-notices";
import {ArchivedInputNotices} from "./archived-input-notices";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useLayoutEffect, useRef, useState, type FormEvent } from "react";
import { formatElapsed, useRecordingSession } from "./session";
import { phaseLabel } from "./recording-phase";
import { meetingStatusLabel } from "./meeting-status";
import { TranscriptEditor } from "./transcript";
import { AnalysisPanel } from "./analysis";
import { backgroundReadFailure } from "./background-read-failure";
import { MinutesPanel } from "./minutes";
import { VocabularyEditor } from "./vocabulary";
import { RecordingVocabulary } from "./recording-vocabulary";
import { flushPendingEdits } from "./draft-save";
import { useSavedNavigation } from "./saved-navigation";
import { AudioImportPanel } from "./audio-import";
import { ModelInspectionPanel } from "./model-inspection";
import { ModelDownloadGuidance } from "./model-download-guidance";
import { CommonOperationProgress, usePublishProgress } from "./operation-progress";
import { RecordingRecoveryNotifications, useRecordingRecovery } from "./recording-recovery";
import { recoveryMeetingState } from "./recording-recovery-state";
import { recordingSaveProgress, deletionProgress, dataJobProgress } from "./operation-progress-state";
import { JobCount, JobList } from "./job-list";
import { useDataOperation } from "./data-operation";
import { DataSavePanel } from "./data-save";
import { MeetingDeletionPanel, useMeetingDeletion } from "./meeting-deletion";
import { AudioDeletionPanel, useAudioDeletion } from "./audio-deletion";
import type { RecordingSnapshot } from "./session";
import "./style.css";
import "./prototype-alignment.css";
import "./recorder-panel.css";
import { ThemeSettingsPanel, ThemeNotifications } from "./theme";
import { useRecordingConfirmation } from "./recording-confirmation";
import { confirmations, useActionConfirmation } from "./confirmation-service";
import { appShortcut, overlayOpen } from "./keyboard";
import { MeetingPlaybackSurface } from "./playback-surface";
import { MeetingAudioProvider } from "./meeting-audio";
import {ActiveMicrophoneSelection,CaptureInputSelection,useCaptureInputs} from "./capture-inputs";
import {RecordingWorkspace} from "./recording-workspace";
import {CaptureTestPanel,useCaptureTest} from "./capture-test";
import {RecordingInputProvider,RecordingInputNotice} from "./recording-input-status";
import {validRecordingPreparation,type RecordingPreparation} from "./recording-preparation-state";
import {useRecordingMicrophoneSelection} from "./microphone-selection-status";
import {useCaptureScope} from "./capture-applications";
import { assertCloseRecording, recordingActive } from "./application-close";
import { useAppName } from "./window-environment";




function MainWindow() {
  const appName = useAppName();
  const platform = usePlatform();
  const state = useRecordingSession();
  const recordingDecision = useRecordingConfirmation("main");
  const dataOperation = useDataOperation();
  usePublishProgress("data-deletion", [deletionProgress(dataOperation.snapshot)]);
  usePublishProgress("data-transfer", dataOperation.jobs.filter(job => job.snapshot.busy || job.snapshot.phase === "ready").map(dataJobProgress));
  const recovery = useRecordingRecovery();
  const [dataProtection, setDataProtection] = useState<"operation" | "navigation" | null>(null);
  const dataPending = dataProtection !== null;
  const [audioDeletionPending, setAudioDeletionPending] = useState(false);
  const [restoredIds, setRestoredIds] = useState<string[]>([]);
  const [restoredFilter, setRestoredFilter] = useState<string[] | null>(null);
  const [originReadFailure] = useState(() => backgroundReadFailure("restore_labels"));
  const originRequest = useRef(0);
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const savingRecording = state.kind === "ready" ? state.snapshot : null;
  usePublishProgress("recording-save", [recordingSaveProgress(savingRecording, meetings.find(meeting => meeting.id === savingRecording?.meetingId)?.title)]);
  const [meetingQuery, setMeetingQuery] = useState("");
  const [meetingError, setMeetingError] = useState("");
  const [meetingBusy, setMeetingBusy] = useState(false);
  const [selectedMeeting, setSelectedMeeting] = useState<Meeting | null>(null);
  const [preparation,setPreparation]=useState<RecordingPreparation|null>(null);
  const [screen, setScreen] = useState<"overview" | "meetings" | "settings" | "recording" | "jobs" | "logs" | "voices" | "prompts">("meetings");
  const [detailTab, setDetailTab] = useState<"transcript" | "minutes">("transcript");
  const [history] = useState(() => new ScreenHistory<{ screen: typeof screen; meeting: Meeting | null; preparation: RecordingPreparation | null; filter: string[] | null }>());
  const [, updateHistory] = useState(0);
  const routeKey = `${screen}:${screen === "overview" || screen === "recording" ? selectedMeeting?.id ?? preparation?.id ?? "" : ""}`;
  useLayoutEffect(() => {
    if (history.visit(routeKey, { screen, meeting: selectedMeeting, preparation, filter: restoredFilter })) { closeInfoPopovers(); updateHistory(value => value + 1); }
  });
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  useLayoutEffect(() => { closeInfoPopovers(); }, [detailTab]);
  const [transcriptActionsHost, setTranscriptActionsHost] = useState<HTMLDivElement | null>(null);
  const [transcriptEditorActionsHost, setTranscriptEditorActionsHost] = useState<HTMLDivElement | null>(null);
  useLayoutEffect(() => { setDetailTab("transcript"); }, [selectedMeeting?.id]);
  const [playbackHost,setPlaybackHost] = useState<HTMLDivElement | null>(null);
  const [audioDropHost, setAudioDropHost] = useState<HTMLDivElement | null>(null);
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
  const captureController = useCaptureActionState();
  const { starting, captureBusy, captureError, captureTestError, microphoneRequest } = captureController;
  const captureOwner=screen==="recording"&&preparation?preparation.id:selectedMeeting?.id??null;
  const captureEnabled=captureAvailable&&(screen==="recording"&&!!preparation&&!preparation.submitted||selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null)&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase);
  const activeMicrophoneInput=screen==="recording"&&state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)&&state.snapshot.meetingId===selectedMeeting?.id;
  const captureInputs=useCaptureInputs(captureEnabled||activeMicrophoneInput,captureOwner);
  const microphoneSession=state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)?state.snapshot.sessionId:null;
  const microphoneSelection=useRecordingMicrophoneSelection(microphoneSession);
  const captureScope=useCaptureScope(captureEnabled,captureOwner);
  const captureTest=useCaptureTest();
  const [closingBusy, setClosingBusy] = useState(false);
  const closing = useRef(false);
  const [recoveredCount, setRecoveredCount] = useState(0);
  const { busy: navigationBusy, quietNavigation, running: navigating, withSavedEdits: saveEdits } = useSavedNavigation(setMeetingError);
  // Navigation and regeneration cannot leave a completed test holding the
  // native admission gate. Capture actions own their own confirmation barrier.
  const withSavedEdits: typeof saveEdits = (action, reason, screenNavigation) => saveEdits(async () => {
    await captureTest.finish();
    await action();
  }, reason, screenNavigation);
  const navigate = (action: () => void | Promise<void>, reason: "leave" | "tab" = "leave") =>
    withSavedEdits(action, reason, true);
  const loadOrigins = async () => {
    const request = ++originRequest.current;
    try {
      const ids = await invoke<string[]>("list_restored_meeting_ids");
      if (request !== originRequest.current) return;
      if (!Array.isArray(ids) || ids.some(id => typeof id !== "string")) throw Error("復元された会議の一覧を確認できません。");
      originReadFailure.recovered();
      setRestoredIds(ids);
    } catch (cause) {
      if (request === originRequest.current) originReadFailure.failed(cause);
    }
  };
  const [analysisEpoch, setAnalysisEpoch] = useState(0);
  const [transcriptMeeting, setTranscriptMeeting] = useState<string | null>(null);
  const priorPhase = useRef<string | null>(null);
  const deletion = useMeetingDeletion({ operation: dataOperation, meetingIds: [...new Set([...meetings.map(meeting => meeting.id), ...(selectedMeeting ? [selectedMeeting.id] : [])])], locked: captureTest.blocked || navigationBusy || dataPending || audioDeletionPending, withSavedEdits,
    onDeleted: async receipt => {
      if (selectedMeeting?.id === receipt.meetingId) {
        setSelectedMeeting(null);
        setScreen("meetings");
      }
      setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins();
    } });
  const audioDeletion = useAudioDeletion({ operation: dataOperation, availability: deletion.audioAvailability, locked: captureTest.blocked || navigationBusy || dataPending || deletion.protectedNow, withSavedEdits,
    onProtectionChange: setAudioDeletionPending, onDeleted: async () => {
      // Audio availability changes, not document heads. Keep mounted editors and
      // their local buffers, including when recovery deliberately skipped writes.
      setMeetings(await invoke<Meeting[]>("list_meetings"));
    } });
  const [modelRefreshEpoch, setModelRefreshEpoch] = useState(0);
  const modelDeletion = useModelDeletion({ operation: dataOperation, locked: captureTest.blocked || navigationBusy || dataPending || deletion.protectedNow || audioDeletion.protectedNow || captureBusy || closingBusy || recordingDecision.visible || recordingDecision.requestBusy || state.kind !== "ready" || !["idle", "error"].includes(state.snapshot.phase), withSavedEdits,
    onDeleted: async () => { setModelRefreshEpoch(value => value + 1); } });
  const operationNavigationBlocked = recordingDecision.visible || recordingDecision.requestBusy || deletion.protectedNow || audioDeletion.protectedNow || modelDeletion.protectedNow || !dataOperation.connected || !!dataOperation.snapshot?.busy;
  const operationNavigationLocked = operationNavigationBlocked || navigationBusy || dataPending;
  const baseNavigationLocked=operationNavigationLocked||captureTest.blocked;
  const deletionBlocked = !!dataOperation.snapshot?.modelDeletionPreview?.pending || !!dataOperation.deletionListing?.writeBlocked || !!dataOperation.snapshot?.deletionPreview?.pending || !!dataOperation.modelDeletionListing?.writeBlocked || !!dataOperation.audioDeletionListing?.writeBlocked || !!dataOperation.snapshot?.audioDeletionPreview?.pending;
  const baseWriteLocked = baseNavigationLocked || deletionBlocked || !!dataOperation.snapshot?.restartRequired;
  const operationWriteLocked=operationNavigationLocked||deletionBlocked||!!dataOperation.snapshot?.restartRequired;
  const startConfirm = useActionConfirmation(baseWriteLocked || closingBusy);
  const closeConfirm = useActionConfirmation(false);
  const navigationLocked = baseNavigationLocked || captureBusy || closingBusy || startConfirm.pending;
  const writeLocked = baseWriteLocked || captureBusy || closingBusy || startConfirm.pending;
  const navigationArrow = quietNavigation && !operationNavigationBlocked && dataProtection !== "operation"
    && !captureTest.blocked && !captureBusy && !closingBusy && !startConfirm.pending;
  const writeNavigationArrow = navigationArrow && !deletionBlocked && !dataOperation.snapshot?.restartRequired;
  // Navigation saves and close confirmation temporarily lock writes without
  // cancelling background preparation. Explicit quit stops it in native shutdown.
  const modelPreparationStopRequested = recordingDecision.visible || recordingDecision.requestBusy || dataPending || deletion.protectedNow || audioDeletion.protectedNow || modelDeletion.protectedNow || !dataOperation.connected || !!dataOperation.snapshot?.busy || captureTest.blocked || deletionBlocked || !!dataOperation.snapshot?.restartRequired || captureBusy || startConfirm.pending;
  const publication = useAnalysisPublication(writeLocked, action => withSavedEdits(action, "transcription"));
  const { root, startRecording, selectRecordingMicrophone, startCaptureTest, cancelCaptureTest, finishCaptureTest, stopRecording, prepareNewRecording, confirmPreparationResult } = useCaptureActions({
    context: { selectedMeeting, preparation, screen, baseWriteLocked, captureAvailable, captureInputs, captureScope, captureTest, state },
    controller: captureController, captureOwner, closing, activeMicrophoneInput, microphoneSession, microphoneSelection,
    startConfirm, withSavedEdits: saveEdits, recordingDecision, navigationLocked, setPreparation, setSelectedMeeting, setRenameTitle, setScreen, setMeetings,
  });
  const goBack = async () => {
    const target = history.previous, from = history.current;
    if (!target || navigationLocked) return;
    await navigate(async () => {
      const route = target.value;
      const owner = route.meeting?.id ?? (route.preparation?.submitted ? route.preparation.id : null);
      const meeting = owner ? await invoke<Meeting | null>("get_meeting", { id: owner }) : route.meeting;
      if (owner && meeting?.id !== owner) throw Error("戻る先の会議がありません。会議一覧から選び直してください。");
      if (history.current?.key !== from?.key || !history.restore(target)) throw Error("画面が変更されています。戻る操作をやり直してください。");
      // Refresh persisted data, but retain local preparation fields. Returning
      // never creates a meeting, starts capture or replays settings operations.
      setSelectedMeeting(meeting); setRenameTitle(meeting?.title ?? "");
      setPreparation(owner ? null : route.preparation); setRestoredFilter(route.filter);

      setScreen(route.screen);
      updateHistory(value => value + 1);
    });
  };
  useWorkspaceClose({ platform, closing, starting, navigating, setClosingBusy, setMeetingError, captureTest, meetings, closeConfirm, withSavedEdits, recordingDecision });
  useEffect(() => {
    if (!isTauri()) return;
    void invoke<Meeting[]>("list_meetings").then(setMeetings).catch((error: unknown) => setMeetingError(String(error)));
    void loadOrigins();
    void invoke<boolean>("capture_available").then(setCaptureAvailable).catch(() => setCaptureAvailable(false));
    return () => { originRequest.current++; };
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
  const recoveredSessions = JSON.stringify(recovery.snapshot?.items.filter(item => item.phase === "succeeded").map(item => item.sessionId).sort() ?? []);
  useEffect(() => {
    if (!isTauri() || recoveredSessions === "[]") return;
    let active = true;
    // Polling reconciles completion even if recordings-recovered was missed.
    void invoke<Meeting[]>("list_meetings").then(items => { if (active) setMeetings(items); }).catch(error => { if (active) setMeetingError(String(error)); });
    if (selectedMeeting && recovery.snapshot?.items.some(item => item.meetingId === selectedMeeting.id && item.phase === "succeeded")) {
      void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then(item => { if (active) setSelectedMeeting(item); }).catch(error => { if (active) setMeetingError(String(error)); });
    }
    return () => { active = false; };
  }, [recoveredSessions, selectedMeeting?.id]);
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
  const selectMeeting = async (id: string) => {
    await navigate(async () => {
      const meeting = await invoke<Meeting | null>("get_meeting", { id });
      if (!meeting) throw new Error("会議が見つかりません");
      setSelectedMeeting(meeting);

      setRenameTitle(meeting.title);
      setScreen("overview");
    });
  };
  const openNotifiedMeeting = async (id: string) => {
    if (navigationLocked) return;
    await navigate(async () => {
      const meeting = await invoke<Meeting | null>("get_meeting", { id });
      if (!meeting || meeting.id !== id) throw Error("通知対象の会議が見つかりません。別の会議には切り替えていません。");
      setNotificationsOpen(false);
      setSelectedMeeting(meeting); setRenameTitle(meeting.title); setScreen("overview");

      requestAnimationFrame(() => document.querySelector<HTMLElement>('[aria-label="会議ワークスペース"]')?.focus({ preventScroll: false }));
    });
  };
  const openRecordingMeeting = async () => {
    const expected = state.kind === "ready" ? state.snapshot : null;
    if (!expected?.meetingId || !expected.sessionId || !recordingActive(expected) || navigationLocked) return;
    await navigate(async () => {
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
    await navigate(() => {
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
        void navigate(() => {
          if (root.current.selectedMeeting?.id !== owner) throw Error("検索対象の会議が変更されています。");
          setDetailTab("transcript");
          requestAnimationFrame(() => { const field = document.querySelector<HTMLInputElement>(".transcript-search input[type=search]"); field?.focus(); field?.select(); });
        }, "tab");
      }
    } else if (shortcut === "recording" && isTauri() && state.kind === "ready") {
      event.preventDefault();
      if (recordingActive(state.snapshot)) { void openRecordingMeeting(); return; }
      if (writeLocked || meetingBusy || navigating.current || starting.current || closing.current) return;
      void navigate(prepareNewRecording);
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
  const dataWorkActive = ["backup", "restore"].includes(dataOperation.snapshot?.kind ?? "") && !!dataOperation.snapshot?.busy;
  const showJobs = () => { if (dataWorkActive) setScreen("jobs"); else if (!navigationLocked) void navigate(() => setScreen("jobs")); };
  const showRestored = async (ids: string[]) => { await navigate(async () => { setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins(); setRestoredFilter(ids); setSelectedMeeting(null); setScreen("meetings"); }); };
  return (
    <MeetingPlaybackSurface host={playbackHost} meetingId={selectedMeeting?.id ?? null} title={selectedMeeting?.title}>
    <MeetingAudioProvider meetingId={selectedMeeting?.id ?? null} sourceType={selectedMeeting?.sourceType} locked={writeLocked}
      audioDeleted={selectedMeeting ? deletion.audioAvailability.rows.get(selectedMeeting.id)?.audioDeleted ?? null : null}
      active={!!selectedMeeting && !["settings", "recording", "jobs", "logs", "voices", "prompts"].includes(screen) && ["recorded", "imported", "archived"].includes(selectedMeeting.lifecycle) && !!selectedMeeting.sourceType}>
    <div className="app-layout" data-navigation-arrow={navigationArrow || undefined} data-write-navigation-arrow={writeNavigationArrow || undefined}>
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark" aria-hidden="true"><img src={appIconUrl} alt="" /></span><span className="brand-name">{appName}</span></div>
        <nav aria-label="メインナビゲーション">
          <button type="button" className="new-recording-nav" data-screen-navigation={isTauri() && state.kind === "ready" && ["idle", "error"].includes(state.snapshot.phase) ? "write" : undefined} aria-label="新しい録音" title="新しい録音" disabled={!isTauri() || writeLocked || state.kind !== "ready" || !["idle","error"].includes(state.snapshot.phase)} onClick={() => void navigate(prepareNewRecording)}><span className="nav-glyph" aria-hidden="true">＋</span><span className="nav-label">新しい録音</span></button>
          <button type="button" aria-label="会議一覧" title="会議一覧" aria-current={screen === "meetings" || screen === "overview" ? "page" : undefined} className={`nav-item ${screen === "meetings" || screen === "overview" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => { setSelectedMeeting(null);  setRestoredFilter(null); setScreen("meetings"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="table" /></span><span className="nav-label">会議一覧</span></button>
          <button type="button" aria-label="ジョブ一覧" title="ジョブ一覧" aria-current={screen === "jobs" ? "page" : undefined} className={`nav-item ${screen === "jobs" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked && !dataWorkActive} onClick={showJobs}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="list" /></span><span className="nav-label">ジョブ一覧</span><JobCount /></button>
          <button type="button" aria-label="ボイス一覧" title="ボイス一覧" aria-current={screen === "voices" ? "page" : undefined} className={`nav-item ${screen === "voices" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => {  setScreen("voices"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="voice" /></span><span className="nav-label">ボイス一覧</span></button>
          <button type="button" aria-label="プロンプト" title="プロンプト" aria-current={screen === "prompts" ? "page" : undefined} className={`nav-item ${screen === "prompts" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => {  setScreen("prompts"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="edit" /></span><span className="nav-label">プロンプト</span></button>
          <button type="button" aria-label="内部ログ" title="内部ログ" aria-current={screen === "logs" ? "page" : undefined} className={`nav-item ${screen === "logs" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => {  setScreen("logs"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="logs" /></span><span className="nav-label">内部ログ</span></button>
          <button type="button" aria-label="設定" title="設定" aria-current={screen === "settings" ? "page" : undefined} className={`nav-item ${screen === "settings" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => setScreen("settings"))}><span className="nav-glyph settings-nav-glyph" aria-hidden="true"><ActionIcon name="gear" /></span><span className="nav-label">設定</span></button>
        </nav>
      </aside>
      <div className="main-column">
        <ResidencyNotifications scope="main"/>
        <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError}/>
        <IssueNotice scope="main" type="recording-command" label="録音の操作" message={captureError}/>
        <IssueNotice scope="main" type="recording-confirmation" label="録音終了の確認" message={recordingDecision.error}/>
        <RecorderOperationNotifications />
        <RecordingRecoveryNotifications snapshot={recovery.snapshot} error={recovery.error} />
        <RecordingInputNotice windowLabel="main"/>
        <ThemeNotifications locked={writeLocked} />
        <ToastNotice scope={state.kind === "ready" ? state.snapshot.sessionId ?? "recording" : "recording"} type="recording-failure" level="error" message={state.kind === "ready" ? state.snapshot.errorMessage || (recordingDecision.snapshot?.phase === "failed" ? recordingDecision.snapshot.errorMessage : "") : ""} />

        <header className="topbar"><nav className="screen-navigation" aria-label="画面の移動">{history.previous && <IconButton label="前の画面へ戻る" icon="previous" data-screen-navigation disabled={navigationLocked} onClick={() => void goBack()} />}{screen !== "meetings" && <><button type="button" className="breadcrumb-button" data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => { setSelectedMeeting(null);  setRestoredFilter(null); setScreen("meetings"); })}>会議一覧</button><span aria-hidden="true">/</span></>}<span aria-current="page">{detail ? "会議詳細" : screen === "recording" ? preparation ? "新しい録音" : "録音" : screen === "overview" || screen === "meetings" ? "会議一覧" : screen === "jobs" ? "ジョブ一覧" : screen === "logs" ? "内部ログ" : screen === "voices" ? "ボイス一覧" : screen === "prompts" ? "プロンプト" : "設定"}</span></nav><div className="topbar-actions"><div className="header-recording" aria-label="共通録音状態"><span className="header-status"><span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} />{phaseLabel(state)}{state.kind === "ready" && recordingActive(state.snapshot) && screen!=="recording" && <span className="header-timer">{formatElapsed(state.snapshot.elapsedMs)}</span>}</span>{state.kind === "ready" && recordingActive(state.snapshot) && screen!=="recording" && <button type="button" className="small-secondary" data-screen-navigation disabled={navigationLocked} onClick={() => void openRecordingMeeting()}>録音画面へ</button>}{state.kind === "ready" && ["preparing", "recording"].includes(state.snapshot.phase) && screen!=="recording" && <button type="button" className="stop-button" disabled={captureBusy || navigationBusy || recordingDecision.requestBusy || recordingDecision.visible || !recordingDecision.connected} onClick={event => void stopRecording(event.currentTarget)}>録音を終了</button>}</div><NotificationPopover open={notificationsOpen} onOpenChange={setNotificationsOpen}><NotificationList visible={notificationsOpen} onOpen={openNotifiedMeeting} locked={navigationLocked}/></NotificationPopover></div><ToastHost docked={detail}/></header>
        <CommonOperationProgress onOpenJobs={showJobs} />
        {detail && <div className="meeting-detail-status"><div><div className="meeting-title-row"><h1 title={selectedMeeting!.title}>{selectedMeeting!.title}</h1><IconButton ref={renameOpener} label="会議名を変更" icon="edit" disabled={writeLocked || meetingBusy} onClick={() => { setRenameTitle(selectedMeeting!.title); setRenameOpen(true); }} /></div><p className="lead">作成 {new Date(selectedMeeting!.createdAt).toLocaleString("ja-JP")}</p></div><IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} /></div>}
        <AppUpdateBanner />
        {detail && <div className="meeting-detail-navigation"><div className="meeting-detail-tabs" role="tablist" aria-label="会議詳細">
          {(["transcript", "minutes"] as const).map(tab => <button key={tab} type="button" role="tab" id={`meeting-tab-${tab}`} aria-controls={`meeting-panel-${tab}`} aria-selected={detailTab === tab} tabIndex={detailTab === tab ? 0 : -1} data-screen-navigation disabled={navigationLocked} onClick={() => void switchDetailTab(tab)} onKeyDown={event => {
            if (event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return;
            const target = event.key === "Home" ? "transcript" : event.key === "End" ? "minutes" : ["ArrowLeft", "ArrowRight"].includes(event.key) ? tab === "transcript" ? "minutes" : "transcript" : null;
            if (target) { event.preventDefault(); document.getElementById(`meeting-tab-${target}`)?.focus(); }
          }}>{tab === "transcript" ? "文字起こし" : "議事録"}</button>)}
        </div><div className="meeting-detail-actions"><RowMenu label="会議の操作" disabled={navigationLocked}><MeetingDeletionMenu recoveryRunning={!recovery.error && recoveryMeetingState(recovery.snapshot, selectedMeeting!.id)?.phase === "running"} meeting={selectedMeeting!} deletion={deletion} audioDeletion={audioDeletion} onOpenJobs={showJobs} onOpenRecording={() => void openRecordingMeeting()} /></RowMenu></div></div>}
        <div className="workspace-surface">
        <main className={`content${detail ? " meeting-detail-content" : ""}`} aria-label="会議ワークスペース" tabIndex={-1}>
          <JobNotifications locked={navigationLocked} onOpen={openNotifiedMeeting} />
          <ArchivedInputNotices meetingIds={meetings.map(meeting=>meeting.id)} locked={navigationLocked} paused={!dataOperation.connected||dataPending||!!dataOperation.snapshot?.busy||dataOperation.snapshot?.phase==="ready"||!!dataOperation.snapshot?.restartRequired||deletionBlocked||closingBusy} onOpen={openNotifiedMeeting}/>
          {!detail && (screen === "overview" || screen === "meetings") && <div className="page-heading"><h1>会議一覧</h1></div>}
          <AudioImportPanel overlayHost={audioDropHost} visible={(screen==="overview" || screen==="meetings") && !detail} showNewRecording={false} locked={writeLocked} recordingActive={state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)} withSavedEdits={withSavedEdits} onNewRecording={prepareNewRecording} onSaved={async(id)=>{
            await flushPendingEdits();
            const [meeting,items]=await Promise.all([invoke<Meeting|null>("get_meeting",{id}),invoke<Meeting[]>("list_meetings")]);
            if(!meeting)throw Error("保存した会議が見つかりません。");
            setSelectedMeeting(meeting);setRenameTitle(meeting.title);setMeetings(items);setScreen("overview");
          }}/>
          {screen === "settings" && <div className="page-heading heading-with-info"><h1>設定</h1><InfoPopover label="設定">録音環境、ローカルAI、表示を管理します。</InfoPopover></div>}
          {screen === "settings" && <ThemeSettingsPanel locked={writeLocked} />}
          <ModelInspectionPanel deletion={modelDeletion} refreshEpoch={modelRefreshEpoch} visible={screen === "settings"} locked={writeLocked} stopRequested={modelPreparationStopRequested} recordingActive={state.kind!=="ready" || !["idle","error"].includes(state.snapshot.phase)} />
          {screen === "prompts" && <div className="page-heading heading-with-info"><h1>プロンプト</h1><InfoPopover label="プロンプト">議事録の作成・統合に使うプロンプトと共通用語を管理します。</InfoPopover></div>}
          <MinutesPromptSettings visible={screen === "prompts"} locked={writeLocked} />
          {screen === "prompts" && <VocabularyEditor locked={writeLocked} />}
          <DataSavePanel operation={dataOperation} visible={screen === "settings"} locked={captureTest.blocked || navigationBusy || deletion.protectedNow || audioDeletion.protectedNow || modelDeletion.protectedNow || deletionBlocked} withSavedEdits={withSavedEdits} onProtectionChange={setDataProtection} onRestored={async () => { setMeetings(await invoke<Meeting[]>("list_meetings")); await loadOrigins(); }} onShowMeetings={showRestored} />
          {screen === "settings" && <SettingsFooter />}
          <ModelDeletionDialog operation={dataOperation} control={modelDeletion} />
          <MeetingDeletionPanel operation={dataOperation} control={deletion} />
          <AudioDeletionPanel operation={dataOperation} control={audioDeletion} meetingTitle={meetings.find(m => m.id === audioDeletion.preview?.meetingId)?.title} />
          {screen === "settings" ? null : screen === "prompts" ? null : screen === "voices" ? <VoicePeopleSettings standalone locked={writeLocked} /> : screen === "logs" ? <InternalLogs /> : screen === "jobs" ? <JobList data={dataOperation} publicationReadStatus={publication.readStatus} onOpenData={() => setScreen("settings")} onShowRestored={showRestored} locked={navigationLocked} onOpenMeeting={openNotifiedMeeting} onOpenModels={() => { if (!navigationLocked) void navigate(() => setScreen("settings")); }} /> : screen==="recording" ? <>
            <RecordingWorkspace preparation={preparation} title={selectedMeeting?.title??preparation?.title??"録音対象を確認してください"} state={state} owner={captureOwner} locked={writeLocked}
              canStart={isTauri()&&captureAvailable&&!!captureInputs.selected&&!!captureScope.scope&&!writeLocked&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase)&&(preparation?validRecordingPreparation(preparation):selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null)}
              canStop={!captureBusy&&!navigationBusy&&!recordingDecision.requestBusy&&!recordingDecision.visible&&recordingDecision.connected&&state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)}
              onTitle={title=>setPreparation(current=>current&&!current.submitted?{...current,title}:current)} onStart={()=>void startRecording()} onStop={source=>void stopRecording(source)}
              onSettings={()=>void navigate(()=>setScreen("settings"))} onConfirmResult={()=>void confirmPreparationResult()}
              inputs={captureAvailable?<CaptureInputSelection control={captureInputs} scopeControl={captureScope} locked={writeLocked||!!preparation?.submitted||state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)}/>:<p>録音ヘルパーを利用できません。アプリのインストール状態と内部ログを確認してください。</p>}
              activeMicrophone={captureAvailable&&activeMicrophoneInput?<ActiveMicrophoneSelection control={captureInputs} locked={writeLocked||!captureInputs.inputs} onSelect={value=>void selectRecordingMicrophone(value)} status={microphoneSelection} pending={microphoneSelection?.source==="pending"||microphoneRequest?.owner===microphoneSession&&(!microphoneSelection||microphoneSelection.requestRevision<microphoneRequest.revision)}/>:null}
              vocabulary={preparation?<RecordingVocabulary key={preparation.id} items={preparation.meetingVocabulary} locked={writeLocked||preparation.submitted}
                onChange={meetingVocabulary=>setPreparation(current=>current&&!current.submitted?{...current,meetingVocabulary}:current)}/>:selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null?<VocabularyEditor key={selectedMeeting.id} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} locked={writeLocked}/>:null}
              test={captureAvailable?<CaptureTestPanel operationError={captureTestError} control={captureTest} currentUid={captureInputs.uid} currentScope={captureScope.scope}
                canStart={!!captureInputs.selected&&!!captureScope.scope&&!captureTest.blocked&&!operationWriteLocked&&!preparation?.submitted&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase)}
                locked={operationWriteLocked||captureBusy||closingBusy||startConfirm.pending} onStart={startCaptureTest} onCancel={()=>void cancelCaptureTest()} onFinish={finishCaptureTest}/>:null}
              notices={<><IssueNotice scope="main" type="recording-command" label="録音の操作" message={captureError}/><IssueNotice scope="main" type="recording-confirmation" label="録音終了の確認" message={recordingDecision.error}/><IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError}/></>}/>
          </> : <>
          {selectedMeeting ? <AnalysisPanel actionsHost={transcriptActionsHost} key={selectedMeeting.id} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} visible={detail && detailTab === "transcript"} hasTranscript={transcriptMeeting === selectedMeeting.id} recorded={["recorded","imported"].includes(selectedMeeting.lifecycle)} audioDeleted={deletion.audioAvailability.rows.get(selectedMeeting.id)?.audioDeleted ?? null} recordingActive={state.kind !== "ready" || state.snapshot.phase !== "idle"} locked={writeLocked} withSavedEdits={action => withSavedEdits(action, "transcription")} onReplaceGenerated={publication.replace} publicationReadStatus={publication.readStatus} onHeadChanged={async read => {
            setAnalysisEpoch((value) => value + 1);
            const [meeting, items] = await Promise.all([
              read("get_meeting", () => invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id })),
              read("list_meetings", () => invoke<Meeting[]>("list_meetings")),
            ]);
            setSelectedMeeting(meeting); setMeetings(items);
          }} /> : null}
          <section className="meetings-card" aria-label="保存済み会議" hidden={detail}>
            <div className="card-head"><input type="search" className="meeting-search" aria-label="会議名を検索" placeholder="会議名を検索" value={meetingQuery} onChange={event => setMeetingQuery(event.target.value)} /><IconButton label="再読込" icon="refresh" onClick={reloadMeetings} disabled={!isTauri() || writeLocked} /></div>
            <ToastNotice scope="main" type="recording-recovered" message={recoveredCount > 0 ? `前回の音声保存 ${recoveredCount} 件を検証して復旧しました。` : ""} onClose={() => setRecoveredCount(0)} />
            {!detail && <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError} />}
            {restoredFilter && <p className="meeting-list-notice">今回復元した会議を表示しています。<button type="button" className="small-secondary" data-screen-navigation="write" disabled={writeLocked} onClick={() => void navigate(() => setRestoredFilter(null))}>すべての会議を表示</button></p>}
            {shownMeetings.length === 0 ? <p className="empty-meetings">{meetingQuery ? "条件に合う会議はありません。" : restoredFilter ? "対象の復元会議は現在の一覧にありません。" : "保存済み会議はありません。"}</p> : <div className="meeting-table-frame"><table className="meeting-table"><thead><tr><th scope="col">会議名</th><th scope="col">作成日時</th><th scope="col">状態</th><th scope="col"><span className="sr-only">操作</span></th></tr></thead><tbody>{shownMeetings.map((meeting) => <tr key={meeting.id}>
              <td><button type="button" className="meeting-name" data-screen-navigation="write" onClick={() => selectMeeting(meeting.id)} disabled={writeLocked}>{meeting.title}</button>{restoredIds.includes(meeting.id) && <small className="restore-badge">バックアップから復元</small>}</td>
              <td className="meeting-date">{new Date(meeting.createdAt).toLocaleString("ja-JP")}</td>
              <td><span className="state-pill">{meetingStatusLabel(meeting, state, recovery.snapshot)}</span>{!["recorded", "imported"].includes(meeting.lifecycle) && recoveryMeetingState(recovery.snapshot, meeting.id)?.error && <small role="status">回復できませんでした。録音は未完了のまま保持しています。</small>}{recovery.error && recoveryMeetingState(recovery.snapshot, meeting.id) && <small role="status">回復状態を確認できません。表示は前回取得した状態です。</small>}</td>
              <td><RowMenu label={`${meeting.title}の操作`} disabled={navigationLocked}><button type="button" role="menuitem" data-screen-navigation="write" onClick={() => selectMeeting(meeting.id)} disabled={writeLocked}>開く</button><MeetingDeletionMenu recoveryRunning={!recovery.error && recoveryMeetingState(recovery.snapshot, meeting.id)?.phase === "running"} meeting={meeting} deletion={deletion} audioDeletion={audioDeletion} onOpenJobs={showJobs} onOpenRecording={() => void openRecordingMeeting()} />{dataOperation.deletionListing?.deletingMeetingIds.includes(meeting.id) && <span role="status">削除中・要確認</span>}</RowMenu></td>
            </tr>)}</tbody></table></div>}
          </section>
          {selectedMeeting && <div className="workspace-tab-panel" id="meeting-panel-transcript" role="tabpanel" aria-labelledby="meeting-tab-transcript" hidden={detailTab !== "transcript"}>
            <div className="card-head transcript-primary-heading">
              <h2>{transcriptMeeting === selectedMeeting.id ? "発言一覧" : "文字起こし"}</h2>
              <div className="plain-minutes-primary-actions transcript-primary-actions">
                <div ref={setTranscriptEditorActionsHost} className="transcript-editor-actions" />
                <VocabularyEditor compact key={selectedMeeting.id} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} locked={writeLocked} />
                <div ref={setTranscriptActionsHost} className="transcript-generation-host" />
              </div>
            </div>
            <TranscriptEditor actionsHost={transcriptEditorActionsHost} key={selectedMeeting.id} sourceEpoch={analysisEpoch} meetingId={selectedMeeting.id} meetingTitle={selectedMeeting.title} locked={writeLocked} playbackHost={playbackHost} visible={detailTab === "transcript"} onAnalysisLoaded={present => setTranscriptMeeting(present ? selectedMeeting.id : null)} onConsolidated={async()=>{
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
        <div ref={setAudioDropHost} className="audio-drop-host" />
        </div>
        <div ref={setPlaybackHost} className="meeting-playback-dock" role="region" aria-label="会議の共通音声プレーヤー" hidden={!selectedMeeting || screen === "settings" || screen === "recording" || screen === "jobs" || screen === "logs" || screen === "voices" || screen === "prompts"} />
      </div>
    </div>
    <CommonModal open={renameOpen && !!selectedMeeting} blocked={meetingBusy} onDismiss={() => setRenameOpen(false)} restoreTarget={() => renameOpener.current} className="meeting-rename-dialog" aria-labelledby="meeting-rename-title" dismissLabel="閉じる">
      <h2 id="meeting-rename-title">会議名を変更</h2>
      <form onSubmit={renameMeeting}>
        <label htmlFor="rename-title">会議名<input id="rename-title" value={renameTitle} maxLength={200} onChange={event => setRenameTitle(event.target.value)} disabled={meetingBusy || writeLocked} /></label>
        <div className="dialog-actions"><button type="button" className="secondary" disabled={meetingBusy} onClick={() => setRenameOpen(false)}>キャンセル</button><button type="submit" disabled={meetingBusy || writeLocked || !renameTitle.trim()}>保存</button></div>
      </form>
    </CommonModal>
    <ModelDownloadGuidance locked={writeLocked} onOpenModels={async () => { await navigate(() => setScreen("settings")); }} />
    </MeetingAudioProvider>
    </MeetingPlaybackSurface>
  );
}

export function MainDesktopWindow() {
  return <AppUpdateProvider><RecorderOperationProvider><RecordingInputProvider windowLabel="main"><SettingsModalScope><MainWindow /></SettingsModalScope><ConfirmationHost /></RecordingInputProvider></RecorderOperationProvider></AppUpdateProvider>;
}
