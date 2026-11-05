// Synthetic metadata and IPC only; never opens user files or executes a model.
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "./startup-ready-mocks";
import { useDataOperation } from "../src/data-operation";
import { ModelDeletionDialog, useModelDeletion } from "../src/model-deletion";
import { ModelInspectionPanel } from "../src/model-inspection";
import { SettingsModalScope } from "../src/settings-modal-scope";
import { CommonOperationProgress, usePublishProgress } from "../src/operation-progress";
import { deletionProgress } from "../src/operation-progress-state";
import { ToastHost, NotificationList } from "../src/toast";
(window as any).skipModelDownloadFixtureMain = true;
const { modelDownloadIPC } = await import("./model-download-browser-fixture");
const f = (window as any).modelDownloadFixture;
f.entries[0].modelFiles = [{ role: "asr", size: 17, sha256: "b".repeat(64) }];
const copy = structuredClone(f.entries[0]); copy.packageId = "55555555-5555-4555-8555-555555555555";
f.entries.push(copy);
f.deletionMode = "normal";
const idle = () => ({ revision: 0, token: null, kind: null, phase: "idle", busy: false, backupPreview: null, restorePreview: null, backupReceipt: null, restoreReceipt: null, relocationPreview: null, relocationReceipt: null, deletionPreview: null, deletionReceipt: null, audioDeletionPreview: null, audioDeletionReceipt: null, modelDeletionPreview: null, modelDeletionReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: false });
let state: any = idle(), pending = false;
const empty = { items: [], writeBlocked: false, restartRequired: false, reason: null };
function preview(entry: any, operationId = crypto.randomUUID()) {
  return { scopeMode: "directory", targetDirectory: `/synthetic/models/${entry.packageId}`, confirmationId: crypto.randomUUID(), operationId, packageId: entry.packageId, reviewHash: entry.reviewHash, modelName: entry.modelName, modelVersion: entry.modelVersion, modelsRoot: "/synthetic/models", phase: "review", pending: false, executeAllowed: true, resumeReason: null, keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: null, pathCount: 1, processedPaths: 0, fileCount: 1, missingCount: 0, changedCount: 0, unregisteredCount: 0, bytes: entry.totalBytes, profileReferences: 1, jobReferences: 0, activeJobs: 0, previousProcessedPaths: null, previousPathCount: null, priorDeletionCompleted: false, reservationRoots: [] };
}
const snap = () => structuredClone(state);
f.managementSnapshot = snap;
mockIPC(async (command, args: any) => {
  if (command === "get_data_operation") return snap();
  if (command === "get_data_operation_availability") return { available: !state.busy, reason: state.busy ? "モデル削除中" : null, reasonCode: state.busy ? "operation_active" : null, deletionAvailable: !state.busy, deletionReason: state.busy ? "モデル削除中" : null, deletionReasonCode: state.busy ? "operation_active" : null };
  if (command === "get_deletion_items") return { ...empty, deletingMeetingIds: [] };
  if (command === "get_audio_deletion_items") return empty;
  if (command === "get_model_deletion_items") return { ...empty, writeBlocked: pending, reason: pending ? "モデル削除が未完了です" : null, items: pending ? [{ operationId: state.modelDeletionPreview.operationId, state: "installing", packageId: state.modelDeletionPreview.packageId, modelName: state.modelDeletionPreview.modelName, reviewAllowed: true, errorMessage: null }] : [] };
  if (command === "review_model_body_deletion" || command === "review_pending_model_deletion") {
    f.calls.push({ command, args });
    if (command === "review_pending_model_deletion") {
      if (args.operationId !== state.modelDeletionPreview?.operationId) throw Error("wrong pending operation");
      state = { ...state, revision: state.revision + 1, token: crypto.randomUUID(), phase: "ready", busy: false, attentionRequired: false, retryAllowed: true, modelDeletionPreview: { ...state.modelDeletionPreview, confirmationId: crypto.randomUUID() } };
    } else {
      const entry = f.entries.find((e: any) => e.packageId === args.packageId && e.reviewHash === args.expectedReviewHash && !e.bodyRemoved);
      if (!entry) throw Error("wrong copy");
      state = { ...idle(), revision: state.revision + 1, token: crypto.randomUUID(), kind: "model_deletion", phase: "ready", retryAllowed: true, modelDeletionPreview: preview(entry) };
    }
    if (f.deletionMode === "delayReview") {
      const prepared = snap(), token = state.token;
      state = { ...state, revision: state.revision + 1, busy: true, phase: "verifying_model_delete_scope", retryAllowed: false, modelDeletionPreview: null };
      f.finishReview = () => {
        if (state.token !== token || !state.busy) return;
        state = state.phase === "cancelling" ? { ...state, revision: state.revision + 1, busy: false, phase: "cancelled", errorMessage: "モデル削除の対象検査を取り消しました" }
          : { ...prepared, revision: state.revision + 1 };
      };
      return snap();
    }
    if (f.deletionMode === "lostReview") { f.deletionMode = "normal"; throw Error("模擬の確認応答喪失"); }
    return snap();
  }
  if (command === "cancel_data_operation" && f.deletionMode === "delayReview" && state.busy) { state = { ...state, revision: state.revision + 1, phase: "cancelling" }; return snap(); }
  if (command === "cancel_data_operation" && f.deletionMode === "finishOnStop" && state.busy) return f.finishDeletion();
  if (command === "cancel_data_operation") { state = { ...idle(), revision: state.revision + 1, phase: "cancelled" }; return snap(); }
  if (command === "start_data_operation") {
    f.calls.push({ command, args });
    const p = state.modelDeletionPreview, r = args.request;
    if (r.token !== state.token || r.confirmationId !== p.confirmationId) throw Error("stale confirmation");
    if (r.action === "refresh" || r.action === "review_remaining") {
      const reconfirm = r.action === "review_remaining";
      state = { ...state, revision: state.revision + 1, phase: "ready", retryAllowed: true, attentionRequired: false, errorMessage: null, modelDeletionPreview: { ...p, confirmationId: crypto.randomUUID(), phase: reconfirm ? "reconfirmation" : p.phase, executeAllowed: true, resumeReason: null, reconfirmationAllowed: false, ...(reconfirm ? { previousProcessedPaths: 0, previousPathCount: 1, processedPaths: 0, includeUnregisteredRecorded: null } : {}) } };
      return snap();
    }
    if (f.deletionMode === "changed") {
      f.deletionMode = "normal"; pending = true;
      state = { ...state, revision: state.revision + 1, phase: "attention", attentionRequired: true, retryAllowed: false, errorMessage: "模擬の削除対象変更", modelDeletionPreview: { ...p, phase: "files", pending: true, executeAllowed: false, resumeReason: "対象が変わりました", reconfirmationAllowed: true, includeUnregisteredRecorded: false, reservationRoots: [`.mon-delete-model-${p.operationId}-0`, p.packageId].sort() } };
      return snap();
    }
    const finish = () => {
      f.entries.find((e: any) => e.packageId === p.packageId).bodyRemoved = true; pending = false;
      state = { ...state, revision: state.revision + 1, busy: false, phase: "completed", retryAllowed: false, attentionRequired: false, errorMessage: null, modelDeletionPreview: { ...p, phase: "completed", pending: false, executeAllowed: false, processedPaths: 1, includeUnregisteredRecorded: false, previousProcessedPaths: null, previousPathCount: null, reconfirmationAllowed: false, resumeReason: null, priorDeletionCompleted: true, reservationRoots: [`.mon-delete-model-${p.operationId}-0`, p.packageId].sort() }, modelDeletionReceipt: { operationId: p.operationId, packageId: p.packageId, bodyDeleted: true, retained: false, historyRetained: true, restartRequired: false } };
      return snap();
    };
    if (["delay", "finishOnStop"].includes(f.deletionMode)) { state = { ...state, revision: state.revision + 1, phase: "deleting_model_files", busy: true, retryAllowed: false }; f.finishDeletion = finish; return snap(); }
    const result = finish();
    if (f.deletionMode === "lost") { f.deletionMode = "normal"; throw Error("模擬の削除完了応答喪失"); }
    return result;
  }
  return modelDownloadIPC(command, args);
}, { shouldMockEvents: true });
function Fixture() {
  const operation = useDataOperation(), [refreshEpoch, setRefreshEpoch] = useState(0), [locked, setLocked] = useState(false);
  f.setManagementLocked = setLocked;
  const deletion = useModelDeletion({ operation, locked, withSavedEdits: async action => { await action(); }, onDeleted: async () => { setRefreshEpoch(n => n + 1); } });
  usePublishProgress("model-deletion", [deletionProgress(operation.snapshot)]);
  return <SettingsModalScope><main><CommonOperationProgress /><ToastHost /><h1>モデル管理の合成検証</h1><ModelInspectionPanel visible locked={locked || !!operation.snapshot?.busy || deletion.protectedNow} recordingActive={false} deletion={deletion} refreshEpoch={refreshEpoch} /><ModelDeletionDialog operation={operation} control={deletion} /><NotificationList visible={false} /></main></SettingsModalScope>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
