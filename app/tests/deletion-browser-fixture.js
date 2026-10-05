// Development-only simulated IPC. No real audio, meeting files or settings.
import { mockIPC } from "@tauri-apps/api/mocks";
window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const f = window.dataFixture;
f.meetings.push({ ...f.meetings[0], id: crypto.randomUUID(), title: "残す会議" });
f.snapshot = { ...f.snapshot, deletionPreview: null, deletionReceipt: null };
const changed = f.changed;
f.changed = patch => changed(patch.kind && patch.kind !== "deletion" ? { ...patch, deletionPreview: null, deletionReceipt: null } : patch);
const idle = structuredClone(f.snapshot);
const d = window.deletionFixture = {
  mode: "success", unknownCount: 0, fileCount: 1, blocked: false, restart: false, cold: false,
  listFail: false, pageFail: false, reviewLost: false, pathsDelay: false, releasePage: null,
  items: [], deleted: 0, kept: 0, reconfirmed: 0, complete: () => {}, last: null, closes: 0,
  prepareHold: false, closeHold: false, closeFail: false, quitFail: false, liveTokens: new Set(),
  mountPlayback: async () => { const { mountPlaybackHarness } = await import("./deletion-playback-browser-harness.tsx"); d.unmountPlayback = mountPlaybackHarness(f.meetings[0].id); },
  pending: async (phase = "not_started", changedContents = false) => {
    d.blocked = true; d.cold = true;
    const p = makePreview(f.meetings[0].id, phase);
    if (changedContents) Object.assign(p, { executeAllowed: false, resumeReason: "新しい未知物を保持しています", reconfirmationAllowed: ["files", "database", "awaiting_ack", "database_files"].includes(phase) });
    d.pendingPreview = p;
    d.items = [{ operationId: p.operationId, state: phase === "awaiting_ack" || phase === "database_files" ? "completed" : phase === "database" ? "cleanup" : "prepared", meetingId: p.rowPhaseCommitted ? null : p.meetingId, errorCode: null }];
  },
};
const makePreview = (meetingId, phase = "review") => ({ confirmationId: crypto.randomUUID(), operationId: crypto.randomUUID(), meetingId,
  title: f.meetings.find(m => m.id === meetingId)?.title ?? `会議 ${meetingId}`, phase,
  pending: phase !== "review", executeAllowed: true, resumeReason: null, keepAllowed: phase === "not_started",
  pathCount: d.fileCount + 2, processedPaths: ["database", "awaiting_ack"].includes(phase) ? d.fileCount + 2 : 0, fileCount: d.fileCount,
  missingCount: 0, changedCount: 0, unregisteredCount: d.unknownCount, bytes: 1024 * 1024, rowCount: 20, rowPhaseCommitted: ["awaiting_ack", "database_files"].includes(phase),
  reconfirmationAllowed: false, previousProcessedPaths: null, previousPathCount: null });
