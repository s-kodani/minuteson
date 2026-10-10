// Development-only synthetic IPC. No native windows, audio, files or models.
import { mockIPC } from "./startup-ready-mocks.ts";
window.skipDataFixtureMain = true;
const { dataSaveIPC } = await import("./data-save-browser-fixture.js");
const f = window.dataFixture;
Object.assign(f.snapshot, { deletionPreview: null, deletionReceipt: null, modelDeletionPreview: null, modelDeletionReceipt: null, audioDeletionPreview: null, audioDeletionReceipt: null, partialRecordingPreview: null, partialRecordingReceipt: null });
const idle = structuredClone(f.snapshot);
let blocked = false, interrupted = false;
mockIPC(async (command, args) => {
  const listing = { items: [], writeBlocked: false, restartRequired: false, reason: null };
  if (command === "get_model_download_catalog") return { supported: true, entries: [], unknownEntries: 0 };
  if (command === "get_deletion_items") return { ...listing, deletingMeetingIds: [] };
  if (command === "get_model_deletion_items") return { ...listing, writeBlocked: blocked, reason: blocked ? "合成：中断したモデル削除のため書込停止" : null };
  if (command === "get_audio_deletion_items") return listing;
  if (command === "get_partial_recording_items") return { items: interrupted ? [{ sessionId: "11111111-1111-4111-8111-111111111111", meetingId: f.meetings[0].id, title: "合成の中断録音", state: "interrupted" }] : [], next: null };
  if (command === "get_partial_recording_results") return { items: [], next: null };
  return dataSaveIPC(command, args);
}, { shouldMockEvents: true });
const controls = document.getElementById("synthetic-controls");
controls.append("合成検証：実データの変更・録音なし ");
for (const [name, action] of [
  ["合成の書込停止", () => { blocked = true; }],
  ["合成の中断録音", () => { interrupted = true; }],
  ["合成の未完了データ", () => {
    const id = crypto.randomUUID();
    f.recoveryItems = [{ source: "scratch", operationId: id, paths: [`.mon-stage-${id}`], registeredPaths: 2, reason: "合成の追加復元中断：所有不明のファイルを保持", retained: false }];
    Object.assign(f.snapshot, { token: crypto.randomUUID(), kind: "restore", phase: "failed", busy: false, errorMessage: "合成の追加復元中断", attentionRequired: true });
  }],
  ["合成の読込失敗", () => { f.failRead = true; }],
  ["合成状態を戻す", () => { blocked = false; interrupted = false; f.failRead = false; f.recoveryItems = []; Object.assign(f.snapshot, { ...idle, revision: f.snapshot.revision }); }],
]) {
  const button = document.createElement("button"); button.textContent = name;
  button.onclick = () => { action(); void f.changed({}); };
  controls.append(button);
}
await import("../src/main.tsx");
