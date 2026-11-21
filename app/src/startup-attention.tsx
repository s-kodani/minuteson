import { ErrorNotice, ToastHost } from "./toast";
import { invoke, isTauri } from "./ipc.ts";
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { acceptStartupReport, type StartupReport } from "./startup-attention-state";
import { CommonModal } from "./common-modal";
import { usePlatform } from "./platform";
import { StartupReset } from "./startup-reset";
import { SalvageEntry } from "./salvage";
import { ResidencyNotifications } from "./residency-notifications";
import { StartupLoading } from "./startup-loading";
import { useAppName } from "./window-environment";
import { displayNativePathsInText } from "./native-path";
import { ActionIcon } from "./action-icon";

/** No theme, storage, recording or job provider is mounted in this route. */
function useStartupReport() {
  const appName = useAppName();
  const platform = usePlatform();
  const [report, setReport] = useState<StartupReport | null>(null), [busy, setBusy] = useState(false), [quitting, setQuitting] = useState(false);
  const [problem, setProblem] = useState<{ source: "report" | "action"; message: string } | null>(null);
  const error = problem?.message ?? "";
  const [retryDelay, setRetryDelay] = useState<number | null>(null);
  const [retryUncertain, setRetryUncertain] = useState(false);
  const current = useRef<StartupReport | null>(null), active = useRef(true), reading = useRef(false), closing = useRef(false);
  const readFailures = useRef(0);
  const read = async (manual = false) => {
    if (!isTauri() || reading.current || closing.current) return;
    if (manual) readFailures.current = 0;
    reading.current = true; setBusy(true); setRetryDelay(null); setProblem(null);
    try {
      let value: unknown;
      try { value = await invoke("get_startup_report"); }
      catch (cause) {
        if (!active.current) return;
        const retries = readFailures.current++;
        if (retries >= 3) throw cause;
        setRetryDelay(250 * 2 ** retries);
        return;
      }
      if (!active.current) return;
      const next = acceptStartupReport(current.current, value);
      if (next.platform && platform !== "other" && next.platform !== platform) throw Error("起動結果のOS情報が一致しません。");
      readFailures.current = 0; current.current = next; setReport(next);
    } catch (cause) {
      if (active.current) {
        console.error("get_startup_report", cause);
        setProblem({ source: "report", message: "アプリを開ける状態か確認できませんでした。" });
      }
    }
    finally { reading.current = false; if (active.current) setBusy(false); }
  };
  useEffect(() => { active.current = true; void read(); return () => { active.current = false; }; }, []);
  useEffect(() => {
    if (busy || error || (retryDelay === null && report?.phase !== "starting")) return;
    const timer = window.setTimeout(() => void read(), retryDelay ?? 200);
    return () => window.clearTimeout(timer);
  }, [report, busy, error, retryDelay]);
  const retry = async () => {
    if (closing.current || retryUncertain || current.current?.phase !== "blocked") return;
    closing.current = true; setQuitting(true); setProblem(null);
    try { await invoke("retry_startup"); }
    catch (cause) {
      if (active.current) {
        console.error("retry_startup", cause);
        setRetryUncertain(true);
        setProblem({ source: "action", message: `再起動できたか確認できませんでした。${appName}を終了してから起動し直してください。` });
      }
    }
    finally { closing.current = false; if (active.current) setQuitting(false); }
  };
  const quit = async () => {
    if (closing.current || current.current?.phase !== "blocked") return;
    closing.current = true; setQuitting(true); setProblem(null);
    try { await invoke("quit_blocked_startup"); }
    catch (cause) {
      if (active.current) {
        console.error("quit_blocked_startup", cause);
        setProblem({ source: "action", message: "アプリを終了できたか確認できませんでした。ウィンドウの閉じるボタンでも終了できます。" });
      }
    }
    finally { closing.current = false; if (active.current) setQuitting(false); }
  };
  return { report, error, problem, busy, quitting, retryUncertain, read, retry, quit };
}
export function StartupAttentionWindow() { return <StartupAttention state={useStartupReport()} />; }
const StartupRecoveryPending = createContext(false);
export const useStartupRecoveryPending = () => useContext(StartupRecoveryPending);
export function StartupGate({ children }: { children: ReactNode }) {
  const state = useStartupReport();
  const [recoveryDismissed, setRecoveryDismissed] = useState(false);
  // Browser-only preview is not a native admission proof and cannot invoke
  // desktop operations. A desktop reader failure never mounts normal UI.
  return !isTauri() || state.report?.phase === "ready" ? <>
    <StartupRecoveryPending value={!!state.report?.recovery && !recoveryDismissed}>{children}</StartupRecoveryPending>
    {state.report?.recovery && <CommonModal portal open={!recoveryDismissed} onDismiss={() => setRecoveryDismissed(true)} dismissLabel="閉じる" dismissText="復旧結果を閉じる" restoreTarget={() => document.querySelector<HTMLElement>(".sidebar button:not(:disabled)")} aria-labelledby="startup-recovery-title" aria-describedby="startup-recovery-description">
      <h2 id="startup-recovery-title">{state.report.recovery.reviewRequired ? "中断操作の再確認が必要です" : "保存先の情報を修復しました"}</h2>
      {state.report.recovery.reviewRequired ? <>
        <p id="startup-recovery-description">中断操作の再確認が必要なため、会議の閲覧のみ利用できます。元の記録とファイルは保持しています。</p>
        <ul>{state.report.recovery.reasons.map(reason => <li key={reason}>{displayNativePathsInText(reason)}</li>)}</ul>
      </> : <p id="startup-recovery-description">会議データを確認し、アプリが使う保存先を更新しました。通常どおり利用できます。</p>}
      <button type="button" onClick={() => setRecoveryDismissed(true)}>閉じる</button>
    </CommonModal>}
  </> : !state.error && (!state.report || state.report.phase === "starting")
    ? <><ToastHost /><ResidencyNotifications scope="startup"/><StartupLoading /></>
    : <StartupAttention state={state} />;
}
function StartupAttention({ state }: { state: ReturnType<typeof useStartupReport> }) {
  const { report, error, problem, busy, quitting, retryUncertain, read, retry, quit } = state, f = report?.failure;
  const [resetBusy, setResetBusy] = useState(false);
  const partialSalvageMode = f?.stage === "partial_salvage";
  const salvageAvailable = f?.salvageAvailable === true && !error;
  const dedicatedSalvage = partialSalvageMode && salvageAvailable;
  const normal = new URL(window.location.href); normal.searchParams.delete("window"); normal.hash = "";
  const actions = (reset?: ReactNode) => <div className="startup-attention-actions">
    <button type="button" disabled={busy || quitting || resetBusy || retryUncertain} onClick={() => void retry()}>起動をやり直す</button>
    {salvageAvailable && <SalvageEntry mode="blocked" disabled={busy || quitting || resetBusy} />}
    {reset}
    <button type="button" className="secondary" aria-label="このアプリを終了" disabled={busy || quitting || resetBusy} onClick={() => void quit()}><span className="startup-exit-prefix">このアプリを</span>終了</button>
  </div>;
  return <main className="startup-attention" aria-label="起動時の案内"><ToastHost />
    <ResidencyNotifications scope="startup"/>
    {f && !dedicatedSalvage && <p className="startup-attention-lead">保存先に問題があり、アプリを開けませんでした。</p>}
    <ErrorNotice scope="startup" type="desktop-connection" active={!isTauri()}>デスクトップアプリに接続できませんでした。</ErrorNotice>
    {problem?.source === "report" && <p role="alert" className="startup-attention-error">{error}</p>}
    {isTauri() && error && <button type="button" className="secondary startup-status-reload" disabled={busy || quitting || resetBusy} onClick={() => void read(true)}><ActionIcon name="refresh" /><span>起動できるか再確認</span></button>}
    {f && !dedicatedSalvage && <section className="startup-attention-cause" aria-label="起動を続けられない理由">
      <h2>原因{f.messageTruncated ? "（一部のみ表示）" : ""}</h2>
      <pre aria-label="アプリを開けなかった原因">{displayNativePathsInText(f.message || "詳しい原因は分かりませんでした。")}</pre>
      {f.salvageAvailable !== true && <p role="status">原因を解消してから、起動をやり直してください。</p>}
    </section>}
    {dedicatedSalvage && <SalvageEntry mode="blocked" dedicated />}
    {report?.phase === "blocked" && !dedicatedSalvage && (partialSalvageMode ? actions() : <StartupReset availability={report.reset} disabled={busy || quitting} onBusy={setResetBusy}>
      {actions}
    </StartupReset>)}
    {report?.phase === "ready" && <p role="status"><a href={normal.href}>通常画面を開く</a></p>}
    <ErrorNotice scope="startup-attention" type="StartupAttention-operation" active={problem?.source === "action"}>{problem?.source === "action" ? error : ""}</ErrorNotice>
  </main>;
}
