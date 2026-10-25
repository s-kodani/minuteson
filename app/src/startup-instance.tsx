import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CommonModal } from "./common-modal";
import { checkedStartupInstance, type StartupInstanceReport } from "./startup-instance-state";

export function StartupInstanceRecovery() {
  const [report, setReport] = useState<StartupInstanceReport | null>(null);
  const [error, setError] = useState(""), [busy, setBusy] = useState(false), [review, setReview] = useState<string | null>(null);
  const [uncertain, setUncertain] = useState(false);
  const executing = useRef(false), mounted = useRef(true), opener = useRef<HTMLButtonElement>(null);
  const read = async () => {
    try { const next = checkedStartupInstance(await invoke("get_startup_instance")); if (mounted.current) setReport(next); }
    catch (cause) { if (mounted.current) setError(String(cause)); }
  };
  useEffect(() => { mounted.current = true; void read(); return () => { mounted.current = false; }; }, []);
  const run = async (command: string, args?: { targetId: string }) => {
    if (executing.current) return;
    executing.current = true; setBusy(true); setError("");
    try {
      const result = await invoke(command, args);
      if (!mounted.current) return;
      if (command === "open_existing_instance" && !result) setError("既存のMONを開けませんでした。確認して再起動するか、もう一度接続できます。");
      setReview(null);
    } catch (cause) {
      if (mounted.current) { setError(`操作の完了を確認できません。自動で再実行しません: ${String(cause)}`); if (command !== "open_existing_instance") setUncertain(true); }
    } finally {
      executing.current = false;
      if (mounted.current) { setBusy(false); await read(); }
    }
  };
  return <section aria-label="起動の復旧" className="status-card">
    <h2>MONを開き直す</h2>
    <p>既存の画面を開くか、MONを再起動して復旧できます。</p>
    {report?.message && <p role="status">{report.message}</p>}
    <div className="analysis-actions">
      <button type="button" disabled={busy} onClick={() => void run("open_existing_instance")}>もう一度開く</button>
      {report?.canRestart && <button ref={opener} type="button" disabled={busy || uncertain} onClick={() => setReview(report.targetId)}>MONを再起動して復旧</button>}
      <button type="button" disabled={busy || uncertain} onClick={() => void run("retry_startup")}>起動をやり直す</button>
    </div>
    {uncertain && <p role="status">終了・再起動の要求は再送していません。「もう一度開く」で現在のMONを確認してください。</p>}
    {error && <p role="alert" className="action-error">{error}</p>}
    <CommonModal open={review !== null} blocked={busy} onDismiss={() => { setReview(null); setUncertain(false); }} restoreTarget={() => opener.current} aria-labelledby="startup-restart-title">
      <h2 id="startup-restart-title">既存のMONを終了して復旧しますか？</h2>
      <p>応答しないMONを終了し、保存されたデータから起動し直します。録音中・保存中だった場合は、未確定の録音や未保存の編集が残る可能性があります。</p>
      <p>元の音声は削除せず、次の起動時に回収を試みます。回収できない録音はその状態を表示します。</p>
      {busy && <p role="status">既存MONの状態と終了を確認しています。</p>}
      {error && <p role="alert" className="action-error">{error}</p>}
      <button type="button" disabled={busy || uncertain || !review} onClick={() => { if (review) void run("recover_startup_instance", { targetId: review }); }}>確認して再起動する</button>
    </CommonModal>
  </section>;
}
