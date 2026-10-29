import { ErrorNotice, ToastHost } from "./toast";
import { IconButton } from "./action-icon";
import { invoke, isTauri } from "./ipc.ts";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { acceptStartupReport, startupStageLabels, type StartupReport } from "./startup-attention-state";
import { CommonModal } from "./common-modal";
import { usePlatform } from "./platform";
import { StartupReset } from "./startup-reset";
import { ResidencyNotifications } from "./residency-notifications";
import { StartupLoading } from "./startup-loading";

/** No theme, storage, recording or job provider is mounted in this route. */
function useStartupReport() {
  const [report, setReport] = useState<StartupReport | null>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false), [quitting, setQuitting] = useState(false);
  const [retryUncertain, setRetryUncertain] = useState(false);
  const current = useRef<StartupReport | null>(null), active = useRef(true), reading = useRef(false), closing = useRef(false);
  const read = async () => {
    if (!isTauri() || reading.current) return;
    reading.current = true; setBusy(true);
    try { const value = await invoke("get_startup_report"); if (active.current) { const next = acceptStartupReport(current.current, value); current.current = next; setReport(next); setError(""); } }
    catch (cause) { if (active.current) setError(String(cause)); }
    finally { reading.current = false; if (active.current) setBusy(false); }
  };
  useEffect(() => { active.current = true; void read(); return () => { active.current = false; }; }, []);
  useEffect(() => {
    // Only reread the in-memory startup result while native setup is pending.
    // A confirmed failure or an invalid/unreadable reply still needs review.
    if (report?.phase !== "starting" || busy || error) return;
    const timer = window.setTimeout(() => void read(), 200);
    return () => window.clearTimeout(timer);
  }, [report, busy, error]);
  const retry = async () => {
    if (closing.current || retryUncertain || current.current?.phase !== "blocked") return;
    closing.current = true; setQuitting(true); setError("");
    try { await invoke("retry_startup"); }
    catch (cause) { if (active.current) { setRetryUncertain(true); setError(`再起動の受付を確認できません。MONを終了してから起動し直してください: ${String(cause)}`); } }
    finally { closing.current = false; if (active.current) setQuitting(false); }
  };
  const quit = async () => {
    if (closing.current || current.current?.phase !== "blocked") return;
    closing.current = true; setQuitting(true); setError("");
    try { await invoke("quit_blocked_startup"); }
    catch (cause) { if (active.current) setError(`終了の受付を確認できません。保存先への操作は行っていません。OSの閉じる操作でも終了できます: ${String(cause)}`); }
    finally { closing.current = false; if (active.current) setQuitting(false); }
  };
  return { report, error, busy, quitting, retryUncertain, read, retry, quit };
}
export function StartupAttentionWindow() { return <StartupAttention state={useStartupReport()} />; }
export function StartupGate({ children }: { children: ReactNode }) {
  const state = useStartupReport();
  const platform = usePlatform();
  const [recoveryDismissed, setRecoveryDismissed] = useState(false);
  // Browser-only preview is not a native admission proof and cannot invoke
  // desktop operations. A desktop reader failure never mounts normal UI.
  return !isTauri() || state.report?.phase === "ready" ? <>
    {state.report?.platform && platform !== "other" && state.report.platform !== platform ? <p role="alert">起動結果のOS情報が一致しません。</p> : children}
    {state.report?.recovery && <CommonModal portal open={!recoveryDismissed} onDismiss={() => setRecoveryDismissed(true)} dismissLabel="閉じる" dismissText="復旧結果を閉じる" restoreTarget={() => document.querySelector<HTMLElement>(".sidebar button:not(:disabled)")} aria-labelledby="startup-recovery-title" aria-describedby="startup-recovery-description">
      <h2 id="startup-recovery-title">{state.report.recovery.reviewRequired ? "中断操作の再確認が必要です" : "保存先の情報を修復しました"}</h2>
      {state.report.recovery.reviewRequired ? <>
        <p id="startup-recovery-description">中断操作の再確認が必要なため、会議の閲覧のみ利用できます。元の記録とファイルは保持しています。</p>
        <ul>{state.report.recovery.reasons.map(reason => <li key={reason}>{reason}</li>)}</ul>
      </> : <p id="startup-recovery-description">保存先とDBを検証し、現在の保存先情報へ更新しました。通常どおり利用できます。</p>}
      <button type="button" onClick={() => setRecoveryDismissed(true)}>閉じる</button>
    </CommonModal>}
  </> : !state.error && (!state.report || state.report.phase === "starting")
    ? <><ToastHost /><ResidencyNotifications scope="startup"/><StartupLoading /></>
    : <StartupAttention state={state} />;
}
function StartupAttention({ state }: { state: ReturnType<typeof useStartupReport> }) {
  const { report, error, busy, quitting, retryUncertain, read, retry, quit } = state, f = report?.failure;
  const [resetBusy, setResetBusy] = useState(false);
  const normal = new URL(window.location.href); normal.searchParams.delete("window"); normal.hash = "";
  return <main className="startup-attention" aria-label="起動診断"><ToastHost />
    <ResidencyNotifications scope="startup"/>
    <h1>{report?.platform === "windows" ? "MONの起動と復旧" : "保存先の起動確認"}</h1>
    <p>通常の操作は停止しています。起動診断の確認と、データの初期化を行えます。</p>
    <ErrorNotice scope="startup" type="desktop-connection" active={!isTauri()}>デスクトップ接続がありません。起動結果を確認できません。</ErrorNotice>
    {(!report || report.phase === "starting") && <p role="status">起動結果を確認できるまで通常操作は利用できません。</p>}
    {f && <section className="status-card" aria-label="起動を続けられない理由">
      <h2>保存先の検証に失敗したため、通常の起動を中止しました</h2>
      <p>停止した段階：{startupStageLabels[f.stage]}</p>
      <dl><dt>選択された会議保存先</dt><dd>{f.meetingRoot ?? "選択先は未確定です。エラー本文から推測しません。"}</dd><dt>起動設定の保存先</dt><dd>{f.configRoot ?? "保存先を取得できていません。"}</dd></dl>
      <h3>診断内容</h3><pre aria-label="保持した起動診断">{f.message || "詳細はありません。原因を推測しません。"}</pre>
      {f.messageTruncated && <p role="status">長い診断の先頭だけを表示しています。完全な診断ではありません。</p>}
      <h3>データを守るための確認</h3>
      <p>起動停止時点のDB・台帳・起動設定・音声は、初期化を実行するまで保持します。空の保存先への自動切替や、ロックファイルの削除は行いません。</p>
      <p>他のMONが同じ保存先を使用していないか、外付け保存先の接続、空き容量やアクセス権を確認してください。これらが原因だと断定しているわけではありません。解決できなければ、表示された段階・保存先・診断内容を確認担当者へ伝えてください。共有前に個人情報を確認してください。</p>
      <p>終了後、原因を確認してから明示的に再起動してください。再起動時は保存先を改めて検証します。保存済みのデータを巻き戻す操作ではありません。</p>
    </section>}
    {report?.phase === "blocked" && <StartupReset availability={report.reset} onBusy={setResetBusy} />}
    {report?.phase === "starting" && report.migration && <p role="status">会議音声を移行しています：{report.migration.completed} / {report.migration.total}件。完了後に通常画面を開きます。</p>}
    {report?.phase === "ready" && <p role="status">保存先の起動確認は完了しています。<a href={normal.href}>通常画面を開く</a></p>}
    <div className="analysis-actions"><IconButton label="保持した診断だけを再読込" icon="refresh" type="button" className="secondary" disabled={!isTauri() || busy || quitting || resetBusy} onClick={() => void read()} />{report?.phase === "blocked" && <><button type="button" disabled={busy || quitting || resetBusy || retryUncertain} onClick={() => void retry()}>起動をやり直す</button><button type="button" disabled={busy || quitting || resetBusy} onClick={() => void quit()}>このアプリを終了</button></>}</div>
    <p>再読込はメモリに保持した診断だけを読みます。保存先を再び開く、検査・回収する、自動再試行する操作ではありません。</p>
    <ErrorNotice scope="startup-attention" type="StartupAttention-operation" active={!!(error)}>{error}</ErrorNotice>
  </main>;
}
