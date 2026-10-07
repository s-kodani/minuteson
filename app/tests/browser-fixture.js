// Development-only browser fixture. Not imported by the application or build.
// Playwright routes index.html to this bootstrap for manual UI verification.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { dataIdleRead } from "./data-idle-browser-fixture.js";

const meetings = ["会議A", "会議B"].map((title, index) => ({ id: `meeting-${index}`, title, lifecycle: "recorded", createdAt: 1791028800000, updatedAt: 1791028800000, lockVersion: 1 }));
const analysis = { id: "analysis", meetingId: meetings[0].id, contentHash: "a".repeat(64), speakers: [{ id: "speaker-a", label: "話者A", displayName: "田中" }, { id: "speaker-b", label: "話者B", displayName: "佐藤" }], utterances: [
  { id: "speech-a", trackId: "microphone", startUs: 0, endUs: 15000000, recognizedText: "次の点検日は来週です。", effectiveText: "次の点検日は来週です。", attributionState: "assigned", speakerIds: ["speaker-a"] },
  { id: "speech-b", trackId: "system", startUs: 20000000, endUs: 25000000, recognizedText: "はい、確認します。", effectiveText: "はい、確認します。", attributionState: "overlap", speakerIds: ["speaker-a", "speaker-b"] },
] };
const candidate = { ...structuredClone(analysis), id: "candidate", contentHash: "b".repeat(64), utterances: analysis.utterances.map((item) => ({ ...item, id: `${item.id}-new`, effectiveText: `${item.effectiveText}（候補）` })) };
const engine = { executablePath: "", modelPath: "", vadModelPath: null, threads: 4, useGpu: false };
const diarEngine = { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [], threads: 4, clusterThresholdMilli: 500, windowShiftMilli: 100, minDurationOnMs: 0, minDurationOffMs: 0 };
const fixture = window.monBrowserFixture = { calls: [], failSave: false, failPlayback: false, failPreferences: false, failAdopt: false, failRetry: false, persistenceDelayMs: 0, draft: null,
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
const versions = () => ({ meetingLockVersion: meetings[0].lockVersion, currentId: fixture.currentId, previousId: fixture.previousId, draft: fixture.draft ? { id: fixture.draft.id, lockVersion: fixture.draft.lockVersion } : null,
  versions: [analysis, fixture.candidate].filter((item) => item.id === fixture.currentId || item.id === fixture.previousId || (item.id === fixture.candidate.id && fixture.candidateVisible)).map((item) => ({ id: item.id, revisionNo: item.id === analysis.id ? 1 : 2, contentHash: item.contentHash, publicationState: item.id === fixture.candidate.id && fixture.candidateVisible ? "candidate" : "committed", utteranceCount: item.utterances.length, createdAt: 1791028800000, profile: { qualityStatus: "unverified" }, config: { diarization: "pending", timelineAlignment: "unverified" } })) });
const switchDraft = (nextId) => {
  if (fixture.draft) fixture.archivedDrafts[fixture.draft.baseAnalysisId] = structuredClone(fixture.draft);
  fixture.draft = fixture.archivedDrafts[nextId] ? structuredClone(fixture.archivedDrafts[nextId]) : null;
};
window.isTauri = true;
mockWindows("main");
mockIPC(async (command, args) => {
  if (command === "get_ui_preferences" && fixture.uiPreferences) return structuredClone(fixture.uiPreferences);
  const baseline = dataIdleRead(command);
  if (baseline !== undefined) return baseline;
  fixture.calls.push({ command, args });
  switch (command) {
    case "get_recording_snapshot": return structuredClone(fixture.recording ?? { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null });
    case "capture_available": return false;
    case "save_ui_preferences": {
      const current = fixture.uiPreferences ?? dataIdleRead("get_ui_preferences");
      if (args.expectedLockVersion !== current.lockVersion) throw Error("模擬表示設定の競合");
      fixture.uiPreferences = { ...current, payload: structuredClone(args.payload), lockVersion: current.lockVersion + (current.payload.theme === args.payload.theme ? 0 : 1) };
      await emit("ui-preferences-changed", fixture.uiPreferences.lockVersion);
      return structuredClone(fixture.uiPreferences);
    }
    case "get_audio_import": return { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, saved: null, errorMessage: null };
    case "get_analysis_flow": return null;
    case "get_diarization_tracks": return { tracks: [{ id: "microphone", kind: "mic", startUs: 0, endUs: 15000000 }, { id: "system", kind: "system", startUs: 20000000, endUs: 25000000 }] };
    case "get_diarization_preferences": return structuredClone(fixture.diarizationPreferences);
    case "save_diarization_preferences":
      await new Promise(resolve => setTimeout(resolve, fixture.persistenceDelayMs));
      if (args.expectedLockVersion !== fixture.diarizationPreferences.lockVersion) throw Error("模擬話者分離設定競合");
      fixture.diarizationPreferences = { ...fixture.diarizationPreferences, payload: structuredClone(args.payload), lockVersion: fixture.diarizationPreferences.lockVersion + 1 };
      return structuredClone(fixture.diarizationPreferences);
    case "get_speaker_reviews": return { analysisId: args.request.analysisId, contentHash: args.request.expectedContentHash, reviews: args.request.utteranceIds.map(utteranceId => ({ utteranceId, trackId: "microphone", estimatedState: "unknown", manualAssignment: false, segmentCount: 0, overlapCount: 0, page: args.request.page, pageCount: 1, segments: [], overlaps: [] })) };
    case "list_meetings": return meetings;
    case "get_meeting": return meetings.find((meeting) => meeting.id === args.id) ?? null;
    case "get_current_analysis": return args.meetingId === meetings[0].id ? structuredClone(currentAnalysis()) : null;
    case "get_transcript_draft": return args.meetingId === meetings[0].id ? fixture.draft : null;
    case "begin_transcript_draft":
      fixture.draft = { id: `draft-${fixture.currentId}`, meetingId: meetings[0].id, baseAnalysisId: fixture.currentId, payload: { schema_version: 1, speaker_names: [], utterances: [] }, lockVersion: 0, updatedAt: 1791028800000 };
      return structuredClone(fixture.draft);
    case "save_transcript_draft":
      await new Promise((resolve) => setTimeout(resolve, fixture.persistenceDelayMs));
      if (fixture.failSave) throw new Error("模擬保存競合");
      if (args.expectedLockVersion !== fixture.draft.lockVersion) throw new Error("版番号不一致");
      fixture.transcriptHistory.push(structuredClone(fixture.draft.payload));
      fixture.draft = { ...fixture.draft, payload: structuredClone(args.payload), lockVersion: fixture.draft.lockVersion + 1 };
      return structuredClone(fixture.draft);
    case "undo_transcript_edit": {
      if (args.draftId !== fixture.draft.id || args.expectedLockVersion !== fixture.draft.lockVersion || !fixture.transcriptHistory.length) throw Error("模擬取り消し対象不一致");
      fixture.draft = { ...fixture.draft, payload: fixture.transcriptHistory.pop(), lockVersion: fixture.draft.lockVersion + 1 };
      return structuredClone(fixture.draft);
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
      const book = fixture.vocabulary[args.request.meetingId] ?? { lockVersion: 0 };
      if (args.request.expectedVocabulary.globalVersion !== fixture.vocabulary.global.lockVersion || args.request.expectedVocabulary.meetingVersion !== book.lockVersion) throw new Error("確認した用語リストが変更されました。用語を再確認して開始してください");
      fixture.jobs.unshift({ id: "job-started", kind: "transcription", state: "queued", completedUnits: 0, totalUnits: 3, errorCode: null, outputAnalysisId: null, pauseReason: null, specHash: "c".repeat(64), retryOfJobId: null, attempt: 1 });
      return structuredClone(fixture.jobs[0]);
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
      return {};
    }
    case "restore_analysis_version": {
      const target = args.expectedPreviousId === analysis.id ? analysis : fixture.candidate;
      const state = versions();
      if (args.expectedPreviousId !== fixture.previousId || args.expectedContentHash !== target.contentHash || args.expectedMeetingLockVersion !== state.meetingLockVersion || JSON.stringify(args.expectedDraft) !== JSON.stringify(state.draft)) throw new Error("復元時の競合");
      const prior = fixture.currentId;
      fixture.currentId = fixture.previousId; fixture.previousId = prior;
      switchDraft(fixture.currentId); meetings[0].lockVersion++;
      return {};
    }
    case "prepare_utterance_playback":
      if (fixture.failPlayback) throw new Error("音声ファイルのハッシュが保存時と一致しません");
      return { token: "audio-token", sampleRate: 48000, channels: 2, totalFrames: 720000, startUs: 0, endUs: 15000000, trackKind: "mic", alignment: "unverified" };
    case "read_playback_chunk": {
      const frames = Math.min(65536, 720000 - args.offsetFrames);
      const bytes = new ArrayBuffer(12 + frames * 2 * 4);
      const view = new DataView(bytes);
      view.setUint32(0, 48000, true);
      view.setUint32(4, 2, true);
      view.setUint32(8, frames, true);
      return bytes; // Silent PCM: browser verification must not produce noise.
    }
    case "close_playback":
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
