import { useCaptureActions, useCaptureActionState, type CaptureMeeting as Meeting } from "./capture-actions";
import { useWorkspaceClose } from "./workspace-close";
import { closeInfoPopovers } from "./info-popover";
import { Callout } from "./callout";
import { ModelDeletionDialog, useModelDeletion } from "./model-deletion";
import { AppUpdateBanner, AppUpdateProvider } from "./app-updates";
import appIconUrl from "../src-tauri/icons/icon.svg";
import { usePlatform } from "./platform";
import { ResidencyNotifications } from "./residency-notifications";
import { invoke, isTauri } from "./ipc.ts";
import { SettingsModalScope } from "./settings-modal-scope";
import { useAnalysisPublication } from "./analysis-publication";
import { ActionIcon, IconButton } from "./action-icon";
import { MeetingDeletionMenu } from "./meeting-deletion-menu";
import { NotificationPopover } from "./notification-popover";
import { ScreenHistory } from "./screen-history";
import { ConfirmationHost } from "./confirmation-host";
import { IssueNotice, NotificationList, ToastHost, ToastNotice } from "./toast";
import { JobNotifications } from "./job-notifications";
import { RecorderOperationProvider, RecorderOperationNotifications } from "./recorder-operation-notices";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useLayoutEffect, useRef, useState, type FormEvent } from "react";
import { formatElapsed, useRecordingSession } from "./session";
import { phaseLabel } from "./recording-phase";
import { meetingStatusLabel } from "./meeting-status";
import { meetingDiagnosticPresentation, meetingDiagnosticSummary } from "./meeting-diagnostics";
import { backgroundReadFailure } from "./background-read-failure";
import { flushPendingEdits } from "./draft-save";
import { useSavedNavigation } from "./saved-navigation";
import { AudioImportPanel } from "./audio-import";
import { useModelCleanup } from "./model-cleanup";
import { ModelDownloadGuidance } from "./model-download-guidance";
import { ModelRuntimeGuidance } from "./model-runtime-guidance";
import { CommonOperationProgress, usePublishProgress } from "./operation-progress";
import { RecordingRecoveryNotifications, RecordingSaveRetry, useRecordingRecovery } from "./recording-recovery";
import { recoveryMeetingState, recordingSaveJobs } from "./recording-recovery-state";
import { deletionProgress, dataJobProgress, modelCleanupProgress } from "./operation-progress-state";
import { JobCount } from "./job-list";
import { useDataOperation } from "./data-operation";
import { DataSavePanel } from "./data-save";
import { MeetingDeletionPanel, useMeetingDeletion } from "./meeting-deletion";
import { AudioDeletionPanel, useAudioDeletion } from "./audio-deletion";
import type { RecordingSnapshot } from "./session";
import "./minutes-markdown-editor.css";
import "./style.css";
import "./prototype-alignment.css";
import "./recorder-panel.css";
import { ThemeNotifications } from "./theme";
import { useRecordingConfirmation } from "./recording-confirmation";
import { confirmations, useActionConfirmation } from "./confirmation-service";
import { appShortcut, overlayOpen } from "./keyboard";
import { MeetingPlaybackSurface } from "./playback-surface";
import { MeetingAudioProvider } from "./meeting-audio";
import {ActiveMicrophoneSelection,CaptureInputSelection,useCaptureInputs} from "./capture-inputs";
import { RecordingShell } from "./main-shells/recording-shell";
import { LibraryPanels, LibraryShell } from "./main-shells/library-shell";
import { MeetingEditorHeader, MeetingEditorShell, MeetingRenameDialog, type DetailTab } from "./main-shells/meeting-editor-shell";
import {CaptureTestPanel,useCaptureTest} from "./capture-test";
import {RecordingInputProvider} from "./recording-input-status";
import {validRecordingPreparation,type RecordingPreparation} from "./recording-preparation-state";
import {useRecordingMicrophoneSelection} from "./microphone-selection-status";
import {useCaptureScope} from "./capture-applications";
import { assertCloseRecording, recordingActive } from "./application-close";
import { useAppName } from "./window-environment";
import { useMeetingList } from "./meeting-list";
import { StorageRecoveryNotice } from "./salvage";

