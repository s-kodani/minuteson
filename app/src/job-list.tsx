import { InfoPopover } from "./info-popover";
import { OperationProgressBar, useJobReadStatus, useProgressItems } from "./operation-progress";
import { operationName, operationStatus, orderedProgress } from "./operation-progress-state";

export function JobCount() {
  const count = useProgressItems().length;
  return count ? <span className="job-nav-count" aria-hidden="true">{count}</span> : null;
}

export function JobList({ locked, onOpenMeeting, onOpenModels }: { locked: boolean; onOpenMeeting: (id: string) => Promise<void>; onOpenModels: () => void }) {
  const items = orderedProgress(useProgressItems()), read = useJobReadStatus();
  return <section className="job-list" aria-label="バックグラウンドのジョブ一覧">
    <div className="page-heading heading-with-info"><h1>ジョブ一覧</h1><InfoPopover label="ジョブ一覧">処理中・待機中の作業を確認できます。</InfoPopover></div>
    {read.state === "loading" && <p role="status">ジョブの状態を確認しています。</p>}
    {read.state === "error" && <p role="status" className="job-read-error">ジョブの状態を取得できません。表示は前回取得した状態です。自動で再確認します。</p>}
    {!items.length && read.state === "ready" && <div className="job-list-empty"><h2>進行中のジョブはありません</h2><p>録音の保存・回復、モデルの準備、音声取り込み、文字起こし、話者分離、議事録生成がここに表示されます。</p></div>}
    {items.length > 0 && <><p className="job-list-count" role="status">{items.length} 件 · 処理中 {items.filter(item => ["running", "pausing"].includes(item.state)).length} · 待機中 {items.filter(item => item.state === "queued").length}</p>
      <ul className="job-list-rows">{items.map(item => <li key={item.id}>
        <div className="job-list-row-heading"><span className="job-kind">{operationName(item.kind)}</span><span className={`job-status job-status-${item.state}`}>{operationStatus(item.state)}</span></div>
        <OperationProgressBar progress={item} />
        {item.meetingId ? <button type="button" className="small-secondary" disabled={locked} onClick={() => void onOpenMeeting(item.meetingId!)}>会議を開く</button> : item.kind === "model" ? <button type="button" className="small-secondary" disabled={locked} onClick={onOpenModels}>モデル設定を開く</button> : null}
      </li>)}</ul>
    </>}
  </section>;
}
