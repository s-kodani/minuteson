import { invoke } from "./ipc.ts";
import { useEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";
import { checkedResetPreview, checkedResetSnapshot, resetRequest, type ResetAvailability, type ResetPreview, type ResetSnapshot } from "./startup-reset-state";
import { StartupResetForm } from "./startup-reset-form";
import { useAppName } from "./window-environment";

export function StartupReset({ availability, onBusy }: { availability?: ResetAvailability; onBusy: (busy: boolean) => void }) {
  const appName = useAppName();
  const [preview, setPreview] = useState<ResetPreview | null>(null), [archive, setArchive] = useState(true), [selecting, setSelecting] = useState(false), [loading, setLoading] = useState(false), [snapshot, setSnapshot] = useState<ResetSnapshot | null>(null), [error, setError] = useState(""), [uncertain, setUncertain] = useState(false);
  const active = useRef(true), executing = useRef(false), reading = useRef(false), generation = useRef(0), opener = useRef<HTMLButtonElement>(null);
  const busy = !!snapshot?.busy || uncertain;
  const read = async () => {
    if (reading.current) return;
    reading.current = true; const requestGeneration = generation.current;
    try { const s = checkedResetSnapshot(await invoke("get_startup_reset_state")); if (active.current && requestGeneration === generation.current) { setSnapshot(s); setUncertain(false); } }
    catch (cause) { if (active.current) setError(String(cause)); }
    finally { reading.current = false; }
  };
  useEffect(() => { active.current = true; void read(); return () => { active.current = false; }; }, []);
  useEffect(() => { onBusy(busy || selecting || loading); }, [busy, selecting, loading, onBusy]);
  useEffect(() => {
    if (!busy) return;
    const timer = window.setInterval(() => void read(), 400);
    return () => window.clearInterval(timer);
  }, [busy]);
  const prepare = async () => {
    if (executing.current) return; executing.current = true; setLoading(true); setError("");
    try { const p = checkedResetPreview(await invoke("get_startup_reset_preview")); if (active.current) { setPreview(p); setArchive(true); } }
    catch (cause) { if (active.current) setError(String(cause)); }
    finally { executing.current = false; if (active.current) setLoading(false); }
  };
  const choose = async () => {
    if (!preview || executing.current) return; executing.current = true; setSelecting(true); setError("");
    try {
      const path = await invoke<string | null>("choose_startup_reset_backup_directory", { operationId: preview.operationId });
      if (active.current && path !== null) setPreview(checkedResetPreview({ ...preview, archivePath: path }));
    } catch (cause) { if (active.current) setError(String(cause)); }
    finally { executing.current = false; if (active.current) setSelecting(false); }
  };
  const start = async () => {
    if (!preview || executing.current || busy) return;
    generation.current += 1; executing.current = true; setUncertain(true); setError("");
    try { const s = checkedResetSnapshot(await invoke("start_startup_reset", resetRequest(preview, archive))); if (active.current) { setSnapshot(s); setUncertain(false); } }
    catch (cause) { if (active.current) setError(`受付状態を再確認しています。初期化は再送しません: ${String(cause)}`); }
    finally { executing.current = false; }
  };
  const allowed = preview?.availability ?? availability;
  return <section className="status-card" aria-label="データの初期化">
    <h2>データを初期化して起動する</h2>
    <p>会議・モデル・設定を初期化して、現在の保存先で起動し直せます。</p>
    {allowed?.available === false && <p role="status">利用できない理由：{allowed.reason}</p>}
    <button type="button" ref={opener} disabled={busy || loading || selecting || !!snapshot?.operationId} onClick={() => void prepare()}>{allowed?.available === false ? "初期化の可否を再確認" : "データを初期化"}</button>
    {!preview && snapshot?.message && <p role="status">{snapshot.message}</p>}
    {snapshot?.operationId && !snapshot.busy && <button type="button" onClick={() => void invoke("retry_startup").catch(cause => setError(String(cause)))}>{appName}を再起動</button>}
    {!preview && (error || snapshot?.error) && <p role="alert">{error || snapshot?.error}</p>}
    <CommonModal open={!!preview?.availability.available} blocked={busy || selecting || loading} hideDismissButton onDismiss={() => { if (!busy && !selecting) setPreview(null); }} restoreTarget={() => opener.current} aria-labelledby="startup-reset-title" aria-describedby="startup-reset-description">
      {preview && <StartupResetForm productName={appName} preview={preview} archive={archive} busy={busy} accepted={!!snapshot?.operationId} selecting={selecting} message={snapshot?.message ?? ""} error={snapshot?.error || error} onArchive={setArchive} onChoose={() => void choose()} onCancel={() => setPreview(null)} onStart={() => { if (snapshot?.operationId) void invoke("retry_startup").catch(cause => setError(String(cause))); else void start(); }} />}
    </CommonModal>
  </section>;
}
