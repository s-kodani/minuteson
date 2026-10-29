// Development-only IPC simulation. Never invokes native audio/file operations.
import { mockIPC } from "./startup-ready-mocks.ts";
import { emit } from "@tauri-apps/api/event";
import { stampFixtureDraft } from "./transcript-draft-fixture.js";
window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const f = window.dataFixture;
f.meetings[0].lifecycle = "recorded";
f.meetings[0].sourceType = "recording";
f.snapshot = { ...f.snapshot, deletionPreview: null, deletionReceipt: null, audioDeletionPreview: null, audioDeletionReceipt: null };
const idle = structuredClone(f.snapshot), changed = f.changed;
f.changed = patch => changed(patch.kind && patch.kind !== "audio_deletion" ? { ...patch, audioDeletionPreview: null, audioDeletionReceipt: null } : patch);
const d = window.audioDeletionFixture = {
  mode: "success", fileCount: 2, unknownCount: 0, longPath: false, blocked: false, cold: false, restart: false, listFail: false, pageFail: false,
  foreignPage: false, pathsDelay: false, releasePage: null, reviewLost: false, items: [], deleted: 0, kept: 0, last: null, complete: () => {}, analysisReads: 0, draftReads: 0,
  pending: (phase = "not_started", altered = false) => {
    d.blocked = true; d.cold = true;
    const p = makePreview(phase);
    if (altered) Object.assign(p, { executeAllowed: false, resumeReason: "確認後に音声が変わりました。保持しています", reconfirmationAllowed: ["files", "availability", "awaiting_ack"].includes(phase) });
    d.pendingPreview = p;
    d.items = [{ operationId: p.operationId, state: ["awaiting_ack", "restored_unavailable"].includes(phase) ? "completed" : "installing", meetingId: p.meetingId, reviewAllowed: true, errorMessage: null, startupReason: phase === "restored_unavailable" ? "restored_audio_scope_reappeared" : null }];
  },
};
const makePreview = (phase = "review") => ({ confirmationId: crypto.randomUUID(), operationId: crypto.randomUUID(), meetingId: f.meetings[0].id, sourceOperationId: null, reconfirmationSourceKind: null,
  phase, pending: phase !== "review", executeAllowed: phase !== "restored_unavailable", resumeReason: phase === "restored_unavailable" ? "元記録に削除同意がありません" : null, keepAllowed: phase === "not_started", reconfirmationAllowed: phase === "restored_unavailable",
  includeUnregisteredRecorded: ["review", "restored_unavailable"].includes(phase) ? null : d.unknownCount > 0, pathCount: d.fileCount + 1,
  processedFiles: phase === "files" ? 1 : ["availability", "awaiting_ack"].includes(phase) ? d.fileCount : 0,
  audioFiles: d.fileCount, audioBytes: 20 * d.fileCount, missingAudioFiles: 0, changedAudioFiles: 0, unregisteredAudioFiles: d.unknownCount, unclassifiedFiles: 1,
  retained: { analysisRevisions: 2, utterances: 105, minutesRevisions: 2, drafts: 2, editOperations: 3, evidenceLinks: 4, jobs: 3, externalExportRecords: 1 },
  tracks: [{ id: "11111111-1111-4111-8111-111111111111", kind: "mic", state: "ready", chunks: 2, durationUs: 60000000 }, { id: "21111111-1111-4111-8111-111111111111", kind: "system", state: "ready", chunks: 2, durationUs: 60000000 }], priorDeletionCompleted: phase === "awaiting_ack" });
