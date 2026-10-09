// Development-only synthetic IPC. No native files, device capture or real data.
import { mockIPC } from "@tauri-apps/api/mocks";
window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const f = window.dataFixture;
f.snapshot = { ...f.snapshot, partialRecordingPreview: null, partialRecordingReceipt: null };
const idle = structuredClone(f.snapshot), changed = f.changed;
f.changed = patch => changed(patch.kind && patch.kind !== "partial_recording" ? { ...patch, partialRecordingPreview: null, partialRecordingReceipt: null } : patch);
const session = "00000002-1111-4111-8111-111111111111";
f.meetings[0].id = "00000001-1111-4111-8111-111111111111";
f.meetings[0].title = "部分救出の合成会議";
f.meetings[0].sourceType = "recording"; f.meetings[0].lifecycle = "recording";
const d = window.partialRecordingFixture = { mode: "success", empty: false, oneLane: false, unreadable: false,
  listFail: false, reviewLost: false, reviewFail: false, cancelFail: false, cancelLost: false, suppressEvents: false,
  count: 1, recovered: 0, kept: 0, last: null, complete: () => {}, committed: null };
const available = lane => ({ lane, state: "available", sealedSegments: 2, frames: 34000, startUs: lane === "microphone" ? 0 : 50_000,
  endUs: lane === "microphone" ? 4_950_000 : 5_000_000, sampleRate: 8000, channels: 2 });
const absent = (lane, state) => ({ lane, state, sealedSegments: 0, frames: 0, startUs: null, endUs: null, sampleRate: null, channels: null });
const summary = () => ({ sessionId: session, meetingId: f.meetings[0].id, reviewHash: "a".repeat(64), durationUs: d.empty ? 0 : d.oneLane ? 4_950_000 : 5_000_000,
  lanes: [d.empty ? absent("microphone", "empty") : available("microphone"), d.empty ? absent("system", "missing") : d.oneLane ? absent("system", d.unreadable ? "unreadable" : "missing") : available("system")],
  acquisitionEndKnown: false, tailLossUs: null });
const listingItems = () => Array.from({ length: d.recovered ? 0 : d.count }, (_, n) => ({ sessionId: n === 0 ? session : `${(n + 2).toString(16).padStart(8, "0")}-1111-4111-8111-111111111111`,
  meetingId: n === 0 ? f.meetings[0].id : `${(n + 1000).toString(16).padStart(8, "0")}-1111-4111-8111-111111111111`, title: n === 0 ? f.meetings[0].title : `中断した合成会議 ${n + 1}`, state: "interrupted" })).sort((a, b) => a.sessionId.localeCompare(b.sessionId));
mockIPC(async (command, args) => {
  if (["get_partial_recording_items", "review_partial_recording", "get_partial_recording_result"].includes(command) ||
    ["start_data_operation", "cancel_data_operation"].includes(command) && f.snapshot.kind === "partial_recording") f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_partial_recording_items") {
    if (d.listFail) throw Error("模擬部分救出一覧失敗");
    const all = listingItems().filter(i => !args.after || i.sessionId > args.after), items = all.slice(0, 200);
    return { items, next: all.length > 200 ? items.at(-1).sessionId : null };
  }
  if (command === "get_partial_recording_result") return d.committed && args.sessionId === d.committed.sessionId && args.reviewHash === d.committed.reviewHash ? structuredClone(d.committed) : null;
  if (command === "review_partial_recording") {
    if (f.snapshot.busy || f.blocker || args.sessionId !== session) throw Error("部分救出の対象または実行排他が不正です");
    d.last = null;
    await f.changed({ ...idle, revision: f.snapshot.revision, token: crypto.randomUUID(), kind: "partial_recording", busy: true, phase: "verifying" });
    setTimeout(() => void f.changed(f.snapshot.phase === "cancelling" ? { phase: "cancelled", busy: false } : d.reviewFail ?
      { phase: "failed", busy: false, errorMessage: "模擬確定範囲の検査失敗", retryAllowed: false } :
      { phase: "ready", busy: false, retryAllowed: true, partialRecordingPreview: { confirmationId: crypto.randomUUID(), summary: summary() } }), 50);
    if (d.reviewLost) throw Error("模擬部分救出検査の応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "start_data_operation" && f.snapshot.kind === "partial_recording") {
    const r = args.request, p = f.snapshot.partialRecordingPreview;
    if (d.mode === "lostNoCommit") throw Error("模擬部分救出受付喪失・未受付");
    if (JSON.stringify(r) === JSON.stringify(d.last) && (f.snapshot.busy || f.snapshot.phase === "completed" || r.action === "refresh" && f.snapshot.phase === "ready")) return structuredClone(f.snapshot);
    if (!p || r.token !== f.snapshot.token || r.confirmationId !== p.confirmationId || f.snapshot.busy) throw Error("部分救出の確認が失効しました");
    d.last = structuredClone(r);
    if (r.action === "refresh") {
      await f.changed({ phase: "ready", busy: false, errorMessage: null, attentionRequired: false, retryAllowed: true, partialRecordingPreview: { ...p, confirmationId: crypto.randomUUID() } });
      if (d.mode === "lostRefresh") throw Error("模擬再確認の応答喪失");
      return structuredClone(f.snapshot);
    }
    if (!["recover", "keep"].includes(r.action) || f.snapshot.phase !== "ready" || r.action === "recover" && p.summary.durationUs <= 0) throw Error("部分救出の明示同意を再確認してください");
    if (d.mode === "stale") { await f.changed({ phase: "failed", busy: false, retryAllowed: true, errorMessage: "確認後に原音声が変更されました。再確認してください" }); return structuredClone(f.snapshot); }
    if (d.mode === "forged") return { ...structuredClone(f.snapshot), revision: f.snapshot.revision + 1, phase: "completed", retryAllowed: false,
      partialRecordingReceipt: { ...p, confirmationId: crypto.randomUUID(), recovered: true } };
    await f.changed({ phase: "publishing", busy: true, retryAllowed: false });
    const finish = async () => {
      if (f.snapshot.phase === "cancelling" && d.mode !== "cancelRace") return f.changed({ phase: "cancelled", busy: false, retryAllowed: true, errorMessage: "合成変換を停止しました。原音声と途中FLACを保持します" });
      if (r.action === "recover") { d.recovered++; d.committed = structuredClone(p.summary); f.meetings[0].lifecycle = "recorded"; f.meetings[0].lockVersion++; }
      else d.kept++;
      await f.changed({ phase: "completed", busy: false, retryAllowed: false, errorMessage: null, partialRecordingReceipt: { ...p, recovered: r.action === "recover" } });
    };
    d.complete = finish;
    if (!["delay", "cancelRace"].includes(d.mode)) setTimeout(() => void finish(), 50);
    if (d.mode === "lost") throw Error("模擬部分救出実行の応答喪失");
    return structuredClone(f.snapshot);
  }
  if (command === "cancel_data_operation" && f.snapshot.kind === "partial_recording") {
    if (d.cancelFail) throw Error("模擬部分救出停止の受付失敗");
    if (args.token !== f.snapshot.token) throw Error("停止対象が変更されました");
    if (f.snapshot.busy) await f.changed({ phase: "cancelling" });
    else if (!f.snapshot.partialRecordingReceipt) await f.changed({ phase: "cancelled", retryAllowed: false, partialRecordingPreview: null });
    if (d.cancelLost) throw Error("模擬部分救出停止の応答喪失");
    return structuredClone(f.snapshot);
  }
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