mockIPC(async (command, args) => {
  if (["get_deletion_items", "review_meeting_deletion", "review_pending_deletion", "get_deletion_paths", "close_playback", "prepare_utterance_playback"].includes(command)
    || (["start_data_operation", "cancel_data_operation"].includes(command) && f.snapshot.kind === "deletion")) f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_deletion_items") {
    if (d.listFail) throw Error("模擬削除一覧失敗");
    return { items: structuredClone(d.items), deletingMeetingIds: d.items.map(i => i.meetingId).filter(Boolean), writeBlocked: d.blocked, restartRequired: d.restart, reason: d.blocked ? d.restart ? "削除解決後の再起動が必要" : "未完了削除のため書込停止" : null };
  }
  if (command === "get_data_operation_availability" && d.restart) return { available: false, reason: "保存先の切替後は終了して再起動が必要です" };
  if (command === "review_meeting_deletion" || command === "review_pending_deletion") {
    if (f.blocker || f.snapshot.busy || (command === "review_meeting_deletion" && d.blocked)) throw Error("処理の停止を待ってください");
    d.last = null;
    const p = command === "review_pending_deletion" ? { ...d.pendingPreview, confirmationId: crypto.randomUUID() } : makePreview(args.meetingId);
    if (!p || (command === "review_pending_deletion" && p.operationId !== args.operationId)) throw Error("削除対象が不明です");
    await f.changed({ ...idle, token: crypto.randomUUID(), kind: "deletion", phase: "verifying_delete_scope", busy: true, restartRequired: d.restart });
    setTimeout(() => void f.changed({ phase: "ready", busy: false, retryAllowed: true, deletionPreview: p }), 30);
    if (d.reviewLost) throw Error("模擬削除確認の応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "get_deletion_paths") {
    const p = structuredClone(f.snapshot.deletionPreview);
    if (d.pageFail) throw Error("模擬一覧取得失敗");
    if (!p || args.confirmationId !== p.confirmationId || args.offset % 50) throw Error("一覧の確認が失効");
    const page = { confirmationId: args.confirmationId, offset: args.offset, total: p.pathCount, nextOffset: args.offset + 50 < p.pathCount ? args.offset + 50 : null,
      entries: Array.from({ length: Math.min(50, p.pathCount - args.offset) }, (_, index) => {
        const n = args.offset + index, directory = n < 2;
        return { relativePath: `meetings/${p.meetingId}${n === 0 ? "" : n === 1 ? "/audio" : `/audio/file-${n}.flac`}`, directory, registered: !directory && n >= 2 + d.unknownCount, missing: false, changed: false, stageHint: null, bytes: directory ? null : 1024 };
      }) };
    if (d.pathsDelay) { d.pathsDelay = false; await new Promise(resolve => { d.releasePage = resolve; }); }
    return page;
  }
  if (command === "start_data_operation" && f.snapshot.kind === "deletion") {
    const r = args.request, p = f.snapshot.deletionPreview;
    if (d.mode === "lostNoCommit") throw Error("模擬受付応答喪失・未受付");
    if (JSON.stringify(r) === JSON.stringify(d.last) && (f.snapshot.busy || ["completed", "attention"].includes(f.snapshot.phase) || (["refresh", "review_remaining"].includes(r.action) && f.snapshot.phase === "ready"))) return structuredClone(f.snapshot);
    if (!p || r.token !== f.snapshot.token || r.confirmationId !== p.confirmationId || f.snapshot.busy) throw Error("確認が変更されています");
    if (["publish", "delete_including_unregistered"].includes(r.action) && (!p.executeAllowed || f.snapshot.phase !== "ready")) throw Error("再検査が必要");
    if (r.action === "publish" && p.unregisteredCount && ["review", "not_started", "reconfirmation", "database_reconfirmation"].includes(p.phase)) throw Error("未登録物の別判断が必要");
    d.last = structuredClone(r);
    if (r.action === "review_remaining") {
      if (!p.reconfirmationAllowed) throw Error("残り範囲を再同意できません");
      d.reconfirmed++; d.mode = "success";
      await f.changed({ phase: "ready", busy: false, attentionRequired: false, errorMessage: null, retryAllowed: true, deletionPreview: { ...p, confirmationId: crypto.randomUUID(), phase: p.phase === "files" ? "reconfirmation" : "database_reconfirmation", previousProcessedPaths: p.processedPaths, previousPathCount: p.pathCount, pathCount: d.fileCount + 2, fileCount: d.fileCount, unregisteredCount: d.unknownCount, processedPaths: 0, reconfirmationAllowed: false, executeAllowed: true, resumeReason: null } });
      return structuredClone(f.snapshot);
    }
    if (r.action === "refresh") {
      if (d.mode === "stale") d.mode = "success";
      await f.changed({ phase: "ready", busy: false, attentionRequired: false, errorMessage: null, retryAllowed: true, deletionPreview: { ...p, confirmationId: crypto.randomUUID(), executeAllowed: d.mode !== "unknown", reconfirmationAllowed: ["files", "database", "awaiting_ack", "database_files"].includes(p.phase) && d.mode === "unknown", resumeReason: d.mode === "unknown" ? "新しい未知物を保持しています" : null } });
      return structuredClone(f.snapshot);
    }
    if (d.mode === "forged") return { ...structuredClone(f.snapshot), revision: f.snapshot.revision + 1, phase: "completed", retryAllowed: false, deletionReceipt: { operationId: crypto.randomUUID(), meetingId: p.meetingId, deleted: true, retained: false, restartRequired: false } };
    if (r.action === "keep") {
      if (!p.keepAllowed) throw Error("開始済み削除は保持で解決できません");
      d.kept++; d.items = []; d.restart = d.cold; d.blocked = d.restart;
      await f.changed({ phase: "completed", retryAllowed: false, attentionRequired: false, errorMessage: null, restartRequired: d.restart, deletionPreview: { ...p, phase: "retained", pending: false, executeAllowed: false }, deletionReceipt: { operationId: p.operationId, meetingId: p.meetingId, deleted: false, retained: true, restartRequired: d.restart } });
      return structuredClone(f.snapshot);
    }
    if (d.mode === "stale") { await f.changed({ phase: "failed", errorMessage: "確認後に会議が変更されています", retryAllowed: true }); return structuredClone(f.snapshot); }
    d.blocked = true;
    d.items = [{ operationId: p.operationId, state: "installing", meetingId: p.meetingId, errorCode: null }];
    await f.changed({ phase: "deleting_files", busy: true, retryAllowed: false });
    d.complete = async () => {
      if (f.snapshot.phase === "cancelling" && d.mode !== "cancelRace" || d.mode === "partial") {
        const saved = { ...p, phase: p.phase === "database_reconfirmation" || p.phase === "database_files" ? "database_files" : "files", pending: true, keepAllowed: false, processedPaths: 1, executeAllowed: false, reconfirmationAllowed: false, previousProcessedPaths: null, previousPathCount: null, resumeReason: "停止しました。残りを再検査してください" };
        d.pendingPreview = saved;
        await f.changed({ phase: "attention", busy: false, attentionRequired: true, deletionPreview: saved, errorMessage: "部分削除を保持します。復元していません" });
        return;
      }
      d.deleted++; d.items = []; d.restart = d.cold; d.blocked = d.restart;
      f.meetings = f.meetings.filter(m => m.id !== p.meetingId);
      await f.changed({ phase: "completed", busy: false, restartRequired: d.restart, attentionRequired: false, errorMessage: null, deletionPreview: { ...p, phase: "completed", pending: false, executeAllowed: false, keepAllowed: false, processedPaths: p.pathCount, rowPhaseCommitted: true, reconfirmationAllowed: false, previousProcessedPaths: null, previousPathCount: null }, deletionReceipt: { operationId: p.operationId, meetingId: p.meetingId, deleted: true, retained: false, restartRequired: d.restart } });
    };
    if (!["delay", "cancelRace"].includes(d.mode)) setTimeout(() => void d.complete(), 40);
    if (d.mode === "lost") throw Error("模擬削除応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "cancel_data_operation" && f.snapshot.kind === "deletion") {
    if (args.token !== f.snapshot.token) throw Error("停止対象が変更されています");
    if (f.snapshot.busy) await f.changed({ phase: "cancelling" });
    else if (!f.snapshot.deletionReceipt && !f.snapshot.deletionPreview?.pending) await f.changed({ phase: "cancelled", retryAllowed: false, deletionPreview: null });
    return structuredClone(f.snapshot);
  }
  if (command === "prepare_utterance_playback") {
    const token = crypto.randomUUID(); d.liveTokens.add(token);
    if (d.prepareHold) await new Promise(resolve => { d.releasePreparation = resolve; });
    return { token, sampleRate: 48000, channels: 1, totalFrames: 48000, startUs: 0, endUs: 1000000, trackKind: "mic", alignment: "unverified" };
  }
  if (command === "close_playback") {
    d.closes++;
    if (d.closeFail) throw Error("模擬ネイティブ再生解放失敗");
    if (d.closeHold) await new Promise(resolve => { d.releaseClose = resolve; });
    d.liveTokens.delete(args.token); return null;
  }
  if (command === "finish_main_close" && d.quitFail) throw Error("模擬終了失敗");
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
