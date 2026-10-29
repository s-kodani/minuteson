import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import { SnapshotResource } from "./snapshot-resource";
import { browserFocus, browserSchedule } from "./snapshot-browser";
import type { StageProgress } from "./stage-progress";

export type RecordingPhase = "idle" | "preparing" | "recording" | "stopping" | "saving" | "error";

export interface RecordingSnapshot {
  progress?: StageProgress | null;
  revision: number;
  phase: RecordingPhase;
  elapsedMs: number;
  microphoneConnected: boolean;
  systemAudioConnected: boolean;
  meetingId: string | null;
  sessionId: string | null;
  errorMessage: string | null;
}

export type BridgeState =
  | { kind: "loading" }
  | { kind: "unavailable" }
  | { kind: "error"; message: string }
  | { kind: "ready"; snapshot: RecordingSnapshot };

const recordingResource = new SnapshotResource<RecordingSnapshot>({
  read: () => invoke<RecordingSnapshot>("get_recording_snapshot"),
  subscribe: accept => listen<RecordingSnapshot>("session-changed", event => accept(event.payload)),
  accept: (current, value) => {
    if (!value || !Number.isSafeInteger(value.revision) || value.revision < 0
      || !["idle", "preparing", "recording", "stopping", "saving", "error"].includes(value.phase)
      || !Number.isFinite(value.elapsedMs) || value.elapsedMs < 0
      || typeof value.microphoneConnected !== "boolean" || typeof value.systemAudioConnected !== "boolean"
      || ![value.meetingId, value.sessionId, value.errorMessage].every(field => field === null || typeof field === "string")) {
      throw Error("録音状態の形式を確認できません。");
    }
    return current && current.revision > value.revision ? current : value;
  },
  intervalMs: 1000, schedule: browserSchedule, focus: browserFocus,
});
const unavailableSubscription = () => () => {};
export function useRecordingSession(): BridgeState {
  const native = isTauri();
  const state = useSyncExternalStore(native ? recordingResource.subscribe : unavailableSubscription, recordingResource.getSnapshot);
  if (!native) return { kind: "unavailable" };
  if (state.error) return { kind: "error", message: state.error };
  return state.connected && state.snapshot ? { kind: "ready", snapshot: state.snapshot } : { kind: "loading" };
}

export function formatElapsed(elapsedMs: number): string {
  const total = Math.floor(elapsedMs / 1000);
  return [Math.floor(total / 3600), Math.floor((total % 3600) / 60), total % 60]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}
