// Development-only IPC simulation. No files, recording or real meetings changed.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { dataIdleRead } from "./data-idle-browser-fixture.js";
const idle = dataIdleRead("get_data_operation");
const recording = { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null };
const f = window.dataFixture = {
  snapshot: idle, calls: [], mode: "success", replacesExisting: false, blocker: null,
  failRead: false, failSave: false, saveDelay: 0, suppressEvents: false, selectLost: false, previewMode: "success", complete: () => {}, archiveId: crypto.randomUUID(),
  book: { id: "global", payload: [], lockVersion: 0 },
  meetings: [{ id: crypto.randomUUID(), title: "既存会議", lifecycle: "draft", createdAt: 1791160000000, updatedAt: 1791160000000, lockVersion: 0 }],
  restored: [], published: 0, lastRequest: null,
  changed: async patch => { f.snapshot = { ...f.snapshot, ...patch, revision: f.snapshot.revision + 1 }; if (!f.suppressEvents) await emit("data-operation-changed", structuredClone(f.snapshot)); },
};
const profile = { executablePath: "", modelPath: "", threads: 4, useGpu: false };
window.isTauri = true; mockWindows("main");
mockIPC(async (command, args) => {
  f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_data_operation") { if (f.failRead) throw Error("模擬状態読込失敗"); return structuredClone(f.snapshot); }
  if (command === "get_data_operation_availability") return { available: !f.snapshot.busy && !f.blocker, reason: f.snapshot.busy ? "バックアップ／復元中" : f.blocker };
  if (command === "list_restored_meeting_ids") return [...f.restored];
  if (command === "select_data_operation") {
    if (f.blocker || f.snapshot.busy) throw Error("処理の停止・完了を待ってください");
    const token = crypto.randomUUID(), confirmationId = crypto.randomUUID();
    f.lastRequest = null;
    await f.changed({ ...idle, revision: f.snapshot.revision, token, kind: args.kind, busy: true, phase: "verifying" });
    const ready = async () => {
      if (f.snapshot.phase === "cancelling") return f.changed({ phase: "cancelled", busy: false });
      if (f.previewMode === "invalid") return f.changed({ phase: "failed", busy: false, errorMessage: "不正な形式またはハッシュです" });
      if (f.previewMode === "cancel") return f.changed({ phase: "idle", busy: false });
      await f.changed({ phase: "ready", busy: false, retryAllowed: true,
        backupPreview: args.kind === "backup" ? { confirmationId, destination: "/safe/会議.monbackup", replacesExisting: f.replacesExisting } : null,
        restorePreview: args.kind === "restore" ? { confirmationId, batchId: crypto.randomUUID(), archiveId: f.archiveId, meetingTitles: ["点検会議", "計画会議"], requiredBytes: 32 * 1024 * 1024, availableBytes: f.previewMode === "capacity" ? 0 : 1024 * 1024 * 1024, vocabularyAdded: 1, vocabularyDeduplicated: 1, vocabularyConflicts: ["点検日"] } : null });
    };
    if (f.previewMode === "delay") f.complete = ready;
    else setTimeout(() => void ready(), 50);
    if (f.selectLost) throw Error("模擬ファイル選択の応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "start_data_operation") {
    const r = args.request;
    if (f.mode === "lostNoCommit") throw Error("模擬受付応答喪失・未受付");
    if (JSON.stringify(r) === JSON.stringify(f.lastRequest) && ["publishing", "completed", "ready", "attention"].includes(f.snapshot.phase)) return structuredClone(f.snapshot);
    const p = f.snapshot.backupPreview || f.snapshot.restorePreview;
    if (r.token !== f.snapshot.token || r.confirmationId !== p?.confirmationId || f.snapshot.busy) throw Error("確認が変更されています");
    f.lastRequest = structuredClone(r);
    if (r.action === "refresh") {
      await f.changed({ phase: "ready", busy: false, retryAllowed: true, errorMessage: null, restorePreview: { ...p, confirmationId: crypto.randomUUID(), availableBytes: 1024 * 1024 * 1024 } });
      return structuredClone(f.snapshot);
    }
    if (f.mode === "forged") return { ...structuredClone(f.snapshot), phase: "completed", retryAllowed: false, revision: f.snapshot.revision + 1 };
    if (f.mode === "attention") { await f.changed({ phase: "attention", attentionRequired: true, retryAllowed: false, errorMessage: "保存後の結果が未確定です" }); return structuredClone(f.snapshot); }
    await f.changed({ phase: "publishing", busy: true, retryAllowed: false });
    const finish = async () => {
      if (f.snapshot.phase === "cancelling" && f.mode !== "cancelRace") return f.changed({ phase: "cancelled", busy: false, retryAllowed: true });
      f.published++;
      if (f.snapshot.kind === "backup") await f.changed({ phase: "completed", busy: false, backupReceipt: { archiveId: crypto.randomUUID(), meetingCount: 1, assetCount: 0, omittedPcmCount: 0, omittedLogCount: 0, size: 1024, sha256: "a".repeat(64) } });
      else {
        const ids = p.meetingTitles.map(title => { const id = crypto.randomUUID(); f.meetings.push({ id, title, lifecycle: "imported", sourceType: "audio_import", createdAt: 1791160000000, updatedAt: 1791160000000, lockVersion: 1 }); return id; });
        f.restored.push(...ids);
        await f.changed({ phase: "completed", busy: false, restoreReceipt: { batchId: p.batchId, archiveId: p.archiveId, meetingIds: ids, alreadyPublished: f.mode === "unknownStats", vocabularyAdded: f.mode === "unknownStats" ? null : 1, vocabularyDeduplicated: f.mode === "unknownStats" ? null : 1, vocabularyConflicts: f.mode === "unknownStats" ? null : ["点検日"] } });
      }
    };
    if (["delay", "cancelRace"].includes(f.mode)) f.complete = finish;
    else setTimeout(() => void finish(), 50);
    if (f.mode === "lost") throw Error("模擬公開応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "cancel_data_operation") {
    if (args.token !== f.snapshot.token) throw Error("取消対象が変更されています");
    if (f.snapshot.busy) await f.changed({ phase: "cancelling" });
    else if (!f.snapshot.backupReceipt && !f.snapshot.restoreReceipt && !f.snapshot.attentionRequired) await f.changed({ phase: "cancelled", retryAllowed: false, backupPreview: null, restorePreview: null });
    return structuredClone(f.snapshot);
  }
  if (command === "get_recording_snapshot") return recording;
  if (command === "capture_available") return false;
  if (command === "list_meetings") return structuredClone(f.meetings);
  if (command === "get_meeting") return structuredClone(f.meetings.find(m => m.id === args.id) || null);
  if (command === "get_vocabulary_book") return structuredClone(f.book);
  if (command === "save_vocabulary_book") {
    if (f.saveDelay) await new Promise(resolve => setTimeout(resolve, f.saveDelay));
    if (f.failSave || args.payload.some(t => !t.term.trim())) throw Error("模擬用語保存失敗");
    f.book = { id: "global", payload: structuredClone(args.payload), lockVersion: f.book.lockVersion + 1 };
    return structuredClone(f.book);
  }
  if (command === "get_model_placements") return [];
  if (command === "get_model_profiles") return ["standard", "high_accuracy"].map(name => ({ name, id: null, profileVersion: 0, components: [], configurationComplete: false }));
  if (["get_analysis_preferences", "get_diarization_preferences", "get_minutes_preferences"].includes(command)) return { id: "profiles", payload: { standard: profile, highAccuracy: profile }, lockVersion: 0 };
  if (["get_audio_import"].includes(command)) return { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, fileName: null, info: null, sourceBytes: null, requiredBytes: null, availableBytes: null, completed: 0, saved: null, errorMessage: null };
  if (command === "finish_main_close") return null;
  throw Error(`Unexpected data fixture command: ${command}`);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
