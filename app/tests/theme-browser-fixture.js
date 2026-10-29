// Dedicated simulated backend. localStorage is fixture storage ONLY, never
// production preference persistence. No native microphone/files/models used.
import { mockIPC, mockWindows } from "./startup-ready-mocks.ts";
import { emit } from "@tauri-apps/api/event";
window.skipAudioDeletionFixtureMain = true;
const { audioDeletionIPC } = await import("./audio-deletion-browser-fixture.js");
mockWindows(new URLSearchParams(location.search).get("window") === "recorder" ? "recorder" : "main");
const key = "MON_THEME_BROWSER_FIXTURE";
const initial = { id: "ui_preferences", payload: { schemaVersion: 1, theme: "system", colorTheme: "monochrome" }, lockVersion: 0 };
const stored = () => JSON.parse(localStorage.getItem(key) || JSON.stringify(initial));
const source = await audioDeletionIPC("get_current_analysis", { meetingId: window.dataFixture.meetings[0].id });
let minutes = {meetingId:source.meetingId,markdown:"# テーマ確認用の議事録\n\n保存済みの点検日メモです。\n",contentHash:"b".repeat(64),lockVersion:1,operationId:crypto.randomUUID(),updatedAt:1791160000000};
const f = window.themeFixture = { calls: [], mode: "success", readFail: false, suppressEvents: false, releaseSave: null,
  minutesSaveDelay: false, releaseMinutesSave: null, savedMinutes: () => structuredClone(minutes),
  stored, reset: () => localStorage.removeItem(key),
  emitHint: value => emit("ui-preferences-changed", value),
  setStored: async value => { localStorage.setItem(key, JSON.stringify(value)); if (!f.suppressEvents) await emit("ui-preferences-changed", value.lockVersion); },
};
mockIPC(async (command, args) => {
  if (["get_ui_preferences", "save_ui_preferences", "get_minutes_markdown", "save_minutes_markdown", "prepare_utterance_playback", "read_playback_chunk", "close_playback"].includes(command)) f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_minutes_markdown") return args.meetingId===minutes.meetingId?structuredClone(minutes):null;
  if (command === "save_minutes_markdown") {
    const r=args.request;
    if (r.meetingId!==minutes.meetingId||r.expected?.lockVersion!==minutes.lockVersion||r.expected?.contentHash!==minutes.contentHash) throw Error("模擬議事録の競合");
    if (f.minutesSaveDelay) await new Promise(resolve => { f.releaseMinutesSave = resolve; });
    minutes = {...minutes,markdown:r.markdown,operationId:r.operationId,contentHash:"c".repeat(64),lockVersion:minutes.lockVersion+1};
    return structuredClone(minutes);
  }
  if (command === "get_ui_preferences") { if (f.readFail) throw Error("模擬表示設定読込失敗"); return stored(); }
  if (command === "save_ui_preferences") {
    if (window.audioDeletionFixture.blocked || window.dataFixture.snapshot.restartRequired || window.dataFixture.snapshot.busy) throw Error("模擬データ保守の書込停止");
    if (f.mode === "delay") await new Promise(resolve => { f.releaseSave = resolve; });
    if (f.mode === "fail") throw Error("模擬テーマ保存失敗");
    const current = stored();
    if (args.expectedLockVersion !== current.lockVersion) throw Error("模擬表示設定の競合");
    if (f.mode === "forged") return { ...current, id: "foreign" };
    const next = { ...current, payload: args.payload, lockVersion: current.lockVersion + (JSON.stringify(current.payload) === JSON.stringify(args.payload) ? 0 : 1) };
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
await import("../src/bootstrap.tsx");