function MainWindow() {
  const appName = useAppName();
  const platform = usePlatform();
  const state = useRecordingSession();
  const recordingBlocksPanelActions = state.kind !== "ready" || recordingActive(state.snapshot);
  const recordingDecision = useRecordingConfirmation("main");
  const dataOperation = useDataOperation();
  const modelCleanup = useModelCleanup();
  const modelCleanupActive = modelCleanup.active !== null;
  const [modelProgressOwnerReady, setModelProgressOwnerReady] = useState(false);
  // Cleanup can start in the background before the Settings bundle loads.
  usePublishProgress("initial-model-cleanup", [modelCleanup.active && !modelProgressOwnerReady ? modelCleanupProgress(modelCleanup.active) : null]);
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
  const meetingList = useMeetingList();
  const { meetings, refresh: refreshMeetings } = meetingList;
  const writeStateUnverified = isTauri() && dataOperation.writeState === null;
  const storageReadOnly = dataOperation.writeState?.readOnly === true;
  const savingRecording = state.kind === "ready" ? state.snapshot : null;
  usePublishProgress("recording-save", recordingSaveJobs(savingRecording, recovery.snapshot, meetings.find(meeting => meeting.id === savingRecording?.meetingId)?.title));
  const [meetingQuery, setMeetingQuery] = useState("");
  const [meetingError, setMeetingError] = useState("");
  const [meetingBusy, setMeetingBusy] = useState(false);
  const [selectedMeeting, setSelectedMeeting] = useState<Meeting | null>(null);
  const selectedDiagnostics = selectedMeeting ? meetingList.byMeeting.get(selectedMeeting.id) : undefined;
  const selectedNotices = selectedDiagnostics?.filter(item => item.reason !== "audio_recovery");
  const selectedDiagnosticSummary = meetingDiagnosticSummary(selectedNotices);
  const selectedProtected = !!selectedMeeting && (
    !!selectedDiagnostics?.some(item => ["meeting_row_invalid", "deletion_pending"].includes(item.reason)) ||
    !!dataOperation.deletionListing?.deletingMeetingIds.includes(selectedMeeting.id) ||
    (dataOperation.snapshot?.deletionPreview?.pending === true && dataOperation.snapshot.deletionPreview.meetingId === selectedMeeting.id)
  );
  const selectedAudioUnavailable = !!selectedDiagnostics?.some(item => ["audio_recovery", "audio_deletion_pending", "audio_unavailable", "audio_missing", "audio_unreadable"].includes(item.reason)) ||
    (!!selectedMeeting && dataOperation.snapshot?.audioDeletionPreview?.pending === true && dataOperation.snapshot.audioDeletionPreview.meetingId === selectedMeeting.id);
  const [preparation,setPreparation]=useState<RecordingPreparation|null>(null);
  const [screen, setScreen] = useState<"overview" | "meetings" | "settings" | "recording" | "jobs" | "logs" | "voices" | "prompts">("meetings");
  const [detailTab, setDetailTab] = useState<DetailTab>("transcript");
  const [minutesOwner, setMinutesOwner] = useState<string | null>(null);
  const [history] = useState(() => new ScreenHistory<{ screen: typeof screen; meeting: Meeting | null; preparation: RecordingPreparation | null; filter: string[] | null }>());
  const [, updateHistory] = useState(0);
  const routeKey = `${screen}:${screen === "overview" || screen === "recording" ? selectedMeeting?.id ?? preparation?.id ?? "" : ""}`;
  useLayoutEffect(() => {
    if (history.visit(routeKey, { screen, meeting: selectedMeeting, preparation, filter: restoredFilter })) { closeInfoPopovers(); updateHistory(value => value + 1); }
  });
  const [notificationsOpen, setNotificationsOpen] = useState(false);
  useLayoutEffect(() => { closeInfoPopovers(); }, [detailTab]);
  useLayoutEffect(() => { setDetailTab("transcript"); setMinutesOwner(null); }, [selectedMeeting?.id]);
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
  const { busy: navigationBusy, quietNavigation, running: navigating, withSavedEdits: saveEdits } = useSavedNavigation(setMeetingError);
  // Navigation and regeneration cannot leave a completed test holding the
  // native admission gate. Capture actions own their own confirmation barrier.
  const withSavedEdits: typeof saveEdits = (action, reason, screenNavigation) => saveEdits(async () => {
    await captureTest.finish();
    await action();
  }, reason, screenNavigation);
  const navigate = (action: () => void | Promise<void>, reason: "leave" | "tab" = "leave") =>
    withSavedEdits(action, reason, true);
  const openSettings = () => navigate(() => setScreen("settings"));
  const openMeetingList = () => navigate(() => {
    setSelectedMeeting(null); setRestoredFilter(null); setScreen("meetings");
  });
  const setOverviewMeeting = (meeting: Meeting) => {
    setSelectedMeeting(meeting); setRenameTitle(meeting.title); setScreen("overview");
  };
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
  const [verifiedTranscript, setVerifiedTranscript] = useState<string | null>(null);
  const priorPhase = useRef<string | null>(null);
  const [deletionMenuMeeting, setDeletionMenuMeeting] = useState<string | null>(null);
  const trackDeletionMenu = (id: string, open: boolean) => {
    setDeletionMenuMeeting(current => open ? id : current === id ? null : current);
  };
  const deletion = useMeetingDeletion({ operation: dataOperation, detailId: screen === "overview" ? selectedMeeting?.id ?? null : null, menuId: deletionMenuMeeting,
    recordingContext: JSON.stringify([savingRecording?.phase, savingRecording?.meetingId, recovery.snapshot?.phase, modelCleanupActive]),
    locked: storageReadOnly || writeStateUnverified || captureTest.blocked || navigationBusy || dataPending || audioDeletionPending, withSavedEdits,
    onDeleted: async receipt => {
      if (selectedMeeting?.id === receipt.meetingId) {
        setSelectedMeeting(null);
        setScreen("meetings");
      }
      await refreshMeetings(); await loadOrigins();
    } });
  const audioDeletion = useAudioDeletion({ operation: dataOperation, availability: deletion.audioAvailability, locked: storageReadOnly || writeStateUnverified || captureTest.blocked || navigationBusy || dataPending || deletion.protectedNow, withSavedEdits,
    onProtectionChange: setAudioDeletionPending, onDeleted: async () => {
      // Audio availability changes, not document heads. Keep mounted editors and
      // their local buffers, including when recovery deliberately skipped writes.
      await refreshMeetings();
    } });
  const [modelRefreshEpoch, setModelRefreshEpoch] = useState(0);
  const [modelRuntimeEpoch, setModelRuntimeEpoch] = useState(0);
  const modelDeletion = useModelDeletion({ operation: dataOperation, locked: writeStateUnverified || captureTest.blocked || navigationBusy || dataPending || deletion.protectedNow || audioDeletion.protectedNow || captureBusy || closingBusy || recordingDecision.visible || recordingDecision.requestBusy || state.kind !== "ready" || !["idle", "error"].includes(state.snapshot.phase), withSavedEdits,
    onDeleted: async () => { setModelRefreshEpoch(value => value + 1); } });
  const operationNavigationBlocked = recordingDecision.visible || recordingDecision.requestBusy || deletion.protectedNow || audioDeletion.protectedNow || modelDeletion.protectedNow || !dataOperation.connected || !!dataOperation.snapshot?.busy;
  const operationNavigationLocked = operationNavigationBlocked || navigationBusy || dataPending;
  const baseNavigationLocked=operationNavigationLocked||captureTest.blocked;
  const confirmedWriteBlocked = storageReadOnly || !!dataOperation.deletionListing?.writeBlocked || !!dataOperation.modelDeletionListing?.writeBlocked || !!dataOperation.audioDeletionListing?.writeBlocked;
  const deletionBlocked = confirmedWriteBlocked || writeStateUnverified;
  const writeProtected = deletionBlocked || !!dataOperation.snapshot?.restartRequired;
  const baseWriteLocked = baseNavigationLocked || writeProtected;
  const operationWriteLocked = operationNavigationLocked || writeProtected;
  const startConfirm = useActionConfirmation(baseWriteLocked || closingBusy);
  const closeConfirm = useActionConfirmation(false);
  const navigationLocked = baseNavigationLocked || captureBusy || closingBusy || startConfirm.pending;
  const writeLocked = navigationLocked || writeProtected;
  const selectedWriteLocked = writeLocked || selectedProtected;
  const navigationArrow = quietNavigation && !operationNavigationBlocked && dataProtection !== "operation"
    && !captureTest.blocked && !captureBusy && !closingBusy && !startConfirm.pending;
  const writeNavigationArrow = navigationArrow && !deletionBlocked && !dataOperation.snapshot?.restartRequired;
  // Navigation saves and close confirmation temporarily lock writes without
  // cancelling background preparation. Explicit quit stops it in native shutdown.
  const modelPreparationStopRequested = recordingDecision.visible || recordingDecision.requestBusy || dataPending || deletion.protectedNow || audioDeletion.protectedNow || modelDeletion.protectedNow || !dataOperation.connected || !!dataOperation.snapshot?.busy || captureTest.blocked || confirmedWriteBlocked || !!dataOperation.snapshot?.restartRequired || captureBusy || startConfirm.pending;
  const publication = useAnalysisPublication(writeLocked, action => withSavedEdits(action, "transcription"));
  const { root, startRecording, selectRecordingMicrophone, startCaptureTest, cancelCaptureTest, finishCaptureTest, stopRecording, prepareNewRecording, confirmPreparationResult } = useCaptureActions({
    context: { selectedMeeting, preparation, screen, baseWriteLocked: baseWriteLocked || (screen === "recording" && selectedProtected), modelCleanupActive, reconcileModelCleanup: modelCleanup.reconcile, captureAvailable, captureInputs, captureScope, captureTest, state },
    controller: captureController, captureOwner, closing, activeMicrophoneInput, microphoneSession, microphoneSelection,
    startConfirm, withSavedEdits: saveEdits, recordingDecision, navigationLocked, setPreparation, setSelectedMeeting, setRenameTitle, setScreen, refreshMeetings,
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
    void refreshMeetings();
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
      void refreshMeetings();
      if (selectedMeeting) void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then(item => { if (active) setSelectedMeeting(item); }).catch(error => { if (active) setMeetingError(String(error)); });
    }).then(stop => { if (active) stopCleanup = stop; else stop(); }).catch(error => { if (active) setMeetingError(String(error)); });
    void listen<string[]>("recordings-recovered", (event) => {
      if (!active) return;
      void refreshMeetings();
      if (selectedMeeting && event.payload.some((id) => id === selectedMeeting.id)) {
        void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then((item) => { if (active) setSelectedMeeting(item); }).catch((error: unknown) => { if (active) setMeetingError(String(error)); });
      }
    }).then((stop) => {
      if (!active) { stop(); return; }
      unlisten = stop;
      // Also refresh after subscribing in case recovery finished before the listener existed.
      void refreshMeetings();
      if (selectedMeeting) void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then((item) => { if (active) setSelectedMeeting(item); }).catch((error: unknown) => { if (active) setMeetingError(String(error)); });
    }).catch((error: unknown) => { if (active) setMeetingError(String(error)); });
    return () => { active = false; unlisten?.(); stopCleanup?.(); };
  }, [selectedMeeting?.id]);
  const recoveredSessions = JSON.stringify(recovery.snapshot?.items.filter(item => item.phase === "succeeded").map(item => item.sessionId).sort() ?? []);
  useEffect(() => {
    if (!isTauri() || recoveredSessions === "[]") return;
    let active = true;
    // Polling reconciles completion even if recordings-recovered was missed.
    void refreshMeetings();
    if (selectedMeeting && recovery.snapshot?.items.some(item => item.meetingId === selectedMeeting.id && item.phase === "succeeded")) {
      void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then(item => { if (active) setSelectedMeeting(item); }).catch(error => { if (active) setMeetingError(String(error)); });
    }
    return () => { active = false; };
  }, [recoveredSessions, selectedMeeting?.id]);
  useEffect(() => {
    const phase = state.kind === "ready" ? state.snapshot.phase : null;
    if (priorPhase.current && priorPhase.current !== phase && (phase === "idle" || phase === "error") && isTauri()) {
      void refreshMeetings();
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
      setOverviewMeeting(meeting);
    });
  };
  const openNotifiedMeeting = async (id: string) => {
    if (navigationLocked) return;
    await navigate(async () => {
      const meeting = await invoke<Meeting | null>("get_meeting", { id });
      if (!meeting || meeting.id !== id) throw Error("通知対象の会議が見つかりません。別の会議には切り替えていません。");
      setNotificationsOpen(false);
      setOverviewMeeting(meeting);
      requestAnimationFrame(() => document.querySelector<HTMLElement>('[aria-label="会議ワークスペース"]')?.focus({ preventScroll: false }));
    });
  };
  const openRecordingMeeting = async (meetingId?: string) => {
    const expected = state.kind === "ready" ? state.snapshot : null;
    if (!expected?.meetingId || (meetingId && expected.meetingId !== meetingId) || !expected.sessionId || !recordingActive(expected) || navigationLocked) return;
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
  const switchDetailTab = async (tab: DetailTab) => {
    if (tab === detailTab || navigationLocked || overlayOpen()) return;
    const owner = selectedMeeting?.id;
    await navigate(() => {
      if (!owner || root.current.selectedMeeting?.id !== owner) throw Error("会議が変更されています。元のタブ切替は実行していません。");
      if (tab === "minutes") setMinutesOwner(owner);
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
    if (!selectedMeeting || !renameOpen || meetingBusy || selectedWriteLocked || !renameTitle.trim()) return;
    setMeetingBusy(true);
    setMeetingError("");
    try {
      const updated = await invoke<Meeting>("rename_meeting", { id: selectedMeeting.id, title: renameTitle, expectedLockVersion: selectedMeeting.lockVersion });
      setSelectedMeeting(updated);
      setRenameTitle(updated.title);
      await refreshMeetings();
      setRenameOpen(false);
    } catch (error) {
      setMeetingError(String(error));
    } finally {
      setMeetingBusy(false);
    }
  };
  const reloadMeetings = async () => {
    await withSavedEdits(async () => {
      await refreshMeetings();
      await loadOrigins();
      if (selectedMeeting) {
        const current = await invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id });
        setSelectedMeeting(current);
        setRenameTitle(current?.title ?? "");
      }
    });
  };
  const shownMeetings = meetings.filter(meeting => (!restoredFilter || restoredFilter.includes(meeting.id)) && meeting.title.toLocaleLowerCase("ja-JP").includes(meetingQuery.toLocaleLowerCase("ja-JP")));
  const meetingScreen = screen === "overview" || screen === "meetings";
  const detail = screen === "overview" && !!selectedMeeting;
  // Keep the data operation mounted and reveal its controls after the model panel above it.
  const settingsReady = screen === "settings" && modelProgressOwnerReady;
  const dataWorkActive = ["backup", "restore"].includes(dataOperation.snapshot?.kind ?? "") && !!dataOperation.snapshot?.busy;
  const showJobs = () => { if (dataWorkActive) setScreen("jobs"); else if (!navigationLocked) void navigate(() => setScreen("jobs")); };
  const deletionMenu = (meeting: Meeting) => {
    const live = !!savingRecording && savingRecording.meetingId === meeting.id && recordingActive(savingRecording);
    const recoveryItem = !recovery.error ? recoveryMeetingState(recovery.snapshot, meeting.id) : undefined;
    const savingCurrentSession = live && recovery.snapshot?.origin === "stop" && savingRecording?.sessionId === recoveryItem?.sessionId;
    const recoveryPhase = !savingCurrentSession && (recoveryItem?.phase === "running" || recoveryItem?.phase === "queued") ? recoveryItem.phase : undefined;
    return <MeetingDeletionMenu recoveryPhase={recoveryPhase} meeting={meeting} deletion={deletion} audioDeletion={audioDeletion} onOpenJobs={showJobs}
      onOpenRecording={live ? () => void openRecordingMeeting(meeting.id) : undefined} />;
  };
  const showRestored = async (ids: string[]) => { await navigate(async () => { await refreshMeetings(); await loadOrigins(); setRestoredFilter(ids); setSelectedMeeting(null); setScreen("meetings"); }); };
  return (
    <MeetingPlaybackSurface host={playbackHost} meetingId={selectedMeeting?.id ?? null} title={selectedMeeting?.title}>
    <MeetingAudioProvider meetingId={selectedMeeting?.id ?? null} sourceType={selectedMeeting?.sourceType} navigationLocked={navigationLocked} storageReadOnly={storageReadOnly} unavailable={selectedAudioUnavailable}
      audioDeleted={selectedMeeting ? deletion.audioAvailability.rows.get(selectedMeeting.id)?.audioDeleted ?? null : null}
      active={!!selectedMeeting && !["settings", "recording", "jobs", "logs", "voices", "prompts"].includes(screen) && ["recorded", "imported", "archived"].includes(selectedMeeting.lifecycle) && !!selectedMeeting.sourceType}>
    <div className="app-layout" data-navigation-arrow={navigationArrow || undefined} data-write-navigation-arrow={writeNavigationArrow || undefined}>
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark" aria-hidden="true"><img src={appIconUrl} alt="" /></span><span className="brand-name">{appName}</span></div>
        <nav aria-label="メインナビゲーション">
          <button type="button" className="new-recording-nav" data-screen-navigation={isTauri() && state.kind === "ready" && ["idle", "error"].includes(state.snapshot.phase) ? "write" : undefined} aria-label="新しい録音" title="新しい録音" disabled={!isTauri() || writeLocked || state.kind !== "ready" || !["idle","error"].includes(state.snapshot.phase)} onClick={() => void navigate(prepareNewRecording)}><span className="nav-glyph" aria-hidden="true">＋</span><span className="nav-label">新しい録音</span></button>
          <button type="button" aria-label="会議一覧" title="会議一覧" aria-current={screen === "meetings" || screen === "overview" ? "page" : undefined} className={`nav-item ${screen === "meetings" || screen === "overview" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={openMeetingList}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="table" /></span><span className="nav-label">会議一覧</span></button>
          <button type="button" aria-label="ジョブ一覧" title="ジョブ一覧" aria-current={screen === "jobs" ? "page" : undefined} className={`nav-item ${screen === "jobs" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked && !dataWorkActive} onClick={showJobs}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="list" /></span><span className="nav-label">ジョブ一覧</span><JobCount /></button>
          <button type="button" aria-label="ボイス一覧" title="ボイス一覧" aria-current={screen === "voices" ? "page" : undefined} className={`nav-item ${screen === "voices" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => {  setScreen("voices"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="voice" /></span><span className="nav-label">ボイス一覧</span></button>
          <button type="button" aria-label="プロンプト" title="プロンプト" aria-current={screen === "prompts" ? "page" : undefined} className={`nav-item ${screen === "prompts" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => {  setScreen("prompts"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="edit" /></span><span className="nav-label">プロンプト</span></button>
          <button type="button" aria-label="内部ログ" title="内部ログ" aria-current={screen === "logs" ? "page" : undefined} className={`nav-item ${screen === "logs" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={() => void navigate(() => {  setScreen("logs"); })}><span className="nav-glyph" aria-hidden="true"><ActionIcon name="logs" /></span><span className="nav-label">内部ログ</span></button>
          <button type="button" aria-label="設定" title="設定" aria-current={screen === "settings" ? "page" : undefined} className={`nav-item ${screen === "settings" ? "selected" : ""}`} data-screen-navigation disabled={navigationLocked} onClick={openSettings}><span className="nav-glyph settings-nav-glyph" aria-hidden="true"><ActionIcon name="gear" /></span><span className="nav-label">設定</span></button>
        </nav>
      </aside>
      <div className="main-column">
        <ResidencyNotifications scope="main"/>
        <IssueNotice scope="main" type="meeting-operation" label="会議の操作" message={meetingError}/>
        <IssueNotice scope="main" type="recording-command" label="録音の操作" message={captureError}/>
        <IssueNotice scope="main" type="recording-confirmation" label="録音終了の確認" message={recordingDecision.error}/>
        <RecorderOperationNotifications />
        <RecordingRecoveryNotifications snapshot={recovery.snapshot} recording={savingRecording} recordingError={recordingDecision.snapshot?.phase === "failed" ? recordingDecision.snapshot.errorMessage : null} />
        <ThemeNotifications locked={writeLocked} />

        <header className="topbar"><nav className="screen-navigation" aria-label="画面の移動">{history.previous && <IconButton label="前の画面へ戻る" icon="previous" data-screen-navigation disabled={navigationLocked} onClick={() => void goBack()} />}{screen !== "meetings" && <><button type="button" className="breadcrumb-button" data-screen-navigation disabled={navigationLocked} onClick={openMeetingList}>会議一覧</button><span aria-hidden="true">/</span></>}<span aria-current="page">{detail ? "会議詳細" : screen === "recording" ? preparation ? "新しい録音" : "録音" : screen === "overview" || screen === "meetings" ? "会議一覧" : screen === "jobs" ? "ジョブ一覧" : screen === "logs" ? "内部ログ" : screen === "voices" ? "ボイス一覧" : screen === "prompts" ? "プロンプト" : "設定"}</span></nav><div className="topbar-actions"><div className="header-recording" aria-label="共通録音状態"><span className="header-status"><span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} />{phaseLabel(state)}{state.kind === "ready" && recordingActive(state.snapshot) && screen!=="recording" && <span className="header-timer">{formatElapsed(state.snapshot.elapsedMs)}</span>}</span>{state.kind === "ready" && recordingActive(state.snapshot) && screen!=="recording" && <button type="button" className="small-secondary" data-screen-navigation disabled={navigationLocked} onClick={() => void openRecordingMeeting()}>録音画面へ</button>}{state.kind === "ready" && ["preparing", "recording"].includes(state.snapshot.phase) && screen!=="recording" && <button type="button" className="stop-button" disabled={captureBusy || navigationBusy || recordingDecision.requestBusy || recordingDecision.visible || !recordingDecision.connected} onClick={event => void stopRecording(event.currentTarget)}>録音を終了</button>}</div><NotificationPopover open={notificationsOpen} onOpenChange={setNotificationsOpen}><NotificationList visible={notificationsOpen} onOpen={openNotifiedMeeting} locked={navigationLocked}/></NotificationPopover></div><ToastHost docked={detail}/></header>
        <CommonOperationProgress onOpenJobs={showJobs} />
        {detail && <MeetingEditorHeader meeting={selectedMeeting} meetingError={meetingError} activeTab={detailTab}
          navigationLocked={navigationLocked} onSwitchTab={switchDetailTab}
          rename={{ opener: renameOpener, locked: selectedWriteLocked || meetingBusy, open: () => { setRenameTitle(selectedMeeting.title); setRenameOpen(true); } }}
          menu={{ content: deletionMenu(selectedMeeting), onOpenChange: open => trackDeletionMenu(selectedMeeting.id, open) }} />}
        <div className="workspace-surface">
        <AppUpdateBanner />
        <main className={`content${detail ? " meeting-detail-content" : ""}`} aria-label="会議ワークスペース" tabIndex={-1}>
          <JobNotifications locked={navigationLocked} onOpen={openNotifiedMeeting} />
          {dataOperation.writeState?.readOnly && <StorageRecoveryNotice state={dataOperation.writeState} onRecheck={dataOperation.refreshWriteState}
            relocationGuidance={dataOperation.snapshot?.restartRequired || dataOperation.availability?.reasonCode === "relocation_restart_required"
              ? "保存先の切替を反映するには、設定の「データと保存」から終了し、アプリを起動してください。"
              : dataOperation.snapshot?.relocationPreview?.pending || dataOperation.snapshot?.kind === "relocation" && dataOperation.snapshot.attentionRequired
                ? "設定の「データと保存」で未完了の移動の状態を確認してください。" : undefined}
            disabled={navigationLocked || modelCleanupActive || !!savingRecording && recordingActive(savingRecording)} />}
          {writeStateUnverified && dataOperation.writeStateError && <p className="workspace-warning meeting-list-notice" role="status">保存先の書き込み可否を再確認しています。会議を閲覧できます。</p>}
          {selectedNotices?.length ? <Callout className="workspace-warning meeting-diagnostic-notice" title={selectedDiagnosticSummary!.label}>
            {[...new Set(selectedNotices.map(item => meetingDiagnosticPresentation(item, { documentEditingBlocked: selectedWriteLocked }).description))].map(message => <p key={message}>{message}</p>)}
          </Callout> : null}
          {meetings.map(meeting => <ToastNotice key={meeting.id} scope={meeting.id} type="meeting-integrity" level="warning" historyOnly meetingId={meeting.id}
            message={meetingList.byMeeting.has(meeting.id) ? `会議「${meeting.title}」：${meetingDiagnosticSummary(meetingList.byMeeting.get(meeting.id))!.label}。` : ""} />)}
          <LibraryPanels
            showMeetingList={meetingScreen && !detail}
            settingsVisible={screen === "settings"} promptsVisible={screen === "prompts"} settingsReady={settingsReady} locked={writeLocked}
            audioImport={<AudioImportPanel modelCleanupActive={modelCleanupActive} overlayHost={audioDropHost} visible={meetingScreen && !detail}
              showNewRecording={false} locked={writeLocked} recordingActive={recordingBlocksPanelActions} withSavedEdits={withSavedEdits} onNewRecording={prepareNewRecording} onSaved={async id => {
                await flushPendingEdits();
                const [meeting] = await Promise.all([invoke<Meeting | null>("get_meeting", { id }), refreshMeetings()]);
                if (!meeting) throw Error("保存した会議が見つかりません。");
                setOverviewMeeting(meeting);
              }} />}
            model={{
              cleanup: modelCleanup.active, onProgressOwnerReady: setModelProgressOwnerReady,
              refreshEpoch: modelRefreshEpoch, runtimeUpdateEpoch: modelRuntimeEpoch,
              stopRequested: modelPreparationStopRequested, recordingActive: recordingBlocksPanelActions,
              deletion: {
                canReviewModel: modelDeletion.canReviewModel, reviewUnavailableReason: modelDeletion.reviewUnavailableReason,
                listing: modelDeletion.listing, canReviewPending: modelDeletion.canReviewPending, reviewPending: modelDeletion.reviewPending,
                protectedNow: modelDeletion.protectedNow, unknown: modelDeletion.unknown, rpcBusy: modelDeletion.rpcBusy,
                retryUnknown: modelDeletion.retryUnknown, reviewModel: modelDeletion.reviewModel,
              },
            }}
            dataSave={<DataSavePanel operation={dataOperation} visible={settingsReady}
              locked={captureTest.blocked || navigationBusy || deletion.protectedNow || audioDeletion.protectedNow || modelDeletion.protectedNow || deletionBlocked}
              withSavedEdits={withSavedEdits} onProtectionChange={setDataProtection}
              onRestored={async () => { await refreshMeetings(); await loadOrigins(); }} onShowMeetings={showRestored} />}
          />
          <ModelDeletionDialog operation={dataOperation} control={modelDeletion} />
          <MeetingDeletionPanel operation={dataOperation} control={deletion} />
          <AudioDeletionPanel operation={dataOperation} control={audioDeletion} meetingTitle={meetings.find(m => m.id === audioDeletion.preview?.meetingId)?.title} />
          {screen === "settings" || screen === "prompts" || screen === "logs" ? <LibraryShell screen={screen} />
          : screen === "voices" ? <LibraryShell screen={screen} locked={writeLocked} />
          : screen === "jobs" ? <LibraryShell screen={screen} jobs={{
            data: {
              jobs: dataOperation.jobs, jobsError: dataOperation.jobsError, activeToken: dataOperation.snapshot?.token ?? null,
              refresh: dataOperation.refresh, dismissJob: dataOperation.dismissJob,
            },
            publicationReadStatus: publication.readStatus, onOpenData: () => setScreen("settings"),
            onShowRestored: showRestored, locked: navigationLocked, onOpenMeeting: openNotifiedMeeting,
            onOpenModels: () => { if (!navigationLocked) void openSettings(); },
          }} /> : screen === "recording" ? <>
            <RecordingShell
              preparation={preparation && { id: preparation.id, title: preparation.title, submitted: preparation.submitted, meetingVocabulary: preparation.meetingVocabulary }}
              meeting={selectedMeeting && { id: selectedMeeting.id, title: selectedMeeting.title, canEditVocabulary: selectedMeeting.lifecycle === "draft" && selectedMeeting.sourceType == null }}
              snapshot={savingRecording && { meetingId: savingRecording.meetingId, sessionId: savingRecording.sessionId, phase: savingRecording.phase, elapsedMs: savingRecording.elapsedMs }}
              owner={captureOwner} locked={selectedWriteLocked} vocabularyLocked={writeLocked}
              canStart={!modelCleanupActive&&isTauri()&&captureAvailable&&!!captureInputs.selected&&!!captureScope.scope&&!selectedWriteLocked&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase)&&(preparation?validRecordingPreparation(preparation):selectedMeeting?.lifecycle==="draft"&&selectedMeeting.sourceType==null)}
              canStop={!captureBusy&&!navigationBusy&&!recordingDecision.requestBusy&&!recordingDecision.visible&&recordingDecision.connected&&state.kind==="ready"&&["preparing","recording"].includes(state.snapshot.phase)}
              actions={{
                setTitle: title => setPreparation(current => current && !current.submitted ? { ...current, title } : current),
                setVocabulary: meetingVocabulary => setPreparation(current => current && !current.submitted ? { ...current, meetingVocabulary } : current),
                start: () => void startRecording(), stop: source => void stopRecording(source),
                openSettings,
                confirmResult: () => void confirmPreparationResult(),
              }}
              inputs={captureAvailable?<CaptureInputSelection control={captureInputs} scopeControl={captureScope} locked={writeLocked||!!preparation?.submitted||state.kind!=="ready"||!["idle","error"].includes(state.snapshot.phase)}/>:<p>録音ヘルパーを利用できません。アプリのインストール状態と内部ログを確認してください。</p>}
              activeMicrophone={captureAvailable&&activeMicrophoneInput?<ActiveMicrophoneSelection control={captureInputs} locked={writeLocked||!captureInputs.inputs} onSelect={value=>void selectRecordingMicrophone(value)} status={microphoneSelection} pending={microphoneSelection?.source==="pending"||microphoneRequest?.owner===microphoneSession&&(!microphoneSelection||microphoneSelection.requestRevision<microphoneRequest.revision)}/>:null}
              test={captureAvailable?<CaptureTestPanel operationError={captureTestError} control={captureTest} currentUid={captureInputs.uid} currentScope={captureScope.scope}
                canStart={!modelCleanupActive&&!!captureInputs.selected&&!!captureScope.scope&&!captureTest.blocked&&!operationWriteLocked&&!preparation?.submitted&&state.kind==="ready"&&["idle","error"].includes(state.snapshot.phase)}
                locked={operationWriteLocked||captureBusy||closingBusy||startConfirm.pending} onStart={startCaptureTest} onCancel={()=>void cancelCaptureTest()} onFinish={finishCaptureTest}/>:null}
              errors={{ capture: captureError, confirmation: recordingDecision.error, meeting: meetingError }} />
          </> : <MeetingEditorShell detail={detail} activeTab={detailTab}
            meeting={selectedMeeting && {
              id: selectedMeeting.id, title: selectedMeeting.title, recorded: ["recorded", "imported"].includes(selectedMeeting.lifecycle),
              audioDeleted: deletion.audioAvailability.rows.get(selectedMeeting.id)?.audioDeleted ?? null,
            }}
            minutesMounted={!!selectedMeeting && minutesOwner === selectedMeeting.id} recordingActive={recordingBlocksPanelActions}
            locks={{
              write: selectedWriteLocked, editing: writeProtected || selectedProtected, navigation: navigationLocked,
              analysis: selectedWriteLocked || selectedAudioUnavailable || verifiedTranscript !== selectedMeeting?.id,
            }}
            analysis={{
              epoch: analysisEpoch, hasTranscript: transcriptMeeting === selectedMeeting?.id,
              publicationStatus: publication.readStatus, replace: publication.replace,
              headChanged: async (id, read) => {
                setAnalysisEpoch(value => value + 1);
                const [meeting] = await Promise.all([
                  read("get_meeting", () => invoke<Meeting | null>("get_meeting", { id })),
                  refreshMeetings(() => read("list_meetings", () => invoke<Meeting[]>("list_meetings"))),
                ]);
                setSelectedMeeting(meeting);
              },
              loaded: (id, present, readOnly) => {
                setTranscriptMeeting(present ? id : null);
                setVerifiedTranscript(readOnly ? null : id);
              },
              consolidated: async id => {
                const [meeting] = await Promise.all([invoke<Meeting | null>("get_meeting", { id }), refreshMeetings()]);
                setSelectedMeeting(meeting);
                setAnalysisEpoch(value => value + 1);
              },
            }}
            playbackHost={playbackHost} withSavedEdits={withSavedEdits}
            library={<LibraryShell screen="meetings" hidden={detail} locked={navigationLocked} meetingError={meetingError} restoredOnly={!!restoredFilter}
            search={{ query: meetingQuery, update: setMeetingQuery }}
            list={{
              hasMeetings: meetings.length > 0, status: meetingList.status, error: meetingList.error,
              diagnosticError: meetingList.diagnosticError, diagnostics: meetingList.byMeeting.get(null) ?? [],
              rows: shownMeetings.map(meeting => {
                const recoveryItem = recoveryMeetingState(recovery.snapshot, meeting.id);
                return {
                  id: meeting.id, title: meeting.title, createdAt: meeting.createdAt, restored: restoredIds.includes(meeting.id),
                  status: meetingStatusLabel(meeting, state, recovery.snapshot, meetingList.byMeeting.get(meeting.id)),
                  recoveryWarning: !!recovery.error && !!recoveryItem,
                  retry: recoveryItem?.error && <RecordingSaveRetry snapshot={recovery.snapshot} meetingId={meeting.id} locked={writeLocked} />,
                  menu: deletionMenu(meeting),
                  deletionPending: !!dataOperation.deletionListing?.deletingMeetingIds.includes(meeting.id),
                };
              }),
            }}
            actions={{ select: selectMeeting, reload: reloadMeetings, clearRestoredFilter: () => void navigate(() => setRestoredFilter(null)), trackMenu: trackDeletionMenu }}
            />}
          />}
        </main>
        <div ref={setAudioDropHost} className="audio-drop-host" />
        </div>
        <div ref={setPlaybackHost} className="meeting-playback-dock" role="region" aria-label="会議の共通音声プレーヤー" hidden={!selectedMeeting || screen === "settings" || screen === "recording" || screen === "jobs" || screen === "logs" || screen === "voices" || screen === "prompts"} />
      </div>
    </div>
    <MeetingRenameDialog open={renameOpen && !!selectedMeeting} busy={meetingBusy} locked={selectedWriteLocked}
      title={renameTitle} opener={renameOpener} onDismiss={() => setRenameOpen(false)} onTitleChange={setRenameTitle} onSubmit={renameMeeting} />
    <ModelDownloadGuidance locked={writeLocked} onOpenModels={async () => { await openSettings(); }} />
    <ModelRuntimeGuidance locked={writeLocked || state.kind !== "ready" || !["idle", "error"].includes(state.snapshot.phase)} onOpenModels={async () => {
      await openSettings();
      setModelRuntimeEpoch(value => value + 1);
    }} />
    </MeetingAudioProvider>
    </MeetingPlaybackSurface>
  );
}

export function MainDesktopWindow() {
  return <AppUpdateProvider><RecorderOperationProvider><RecordingInputProvider windowLabel="main"><SettingsModalScope><MainWindow /></SettingsModalScope><ConfirmationHost /></RecordingInputProvider></RecorderOperationProvider></AppUpdateProvider>;
}