const analysis = { id: crypto.randomUUID(), meetingId: f.meetings[0].id, contentHash: "a".repeat(64), speakers: [{ id: crypto.randomUUID(), label: "話者1", displayName: "田中" }], utterances: [{ id: crypto.randomUUID(), trackId: "11111111-1111-4111-8111-111111111111", startUs: 0, endUs: 1000000, recognizedText: "点検日です。", effectiveText: "点検日です。", attributionState: "assigned", speakerIds: [] }] };
d.draft = { id: crypto.randomUUID(), meetingId: analysis.meetingId, baseAnalysisId: analysis.id, payload: { schema_version:4, speaker_names: [], utterances: [] }, lockVersion: 0, updatedAt: 1791160000000 };
export const audioDeletionIPC = async (command, args) => {
  if (command === "get_meeting_deletion_availability") {
    const rows = await dataSaveIPC(command, args);
    for (const row of rows) if (row.meetingId === f.meetings[0].id) row.audioDeleted = d.audioStateUnknown ? null : d.deleted > 0;
    if (d.holdAudioAvailability) { d.holdAudioAvailability = false; await new Promise(resolve => { d.releaseAudioAvailability = resolve; }); }
    return rows;
  }
  if (["get_audio_deletion_items", "review_meeting_audio_deletion", "review_pending_audio_deletion", "get_audio_deletion_paths"].includes(command)
    || (["start_data_operation", "cancel_data_operation"].includes(command) && f.snapshot.kind === "audio_deletion")) f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_deletion_items") return { items: [], deletingMeetingIds: [], writeBlocked: d.blocked, restartRequired: d.restart, reason: d.blocked ? "音声削除のため書込停止" : null };
  if (command === "get_model_deletion_items") return { items: [], writeBlocked: d.blocked, restartRequired: d.restart, reason: d.blocked ? "音声削除のため書込停止" : null };
  if (command === "get_audio_deletion_items") {
    if (d.listFail) throw Error("模擬音声削除一覧失敗");
    return { items: structuredClone(d.items), writeBlocked: d.blocked, restartRequired: d.restart, reason: d.blocked ? "音声削除のため書込停止" : null };
  }
  if (command === "get_data_operation_availability" && d.restart) return { available: false, reason: "保存先の切替後は終了して再起動が必要です", reasonCode: "relocation_restart_required" };
  if (command === "get_current_analysis") { d.analysisReads++; return structuredClone(analysis); }
  if (command === "get_transcript_draft") { d.draftReads++; return stampFixtureDraft(d.draft); }
  if (command === "save_transcript_draft") {
    f.calls.push({ command, args: structuredClone(args) });
    if (d.blocked || args.expectedLockVersion !== d.draft.lockVersion) throw Error("音声削除の書込停止中、または編集の競合です");
    d.draft = { ...d.draft, payload: structuredClone(args.payload), lockVersion: d.draft.lockVersion + 1 };
    return stampFixtureDraft(d.draft);
  }
  if (command === "get_analysis_versions") return { meetingLockVersion: 0, currentId: analysis.id, previousId: null, draft: { id: d.draft.id, lockVersion: d.draft.lockVersion }, versions: [{ id: analysis.id, revisionNo: 1, contentHash: analysis.contentHash, publicationState: "committed", utteranceCount: 1, createdAt: 1791160000000, profile: { qualityStatus: "unverified" }, config: { diarization: "pending", timelineAlignment: "unverified" } }] };
  if (command === "get_diarization_preferences") { const engine = { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [], threads: 4, clusterThresholdMilli: 1000, windowShiftMilli: 500, minDurationOnMs: 0, minDurationOffMs: 0 }; return { id: "profiles", payload: { standard: engine, highAccuracy: engine }, lockVersion: 0 }; }
  if (command === "get_meeting_analysis_audio") return { tracks: d.deleted ? [] : [{ id: analysis.utterances[0].trackId, kind: "mic", startUs: 0, endUs: 1000000 }] };
  if (["get_analysis_jobs", "get_minutes_exports"].includes(command)) return [];
  if (command === "get_analysis_flow") return null;
  if (command === "get_minutes_versions") return { meetingLockVersion: 0, currentId: null, previousId: null, draft: null, versions: [] };
  if (command === "review_meeting_audio_deletion" || command === "review_pending_audio_deletion") {
    if(d.admissionHold){
      const id=crypto.randomUUID();
      await emit("deletion-notice-wait",{id,waiting:true});
      await new Promise(resolve=>{d.releaseAdmission=resolve;});
      await emit("deletion-notice-wait",{id,waiting:false});
      if(d.admissionTimeout)throw "audio_deletion_not_started:通知履歴の処理が終わらないため、削除を開始できませんでした";
    }
    if ((f.blocker && f.blocker!=="録音通知履歴の確認・更新中") || f.snapshot.busy || (command === "review_meeting_audio_deletion" && (d.blocked || args.meetingId !== f.meetings[0].id))) throw Error("音声の停止・再検査が必要です");
    const p = command === "review_meeting_audio_deletion" ? makePreview() : { ...d.pendingPreview, confirmationId: crypto.randomUUID() };
    if (!p || (command === "review_pending_audio_deletion" && p.operationId !== args.operationId)) throw Error("不明な操作");
    d.last = null;
    await f.changed({ ...idle, revision: f.snapshot.revision, token: crypto.randomUUID(), kind: "audio_deletion", phase: "verifying_audio_delete_scope", busy: true, restartRequired: d.restart });
    setTimeout(() => void f.changed({ phase: "ready", busy: false, retryAllowed: true, audioDeletionPreview: p }), 30);
    if (d.reviewLost) throw Error("模擬音声確認応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "get_audio_deletion_paths") {
    const p = structuredClone(f.snapshot.audioDeletionPreview);
    if (d.pageFail) throw Error("模擬音声対象一覧失敗");
    if (!p || p.confirmationId !== args.confirmationId || args.offset % 50) throw Error("確認が失効");
    const page = { confirmationId: p.confirmationId, offset: args.offset, total: p.pathCount, nextOffset: args.offset + 50 < p.pathCount ? args.offset + 50 : null,
      entries: Array.from({ length: Math.min(50, p.pathCount - args.offset) }, (_, i) => {
        const n = args.offset + i, meeting = d.foreignPage ? crypto.randomUUID() : p.meetingId;
        if (d.longPath && n === 0 && p.unregisteredAudioFiles > 0) return {
          relativePath: `meetings/${meeting}/audio/a-${"あ".repeat(65)}/.mon-delete-audio-31111111-1111-4111-8111-111111111111-r32-199999`,
          assetId: null, disposition: "historical_audio", present: true, changed: false, bytes: 20, sha256: "b".repeat(64),
        };
        return { relativePath: `meetings/${meeting}/audio/file-${String(n).padStart(6, "0")}.caf`, assetId: n >= p.unregisteredAudioFiles && n < p.audioFiles ? crypto.randomUUID() : null,
          disposition: n === p.audioFiles ? "unclassified" : n < p.unregisteredAudioFiles ? "producer_audio_leftover" : "registered_audio", present: true, changed: false, bytes: 20, sha256: "b".repeat(64) };
      }) };
    if (d.pathsDelay) { d.pathsDelay = false; await new Promise(resolve => { d.releasePage = resolve; }); }
    return page;
  }
  if (command === "start_data_operation" && f.snapshot.kind === "audio_deletion") {
    const r = args.request, p = f.snapshot.audioDeletionPreview;
    if (d.mode === "lostNoCommit") throw Error("模擬未受付応答喪失");
    if (JSON.stringify(r) === JSON.stringify(d.last) && (f.snapshot.busy || ["completed", "attention"].includes(f.snapshot.phase) || (["refresh", "review_remaining"].includes(r.action) && f.snapshot.phase === "ready"))) return structuredClone(f.snapshot);
    if (!p || r.token !== f.snapshot.token || r.confirmationId !== p.confirmationId || f.snapshot.busy) throw Error("確認が変更されています");
    d.last = structuredClone(r);
    if (r.action === "refresh") {
      await f.changed({ phase: "ready", retryAllowed: true, busy: false, attentionRequired: false, errorMessage: null, audioDeletionPreview: { ...p, confirmationId: crypto.randomUUID(), executeAllowed: p.phase !== "restored_unavailable" && d.mode !== "unknown", resumeReason: p.phase === "restored_unavailable" ? "元記録に削除同意がありません" : d.mode === "unknown" ? "変更された音声を保持" : null, reconfirmationAllowed: p.phase === "restored_unavailable" || (d.mode === "unknown" && ["files", "availability", "awaiting_ack"].includes(p.phase)) } });
      return structuredClone(f.snapshot);
    }
    if (r.action === "review_remaining") {
      if (!p.reconfirmationAllowed) throw Error("再同意できません");
      if (p.phase === "restored_unavailable") d.originalRestoredItem = structuredClone(d.items.find(i => i.operationId === p.operationId));
      const remaining = ["files", "availability"].includes(p.phase);
      await f.changed({ phase: "ready", retryAllowed: true, busy: false, attentionRequired: false, errorMessage: null, audioDeletionPreview: { ...p, confirmationId: crypto.randomUUID(), operationId: remaining ? p.operationId : crypto.randomUUID(), sourceOperationId: p.operationId, reconfirmationSourceKind: remaining ? "native_audio_pending" : p.phase === "restored_unavailable" ? "restored_audio_unavailable" : "native_audio_deletion", phase: "reconfirmation", pending: true, executeAllowed: true, resumeReason: null, reconfirmationAllowed: false, keepAllowed: false, includeUnregisteredRecorded: null, processedFiles: 0, unregisteredAudioFiles: d.unknownCount, priorDeletionCompleted: p.phase === "awaiting_ack" } });
      return structuredClone(f.snapshot);
    }
    if (r.action === "keep") {
      if (!p.keepAllowed) throw Error("開始済み削除は保持不可");
      d.kept++; d.items = d.originalRestoredItem ? [d.originalRestoredItem] : []; d.restart = d.cold && !d.items.length; d.blocked = d.restart || !!d.items.length;
      await f.changed({ phase: "completed", retryAllowed: false, attentionRequired: false, errorMessage: null, restartRequired: d.restart, audioDeletionPreview: { ...p, phase: "retained", pending: false, executeAllowed: false }, audioDeletionReceipt: { operationId: p.operationId, meetingId: p.meetingId, audioDeleted: false, retained: true, historyRetained: true, restartRequired: d.restart } });
      return structuredClone(f.snapshot);
    }
    if (!p.executeAllowed || f.snapshot.phase !== "ready" || (r.action === "publish" && ["review", "reconfirmation"].includes(p.phase) && p.unregisteredAudioFiles) || (r.action === "delete_including_unregistered" && !["review", "reconfirmation"].includes(p.phase))) throw Error("音声削除の同意が必要");
    if (d.mode === "forged") return { ...structuredClone(f.snapshot), revision: f.snapshot.revision + 1, phase: "completed", retryAllowed: false, audioDeletionReceipt: { operationId: crypto.randomUUID(), meetingId: p.meetingId, audioDeleted: true, retained: false, historyRetained: true, restartRequired: false } };
    d.blocked = true;
    const policy = p.includeUnregisteredRecorded ?? r.action === "delete_including_unregistered";
    // Same-operation remaining consent does not make the old operation unstarted.
    const remaining = p.reconfirmationSourceKind === "native_audio_pending";
    const savedSource = remaining ? null : p.sourceOperationId;
    d.items = [{ operationId: p.operationId, state: "installing", meetingId: p.meetingId, reviewAllowed: true, errorMessage: null, startupReason: null }];
    if (d.mode === "unstarted") {
      d.pendingPreview = { ...p, phase: remaining ? "files" : "not_started", sourceOperationId: savedSource, reconfirmationSourceKind: null, pending: true, executeAllowed: false, resumeReason: remaining ? "再同意保存後に停止。残り音声を保持" : "同意保存後に停止。音声は未開始のまま保持", keepAllowed: !remaining, reconfirmationAllowed: false, includeUnregisteredRecorded: policy, processedFiles: 0 };
      if (d.originalRestoredItem) d.items.push(d.originalRestoredItem);
      await f.changed({ phase: "attention", busy: false, retryAllowed: false, attentionRequired: true, audioDeletionPreview: d.pendingPreview, errorMessage: d.pendingPreview.resumeReason });
      return structuredClone(f.snapshot);
    }
    await f.changed({ phase: "deleting_audio_files", busy: true, retryAllowed: false });
    d.complete = async () => {
      if ((f.snapshot.phase === "cancelling" && d.mode !== "cancelRace") || d.mode === "partial") {
        d.pendingPreview = { ...p, phase: "files", sourceOperationId: savedSource, reconfirmationSourceKind: null, pending: true, executeAllowed: false, resumeReason: "停止後に残りを再検査してください", keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: policy, processedFiles: 1 };
        await f.changed({ phase: "attention", busy: false, attentionRequired: true, audioDeletionPreview: d.pendingPreview, errorMessage: "部分削除を保持しています。復元していません" }); return;
      }
      d.deleted++; d.items = []; d.restart = d.cold; d.blocked = d.restart;
      await f.changed({ phase: "completed", busy: false, retryAllowed: false, attentionRequired: false, errorMessage: null, restartRequired: d.restart,
        audioDeletionPreview: { ...p, phase: "completed", sourceOperationId: savedSource, reconfirmationSourceKind: null, pending: false, executeAllowed: false, keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: policy, processedFiles: p.audioFiles - (policy ? 0 : p.unregisteredAudioFiles), priorDeletionCompleted: true },
        audioDeletionReceipt: { operationId: p.operationId, meetingId: p.meetingId, audioDeleted: true, retained: false, historyRetained: true, restartRequired: d.restart } });
    };
    if (!["delay", "cancelRace"].includes(d.mode)) setTimeout(() => void d.complete(), 40);
    if (d.mode === "lost") throw Error("模擬音声削除応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "cancel_data_operation" && f.snapshot.kind === "audio_deletion") {
    if (args.token !== f.snapshot.token) throw Error("別の操作");
    if (f.snapshot.busy) await f.changed({ phase: "cancelling" });
    else if (!f.snapshot.audioDeletionReceipt && !f.snapshot.audioDeletionPreview?.pending) await f.changed({ phase: "cancelled", retryAllowed: false, audioDeletionPreview: null });
    return structuredClone(f.snapshot);
  }
  return dataSaveIPC(command, args);
};
mockIPC(audioDeletionIPC, { shouldMockEvents: true });
if (!window.skipAudioDeletionFixtureMain) await import("../src/bootstrap.tsx");
