// Synthetic IPC only: production MainWindow, no device or audio access.
import { mockIPC, mockWindows } from "./startup-ready-mocks.ts";
import { emit } from "@tauri-apps/api/event";
window.emitResidencyFixture = (event,payload) => emit(event,payload);
import { registerPendingEdits } from "../src/draft-save.ts";
import { queuedProjection, queuedAction } from "./input-notice-queue-browser-fixture.js";
import {archivedFixture,archivedPage,archivedRows,archivedAction,newArchivedRow,reopenArchived} from "./archived-input-browser-fixture.js";
import { toasts } from "../src/toast";
window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const data = window.dataFixture;
const clone = value => structuredClone(value);
const recording = { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null };
const importing = { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, fileName: null, info: null, sourceBytes: null, requiredBytes: null, availableBytes: null, completed: 0, saved: null, errorMessage: null };
const recoverySeed=sessionStorage.getItem("MON_CAPTURE_TEST_RECOVERY_FIXTURE");sessionStorage.removeItem("MON_CAPTURE_TEST_RECOVERY_FIXTURE");
const queueSeed=sessionStorage.getItem("MON_INPUT_QUEUE_FIXTURE");sessionStorage.removeItem("MON_INPUT_QUEUE_FIXTURE");
const archivedSeed=sessionStorage.getItem("MON_ARCHIVED_INPUT_FIXTURE");sessionStorage.removeItem("MON_ARCHIVED_INPUT_FIXTURE");
const f = window.closeFixture = {
  notificationHistory:()=>toasts.getHistorySnapshot(),
  recording, importing, activity: { analysisBusy: false, modelPreparationBusy: false }, calls: [],
  uiPreferences:{id:"ui_preferences",lockVersion:0,payload:{schemaVersion:1,theme:"system"}},
  failFlush: false, flushDelay: 0, flushes: 0, mutateDuringFlush: null,
  startLost: false, startBeforeCommitFailure:false, cancelImportLost: false, cancelDataLost: false, finishLost: false, failRead: false,
  captureInputs:{schemaVersion:1,microphonePermission:"notDetermined",selectedMicrophoneUID:null,devices:[{uid:"built-in",name:"内蔵マイク",isDefault:true},{uid:"usb-123",name:"ヘッドセットのマイク",isDefault:false}]},
  failCaptureInputs:false,captureInputsDelay:0,holdCaptureInputs:false,pendingInputs:[],
  microphoneSelection:null,recordingMicrophoneScope:null,
  captureApplications:{schemaVersion:1,systemPermission:"unknown",applications:[{pid:123,bundleIdentifier:"com.example.browser",launchTimeUs:1791100000000001,name:"会議ブラウザ"},{pid:456,bundleIdentifier:"com.example.other",launchTimeUs:1791100000000002,name:"会議ブラウザ"}]},
  failCaptureApplications:false,holdCaptureApplications:false,pendingApplications:[],mutateApplicationDuringFlush:false,mutateApplicationAtStart:false,
  inputLevels:null,holdInputLevels:false,pendingInputLevels:[],failInputLevels:false,refreshInputLevels:false,
  writes:null,holdWrites:false,pendingWrites:[],failWrites:false,refreshWrites:false,
  inputStatus:null,holdInputStatus:false,pendingInputStatus:[],failInputStatus:false,refreshInputStatus:false,
  journalStatus:null,journalRevision:0,holdJournal:false,pendingJournal:[],failJournal:false,refreshJournal:true,
  queueState:null,queueHold:{receive:false,render:false,close:false},queueLost:{receive:false,render:false,close:false},queueReplies:[],
  queuedAction:(command,args)=>queuedAction(f,command,args),
  archived:archivedFixture(),newArchivedRow:options=>newArchivedRow(data,options),archivedAction:args=>archivedAction(f,args),
  persistArchivedFixture:()=>sessionStorage.setItem("MON_ARCHIVED_INPUT_FIXTURE",JSON.stringify({rows:f.archived.rows,meetings:data.meetings})),
  showQueueBlockers:()=>{for(const key of ["block1","block2","block3"])toasts.show({key,level:"error",message:`合成優先通知 ${key}`});},
  failRecordingSnapshot:false,
  persistInputQueueFixture:()=>sessionStorage.setItem("MON_INPUT_QUEUE_FIXTURE",JSON.stringify({recording:f.recording,inputStatus:f.inputStatus,queueState:f.queueState,refreshInputStatus:f.refreshInputStatus,meetings:data.meetings})),
  captureTest:null,testLost:false,testReadFailure:false,testAbsentStart:false,testReconcileFailure:false,testRetired:[],testCancelLost:false,testDiscardFailure:false,testDiscardLost:false,
  captureRecovery:recoverySeed?JSON.parse(recoverySeed):{schemaVersion:1,revision:2,phase:"clear",runId:null,errorMessage:null},recoveryReadFailure:false,recoveryRetryLost:false,recoveryRetryKeepsAttention:false,recoveryRetryHoldAck:false,recoveryPendingReplies:[],
  testLevels:null,testPlayback:{reservations:{},tokens:{},hold:false,pending:[],failClose:false,losePrepare:false,loseReceipt:false},
  testChanged:patch=>{if(!f.captureTest)throw Error("模擬テストなし");f.captureTest={...f.captureTest,...patch,revision:f.captureTest.revision+1};},
  testReady:()=>f.testChanged({phase:"ready",busy:false,microphone:{frames:240000,sampleRate:48000,channels:1,durationMs:5000,hasGaps:false},system:{frames:192000,sampleRate:48000,channels:2,durationMs:4000,hasGaps:true}}),
  request: quit => emit("main-close-requested", { quit }),
  recordChanged: async patch => { f.recording = { ...f.recording, ...patch, revision: f.recording.revision + 1 }; await emit("session-changed", clone(f.recording)); },
  importChanged: async patch => { f.importing = { ...f.importing, ...patch, revision: f.importing.revision + 1 }; await emit("audio-import-changed", clone(f.importing)); },
  finishImport: saved => f.importChanged({ busy: false, phase: saved ? "saved" : "cancelled", saved: saved ? { id: crypto.randomUUID(), meetingId: data.meetings[0].id, state: "saved" } : null }),
  finishData: published => data.changed({ busy: false, phase: published ? "completed" : "cancelled", retryAllowed: false, backupPreview: null, backupReceipt: published ? { archiveId: crypto.randomUUID(), meetingCount: 1, assetCount: 0, omittedPcmCount: 0, omittedLogCount: 0, deletedAudioMeetingCount: 0, deletedAudioAssetCount: 0, size: 1024, sha256: "a".repeat(64) } : null }),
};
if(queueSeed){const {meetings,...queueState}=JSON.parse(queueSeed);Object.assign(f,queueState);data.meetings=meetings;}
if(archivedSeed){const seed=JSON.parse(archivedSeed);data.meetings=seed.meetings;f.archived.rows=reopenArchived(seed.rows);}
registerPendingEdits(async () => {
  f.flushes++;
  if (f.flushDelay) await new Promise(resolve => setTimeout(resolve, f.flushDelay));
  if (f.failFlush) throw Error("模擬編集保存失敗・入力を保持");
  if (f.mutateDuringFlush === "import") await f.importChanged({ token: crypto.randomUUID() });
  if (f.mutateDuringFlush === "meeting") {
    const id = f.calls.filter(c => c.command === "get_meeting").at(-1)?.args.id;
    if (!id) throw Error("合成の確認済み会議IDがありません");
    const existing = data.meetings.find(m => m.id === id);
    if (existing) { existing.title = "確認後に変更された会議"; existing.lockVersion++; }
    else data.meetings.push({ ...data.meetings[0], id, title: "確認後に作成された会議" });
  }
  if(f.mutateApplicationDuringFlush){f.captureApplications.applications[0].launchTimeUs++;f.mutateApplicationDuringFlush=false;}
  f.mutateDuringFlush = null;
});
mockIPC(async (command, args) => {
  f.calls.push({ command, args: clone(args) });
  if(command==="get_archived_recording_input_notices")return archivedPage(f,args);
  if(command==="get_archived_recording_input_notice_rows")return archivedRows(f,args);
  if(command==="acknowledge_archived_recording_input_notice")return archivedAction(f,args.request);
  if(command==="get_meeting_analysis_audio"&&f.archived.rows.some(row=>row.meetingId===args.meetingId))return {tracks:[]};
  if(command==="get_ui_preferences")return clone(f.uiPreferences);
  if(command==="save_ui_preferences"){
    if(args.expectedLockVersion!==f.uiPreferences.lockVersion||data.snapshot.busy||data.snapshot.restartRequired)throw Error("模擬表示設定競合／書込停止");
    f.uiPreferences={...f.uiPreferences,lockVersion:f.uiPreferences.lockVersion+1,payload:clone(args.payload)};
    await emit("ui-preferences-changed",clone(f.uiPreferences));return clone(f.uiPreferences);
  }
  if (command === "get_recording_snapshot") {if(f.failRecordingSnapshot)throw Error("模擬録音状態取得失敗");return clone(f.recording);}
  if(["get_capture_test_recovery","retry_capture_test_recovery"].includes(command))throw Error("removed cleanup UI command");
  if(command==="get_capture_test_snapshot"){
    if(f.testReadFailure)throw Error("模擬テスト状態読戻し失敗");
    if(args?.runId&&args.runId!==f.captureTest?.runId)throw Error("模擬テスト所有変更");return clone(f.captureTest);
  }
  if(command==="reconcile_capture_test_start"){
    if(f.testReconcileFailure)throw Error("模擬テスト受付確認失敗");
    if(f.captureTest){if(f.captureTest.runId!==args.runId)throw Error("模擬テスト所有変更");return clone(f.captureTest);}
    f.testRetired.push(args.runId);return null;
  }
  if(command==="start_capture_test"){
    if(f.captureRecovery.phase!=="clear")throw Error("模擬テスト音声の回復が未完了");
    const r=args.request;
    if(f.testRetired.includes(r.runId))throw Error("模擬テスト開始IDは解放済み");
    if(f.captureTest||!["idle","error"].includes(f.recording.phase)||r.expectedImport.token!==f.importing.token||(!r.expectedImport.busy&&r.expectedImport.revision!==f.importing.revision))throw Error("模擬テスト開始条件変更");
    if(f.testAbsentStart)throw Error("模擬テスト受付結果不明");
    f.captureTest={schemaVersion:1,revision:1,runId:r.runId,phase:"preparing",busy:true,microphoneUid:r.microphoneUid,
      microphoneName:f.captureInputs.devices.find(d=>d.uid===r.microphoneUid)?.name??null,captureScope:clone(r.captureScope),
      applicationName:r.captureScope.mode==="application"?f.captureApplications.applications.find(a=>a.pid===r.captureScope.application.pid)?.name??null:null,
      microphone:null,system:null,cleanupPending:false,errorMessage:null};
    if(f.importing.busy)await f.importChanged({phase:"cancelled",busy:false});
    if(f.testLost)throw Error("模擬テスト開始応答喪失");return clone(f.captureTest);
  }
  if(command==="cancel_capture_test"){
    if(args.runId!==f.captureTest?.runId)throw Error("模擬取消所有変更");
    if(f.captureTest.busy)f.testChanged({phase:"cancelling"});
    if(f.testCancelLost)throw Error("模擬テスト取消応答喪失");return clone(f.captureTest);
  }
  if(command==="discard_capture_test"){
    if(args.runId!==f.captureTest?.runId||f.captureTest.busy||f.testDiscardFailure||Object.keys(f.testPlayback.tokens).length||Object.keys(f.testPlayback.reservations).length)throw Error("模擬テスト回収未完了");
    f.captureTest=null;if(f.testDiscardLost)throw Error("模擬回収応答喪失");return null;
  }
  if(command==="get_capture_test_levels"){
    if(args.runId!==f.captureTest?.runId||!f.testLevels)throw Error("模擬テスト入力未確認");return clone(f.testLevels);
  }
  if(command==="register_playback_preparation"){
    if(f.testPlayback.reservations[args.preparationId])throw Error("模擬重複準備");
    f.testPlayback.reservations[args.preparationId]={cancelled:false,token:null};return null;
  }
  if(command==="prepare_capture_test_playback"){
    const p=f.testPlayback,reservation=p.reservations[args.preparationId],run=f.captureTest;
    if(!reservation||reservation.cancelled||run?.runId!==args.runId||run.phase!=="ready")throw Error("模擬テスト再生所有変更");
    if(p.hold)await new Promise(resolve=>p.pending.push(resolve));
    if(reservation.cancelled)throw Error("模擬テスト再生準備取消");
    const track=args.trackKind==="mic"?run.microphone:run.system;
    const info={token:crypto.randomUUID(),sampleRate:track.sampleRate,channels:track.channels,totalFrames:track.frames,startUs:0,endUs:Math.floor(track.frames*1000000/track.sampleRate),trackKind:args.trackKind,alignment:"unverified"};
    p.tokens[info.token]=info;reservation.token=info.token;
    if(p.losePrepare)throw Error("模擬テスト再生準備応答喪失");return clone(info);
  }
  if(command==="finish_playback_preparation"){
    const p=f.testPlayback,reservation=p.reservations[args.preparationId];
    if(!reservation||reservation.cancelled||reservation.token!==args.playbackToken)throw Error("模擬テスト受取不一致");
    delete p.reservations[args.preparationId];if(p.loseReceipt)throw Error("模擬テスト受取応答喪失");return null;
  }
  if(command==="cancel_playback_preparation"){
    const p=f.testPlayback,reservation=p.reservations[args.preparationId];
    if(reservation){reservation.cancelled=true;if(reservation.token)delete p.tokens[reservation.token];delete p.reservations[args.preparationId];}return null;
  }
  if(command==="close_playback"){if(f.testPlayback.failClose)throw Error("模擬テスト再生解放失敗");delete f.testPlayback.tokens[args.token];return null;}
  if(command==="read_playback_chunk"){
    const info=f.testPlayback.tokens[args.token];if(!info)throw Error("模擬テスト再生なし");
    const frames=Math.max(0,Math.min(65536,info.totalFrames-args.offsetFrames)),bytes=new ArrayBuffer(12+frames*info.channels*4),view=new DataView(bytes);
    view.setUint32(0,info.sampleRate,true);view.setUint32(4,info.channels,true);view.setUint32(8,frames,true);return bytes; // Silent synthetic PCM only.
  }
  if(command==="get_recording_levels"){
    if(f.refreshInputLevels&&f.inputLevels)f.inputLevels.revision++;
    const value=clone(f.inputLevels);
    if(f.holdInputLevels)await new Promise(resolve=>f.pendingInputLevels.push(resolve));
    if(f.failInputLevels||!value)throw Error("模擬入力レベル未確認");
    return value;
  }
  if(command==="get_recording_writes"){
    if(args.sessionId!==f.recording.sessionId||!["preparing","recording"].includes(f.recording.phase))throw Error("模擬書込み所有変更／停止済み");
    if(f.refreshWrites&&f.writes){f.writes.revision++;for(const name of ["microphone","system"])if(f.writes[name].state==="written")f.writes[name].frames+=480;}
    const value=clone(f.writes);if(f.holdWrites)await new Promise(resolve=>f.pendingWrites.push(resolve));
    if(f.failWrites||!value)throw Error("模擬書込み未確認");return value;
  }
  if (command === "capture_available") return true;
  if(command==="get_recording_input_history_status"){
    if(args.sessionId!==f.recording.sessionId||!["preparing","recording"].includes(f.recording.phase))throw Error("模擬履歴診断の所有変更");
    if(f.refreshJournal)f.journalRevision++;
    const value={schemaVersion:1,ownerId:args.sessionId,revision:f.journalRevision,reportAgeMs:0,persistence:clone(f.journalStatus??{requestedRevision:1,committedRevision:1,state:"confirmed",closed:false})};
    if(f.holdJournal)await new Promise(resolve=>f.pendingJournal.push(resolve));
    if(f.failJournal)throw Error("模擬履歴診断読込失敗");return value;
  }
  if(command==="get_recording_input_status"){
    if(args.sessionId!==f.recording.sessionId||!["preparing","recording"].includes(f.recording.phase))throw Error("模擬マイク状態所有変更／停止済み");
    if(f.refreshInputStatus&&f.inputStatus)f.inputStatus.revision++;
    if(f.holdInputStatus)await new Promise(resolve=>f.pendingInputStatus.push(resolve));
    if(f.failInputStatus||f.inputStatus?.schemaVersion!==4||!f.queueState)throw Error("模擬マイク状態未確認");return queuedProjection(f);
  }
  if(["receive_recording_input_notice","render_recording_input_notice","close_recording_input_notice"].includes(command))return queuedAction(f,command,args);
  if (command === "get_capture_inputs") {
    const snapshot=clone(f.captureInputs);
    if(f.holdCaptureInputs)await new Promise(resolve=>f.pendingInputs.push(resolve));
    if(f.captureInputsDelay)await new Promise(resolve=>setTimeout(resolve,f.captureInputsDelay));
    if(f.failCaptureInputs)throw Error("模擬マイク一覧取得失敗");
    return snapshot;
  }
  if(command==="get_capture_applications"){
    const snapshot=clone(f.captureApplications);
    if(f.holdCaptureApplications)await new Promise(resolve=>f.pendingApplications.push(resolve));
    if(f.failCaptureApplications)throw Error("模擬アプリ一覧取得失敗");
    return snapshot;
  }
  if (command === "get_audio_import") { if (f.failRead) throw Error("模擬終了対象読戻し失敗"); return clone(f.importing); }
  if (command === "get_application_close_activity") return clone(f.activity);
  if (command === "cancel_audio_import") {
    if (args.token !== f.importing.token) throw Error("模擬取り込み対象変更");
    await f.importChanged({ phase: f.importing.busy ? "cancelling" : "cancelled" });
    if (f.cancelImportLost) throw Error("模擬取消受付の応答喪失");
    return clone(f.importing);
  }
  if (command === "cancel_data_operation") {
    if (args.token !== data.snapshot.token) throw Error("模擬データ対象変更");
    await data.changed({ phase: data.snapshot.busy ? "cancelling" : "cancelled", retryAllowed: false });
    if (f.cancelDataLost) throw Error("模擬データ取消受付の応答喪失");
    return clone(data.snapshot);
  }
  if(command==="get_recording_microphone_selection"){
    if(args.sessionId!==f.recording.sessionId||f.microphoneSelection?.ownerId!==args.sessionId)throw Error("模擬マイク選択状態は未確認");
    return clone(f.microphoneSelection);
  }
  if(command==="select_recording_microphone"){
    if(args.sessionId!==f.recording.sessionId||!["preparing","recording"].includes(f.recording.phase)||f.microphoneSelection?.ownerId!==args.sessionId)throw Error("模擬マイク選択の所有が変更");
    const device=args.microphoneUid===null?f.captureInputs.devices.find(d=>d.isDefault):f.captureInputs.devices.find(d=>d.uid===args.microphoneUid);
    if(!device||["denied","restricted"].includes(f.captureInputs.microphonePermission))throw Error("模擬マイク対象変更");
    const automatic=args.microphoneUid===null,previous=f.microphoneSelection;
    f.microphoneSelection={...previous,revision:previous.revision+1,requestRevision:previous.requestRevision+1,mode:automatic?"automatic":"fixed",deviceUID:device.uid,source:automatic?"systemDefault":"fixed",fallbackReason:automatic?(f.recordingMicrophoneScope?.mode==="application"?"application_input_unavailable":"mon_direct_recording"):null};
    return f.microphoneSelection.requestRevision;
  }
  if (command === "start_recording") {
    if(f.captureRecovery.phase!=="clear")throw Error("模擬テスト音声の回復が未完了");
    if(f.captureTest)throw Error("模擬テスト所有が残っています");
    if (!["idle", "error"].includes(f.recording.phase)||f.startBeforeCommitFailure) throw Error("模擬録音対象変更・受付前失敗");
    const microphone=args.microphoneUid===null?f.captureInputs.devices.find(d=>d.isDefault):f.captureInputs.devices.find(d=>d.uid===args.microphoneUid);
    if(!microphone||["denied","restricted"].includes(f.captureInputs.microphonePermission))throw Error("模擬マイク対象変更");
    if(f.mutateApplicationAtStart){f.captureApplications.applications[0].launchTimeUs++;f.mutateApplicationAtStart=false;}
    const scope=args.captureScope;
    if(!scope||!(scope.mode==="system"&&scope.application===null||scope.mode==="application"&&scope.application&&f.captureApplications.applications.some(a=>a.pid===scope.application.pid&&a.bundleIdentifier===scope.application.bundleIdentifier&&a.launchTimeUs===scope.application.launchTimeUs)))throw Error("模擬アプリ対象変更・全体へ切り替えていません");
    if(args.newTitle!=null){
      if(!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(args.meetingId)||typeof args.newTitle!=="string"||!args.newTitle.trim()||Array.from(args.newTitle.trim()).length>200||data.meetings.some(m=>m.id===args.meetingId))throw Error("模擬新規会議受付拒否");
      const now=Date.now();data.meetings.push({id:args.meetingId,title:args.newTitle.trim(),lifecycle:"draft",sourceType:null,createdAt:now,updatedAt:now,lockVersion:0});
    }
    const meeting=data.meetings.find(m=>m.id===args.meetingId);
    if(!meeting||meeting.lifecycle!=="draft"||meeting.sourceType!=null)throw Error("模擬録音対象変更");
    f.recording = { ...f.recording, revision: f.recording.revision + 1, phase: "preparing", meetingId: args.meetingId, sessionId: crypto.randomUUID() };
    f.recordingMicrophoneScope=clone(scope);
    f.microphoneSelection={schemaVersion:1,ownerId:f.recording.sessionId,revision:1,requestRevision:0,mode:args.microphoneUid===null?"automatic":"fixed",deviceUID:microphone.uid,source:args.microphoneUid===null?"systemDefault":"fixed",fallbackReason:args.microphoneUid===null?(scope.mode==="application"?"application_input_unavailable":"mon_direct_recording"):null};
    await emit("session-changed", clone(f.recording));
    if (f.startLost) throw Error("模擬録音開始受付の応答喪失");
    return null;
  }
  if (command === "finish_main_close") {
    if(f.captureRecovery.phase==="checking")throw Error("模擬テスト音声の起動時確認中");
    if(f.captureTest)throw Error("模擬テストを回収していません");
    // Mirror native last-hop guards without simulating process exit.
    if (args.expected && (args.expected.importToken !== f.importing.token || args.expected.dataToken !== data.snapshot.token)) throw Error("模擬終了対象変更");
    if (f.finishLost) throw Error("模擬終了受付の応答喪失");
    return null;
  }
  if(command === "get_residency_state") return clone(f.residency ?? { supported: true, canBackground: true, trayAvailable: true, dockAvailable: false, error: null });
  if(command === "hide_main_to_background") {
    if(f.hideDelay)await new Promise(resolve=>{f.pendingHide=resolve;});
    if(f.hideFailure || f.residency?.canBackground===false)throw Error("合成の常駐入口消失");
    if(f.hideLost)throw Error("合成の収納応答喪失");
    f.hidden=true;return null;
  }
  if (["get_analysis_flow", "get_current_analysis", "get_transcript_draft", "get_current_minutes", "get_minutes_draft", "get_minutes_markdown"].includes(command)) return null;
  if (["get_analysis_jobs", "get_ready_analysis_flows", "get_minutes_exports"].includes(command)) return [];
  if (command === "get_analysis_publication_status") return null;
  if (["get_analysis_versions", "get_minutes_versions"].includes(command)) return { meetingLockVersion: data.meetings[0].lockVersion, currentId: null, previousId: null, draft: null, versions: [] };
  if (command === "get_meeting_vocabulary") return { global: clone(data.book), meeting: { id: `meeting:${args.meetingId}`, payload: [], lockVersion: 0 } };
  if (command === "get_diarization_preferences") {
    const engine = { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [], threads: 4, clusterThresholdMilli: 500, windowShiftMilli: 100, minDurationOnMs: 0, minDurationOffMs: 0 };
    return { id: "sherpa_profiles", lockVersion: 0, payload: { standard: clone(engine), highAccuracy: clone(engine) } };
  }
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
mockWindows(new URLSearchParams(location.search).get("window")??"main");
await import("../src/main.tsx");
