import { ErrorNotice, ToastHost } from "./toast";
import { invoke, isTauri } from "./ipc.ts";
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { acceptStartupReport, type StartupReport } from "./startup-attention-state";
import { CommonModal } from "./common-modal";
import { usePlatform } from "./platform";
import { StartupReset } from "./startup-reset";
import { ResidencyNotifications } from "./residency-notifications";
import { StartupLoading } from "./startup-loading";
import { useAppName } from "./window-environment";
import { displayNativePathsInText } from "./native-path";

/** No theme, storage, recording or job provider is mounted in this route. */
function useStartupReport() {
  const appName = useAppName();
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
    catch (cause) { if (active.current) { setRetryUncertain(true); setError(`再起動の受付を確認できません。${appName}を終了してから起動し直してください: ${String(cause)}`); } }
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
const StartupRecoveryPending = createContext(false);
export const useStartupRecoveryPending = () => useContext(StartupRecoveryPending);
export function StartupGate({ children }: { children: ReactNode }) {
  const state = useStartupReport();
  const platform = usePlatform();
  const [recoveryDismissed, setRecoveryDismissed] = useState(false);
  // Browser-only preview is not a native admission proof and cannot invoke
  // desktop operations. A desktop reader failure never mounts normal UI.
  return !isTauri() || state.report?.phase === "ready" ? <>
    {state.report?.platform && platform !== "other" && state.report.platform !== platform ? <p role="alert">起動結果のOS情報が一致しません。</p> : <StartupRecoveryPending value={!!state.report?.recovery && !recoveryDismissed}>{children}</StartupRecoveryPending>}
    {state.report?.recovery && <CommonModal portal open={!recoveryDismissed} onDismiss={() => setRecoveryDismissed(true)} dismissLabel="閉じる" dismissText="復旧結果を閉じる" restoreTarget={() => document.querySelector<HTMLElement>(".sidebar button:not(:disabled)")} aria-labelledby="startup-recovery-title" aria-describedby="startup-recovery-description">
      <h2 id="startup-recovery-title">{state.report.recovery.reviewRequired ? "中断操作の再確認が必要です" : "保存先の情報を修復しました"}</h2>
      {state.report.recovery.reviewRequired ? <>
        <p id="startup-recovery-description">中断操作の再確認が必要なため、会議の閲覧のみ利用できます。元の記録とファイルは保持しています。</p>
        <ul>{state.report.recovery.reasons.map(reason => <li key={reason}>{displayNativePathsInText(reason)}</li>)}</ul>
      </> : <p id="startup-recovery-description">保存先とDBを検証し、現在の保存先情報へ更新しました。通常どおり利用できます。</p>}
      <button type="button" onClick={() => setRecoveryDismissed(true)}>閉じる</button>
    </CommonModal>}
  </> : !state.error && (!state.report || state.report.phase === "starting")
    ? <><ToastHost /><ResidencyNotifications scope="startup"/><StartupLoading /></>
    : <StartupAttention state={state} />;
}
function StartupAttention({ state }: { state: ReturnType<typeof useStartupReport> }) {
  const { report, error, busy, quitting, retryUncertain, retry, quit } = state, f = report?.failure;
  const [resetBusy, setResetBusy] = useState(false);
  const normal = new URL(window.location.href); normal.searchParams.delete("window"); normal.hash = "";
  return <main className="startup-attention" aria-label="起動診断"><ToastHost />
    <ResidencyNotifications scope="startup"/>
    <p className="startup-attention-lead">{f ? "保存先の検証に失敗したため、起動を中止しました。" : report?.phase === "ready" ? "保存先の起動確認が完了しました。" : "起動結果を確認できません。"}</p>
    <ErrorNotice scope="startup" type="desktop-connection" active={!isTauri()}>デスクトップ接続がありません。起動結果を確認できません。</ErrorNotice>
    {(!report || report.phase === "starting") && <p role="status">起動結果を確認できるまで通常操作は利用できません。</p>}
    {f && <section className="startup-attention-cause" aria-label="起動を続けられない理由">
      <h2>原因{f.messageTruncated ? "（一部のみ表示）" : ""}</h2>
      <pre aria-label="保持した起動診断">{displayNativePathsInText(f.message || "詳細を取得できませんでした。")}</pre>
    </section>}
    {report?.phase === "blocked" && <StartupReset availability={report.reset} disabled={busy || quitting} onBusy={setResetBusy}>
      {(restore, reset) => <div className="startup-attention-actions">
        <button type="button" disabled={busy || quitting || resetBusy || retryUncertain} onClick={() => void retry()}>起動をやり直す</button>
        {restore}{reset}
        <button type="button" className="secondary" aria-label="このアプリを終了" disabled={busy || quitting || resetBusy} onClick={() => void quit()}><span className="startup-exit-prefix">このアプリを</span>終了</button>
      </div>}
    </StartupReset>}
    {report?.phase === "ready" && <p role="status">保存先の起動確認は完了しています。<a href={normal.href}>通常画面を開く</a></p>}
    <ErrorNotice scope="startup-attention" type="StartupAttention-operation" active={!!(error)}>{error}</ErrorNotice>
  </main>;
}
