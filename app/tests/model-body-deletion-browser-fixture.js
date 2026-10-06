// Development-only IPC simulation. Never invokes native model/file operations.
import { mockIPC } from "@tauri-apps/api/mocks";
window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const f = window.dataFixture;
f.snapshot = { ...f.snapshot, deletionPreview: null, deletionReceipt: null, modelDeletionPreview: null, modelDeletionReceipt: null };
const idle = structuredClone(f.snapshot), changed = f.changed;
f.changed = patch => changed(patch.kind && patch.kind !== "model_deletion" ? { ...patch, modelDeletionPreview: null, modelDeletionReceipt: null } : patch);
const entry = { packageId: crypto.randomUUID(), modelName: "合成モデル", modelVersion: "1", engine: "whisper.cpp", engineVersion: "test", totalBytes: 40, reviewHash: "a".repeat(64), operationId: crypto.randomUUID(), state: "completed", placementRecorded: true, ready: false, compatibility: null, errorMessage: null };
const d = window.modelBodyFixture = {
  mode: "success", fileCount: 2, unknownCount: 0, blocked: false, cold: false, restart: false, listFail: false, pageFail: false,
  foreignPage: false, pathsDelay: false, releasePage: null, reviewLost: false, items: [], deleted: 0, kept: 0, catalogReads: 0, last: null, complete: () => {},
  pending: (phase = "not_started", altered = false) => {
    d.blocked = true; d.cold = true;
    const p = makePreview(phase);
    if (altered) Object.assign(p, { executeAllowed: false, resumeReason: "新しい未知物を保持しています", reconfirmationAllowed: ["files", "awaiting_ack"].includes(phase) });
    d.pendingPreview = p; d.items = [{ operationId: p.operationId, state: "installing", packageId: p.packageId, modelName: p.modelName, reviewAllowed: true, errorMessage: null }];
  },
};
const makePreview = (phase = "review") => ({ confirmationId: crypto.randomUUID(), operationId: crypto.randomUUID(), packageId: entry.packageId, reviewHash: entry.reviewHash,
  modelName: entry.modelName, modelVersion: entry.modelVersion, modelsRoot: "/synthetic/app/models", phase, pending: phase !== "review", executeAllowed: true, resumeReason: null,
  keepAllowed: phase === "not_started", reconfirmationAllowed: false, includeUnregisteredRecorded: phase === "review" ? null : d.unknownCount > 0,
  pathCount: d.fileCount + 1, processedPaths: phase === "files" ? 1 : phase === "awaiting_ack" ? d.fileCount + 1 : 0,
  fileCount: d.fileCount, missingCount: 0, changedCount: 0, unregisteredCount: d.unknownCount, bytes: 20 * d.fileCount,
  profileReferences: 1, jobReferences: 3, activeJobs: 0, previousProcessedPaths: null, previousPathCount: null, priorDeletionCompleted: false, reservationRoots: [] });
