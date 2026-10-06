// Dedicated simulated backend. localStorage is fixture storage ONLY, never
// production preference persistence. No native microphone/files/models used.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
window.skipAudioDeletionFixtureMain = true;
const { audioDeletionIPC } = await import("./audio-deletion-browser-fixture.js");
mockWindows(new URLSearchParams(location.search).get("window") === "recorder" ? "recorder" : "main");
const key = "MON_THEME_BROWSER_FIXTURE";
const initial = { id: "ui_preferences", payload: { schemaVersion: 1, theme: "system" }, lockVersion: 0 };
const stored = () => JSON.parse(localStorage.getItem(key) || JSON.stringify(initial));
const source = await audioDeletionIPC("get_current_analysis", { meetingId: window.dataFixture.meetings[0].id });
const minutesId = crypto.randomUUID(), draftId = crypto.randomUUID();
const text = value => ({ type: "text", text: value, bold: false, italic: false });
const content = { schemaVersion: 1, blocks: [
  { type: "heading", level: 1, content: [text("テーマ確認用の議事録")] },
  { type: "paragraph", itemId: null, content: [text("保存済みの点検日メモです。")] },
  { type: "table", headers: [[text("内容")], [text("担当者")]], rows: [{ itemId: null, cells: [[text("点検日を確認する")], [text("未確認")]] }] },
], items: [], evidence: [] };
const minutes = { id: minutesId, meetingId: source.meetingId, inputAnalysisId: source.id, inputContentHash: source.contentHash, contentHash: "b".repeat(64), content, generatedContent: structuredClone(content) };
let draft = { id: draftId, meetingId: source.meetingId, baseMinutesId: minutesId, contentHash: "c".repeat(64), payload: structuredClone(content), lockVersion: 0, updatedAt: 1791160000000 };
const f = window.themeFixture = { calls: [], mode: "success", readFail: false, suppressEvents: false, releaseSave: null,
  minutesSaveDelay: false, releaseMinutesSave: null, savedMinutes: () => structuredClone(draft),
  stored, reset: () => localStorage.removeItem(key),
  emitHint: value => emit("ui-preferences-changed", value),
  setStored: async value => { localStorage.setItem(key, JSON.stringify(value)); if (!f.suppressEvents) await emit("ui-preferences-changed", value.lockVersion); },
};
mockIPC(async (command, args) => {
  if (["get_ui_preferences", "save_ui_preferences", "get_minutes_preview", "get_minutes_draft", "create_minutes_draft", "save_minutes_draft", "prepare_utterance_playback", "read_playback_chunk", "close_playback"].includes(command)) f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_minutes_versions") return { meetingLockVersion: 0, currentId: minutesId, previousId: null, draft: { id: draftId, lockVersion: draft.lockVersion }, versions: [{ id: minutesId, revisionNo: 1, contentHash: minutes.contentHash, publicationState: "committed", inputAnalysisId: source.id, inputUpdated: false, itemCount: 0, createdAt: 1791160000000, profile: { qualityStatus: "unverified" } }] };
  if (command === "get_minutes_preview") return structuredClone(minutes);
  if (["get_minutes_draft", "create_minutes_draft"].includes(command)) return structuredClone(draft);
  if (command === "save_minutes_draft") {
    if (args.draftId !== draft.id || args.expectedLockVersion !== draft.lockVersion) throw Error("模擬議事録の競合");
    if (f.minutesSaveDelay) await new Promise(resolve => { f.releaseMinutesSave = resolve; });
    draft = { ...draft, payload: args.payload, lockVersion: draft.lockVersion + 1 };
    return structuredClone(draft);
  }
  if (command === "get_ui_preferences") { if (f.readFail) throw Error("模擬表示設定読込失敗"); return stored(); }
  if (command === "save_ui_preferences") {
    if (window.audioDeletionFixture.blocked || window.dataFixture.snapshot.restartRequired || window.dataFixture.snapshot.busy) throw Error("模擬データ保守の書込停止");
    if (f.mode === "delay") await new Promise(resolve => { f.releaseSave = resolve; });
    if (f.mode === "fail") throw Error("模擬テーマ保存失敗");
    const current = stored();
    if (args.expectedLockVersion !== current.lockVersion) throw Error("模擬表示設定の競合");
    if (f.mode === "forged") return { ...current, id: "foreign" };
    const next = { ...current, payload: args.payload, lockVersion: current.lockVersion + (current.payload.theme === args.payload.theme ? 0 : 1) };
    await f.setStored(next);
    if (f.mode === "lostUnreadable") f.readFail = true;
    if (["lost", "lostUnreadable"].includes(f.mode)) throw Error("模擬保存結果の応答喪失");
    return next;
  }
  if (command === "prepare_utterance_playback") return { token: "theme-silent-audio", sampleRate: 16000, channels: 1, totalFrames: 16000, startUs: 0, endUs: 1000000, trackKind: "mic", alignment: "unverified" };
  if (command === "read_playback_chunk") {
    const frames = Math.min(65536, 16000 - args.offsetFrames), bytes = new ArrayBuffer(12 + frames * 4), view = new DataView(bytes);
    view.setUint32(0, 16000, true); view.setUint32(4, 1, true); view.setUint32(8, frames, true);
    return Array.from(new Uint8Array(bytes)); // generated silence, no audio files
  }
  if (command === "close_playback") return null;
  return audioDeletionIPC(command, args);
}, { shouldMockEvents: true });
await import("../src/main.tsx");
