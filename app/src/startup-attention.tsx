import { IconButton } from "./action-icon";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { acceptStartupReport, startupStageLabels, type StartupReport } from "./startup-attention-state";
import { CommonModal } from "./common-modal";
import { PlatformContext } from "./platform";
import { StartupInstanceRecovery } from "./startup-instance";

/** No theme, storage, recording or job provider is mounted in this route. */
function useStartupReport() {
  const [report, setReport] = useState<StartupReport | null>(null), [error, setError] = useState(""), [busy, setBusy] = useState(false), [quitting, setQuitting] = useState(false);
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
    if (report?.platform !== "windows" && report?.platform !== "macos") return;
    let disposed=false, stop:(()=>void)|undefined;
    void listen<string>("residency-error",event=>{if(!disposed)setError(event.payload);}).then(unlisten=>{if(disposed)unlisten();else stop=unlisten;}).catch(()=>{});
    return()=>{disposed=true;stop?.();};
  },[report?.platform]);
  useEffect(() => {
    if (report?.platform !== "windows") return;
    let disposed = false, stop: (() => void) | undefined;
    void listen<{ requestId: string }>("startup-activate-requested", event => {
      if (!disposed) void invoke("acknowledge_startup_activation", { requestId: event.payload.requestId }).catch(() => {});
    }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; }).catch(() => {});
    return () => { disposed = true; stop?.(); };
  }, [report?.platform]);
  useEffect(() => {
    // Only reread the in-memory startup result while native setup is pending.
    // A confirmed failure or an invalid/unreadable reply still needs review.
    if (report?.phase !== "starting" || busy || error) return;
    const timer = window.setTimeout(() => void read(), 200);
    return () => window.clearTimeout(timer);
  }, [report, busy, error]);
  const quit = async () => {
    if (closing.current || current.current?.phase !== "blocked") return;
    closing.current = true; setQuitting(true); setError("");
    try { await invoke("quit_blocked_startup"); }
    catch (cause) { if (active.current) setError(`終了の受付を確認できません。保存先への操作は行っていません。OSの閉じる操作でも終了できます: ${String(cause)}`); }
    finally { closing.current = false; if (active.current) setQuitting(false); }
  };
  return { report, error, busy, quitting, read, quit };
}
export function StartupAttentionWindow() { return <StartupAttention state={useStartupReport()} />; }
export function StartupGate({ children }: { children: ReactNode }) {
  const state = useStartupReport();
  const [recoveryDismissed, setRecoveryDismissed] = useState(false);
  // Browser-only preview is not a native admission proof and cannot invoke
  // desktop operations. A desktop reader failure never mounts normal UI.
  return !isTauri() || state.report?.phase === "ready" ? <>
    <PlatformContext.Provider value={state.report?.platform ?? "other"}>{children}</PlatformContext.Provider>
    {state.report?.recovery && <CommonModal portal open={!recoveryDismissed} onDismiss={() => setRecoveryDismissed(true)} dismissLabel="閉じる" dismissText="復旧結果を閉じる" restoreTarget={() => document.querySelector<HTMLElement>(".sidebar button:not(:disabled)")} aria-labelledby="startup-recovery-title" aria-describedby="startup-recovery-description">
      <h2 id="startup-recovery-title">{state.report.recovery.reviewRequired ? "中断操作の再確認が必要です" : "保存先の情報を修復しました"}</h2>
      {state.report.recovery.reviewRequired ? <>
        <p id="startup-recovery-description">中断操作の再確認が必要なため、会議の閲覧のみ利用できます。元の記録とファイルは保持しています。</p>
        <ul>{state.report.recovery.reasons.map(reason => <li key={reason}>{reason}</li>)}</ul>
      </> : <p id="startup-recovery-description">保存先とDBを検証し、現在の保存先情報へ更新しました。通常どおり利用できます。</p>}
      <button type="button" onClick={() => setRecoveryDismissed(true)}>閉じる</button>
    </CommonModal>}
  </> : !state.error && (!state.report || state.report.phase === "starting")
    ? <main className="startup-loading" aria-label="起動確認"><p role="status">MONを起動しています…</p></main>
    : <StartupAttention state={state} />;
}
function StartupAttention({ state }: { state: ReturnType<typeof useStartupReport> }) {
  const { report, error, busy, quitting, read, quit } = state, f = report?.failure;
  const normal = new URL(window.location.href); normal.searchParams.delete("window"); normal.hash = "";
  return <main className="startup-attention" aria-label="起動診断">
    <h1>{report?.platform === "windows" ? "MONの起動と復旧" : "保存先の起動確認"}</h1>
    <p>この画面では録音・取り込み・解析・編集・削除・保存先変更を開始しません。</p>
    {!isTauri() && <p role="alert">デスクトップ接続がありません。起動結果を確認できません。</p>}
    {(!report || report.phase === "starting") && <p role="status">起動結果を確認できるまで通常操作は利用できません。</p>}
    {report?.phase === "blocked" && report.platform === "windows" && <StartupInstanceRecovery />}
    {f && <section className="status-card" aria-label="起動を続けられない理由">
      <h2>保存先の検証に失敗したため、通常の起動を中止しました</h2>
      <p>停止した段階：{startupStageLabels[f.stage]}</p>
      <dl><dt>選択された会議保存先</dt><dd>{f.meetingRoot ?? "選択先は未確定です。エラー本文から推測しません。"}</dd><dt>起動設定の保存先</dt><dd>{f.configRoot ?? "保存先を取得できていません。"}</dd></dl>
      <h3>診断内容</h3><pre aria-label="保持した起動診断">{f.message || "詳細はありません。原因を推測しません。"}</pre>
      {f.messageTruncated && <p role="status">長い診断の先頭だけを表示しています。完全な診断ではありません。</p>}
      <h3>データを守るための確認</h3>
      <p>DB・台帳・起動設定・音声は保持しています。空の保存先への自動切替や、ロックファイルの削除は行いません。</p>
      <p>他のMONが同じ保存先を使用していないか、外付け保存先の接続、空き容量やアクセス権を確認してください。これらが原因だと断定しているわけではありません。解決できなければ、表示された段階・保存先・診断内容を確認担当者へ伝えてください。共有前に個人情報を確認してください。</p>
      <p>{report?.platform === "windows" ? "上の復旧操作から、既存画面の表示や起動の再試行ができます。" : "終了後、原因を確認してから明示的に再起動してください。"}再起動時は保存先を改めて検証します。保存済みのデータを巻き戻す操作ではありません。</p>
    </section>}
    {report?.phase === "ready" && <p role="status">保存先の起動確認は完了しています。<a href={normal.href}>通常画面を開く</a></p>}
    <div className="analysis-actions"><IconButton label="保持した診断だけを再読込" icon="refresh" type="button" className="secondary" disabled={!isTauri() || busy || quitting} onClick={() => void read()} />{report?.phase === "blocked" && <button type="button" disabled={busy || quitting} onClick={() => void quit()}>このアプリを終了</button>}</div>
    <p>再読込はメモリに保持した診断だけを読みます。保存先を再び開く、検査・回収する、自動再試行する操作ではありません。</p>
    {error && <p className="action-error" role="alert">{error}</p>}
  </main>;
}
