import { errorMessage } from "./app-error";
import { invoke } from "./ipc";

type BackgroundReadSource =
  | "restore_labels" | "model_download_guidance"
  | "window_title" | "window_maximized" | "window_resize_subscription" | "window_focus_subscription"
  | "transcription_publication" | "audio_import_status" | "audio_import_subscription";

const sustainedFailureMs = 6_000;

/** Report one diagnostic per outage. Only another failed read can establish a sustained outage. */
export function backgroundReadFailure(source: BackgroundReadSource) {
  let firstFailure: number | null = null;
  return {
    failed(cause: unknown): boolean {
      const now = performance.now();
      if (firstFailure !== null) return now - firstFailure >= sustainedFailureMs;
      firstFailure = now;
      const message = Array.from(errorMessage(cause).trim()).slice(0, 1024).join("") || "詳細なし";
      void invoke("record_background_read_failure", { source, message }).catch(() => {});
      return false;
    },
    recovered() { firstFailure = null; },
  };
}
