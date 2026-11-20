import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { acceptRecovery, recoveryMeetingState, type RecoverySnapshot } from "./recording-recovery-state";
import { usePublishJobReadStatus } from "./operation-progress";
import { IssueNotice, ToastNotice } from "./toast";
import type { RecoverySnapshot as Snapshot } from "./recording-recovery-state";
import type { RecordingSnapshot } from "./session";

export function RecordingRecoveryNotifications({ snapshot, recording, recordingError }: { snapshot: Snapshot | null; recording: RecordingSnapshot | null; recordingError?: string | null }) {
  const fallback = recording && !snapshot?.items.some(item => item.sessionId === recording.sessionId) ? recording.errorMessage || recordingError : null;
  const representedError = snapshot?.items.some(item => item.phase === "failed" && item.error === snapshot.error && (snapshot.origin === "startup" || item.sessionId === snapshot.targetSessionId));
  return <>
    <IssueNotice scope="recording-save" type="failure" label="録音の保存" message={representedError ? null : snapshot?.error} />
    {fallback && <ToastNotice key={recording?.sessionId ?? "recording"} scope={recording?.sessionId ?? "recording"} type="recording-save-result" level="error" eventId={`recording_save:${recording?.sessionId}:failed`} replaceEventHistory message={fallback} />}
    {snapshot?.items.map(item =>
      <ToastNotice key={item.sessionId} scope={item.sessionId} type="recording-save-result"
        meetingId={item.meetingId} eventId={`recording_save:${item.sessionId}:${item.phase}`} replaceEventHistory
        level={item.phase === "failed" ? "error" : item.phase === "succeeded" && item.warning ? "warning" : "success"}
        action={item.phase === "failed" && !item.noAudio ? {kind:"retry", label:"保存を再試行", disabled:snapshot.phase === "running" || snapshot.phase === "scanning", run:async()=>{await invoke("retry_recording_recovery",{sessionId:item.sessionId});}} : undefined}
        message={item.phase === "failed" ? item.noAudio ? `「${item.meetingTitle}」の録音に失敗しました。マイク・PC音声のどちらも音声を取得できませんでした。` : `「${item.meetingTitle}」の音声を保存できませんでした。元音声は保持しています。「保存を再試行」でやり直せます。${item.error ?? "詳細は内部ログで確認してください。"}` : item.phase === "succeeded" ? item.warning ? `「${item.meetingTitle}」: ${item.warning}` : "録音を保存完了しました。" : null} />)}
  </>;
}

export function RecordingSaveRetry({snapshot, meetingId, locked}: {snapshot: Snapshot | null; meetingId:string; locked:boolean}) {
  const item = recoveryMeetingState(snapshot, meetingId);
  const pending = useRef(false);
  const [busy,setBusy] = useState(false);
  const [error,setError] = useState("");
  if (!item || item.phase !== "failed" || item.noAudio) return null;
  return <><IssueNotice scope={item.sessionId} type="recording-save-retry" label="音声の保存" message={error}/><button type="button" className="small-secondary" disabled={locked || busy || snapshot?.phase === "running" || snapshot?.phase === "scanning"} onClick={async()=>{
    if(pending.current)return;
    pending.current=true;setBusy(true);setError("");
    try { await invoke("retry_recording_recovery",{sessionId:item.sessionId}); }
    catch(cause){setError(String(cause));}
    finally{pending.current=false;setBusy(false);}
  }}>保存を再試行</button></>;
}

export function useRecordingRecovery() {
  const latest = useRef<RecoverySnapshot | null>(null);
  const [snapshot, setSnapshot] = useState<RecoverySnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
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
