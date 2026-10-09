// Development-only browser fixture. Not imported by the application or build.
// Playwright routes index.html to this bootstrap for manual UI verification.
import { mockIPC, mockWindows } from "./startup-ready-mocks.ts";
import { toasts } from "../src/toast";
import { emit } from "@tauri-apps/api/event";
import { dataIdleRead } from "./data-idle-browser-fixture.js";
import { recoveryFixture,stampFixtureDraft } from "./transcript-recovery-browser-fixture.js";

const meetings = ["会議A", "会議B"].map((title, index) => ({ id: `meeting-${index}`, title, lifecycle: "recorded", sourceType:"recording", createdAt: 1791028800000, updatedAt: 1791028800000, lockVersion: 1 }));
const analysis = { id: "analysis", meetingId: meetings[0].id, contentHash: "a".repeat(64), speakers: [{ id: "speaker-a", label: "話者A", displayName: "田中" }, { id: "speaker-b", label: "話者B", displayName: "佐藤" }], utterances: [
  { id: "speech-a", trackId: "microphone", startUs: 0, endUs: 15000000, recognizedText: "次の点検日は来週です。", effectiveText: "次の点検日は来週です。", attributionState: "assigned", speakerIds: ["speaker-a"] },
  { id: "speech-b", trackId: "system", startUs: 20000000, endUs: 25000000, recognizedText: "はい、確認します。", effectiveText: "はい、確認します。", attributionState: "overlap", speakerIds: ["speaker-a", "speaker-b"] },
] };
const candidate = { ...structuredClone(analysis), id: "candidate", contentHash: "b".repeat(64), utterances: analysis.utterances.map((item) => ({ ...item, id: `${item.id}-new`, effectiveText: `${item.effectiveText}（候補）` })) };
const engine = { executablePath: "", modelPath: "", vadModelPath: null, threads: 4, useGpu: false };
const diarEngine = { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [], threads: 4, clusterThresholdMilli: 500, windowShiftMilli: 100, minDurationOnMs: 0, minDurationOffMs: 0 };
const fixture = window.monBrowserFixture = { calls: [], failSave: false, failPlayback: false, failPreferences: false, failAdopt: false, failRetry: false, persistenceDelayMs: 0, draft: null,
  showToast: input => toasts.show(input),
  notificationJobs: [], notificationProofMode: "valid", notificationReadFail: false, notificationProofReads: [],
  lostRetryResponse: false,
  transcriptHistory: [],
  diarizationPreferences: { id: "sherpa_profiles", lockVersion: 0, payload: { standard: structuredClone(diarEngine), highAccuracy: structuredClone(diarEngine) } },
  vocabulary: { global: { id: "global", payload: [], lockVersion: 0 } }, failVocabulary: false,
  preferences: { id: "whisper_profiles", payload: { standard: structuredClone(engine), highAccuracy: structuredClone(engine) }, lockVersion: 0 },
  jobs: [{ id: "job-existing", kind: "transcription", state: "paused", completedUnits: 1, totalUnits: 3, errorCode: null, outputAnalysisId: null, pauseReason: "user", specHash: "a".repeat(64), retryOfJobId: null, attempt: 1 },
    { id: "job-failed", kind: "transcription", state: "failed", completedUnits: 2, totalUnits: 3, errorCode: "engine_failed", outputAnalysisId: null, pauseReason: null, specHash: "b".repeat(64), retryOfJobId: null, attempt: 1 }],
  currentId: analysis.id, previousId: null, candidateVisible: true, candidate, archivedDrafts: {},
};
const currentAnalysis = () => fixture.currentId === analysis.id ? analysis : fixture.candidate;
fixture.recovery = recoveryFixture({ drafts: () => [fixture.draft,...Object.values(fixture.archivedDrafts)],current: () => fixture.draft });
const recoveredSeed = sessionStorage.getItem("MON_TRANSCRIPT_RECOVERY_FIXTURE");
if(recoveredSeed) { sessionStorage.removeItem("MON_TRANSCRIPT_RECOVERY_FIXTURE"); const seed = JSON.parse(recoveredSeed); Object.assign(fixture,seed.fixture); fixture.recovery.records = seed.records; if(seed.utterances) analysis.utterances = seed.utterances; }
fixture.crashSeed = () => sessionStorage.setItem("MON_TRANSCRIPT_RECOVERY_FIXTURE",JSON.stringify({ fixture: { draft: fixture.draft,archivedDrafts: fixture.archivedDrafts,currentId: fixture.currentId,previousId: fixture.previousId },records: fixture.recovery.records,utterances: analysis.utterances }));
const versions = () => ({ meetingLockVersion: meetings[0].lockVersion, currentId: fixture.currentId, previousId: fixture.previousId, draft: fixture.draft ? { id: fixture.draft.id, lockVersion: fixture.draft.lockVersion } : null,
  versions: [analysis, fixture.candidate].filter((item) => item.id === fixture.currentId || item.id === fixture.previousId || (item.id === fixture.candidate.id && fixture.candidateVisible)).map((item) => ({ id: item.id, revisionNo: item.id === analysis.id ? 1 : 2, contentHash: item.contentHash, publicationState: item.id === fixture.candidate.id && fixture.candidateVisible ? "candidate" : "committed", utteranceCount: item.utterances.length, createdAt: 1791028800000, profile: { qualityStatus: "unverified" }, config: { diarization: "pending", timelineAlignment: "unverified" } })) });
