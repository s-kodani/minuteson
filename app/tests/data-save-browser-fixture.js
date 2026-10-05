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
  restored: [], published: 0, lastRequest: null, pendingMove: null, keptMoves: [], resumeAllowed: true, reviewLost: false,
  recoveryItems: [], recoveryMode: "success", recoveryReadFail: false, recoverySelectLost: false, recoveryUnsupported: false,
  changed: async patch => { f.snapshot = { ...f.snapshot, ...patch, revision: f.snapshot.revision + 1 }; if (!f.suppressEvents) await emit("data-operation-changed", structuredClone(f.snapshot)); },
};
const profile = { executablePath: "", modelPath: "", threads: 4, useGpu: false };
window.isTauri = true; mockWindows("main");
mockIPC(async (command, args) => {
  f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_data_operation") { if (f.failRead) throw Error("模擬状態読込失敗"); return structuredClone(f.snapshot); }
  if (command === "get_data_operation_availability") { const reason = f.snapshot.restartRequired ? "保存先切替後の再起動が必要です" : f.snapshot.busy ? "バックアップ／復元中" : f.blocker; return { available: reason === null, reason }; }
  if (command === "get_storage_bootstrap") {
    const pending = f.pendingMove ?? (f.snapshot.kind === "relocation" && f.snapshot.attentionRequired && !f.snapshot.restartRequired ? f.snapshot.relocationPreview : null);
    return { meetingRoot: f.snapshot.restartRequired ? f.snapshot.relocationReceipt?.destination ?? "/safe/root" : "/safe/root", activeMeetingRoot: "/safe/root", restartRequired: f.snapshot.restartRequired, modelsRoot: "/safe/app-data/models", bootstrapPath: "/safe/config/bootstrap.json", revision: f.snapshot.relocationReceipt ? 2 : 0, origin: f.snapshot.restartRequired ? "relocated" : "default", pendingOperationId: pending?.operationId ?? null, pendingTarget: pending?.destination ?? null, keptMoves: structuredClone(f.keptMoves) };
  }
  if (command === "review_storage_move") {
    if (!f.pendingMove || f.blocker || f.snapshot.busy) throw Error("未完了移動を確認できません");
    f.lastRequest = null;
    await f.changed({ ...idle, revision: f.snapshot.revision, token: crypto.randomUUID(), kind: "relocation", busy: true, phase: "verifying" });
    setTimeout(() => void f.changed({ phase: "ready", busy: false, retryAllowed: true, relocationPreview: { ...f.pendingMove, confirmationId: crypto.randomUUID(), pending: true, resumeAllowed: f.resumeAllowed, resumeReason: f.resumeAllowed ? null : "旧保存先が変更されています", fileCount: f.resumeAllowed ? 8 : null, copiedPaths: f.resumeAllowed ? 2 : null, expectedPaths: f.resumeAllowed ? 8 : null } }), 50);
    if (f.reviewLost) throw Error("模擬未完了確認の応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "list_restored_meeting_ids") return [...f.restored];
  if (command === "get_recovery_items") { if (f.recoveryReadFail) throw Error("模擬回収一覧失敗"); return { supported: !f.recoveryUnsupported, root: "/safe/root", items: structuredClone(f.recoveryItems) }; }
  if (command === "review_recovery_item") {
    if (f.blocker || f.snapshot.busy) throw Error("処理の停止を待ってください");
    const item = f.recoveryItems.find(i => i.source === args.source && i.operationId === args.operationId);
    if (!item || item.retained) throw Error("対象は確認済みまたは不明です");
    f.lastRequest = null;
    await f.changed({ ...idle, revision: f.snapshot.revision, token: crypto.randomUUID(), kind: "recovery", phase: "verifying", busy: true });
    setTimeout(() => void f.changed({ phase: "ready", busy: false, retryAllowed: true, recoveryPreview: { confirmationId: crypto.randomUUID(), item: structuredClone(item), root: "/safe/root", observedPaths: 4, inspectionComplete: true } }), 50);
    if (f.recoverySelectLost) throw Error("模擬回収選択の応答喪失");
    return structuredClone(f.snapshot);
  }
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
        restorePreview: args.kind === "restore" ? { confirmationId, batchId: crypto.randomUUID(), archiveId: f.archiveId, meetingTitles: ["点検会議", "計画会議"], requiredBytes: 32 * 1024 * 1024, availableBytes: f.previewMode === "capacity" ? 0 : 1024 * 1024 * 1024, vocabularyAdded: 1, vocabularyDeduplicated: 1, vocabularyConflicts: ["点検日"] } : null,
        relocationPreview: args.kind === "relocation" ? { confirmationId, operationId: crypto.randomUUID(), source: "/safe/root", destination: "/safe/chosen/MON-meetings-new", meetingCount: 3, fileCount: 8, requiredBytes: 64 * 1024 * 1024, availableBytes: f.previewMode === "capacity" ? 0 : 1024 * 1024 * 1024, pending: false, resumeAllowed: true, resumeReason: null, copiedPaths: null, expectedPaths: null } : null });
    };
    if (f.previewMode === "delay") f.complete = ready;
    else setTimeout(() => void ready(), 50);
    if (f.selectLost) throw Error("模擬ファイル選択の応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "start_data_operation") {
    const r = args.request;
    if (f.mode === "lostNoCommit") throw Error("模擬受付応答喪失・未受付");
    if (JSON.stringify(r) === JSON.stringify(f.lastRequest) && (f.snapshot.busy || ["completed", "attention"].includes(f.snapshot.phase) || (r.action === "refresh" && f.snapshot.phase === "ready"))) return structuredClone(f.snapshot);
    const p = f.snapshot.backupPreview || f.snapshot.restorePreview || f.snapshot.recoveryPreview || f.snapshot.relocationPreview;
    if (r.token !== f.snapshot.token || r.confirmationId !== p?.confirmationId || f.snapshot.busy) throw Error("確認が変更されています");
    f.lastRequest = structuredClone(r);
    if (f.snapshot.kind === "relocation") {
      if (r.action === "keep") {
        if (!p.pending) throw Error("未完了移動ではありません");
        f.pendingMove = null;
        f.keptMoves.push({ operationId: p.operationId, source: p.source, destination: p.destination, decisionPending: false });
        await f.changed({ phase: "completed", busy: false, retryAllowed: false, attentionRequired: false, restartRequired: false, relocationReceipt: { operationId: p.operationId, source: p.source, destination: p.destination, restartRequired: false, retained: true } });
        if (f.mode === "lost") throw Error("模擬保持結果の応答喪失");
        return structuredClone(f.snapshot);
      }
      if (r.action === "refresh") { await f.changed({ phase: "ready", busy: false, retryAllowed: true, errorMessage: null, relocationPreview: { ...p, confirmationId: crypto.randomUUID(), availableBytes: 1024 * 1024 * 1024 } }); return structuredClone(f.snapshot); }
      if (f.mode === "stale") { await f.changed({ phase: "failed", retryAllowed: true, errorMessage: "確認後に会議・編集・音声が変更されています。再確認してください" }); return structuredClone(f.snapshot); }
      if (f.mode === "forged") return { ...structuredClone(f.snapshot), revision: f.snapshot.revision + 1, phase: "completed", retryAllowed: false, restartRequired: true, relocationReceipt: { operationId: crypto.randomUUID(), source: p.source, destination: p.destination, restartRequired: true, retained: false } };
      await f.changed({ phase: "copying", busy: true, retryAllowed: false });
      const finish = async () => {
        if ((f.snapshot.phase === "cancelling" && f.mode !== "cancelRace") || f.mode === "attention") { f.pendingMove = structuredClone(p); return f.changed({ phase: "attention", busy: false, attentionRequired: true, errorMessage: "旧保存先と途中コピーを保持します" }); }
        f.published++;
        f.pendingMove = null;
        await f.changed({ phase: "completed", busy: false, restartRequired: true, relocationReceipt: { operationId: p.operationId, source: p.source, destination: p.destination, restartRequired: true, retained: false } });
      };
      if (["delay", "cancelRace", "lostDelay"].includes(f.mode)) f.complete = finish; else setTimeout(() => void finish(), 50);
      if (["lost", "lostDelay"].includes(f.mode)) throw Error("模擬保存先切替の応答喪失");
      return structuredClone(f.snapshot);
    }
    if (f.snapshot.kind === "recovery") {
      if (r.action === "refresh") { await f.changed({ phase: "ready", retryAllowed: true, errorMessage: null, recoveryPreview: { ...p, confirmationId: crypto.randomUUID() } }); if (f.recoveryMode === "stale") f.recoveryMode = "success"; return structuredClone(f.snapshot); }
      if (f.recoveryMode === "lostNoCommit") throw Error("模擬回収受付喪失・未受付");
      if (f.recoveryMode === "stale") { await f.changed({ phase: "failed", retryAllowed: true, errorMessage: "確認後に対象が変更されています。最新の内容を再確認してください" }); return structuredClone(f.snapshot); }
      if (f.recoveryMode === "forged") return { ...structuredClone(f.snapshot), phase: "completed", retryAllowed: false, revision: f.snapshot.revision + 1, recoveryReceipt: { source: p.item.source, operationId: crypto.randomUUID(), outcome: "retained", reason: null } };
      await f.changed({ phase: "publishing", busy: true, retryAllowed: false });
      const finish = async () => {
        if (f.snapshot.phase === "cancelling" && f.recoveryMode !== "cancelRace") return f.changed({ phase: "cancelled", busy: false, retryAllowed: true });
        const outcome = r.action === "keep" ? "retained" : f.recoveryMode === "attention" ? "attention" : "recovered";
        const index = f.recoveryItems.findIndex(i => i.source === p.item.source && i.operationId === p.item.operationId);
        if (outcome === "retained") f.recoveryItems[index].retained = true;
        if (outcome === "recovered") f.recoveryItems.splice(index, 1);
        await f.changed({ phase: "completed", busy: false, recoveryReceipt: { source: p.item.source, operationId: p.item.operationId, outcome, reason: outcome === "attention" ? "所有不明の対象を保持します" : null } });
      };
      if (["delay", "cancelRace"].includes(f.recoveryMode)) f.complete = finish; else setTimeout(() => void finish(), 50);
      if (f.recoveryMode === "lost") throw Error("模擬保持確認の応答喪失");
      return structuredClone(f.snapshot);
    }
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
    else if (!f.snapshot.backupReceipt && !f.snapshot.restoreReceipt && !f.snapshot.recoveryReceipt && !f.snapshot.relocationReceipt && !f.snapshot.attentionRequired) await f.changed({ phase: "cancelled", retryAllowed: false, backupPreview: null, restorePreview: null, recoveryPreview: null, relocationPreview: null });
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
