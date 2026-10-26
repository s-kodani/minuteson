import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { acceptRecovery, recoveryProgress, type RecoverySnapshot } from "./recording-recovery-state";
import { usePublishJobReadStatus, usePublishProgress } from "./operation-progress";
import { IssueNotice, ToastNotice } from "./toast";
import type { RecoverySnapshot as Snapshot } from "./recording-recovery-state";

export function RecordingRecoveryNotifications({ snapshot, error }: { snapshot: Snapshot | null; error: string | null }) {
  return <>
    <IssueNotice scope="recording-recovery" type="read" label="録音回復の状態" message={error ? `回復状態を確認できません。表示は前回取得した状態です。${error}` : snapshot?.error} />
    {snapshot?.items.filter(item => item.phase === "failed" || item.phase === "succeeded").map(item =>
      <ToastNotice key={item.sessionId} scope={item.sessionId} type="recording-recovery-result"
        meetingId={item.meetingId} eventId={`${snapshot.runId}:${item.sessionId}:${item.phase}`}
        level={item.phase === "failed" ? "error" : "success"}
        message={item.phase === "failed" ? `「${item.meetingTitle}」を回復できませんでした。録音は未完了のまま保持しています。元ファイルは保持しています。${item.error ?? "詳細は内部ログで確認してください。"}` : `「${item.meetingTitle}」の録音を回復し、保存を完了しました。`} />)}
  </>;
}

export function useRecordingRecovery() {
  const latest = useRef<RecoverySnapshot | null>(null);
  const [snapshot, setSnapshot] = useState<RecoverySnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  usePublishProgress("recording-recovery", recoveryProgress(snapshot));
  usePublishJobReadStatus(error, "recording-recovery");
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false, reading = false, again = false;
    let stop: (() => void) | undefined;
    const refresh = async () => {
      if (disposed) return;
      if (reading) { again = true; return; }
      reading = true;
      try {
        do {
          again = false;
          try {
            const next = acceptRecovery(latest.current, await invoke("get_recording_recovery"));
            if (!disposed) { latest.current = next; setSnapshot(next); setError(""); }
          } catch (cause) { if (!disposed) setError(String(cause)); }
        } while (again && !disposed);
      } finally { reading = false; }
    };
    void listen("recording-recovery-changed", () => void refresh()).then(unlisten => {
      if (disposed) unlisten(); else { stop = unlisten; void refresh(); }
    }).catch(() => void refresh());
    void refresh();
    const timer = window.setInterval(() => void refresh(), 1000);
    window.addEventListener("focus", refresh);
    const visible = () => { if (document.visibilityState === "visible") void refresh(); };
    document.addEventListener("visibilitychange", visible);
    return () => { disposed = true; stop?.(); window.clearInterval(timer); window.removeEventListener("focus", refresh); document.removeEventListener("visibilitychange", visible); };
  }, []);
  return {snapshot, error};
}
