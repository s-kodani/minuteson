import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";

export type RecordingPhase = "idle" | "preparing" | "recording" | "stopping" | "saving" | "error";

export interface RecordingSnapshot {
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

export function useRecordingSession(): BridgeState {
  const [state, setState] = useState<BridgeState>({ kind: "loading" });

  useEffect(() => {
    if (!isTauri()) {
      setState({ kind: "unavailable" });
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | undefined;
    const accept = (snapshot: RecordingSnapshot) => {
      if (disposed) return;
      setState((current) =>
        current.kind === "ready" && current.snapshot.revision > snapshot.revision
          ? current
          : { kind: "ready", snapshot },
      );
    };
    const refresh = () => {
      void invoke<RecordingSnapshot>("get_recording_snapshot")
        .then(accept)
        .catch((error: unknown) => {
          if (!disposed) setState({ kind: "error", message: String(error) });
        });
    };

    // Subscribe before fetching: a state change between these operations is
    // resolved by monotonically increasing Rust-side revisions.
    void listen<RecordingSnapshot>("session-changed", (event) => accept(event.payload))
      .then((stop) => {
        if (disposed) stop();
        else {
          unlisten = stop;
          refresh();
        }
      })
      .catch((error: unknown) => {
        if (!disposed) setState({ kind: "error", message: String(error) });
      });
    window.addEventListener("focus", refresh);
    const timer = window.setInterval(refresh, 1000);
    return () => {
      disposed = true;
      unlisten?.();
      window.removeEventListener("focus", refresh);
      window.clearInterval(timer);
    };
  }, []);

  return state;
}

export function formatElapsed(elapsedMs: number): string {
  const total = Math.floor(elapsedMs / 1000);
  return [Math.floor(total / 3600), Math.floor((total % 3600) / 60), total % 60]
    .map((part) => String(part).padStart(2, "0"))
    .join(":");
}