mockIPC(async (command, args) => {
  if (["get_model_deletion_items", "review_model_body_deletion", "review_pending_model_deletion", "get_model_deletion_paths", "get_model_placements"].includes(command)
    || (["start_data_operation", "cancel_data_operation"].includes(command) && f.snapshot.kind === "model_deletion")) f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_deletion_items") return { items: [], deletingMeetingIds: [], writeBlocked: d.blocked, restartRequired: d.restart, reason: d.blocked ? "モデル削除のため書込停止" : null };
  if (command === "get_model_deletion_items") {
    if (d.listFail) throw Error("模擬モデル削除一覧失敗");
    return { items: structuredClone(d.items), writeBlocked: d.blocked, restartRequired: d.restart, reason: d.blocked ? "モデル削除のため書込停止" : null };
  }
  if (command === "get_model_placements") { d.catalogReads++; return [{ ...entry, bodyRemoved: d.deleted > 0 }]; }
  if (command === "get_data_operation_availability" && d.restart) return { available: false, reason: "保存先の切替後は終了して再起動が必要です" };
  if (command === "review_model_body_deletion" || command === "review_pending_model_deletion") {
    if (f.blocker || f.snapshot.busy || (command === "review_model_body_deletion" && (d.blocked || args.packageId !== entry.packageId || args.expectedReviewHash !== entry.reviewHash))) throw Error("モデルの停止・再検査が必要です");
    const p = command === "review_model_body_deletion" ? makePreview() : { ...d.pendingPreview, confirmationId: crypto.randomUUID() };
    if (!p || (command === "review_pending_model_deletion" && p.operationId !== args.operationId)) throw Error("不明な操作");
    d.last = null;
    await f.changed({ ...idle, revision: f.snapshot.revision, token: crypto.randomUUID(), kind: "model_deletion", phase: "verifying_model_delete_scope", busy: true, restartRequired: d.restart });
    setTimeout(() => void f.changed({ phase: "ready", busy: false, retryAllowed: true, modelDeletionPreview: p }), 30);
    if (d.reviewLost) throw Error("模擬モデル確認応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "get_model_deletion_paths") {
    const p = structuredClone(f.snapshot.modelDeletionPreview);
    if (d.pageFail) throw Error("模擬モデル対象一覧失敗");
    if (!p || p.confirmationId !== args.confirmationId || args.offset % 50) throw Error("確認が失効");
    const page = { confirmationId: p.confirmationId, offset: args.offset, total: p.pathCount, nextOffset: args.offset + 50 < p.pathCount ? args.offset + 50 : null,
      entries: Array.from({ length: Math.min(50, p.pathCount - args.offset) }, (_, i) => {
        const n = args.offset + i, root = d.foreignPage ? crypto.randomUUID() : p.packageId;
        return { relativePath: root + (n === 0 ? "" : `/file-${String(n).padStart(3, "0")}.bin`), directory: n === 0, registered: n === 0 || n > p.unregisteredCount, present: true, changed: false, bytes: n === 0 ? null : 20, sha256: n === 0 ? null : "b".repeat(64) };
      }) };
    if (d.pathsDelay) { d.pathsDelay = false; await new Promise(resolve => { d.releasePage = resolve; }); }
    return page;
  }
  if (command === "start_data_operation" && f.snapshot.kind === "model_deletion") {
    const r = args.request, p = f.snapshot.modelDeletionPreview;
    if (d.mode === "lostNoCommit") throw Error("模擬未受付応答喪失");
    if (JSON.stringify(r) === JSON.stringify(d.last) && (f.snapshot.busy || ["completed", "attention"].includes(f.snapshot.phase) || (["refresh", "review_remaining"].includes(r.action) && f.snapshot.phase === "ready"))) return structuredClone(f.snapshot);
    if (!p || r.token !== f.snapshot.token || r.confirmationId !== p.confirmationId || f.snapshot.busy) throw Error("確認が変更されています");
    d.last = structuredClone(r);
    if (r.action === "refresh") {
      await f.changed({ phase: "ready", retryAllowed: true, busy: false, attentionRequired: false, errorMessage: null, modelDeletionPreview: { ...p, confirmationId: crypto.randomUUID(), executeAllowed: d.mode !== "unknown", resumeReason: d.mode === "unknown" ? "未知物を保持" : null, reconfirmationAllowed: d.mode === "unknown" && ["files", "awaiting_ack"].includes(p.phase) } });
      return structuredClone(f.snapshot);
    }
    if (r.action === "review_remaining") {
      if (!p.reconfirmationAllowed) throw Error("再同意できません");
      await f.changed({ phase: "ready", retryAllowed: true, busy: false, attentionRequired: false, errorMessage: null, modelDeletionPreview: { ...p, confirmationId: crypto.randomUUID(), phase: "reconfirmation", executeAllowed: true, resumeReason: null, reconfirmationAllowed: false, includeUnregisteredRecorded: null, previousProcessedPaths: p.processedPaths, previousPathCount: p.pathCount, processedPaths: 0, unregisteredCount: d.unknownCount } });
      return structuredClone(f.snapshot);
    }
    if (r.action === "keep") {
      if (!p.keepAllowed) throw Error("開始済み削除は保持不可");
      d.kept++; d.items = []; d.restart = d.cold; d.blocked = d.restart;
      await f.changed({ phase: "completed", retryAllowed: false, attentionRequired: false, errorMessage: null, restartRequired: d.restart, modelDeletionPreview: { ...p, phase: "retained", pending: false, executeAllowed: false, includeUnregisteredRecorded: null }, modelDeletionReceipt: { operationId: p.operationId, packageId: p.packageId, bodyDeleted: false, retained: true, historyRetained: true, restartRequired: d.restart } });
      return structuredClone(f.snapshot);
    }
    if (!p.executeAllowed || f.snapshot.phase !== "ready" || (r.action === "publish" && ["review", "reconfirmation"].includes(p.phase) && p.unregisteredCount) || (r.action === "delete_including_unregistered" && !["review", "reconfirmation"].includes(p.phase))) throw Error("モデル削除の同意が必要");
    if (d.mode === "forged") return { ...structuredClone(f.snapshot), revision: f.snapshot.revision + 1, phase: "completed", retryAllowed: false, modelDeletionReceipt: { operationId: crypto.randomUUID(), packageId: p.packageId, bodyDeleted: true, retained: false, historyRetained: true, restartRequired: false } };
    d.blocked = true;
    const policy = p.includeUnregisteredRecorded ?? r.action === "delete_including_unregistered";
    d.items = [{ operationId: p.operationId, state: "installing", packageId: p.packageId, modelName: p.modelName, reviewAllowed: true, errorMessage: null }];
    await f.changed({ phase: "deleting_model_files", busy: true, retryAllowed: false });
    d.complete = async () => {
      if (f.snapshot.phase === "cancelling" && d.mode !== "cancelRace" || d.mode === "partial") {
        d.pendingPreview = { ...p, phase: "files", pending: true, executeAllowed: false, resumeReason: "停止後に残りを再検査してください", keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: policy, processedPaths: 1, previousProcessedPaths: null, previousPathCount: null };
        await f.changed({ phase: "attention", busy: false, attentionRequired: true, modelDeletionPreview: d.pendingPreview, errorMessage: "部分削除を保持しています。復元していません" }); return;
      }
      d.deleted++; d.items = []; d.restart = d.cold; d.blocked = d.restart;
      await f.changed({ phase: "completed", busy: false, retryAllowed: false, attentionRequired: false, errorMessage: null, restartRequired: d.restart,
        modelDeletionPreview: { ...p, phase: "completed", pending: false, executeAllowed: false, keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: policy, processedPaths: p.pathCount, previousProcessedPaths: null, previousPathCount: null, priorDeletionCompleted: true },
        modelDeletionReceipt: { operationId: p.operationId, packageId: p.packageId, bodyDeleted: true, retained: false, historyRetained: true, restartRequired: d.restart } });
    };
    if (!["delay", "cancelRace"].includes(d.mode)) setTimeout(() => void d.complete(), 40);
    if (d.mode === "lost") throw Error("模擬モデル削除応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "cancel_data_operation" && f.snapshot.kind === "model_deletion") {
    if (args.token !== f.snapshot.token) throw Error("別の操作");
    if (f.snapshot.busy) await f.changed({ phase: "cancelling" });
    else if (!f.snapshot.modelDeletionReceipt && !f.snapshot.modelDeletionPreview?.pending) await f.changed({ phase: "cancelled", retryAllowed: false, modelDeletionPreview: null });
    return structuredClone(f.snapshot);
  }
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
