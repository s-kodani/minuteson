import { InfoPopover } from "./info-popover";
import { OperationProgressBar, useJobReadStatus, useProgressItems } from "./operation-progress";
import { operationName, operationStatus, orderedProgress } from "./operation-progress-state";
import { dataJobProgress } from "./operation-progress-state";
import type { DataOperation } from "./data-operation";
import { invoke } from "./ipc";
import { useState } from "react";
import { JobHistory } from "./job-history";

export function JobCount() {
  const count = useProgressItems().length;
  return count ? <span className="job-nav-count" aria-hidden="true">{count}</span> : null;
}

export function JobList({ locked, onOpenMeeting, onOpenModels, data, onOpenData, onShowRestored, publicationReadStatus }: { locked: boolean; onOpenMeeting: (id: string) => Promise<void>; onOpenModels: () => void; data?: DataOperation; onOpenData?: () => void; onShowRestored?: (ids: string[]) => Promise<void>; publicationReadStatus?: string }) {
  const items = orderedProgress(useProgressItems()), read = useJobReadStatus();
  const runningCount = items.filter(item => item.state === "running" || item.state === "pausing").length;
  const queuedCount = items.filter(item => item.state === "queued").length;
  const [error, setError] = useState("");
  const [view, setView] = useState<"active"|"history">("active");
  const run = async (action: () => Promise<unknown>) => { setError(""); try { await action(); } catch (cause) { setError(String(cause)); } };
  const ended = data?.jobs.filter(job => job.snapshot.phase === "attention") ?? [];
  return <section className="job-list" aria-label="バックグラウンドのジョブ一覧">
    <div className="page-heading heading-with-info"><h1>ジョブ一覧</h1><InfoPopover label="ジョブ一覧">処理中・待機中の作業と、過去90日間の処理結果を確認できます。履歴はアプリを終了しても保持します。</InfoPopover></div>
    <div className="job-list-tabs" role="tablist" aria-label="ジョブの表示">{(["active", "history"] as const).map(tab => <button type="button" key={tab} role="tab" id={`job-${tab}-tab`} aria-controls={`job-${tab}-panel`} aria-selected={view === tab} tabIndex={view === tab ? 0 : -1} onClick={() => setView(tab)} onKeyDown={event => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const next = event.key === "Home" ? "active" : event.key === "End" ? "history" : view === "active" ? "history" : "active";
      setView(next);
      document.getElementById(`job-${next}-tab`)?.focus();
    }}>
      {tab === "active" ? <>進行中{items.length > 0 && <> <span className="job-tab-count">{items.length}</span></>}</> : "履歴"}
    </button>)}</div>
    <div id="job-history-panel" role="tabpanel" aria-labelledby="job-history-tab" hidden={view!=="history"}>{view==="history"&&<JobHistory locked={locked} onOpenMeeting={onOpenMeeting} onOpenModels={onOpenModels} onOpenData={onOpenData} onShowRestored={onShowRestored}/>}</div>
    <div id="job-active-panel" role="tabpanel" aria-labelledby="job-active-tab" hidden={view!=="active"}>
    {read.state === "loading" && <p role="status">ジョブの状態を確認しています。</p>}
    {read.state === "error" && <p role="status">ジョブの状態を確認中です。表示は前回取得した状態です。</p>}
    {publicationReadStatus && <p role="status">{publicationReadStatus}</p>}
    {data?.jobsError && <p role="status">バックアップ・復元の状態を再確認しています。表示は前回取得した状態です。</p>}
    {error && <p role="alert">{error}</p>}
    {!items.length && read.state === "ready" && <div className="job-list-empty"><h2>進行中のジョブはありません</h2><p>録音の保存・回復、モデルの準備、音声取り込み、文字起こし、話者分離、議事録生成、バックアップ、復元がここに表示されます。</p></div>}
    {items.length > 0 && <><p className="job-list-count" role="status">
      <span className="job-chip job-chip-total">合計 <strong>{items.length}件</strong></span>{" "}
      <span className="job-chip job-chip-running">処理中 <strong>{runningCount}件</strong></span>{" "}
      <span className="job-chip job-chip-queued">待機中 <strong>{queuedCount}件</strong></span>
    </p>
      <ul className="job-list-rows">{items.map(item => <li key={item.id}>
        <div className="job-list-row-heading"><span className="job-kind">{operationName(item.kind)}</span><span className={`job-chip job-status job-chip-${item.state}`}>{operationStatus(item.state)}</span></div>
        <OperationProgressBar progress={item} />
        {["backup", "restore"].includes(item.kind) && (() => {
          const job = data?.jobs.find(job => `data:${job.snapshot.token}` === item.id);
          return job ? <div className="analysis-actions">
            {job.snapshot.busy ? <button type="button" className="small-secondary" disabled={job.snapshot.phase === "cancelling"} onClick={() => void run(async () => { await invoke("cancel_data_operation", { token: job.snapshot.token! }); await data?.refresh(); })}>処理を取り消す</button> : <button type="button" className="small-secondary" onClick={onOpenData}>確認内容を開く</button>}
          </div> : null;
        })()}
        {item.meetingId ? <button type="button" className="small-secondary" data-screen-navigation disabled={locked} onClick={() => void onOpenMeeting(item.meetingId!)}>会議を開く</button> : item.kind === "model" ? <button type="button" className="small-secondary" data-screen-navigation disabled={locked} onClick={onOpenModels}>モデル設定を開く</button> : null}
      </li>)}</ul>
    </>}
    {ended.length > 0 && <><h2>バックアップ・復元の結果</h2><ul className="job-list-rows" aria-label="バックアップと復元の結果">{[...ended].reverse().map(job => {
      const s = job.snapshot, receipt = s.restoreReceipt;
      return <li key={s.token!}>
        <div className="job-list-row-heading"><span className="job-kind">{operationName(s.kind as "backup" | "restore")}</span><span>{({ completed:"完了", failed:"失敗", cancelled:"取消済み", attention:"結果の確認が必要" } as Record<string,string>)[s.phase]}</span></div>
        <OperationProgressBar progress={dataJobProgress(job)} />
        {receipt && <p>会議 {receipt.meetingIds.length} 件を追加しました。{receipt.registry && `人物 ${receipt.registry.people} 人・声 ${receipt.registry.samples} 件・紐付き ${receipt.registry.links} 件。`}</p>}
        <div className="analysis-actions">
          {receipt && <button type="button" className="small-secondary" data-screen-navigation disabled={locked} onClick={() => void run(() => onShowRestored?.(receipt.meetingIds) ?? Promise.resolve())}>復元した会議を開く</button>}
          {s.phase === "attention" && data?.snapshot?.token === s.token ? <button type="button" className="small-secondary" onClick={onOpenData}>結果を確認する</button> : <button type="button" className="small-secondary" onClick={() => void run(() => data!.dismissJob(s.token!))}>結果を閉じる</button>}
        </div>
      </li>;
    })}</ul></>}
    </div>
  </section>;
}
