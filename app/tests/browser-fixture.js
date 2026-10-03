// Development-only browser fixture. Not imported by the application or build.
// Playwright routes index.html to this bootstrap for manual UI verification.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

const meetings = ["会議A", "会議B"].map((title, index) => ({ id: `meeting-${index}`, title, lifecycle: "recorded", createdAt: 1791028800000, updatedAt: 1791028800000, lockVersion: 1 }));
const analysis = { id: "analysis", meetingId: meetings[0].id, contentHash: "a".repeat(64), speakers: [{ id: "speaker-a", label: "話者A", displayName: "田中" }, { id: "speaker-b", label: "話者B", displayName: "佐藤" }], utterances: [
  { id: "speech-a", trackId: "microphone", startUs: 0, endUs: 15000000, recognizedText: "次の点検日は来週です。", effectiveText: "次の点検日は来週です。", attributionState: "assigned", speakerIds: ["speaker-a"] },
  { id: "speech-b", trackId: "system", startUs: 20000000, endUs: 25000000, recognizedText: "はい、確認します。", effectiveText: "はい、確認します。", attributionState: "overlap", speakerIds: ["speaker-a", "speaker-b"] },
] };
const fixture = window.monBrowserFixture = { calls: [], failSave: false, failPlayback: false, persistenceDelayMs: 0, draft: null };
window.isTauri = true;
mockWindows("main");
mockIPC(async (command, args) => {
  fixture.calls.push({ command, args });
  switch (command) {
    case "get_recording_snapshot": return { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null };
    case "capture_available": return false;
    case "list_meetings": return meetings;
    case "get_meeting": return meetings.find((meeting) => meeting.id === args.id) ?? null;
    case "get_current_analysis": return args.meetingId === meetings[0].id ? analysis : null;
    case "get_transcript_draft": return args.meetingId === meetings[0].id ? fixture.draft : null;
    case "begin_transcript_draft":
      fixture.draft = { id: "draft", meetingId: meetings[0].id, baseAnalysisId: analysis.id, payload: { schema_version: 1, speaker_names: [], utterances: [] }, lockVersion: 0, updatedAt: 1791028800000 };
      return structuredClone(fixture.draft);
    case "save_transcript_draft":
      await new Promise((resolve) => setTimeout(resolve, fixture.persistenceDelayMs));
      if (fixture.failSave) throw new Error("模擬保存競合");
      if (args.expectedLockVersion !== fixture.draft.lockVersion) throw new Error("版番号不一致");
      fixture.draft = { ...fixture.draft, payload: structuredClone(args.payload), lockVersion: fixture.draft.lockVersion + 1 };
      return structuredClone(fixture.draft);
    case "prepare_utterance_playback":
      if (fixture.failPlayback) throw new Error("音声ファイルのハッシュが保存時と一致しません");
      return { token: "audio-token", sampleRate: 48000, channels: 2, totalFrames: 720000, startUs: 0, endUs: 15000000, trackKind: "mic", alignment: "unverified" };
    case "read_playback_chunk": {
      const frames = Math.min(65536, 720000 - args.offsetFrames);
      const bytes = new ArrayBuffer(12 + frames * 2 * 4);
      const view = new DataView(bytes);
      view.setUint32(0, 48000, true);
      view.setUint32(4, 2, true);
      view.setUint32(8, frames, true);
      return bytes; // Silent PCM: browser verification must not produce noise.
    }
    case "close_playback":
    case "finish_main_close": return null;
    default: throw new Error(`Unexpected fixture command: ${command}`);
  }
}, { shouldMockEvents: true });
await import("../src/main.tsx");