const switchDraft = (nextId) => {
  if (fixture.draft) fixture.archivedDrafts[fixture.draft.baseAnalysisId] = structuredClone(fixture.draft);
  fixture.draft = fixture.archivedDrafts[nextId] ? structuredClone(fixture.archivedDrafts[nextId]) : null;
};
window.isTauri = true;
fixture.emitNotification = (payload = {}) => emit("analysis-job-changed", payload);
fixture.emitWorkerError = message => emit("analysis-worker-error", message);
fixture.emitUiPreferences = () => emit("ui-preferences-changed",fixture.uiPreferences?.lockVersion??0);
fixture.setUtterances = utterances => { analysis.utterances = structuredClone(utterances); };
fixture.setSpeakers = speakers => { analysis.speakers = structuredClone(speakers); };
fixture.setMeetingTitle = (id,title) => { const meeting = meetings.find(m => m.id === id); if(!meeting) throw Error("Unknown fixture meeting"); meeting.title = title; };
fixture.setMeetingSource = (id,sourceType) => { const meeting = meetings.find(m => m.id === id); if(!meeting) throw Error("Unknown fixture meeting"); meeting.sourceType=sourceType;meeting.lifecycle=sourceType === "audio_import" ? "imported" : "recorded"; };
fixture.meetingAudio = { endUs:3600000000, sessions:{},holdModes:[],pending:[],failModes:[],corrupt:null,gaps:[] };
// Opt-in native preparation protocol for cancellation checks. Older suites
// intentionally keep their late-reply bridge to exercise client fallback.
fixture.preparation = { strict:false, reservations:{}, holdRegister:false, registerPending:[], responsive:false, losePrepare:false, loseReceipt:false, failCancel:false, failClose:false };
const preparationEntry = args => {
  if(!fixture.preparation.strict)return null;
  const entry=fixture.preparation.reservations[args.preparationId];
  if(!entry||entry.claimed||entry.cancelled)throw Error("未登録・使用中・取消済みの音声準備です");
  entry.claimed=true;return entry;
};
const publishPlayback = (entry,info) => {
  if(entry?.cancelled){delete fixture.preparation.reservations[entry.id];throw Error("音声の再生準備を取り消しました");}
  fixture.meetingAudio.sessions[info.token]=info;
  if(entry)entry.token=info.token;
  if(entry&&fixture.preparation.losePrepare)throw Error("模擬音声準備の応答喪失");
  return info;
};
fixture.makeNotificationJob = (patch = {}) => ({ id: crypto.randomUUID(), kind: "transcription", meetingId: meetings[0].id, meetingTitle: meetings[0].title, operationId: crypto.randomUUID(), inputHash: "b".repeat(64), specHash: "a".repeat(64), state: "queued", createdAt: Date.now(), updatedAt: Date.now(), attempt: 1, retryOfJobId: null, completedUnits: 0, totalUnits: 2, errorCode: null, pauseReason: null, outputAnalysisId: null, outputMinutesId: null, ...patch });
mockWindows("main");
mockIPC(async (command, args) => {
  if(fixture.preparation.strict&&["register_playback_preparation","cancel_playback_preparation","finish_playback_preparation"].includes(command)){
    fixture.calls.push({command,args:structuredClone(args)});
    const data=fixture.preparation,id=args.preparationId;
    if(command==="register_playback_preparation"){
      if(data.reservations[id])throw Error("音声準備IDが重複しています");
      data.reservations[id]={id,claimed:false,cancelled:false,token:null,reject:null};
      if(data.holdRegister)await new Promise(resolve=>data.registerPending.push({id,release:resolve}));
    }else if(command==="cancel_playback_preparation"){
      if(data.failCancel)throw Error("模擬取消要求の失敗");
      const entry=data.reservations[id];
      if(entry){entry.cancelled=true;if(entry.token){delete fixture.meetingAudio.sessions[entry.token];delete data.reservations[id];}
        else if(!entry.claimed)delete data.reservations[id];
        else if(data.responsive&&entry.reject){entry.reject(Error("音声の照合を中断しました"));delete data.reservations[id];}}
    }else{
      const entry=data.reservations[id];
      if(!entry||entry.cancelled||entry.token!==args.playbackToken)throw Error("音声準備と受取トークンが一致しません");
      delete data.reservations[id];
      if(data.loseReceipt)throw Error("模擬受取確認の応答喪失");
    }
    return null;
  }
  const recovery = await fixture.recovery.invoke(command,args);
  if(recovery !== undefined) { fixture.calls.push({ command,args: structuredClone(args) }); return recovery; }
  if (command === "get_analysis_notification_jobs") {
    fixture.calls.push({ command, args: structuredClone(args) });
    if (fixture.notificationReadFail) throw Error("模擬解析通知読込失敗");
    const latest = new Set(), active = new Set(["queued", "running", "pausing", "paused", "interrupted"]);
    return structuredClone([...fixture.notificationJobs].sort((a, b) => b.createdAt - a.createdAt).filter(job => {
      const group = JSON.stringify([job.meetingId, job.kind]), first = !latest.has(group); latest.add(group);
      return first || active.has(job.state) || args.watchedJobIds.includes(job.id);
    }));
  }
  if (command === "get_analysis_notification_output") {
    fixture.calls.push({ command, args: structuredClone(args) }); fixture.notificationProofReads.push(args.jobId);
    if (fixture.notificationProofMode === "lost") throw Error("模擬出力照合の応答喪失");
    if (fixture.notificationProofMode === "delay") await new Promise(resolve => { fixture.releaseNotificationProof = resolve; });
    const job = fixture.notificationJobs.find(job => job.id === args.jobId);
    if (!job || job.state !== "succeeded") throw Error("模擬通知対象の成功ジョブがありません");
    return { job: { ...structuredClone(job), ...(fixture.notificationProofMode === "foreign" ? { meetingId: meetings[1].id } : {}) }, contentHash: "c".repeat(64), publicationState: fixture.notificationProofMode === "hidden" ? "internal_snapshot" : fixture.notificationPublication ?? "candidate" };
  }
  if (command === "get_ui_preferences" && fixture.uiReadFail) throw Error("模擬表示設定読込失敗");
  if (command === "get_ui_preferences" && fixture.uiPreferences) return structuredClone(fixture.uiPreferences);
  const baseline = dataIdleRead(command);
  if (baseline !== undefined) return baseline;
  fixture.calls.push({ command, args });
  switch (command) {
    case "get_recording_snapshot": return structuredClone(fixture.recording ?? { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null });
    case "capture_available": return false;
    case "save_ui_preferences": {
      if(fixture.uiMode === "delay") await new Promise(resolve=>{fixture.releaseUiSave=resolve;});
      if(fixture.uiMode === "fail") throw Error("模擬表示設定保存失敗");
      const current = fixture.uiPreferences ?? dataIdleRead("get_ui_preferences");
      if (args.expectedLockVersion !== current.lockVersion) throw Error("模擬表示設定の競合");
      fixture.uiPreferences = { ...current, payload: structuredClone(args.payload), lockVersion: current.lockVersion + (JSON.stringify(current.payload) === JSON.stringify(args.payload) ? 0 : 1) };
      await emit("ui-preferences-changed", fixture.uiPreferences.lockVersion);
      if(fixture.uiMode === "lostUnreadable") fixture.uiReadFail=true;
      if(["lost","lostUnreadable"].includes(fixture.uiMode)) throw Error("模擬表示設定の応答喪失");
      return structuredClone(fixture.uiPreferences);
    }
    case "get_audio_import": return { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, saved: null, errorMessage: null };
    case "get_model_placements": return [];
    case "get_model_profiles": return ["standard", "high_accuracy"].map(name => ({ name, id: null, profileVersion: 0, components: [], configurationComplete: false }));
    case "get_analysis_flow": return null;
    case "get_diarization_tracks": if (fixture.failTracks) throw Error("模擬トラック読込失敗"); return fixture.trackPlan??{ tracks: [{ id: "microphone", kind: "mic", startUs: 0, endUs: 15000000 }, { id: "system", kind: "system", startUs: 20000000, endUs: 25000000 }] };
    case "get_diarization_preferences": return structuredClone(fixture.diarizationPreferences);
    case "save_diarization_preferences":
      await new Promise(resolve => setTimeout(resolve, fixture.persistenceDelayMs));
      if (fixture.failDiarizationPreferences) throw Error("模擬話者分離設定の保存失敗");
      if (args.expectedLockVersion !== fixture.diarizationPreferences.lockVersion) throw Error("模擬話者分離設定競合");
      fixture.diarizationPreferences = { ...fixture.diarizationPreferences, payload: structuredClone(args.payload), lockVersion: fixture.diarizationPreferences.lockVersion + 1 };
      return structuredClone(fixture.diarizationPreferences);
    case "get_speaker_reviews": return { analysisId: args.request.analysisId, contentHash: args.request.expectedContentHash, reviews: args.request.utteranceIds.map(utteranceId => {
      const review={utteranceId,trackId:"microphone",estimatedState:"unknown",manualAssignment:false,segmentCount:0,overlapCount:0,page:args.request.page,pageCount:1,segments:[],overlaps:[],...structuredClone(fixture.speakerReviews?.[utteranceId]??{})};
      return args.request.includeIntervals?review:{...review,segments:[],overlaps:[]};
    }) };
    case "list_meetings": return meetings;
    case "get_meeting":
      if (fixture.failMeetingRead) throw Error("模擬会議読込失敗");
      return fixture.missingMeetingId === args.id ? null : meetings.find((meeting) => meeting.id === args.id) ?? null;
    case "get_current_analysis": return args.meetingId === meetings[0].id ? structuredClone(currentAnalysis()) : null;
    case "get_transcript_draft":
      if (fixture.failDraftRead) throw Error("模擬下書き読込失敗");
      return args.meetingId === meetings[0].id ? stampFixtureDraft(fixture.draft) : null;
    case "begin_transcript_draft":
      fixture.draft = { id: crypto.randomUUID(), meetingId: meetings[0].id, baseAnalysisId: fixture.currentId, payload: { schema_version: 1, speaker_names: [], utterances: [] }, lockVersion: 0, updatedAt: 1791028800000 };
      return stampFixtureDraft(fixture.draft);
    case "save_transcript_draft":
      await new Promise((resolve) => setTimeout(resolve, fixture.persistenceDelayMs));
      if (fixture.failSave) throw new Error("模擬保存競合");
      if (args.expectedLockVersion !== fixture.draft.lockVersion) throw new Error("版番号不一致");
      fixture.transcriptHistory.push(structuredClone(fixture.draft.payload));
      fixture.draft = { ...fixture.draft, payload: structuredClone(args.payload), lockVersion: fixture.draft.lockVersion + 1 };
      await stampFixtureDraft(fixture.draft);
      if (fixture.lostTranscriptSaveResponse) { fixture.lostTranscriptSaveResponse = false; throw Error("模擬本文保存応答喪失"); }
      return stampFixtureDraft(fixture.draft);
    case "undo_transcript_edit": {
      if (args.draftId !== fixture.draft.id || args.expectedLockVersion !== fixture.draft.lockVersion || !fixture.transcriptHistory.length) throw Error("模擬取り消し対象不一致");
      fixture.draft = { ...fixture.draft, payload: fixture.transcriptHistory.pop(), lockVersion: fixture.draft.lockVersion + 1 };
      return stampFixtureDraft(fixture.draft);
    }
    case "get_analysis_preferences": return structuredClone(fixture.preferences);
    case "get_vocabulary_book": return structuredClone(fixture.vocabulary[args.meetingId ?? "global"] ?? { id: `meeting:${args.meetingId}`, payload: [], lockVersion: 0 });
    case "get_meeting_vocabulary": return { global: structuredClone(fixture.vocabulary.global), meeting: structuredClone(fixture.vocabulary[args.meetingId] ?? { id: `meeting:${args.meetingId}`, payload: [], lockVersion: 0 }) };
    case "save_vocabulary_book": {
      await new Promise((resolve) => setTimeout(resolve, fixture.persistenceDelayMs));
      const key = args.meetingId ?? "global";
      const book = fixture.vocabulary[key] ?? { id: `meeting:${key}`, payload: [], lockVersion: 0 };
      if (fixture.failVocabulary || book.lockVersion !== args.expectedLockVersion) throw new Error("模擬用語保存競合");
      const terms = new Set();
      for (const entry of args.payload) { if (!entry.term.trim() || entry.term.trim() !== entry.term || terms.has(entry.term)) throw new Error("用語の空欄・重複を修正してください"); terms.add(entry.term); }
      fixture.vocabulary[key] = { ...book, payload: structuredClone(args.payload), lockVersion: book.lockVersion + 1 };
      return structuredClone(fixture.vocabulary[key]);
    }
    case "save_analysis_preferences":
      await new Promise((resolve) => setTimeout(resolve, fixture.persistenceDelayMs));
      if (fixture.failPreferences) throw new Error("模擬設定保存競合");
      if (args.expectedLockVersion !== fixture.preferences.lockVersion) throw new Error("設定版番号不一致");
      fixture.preferences = { ...fixture.preferences, payload: structuredClone(args.payload), lockVersion: fixture.preferences.lockVersion + 1 };
      return structuredClone(fixture.preferences);
    case "get_analysis_jobs": return args.meetingId === meetings[0].id ? structuredClone(fixture.jobs) : [];
    case "get_analysis_versions": return args.meetingId === meetings[0].id ? structuredClone(versions()) : { meetingLockVersion: 1, currentId: null, previousId: null, draft: null, versions: [] };
    case "get_analysis_preview": return structuredClone(args.analysisId === analysis.id ? analysis : fixture.candidate);
    case "get_analysis_version_draft": return structuredClone(fixture.draft?.baseAnalysisId === args.analysisId ? fixture.draft : fixture.archivedDrafts[args.analysisId] ?? null);
    case "start_transcription": {
      if (fixture.failStart) throw Error("模擬文字起こし開始失敗");
      const book = fixture.vocabulary[args.request.meetingId] ?? { lockVersion: 0 };
      if (args.request.expectedVocabulary.globalVersion !== fixture.vocabulary.global.lockVersion || args.request.expectedVocabulary.meetingVersion !== book.lockVersion) throw new Error("確認した用語リストが変更されました。用語を再確認して開始してください");
      fixture.jobs.unshift({ id: "job-started", kind: "transcription", state: "queued", completedUnits: 0, totalUnits: 3, errorCode: null, outputAnalysisId: null, pauseReason: null, specHash: "c".repeat(64), retryOfJobId: null, attempt: 1 });
      return structuredClone(fixture.jobs[0]);
    }
    case "start_diarization": {
      if (fixture.failStart) throw Error("模擬話者分離開始失敗");
      const job = { ...fixture.jobs[0], id: crypto.randomUUID(), kind: "diarization", state: "queued", operationId: args.request.input.operationId, completedUnits: 0, retryOfJobId: null, attempt: 1 };
      fixture.jobs.unshift(job); return structuredClone(job);
    }
    case "retry_transcription":
    case "retry_diarization": {
      if (fixture.failRetry) throw new Error("模擬再試行エラー");
      const request = args.request;
      const source = fixture.jobs.find((job) => job.id === request.jobId);
      if (!source || request.meetingId !== meetings[0].id || request.expectedSpecHash !== source.specHash || source.kind !== (command === "retry_diarization" ? "diarization" : "transcription") || !["failed", "cancelled"].includes(source.state)) throw new Error("再試行条件不一致");
      const existing = fixture.jobs.find((job) => job.operationId === request.operationId);
      if (existing) return structuredClone(existing);
      const next = { ...source, id: `retry-${request.operationId}`, state: "queued", retryOfJobId: source.id, attempt: source.attempt + 1, operationId: request.operationId, errorCode: null };
      fixture.jobs.unshift(next);
      if (fixture.lostRetryResponse) { fixture.lostRetryResponse = false; throw Error("模擬再試行応答喪失"); }
      return structuredClone(next);
    }
    case "pause_analysis_job":
    case "resume_analysis_job":
    case "cancel_analysis_job": {
      const job = fixture.jobs.find((item) => item.id === args.jobId);
      job.state = command === "pause_analysis_job" ? "paused" : command === "resume_analysis_job" ? "running" : "cancelled";
      return structuredClone(job);
    }
    case "adopt_analysis_candidate": {
      if (fixture.failAdopt) throw new Error("模擬採用競合");
      const request = args.request;
      const state = versions();
      if (request.expectedContentHash !== fixture.candidate.contentHash || request.expectedMeetingLockVersion !== state.meetingLockVersion || JSON.stringify(request.expectedDraft) !== JSON.stringify(state.draft)) throw new Error("採用時の競合");
      fixture.previousId = fixture.currentId;
      fixture.currentId = fixture.candidate.id;
      fixture.candidateVisible = false;
      switchDraft(fixture.currentId);
      meetings[0].lockVersion++;
      if (fixture.lostPublicationResponse) { fixture.lostPublicationResponse = false; throw Error("模擬採用応答喪失"); }
      return fixture.foreignPublicationReceipt ? { id: "foreign", contentHash: "wrong", previousId: null, meetingLockVersion: 0 } : { id: fixture.currentId, contentHash: fixture.candidate.contentHash, previousId: fixture.previousId, meetingLockVersion: meetings[0].lockVersion };
    }
    case "restore_analysis_version": {
      const target = args.expectedPreviousId === analysis.id ? analysis : fixture.candidate;
      const state = versions();
      if (args.expectedPreviousId !== fixture.previousId || args.expectedContentHash !== target.contentHash || args.expectedMeetingLockVersion !== state.meetingLockVersion || JSON.stringify(args.expectedDraft) !== JSON.stringify(state.draft)) throw new Error("復元時の競合");
      const prior = fixture.currentId;
      fixture.currentId = fixture.previousId; fixture.previousId = prior;
      switchDraft(fixture.currentId); meetings[0].lockVersion++;
      if (fixture.lostPublicationResponse) { fixture.lostPublicationResponse = false; throw Error("模擬復元応答喪失"); }
      return fixture.foreignPublicationReceipt ? { id: "foreign", contentHash: "wrong", previousId: null, meetingLockVersion: 0 } : { id: fixture.currentId, contentHash: target.contentHash, previousId: fixture.previousId, meetingLockVersion: meetings[0].lockVersion };
    }
    case "prepare_meeting_playback": {
      const entry=preparationEntry(args);
      const data=fixture.meetingAudio,mode=args.request.mode;
      const info={token:crypto.randomUUID(),sampleRate:48000,channels:1,totalFrames:Math.ceil(data.endUs*48000/1000000),startUs:0,endUs:data.endUs,trackKind:mode,alignment:"unverified"};
      const gaps=data.gaps.filter(gap=>mode==="both"? ["mic","system"].includes(gap.trackKind):gap.trackKind===mode)
        .map(gap=>({...structuredClone(gap),startFrame:Math.floor(gap.startUs*48000/1e6),endFrame:Math.ceil(gap.endUs*48000/1e6)}))
        .sort((a,b)=>a.startFrame-b.startFrame||a.endFrame-b.endFrame||a.trackKind.localeCompare(b.trackKind));
      if(gaps.length)info.missingRanges=gaps;
      if(data.holdModes.includes(mode)) await new Promise((resolve,reject)=>{if(entry)entry.reject=reject;data.pending.push({mode,preparationId:args.preparationId,token:info.token,release:resolve});});
      if(data.failModes.includes(mode)) throw Error("指定した会議音声に欠損があります");
      Object.assign(info,data.corrupt??{});return publishPlayback(entry,info);
    }
    case "prepare_utterance_playback": {
      const entry=preparationEntry(args);
      if (fixture.holdPlayback) await new Promise((resolve,reject) => { fixture.releasePlayback = resolve;if(entry)entry.reject=reject; });
      if (fixture.failPlayback) throw new Error("音声ファイルのハッシュが保存時と一致しません");
      return publishPlayback(entry,{ token: "audio-token", sampleRate: 48000, channels: 2, totalFrames: 720000, startUs: 0, endUs: 15000000, trackKind: "mic", alignment: "unverified" });
    }
    case "read_playback_chunk": {
      const info=fixture.meetingAudio.sessions[args.token];
      let frames = Math.min(65536, (info?.totalFrames??720000) - args.offsetFrames);
      for(const gap of info?.missingRanges??[]) {
        if(gap.startFrame<=args.offsetFrames&&args.offsetFrames<gap.endFrame)throw Error("現在位置の音声は欠損しています");
        if(gap.startFrame>args.offsetFrames)frames=Math.min(frames,gap.startFrame-args.offsetFrames);
      }
      const channels=info?.channels??2;
      const bytes = new ArrayBuffer(12 + frames * channels * 4);
      const view = new DataView(bytes);
      view.setUint32(0, 48000, true);
      view.setUint32(4, channels, true);
      view.setUint32(8, frames, true);
      return bytes; // Silent PCM: browser verification must not produce noise.
    }
    case "close_playback": if(fixture.preparation.failClose)throw Error("模擬音声解放の失敗");delete fixture.meetingAudio.sessions[args.token]; return null;
    case "finish_main_close": return null;
    case "get_minutes_preferences": return { id: "llama_profiles", lockVersion: 0, payload: { standard: { executablePath: "", modelPath: "", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 }, highAccuracy: { executablePath: "", modelPath: "", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 } } };
    case "get_minutes_versions": return { meetingLockVersion: 1, currentId: null, previousId: null, draft: null, versions: [] };
    case "get_current_minutes": return null;
    case "get_minutes_draft": return null;
    case "get_minutes_exports": return [];
    default: throw new Error(`Unexpected fixture command: ${command}`);
  }
}, { shouldMockEvents: true });
await import("../src/main.tsx");
