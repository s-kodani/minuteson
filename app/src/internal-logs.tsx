import { useEffect, useId, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { copyLogText, logCategories, logElapsed, logStates, mergeLogEntries, type InternalLogEntry, type InternalLogFilter, type InternalLogPage } from "./internal-logs-state";
import { IconButton } from "./action-icon";
import "./internal-logs.css";

type Mode = "initial" | "newer" | "older";
const levels: Record<string, string> = { info: "情報", warn: "警告", error: "エラー", raw: "生ログ" };
const dateTime = (value: number) => new Date(value).toLocaleString("ja-JP", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

export function InternalLogs() {
  const [filtersOpen, setFiltersOpen] = useState(false);
  const filtersId = useId();
  const [kind, setKind] = useState<"process" | "engine">("process");
  const [category, setCategory] = useState(""), [level, setLevel] = useState(""), [relatedId, setRelatedId] = useState(""), [text, setText] = useState("");
  const [from, setFrom] = useState(""), [to, setTo] = useState("");
  const [entries, setEntries] = useState<InternalLogEntry[]>([]), [selected, setSelected] = useState<number | null>(null);
  const [paused, setPaused] = useState(false), [following, setFollowing] = useState(true);
  const [loading, setLoading] = useState(true), [error, setError] = useState(""), [copyStatus, setCopyStatus] = useState("");
  const [pageStatus, setPageStatus] = useState<InternalLogPage["status"] | null>(null);
  const [hasOlder, setHasOlder] = useState(false), [gap, setGap] = useState(false), [trimmed, setTrimmed] = useState(false);
  const pausedRef = useRef(paused), followingRef = useRef(following), scroll = useRef<HTMLDivElement>(null);
  const detailPanel = useRef<HTMLElement>(null), detailTrigger = useRef<HTMLButtonElement | null>(null);
  useEffect(() => { if (selected !== null && detailPanel.current) { detailPanel.current.focus({ preventScroll: true }); detailPanel.current.scrollIntoView({ block: "nearest" }); } }, [selected]);
  pausedRef.current = paused; followingRef.current = following;
  const request = useRef<(mode: Mode, automatic?: boolean) => Promise<void>>(async () => {});
  const filter: InternalLogFilter = { kind, ...(category && { category }), ...(kind === "process" && level && { level }), ...(relatedId && { relatedId }), ...(text && { text }), ...(from && { from: new Date(from).getTime() }), ...(to && { to: new Date(to).getTime() }) };
  const filterKey = JSON.stringify(filter);
  const [activeFilter, setActiveFilter] = useState(filterKey);
  useEffect(() => { const timer = window.setTimeout(() => setActiveFilter(filterKey), 250); return () => window.clearTimeout(timer); }, [filterKey]);
  const invalidDate = !!from && !!to && new Date(from).getTime() > new Date(to).getTime();

  useEffect(() => {
    let alive = true, busy = false, initialized = false, older = 0, newer = 0;
    let buffer: InternalLogEntry[] = [];
    const queryFilter = JSON.parse(activeFilter) as InternalLogFilter;
    setEntries([]); setSelected(null); setPageStatus(null); setGap(false); setTrimmed(false); setHasOlder(false); setError(""); setCopyStatus("");
    const invalid = queryFilter.from !== undefined && queryFilter.to !== undefined && queryFilter.from > queryFilter.to;
    const fetch = async (mode: Mode, automatic = false) => {
      if (!alive || busy || invalid || automatic && (pausedRef.current || document.hidden)) return;
      if (!isTauri()) { setError("デスクトップ接続なし。内部ログはデスクトップアプリで確認できます。"); setLoading(false); return; }
      busy = true; if (!automatic) setLoading(true);
      try {
        const page = await invoke<InternalLogPage>("get_internal_logs", { query: { filter: queryFilter, limit: 200, ...(mode === "older" ? { before: older } : mode === "newer" && initialized ? { after: newer } : {}) } });
        if (!alive || automatic && pausedRef.current) return;
        setError(""); setPageStatus(page.status); setGap(value => value || page.expired);
        if (mode === "initial" || !initialized) {
          buffer = page.entries; setEntries(buffer); older = page.olderCursor; newer = page.newerCursor; setHasOlder(page.hasMore); initialized = true;
        } else {
          const result = mergeLogEntries(buffer, page.entries, mode === "older" ? "older" : "newer");
          buffer = result.entries; setEntries(buffer);
          if (result.trimmed) setTrimmed(true);
          if (mode === "older") { older = page.olderCursor; setHasOlder(page.hasMore); }
          else {
            newer = page.newerCursor;
            if (result.trimmed) { older = buffer[0]?.id ?? older; setHasOlder(true); }
          }
        }
        if (followingRef.current && mode !== "older") requestAnimationFrame(() => { if (alive && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight; });
      } catch (cause) { if (alive) setError(`ログを取得できませんでした。${String(cause)}`); }
      finally { busy = false; if (alive && !automatic) setLoading(false); }
    };
    request.current = fetch;
    if (invalid) { setLoading(false); }
    else void fetch("initial");
    const timer = window.setInterval(() => { void fetch(initialized ? "newer" : "initial", true); }, 1000);
    return () => { alive = false; window.clearInterval(timer); };
  }, [activeFilter]);

  const detail = entries.find(entry => entry.id === selected);
  const copy = async () => {
    try { await navigator.clipboard.writeText(copyLogText(entries)); setCopyStatus(`${entries.length}件のログをコピーしました。`); }
    catch { setCopyStatus("コピーできませんでした。もう一度お試しください。"); }
  };
  return <section className="internal-logs" aria-label="内部ログ">
    <div className="page-heading"><h1>内部ログ</h1><p className="lead">処理の経過とエンジンの出力を確認できます。</p></div>
    <p className="internal-logs-retention">最大30日・合計200 MiBまで保存し、上限を超えると古いログから削除します。会議の削除後もログは保持期限まで残ります。</p>
    <div className="internal-logs-controls">
    <div className="internal-logs-tabs" role="group" aria-label="ログの種類">
      <button type="button" className={kind === "process" ? "small-primary" : "small-secondary"} aria-pressed={kind === "process"} onClick={() => setKind("process")}>処理経過</button>
      <button type="button" className={kind === "engine" ? "small-primary" : "small-secondary"} aria-pressed={kind === "engine"} onClick={() => setKind("engine")}>エンジンログ</button>
    </div>
    <IconButton label="ログを絞り込み" icon="filter" aria-expanded={filtersOpen} aria-controls={filtersId} onClick={() => setFiltersOpen(open => !open)} />
    </div>
    {kind === "engine" && <p className="internal-logs-raw-note">エンジンの診断出力をそのまま表示します。会議本文やローカルパスが含まれる場合があります。stderrは必ずしもエラーではありません。</p>}
    <div id={filtersId} className="internal-logs-filters" role="region" aria-label="ログの絞り込み" hidden={!filtersOpen}>
      <label>開始日時<input type="datetime-local" value={from} onChange={event => setFrom(event.target.value)} /></label>
      <label>終了日時<input type="datetime-local" value={to} onChange={event => setTo(event.target.value)} /></label>
      <label>処理種別<select value={category} onChange={event => setCategory(event.target.value)}><option value="">すべて</option>{Object.entries(logCategories).map(([value, name]) => <option key={value} value={value}>{name}</option>)}</select></label>
      {kind === "process" && <label>重要度<select value={level} onChange={event => setLevel(event.target.value)}><option value="">すべて</option>{Object.entries(levels).filter(([value]) => value !== "raw").map(([value, name]) => <option key={value} value={value}>{name}</option>)}</select></label>}
      <label>会議／ジョブ／操作ID<input type="search" maxLength={512} value={relatedId} onChange={event => setRelatedId(event.target.value)} placeholder="IDの一部で検索" /></label>
      <label>文字列<input type="search" maxLength={512} value={text} onChange={event => setText(event.target.value)} placeholder="ログ本文を検索" /></label>
    </div>
    {invalidDate && <p role="alert">終了日時は開始日時以降を指定してください。</p>}
    <div className="internal-logs-toolbar">
      <button type="button" className="small-secondary" aria-pressed={paused} onClick={() => { pausedRef.current = !paused; setPaused(!paused); if (paused) void request.current("newer"); }}>{paused ? "自動更新を再開" : "自動更新を一時停止"}</button>
      <button type="button" className="small-secondary" disabled={loading || invalidDate || filterKey !== activeFilter} onClick={() => void request.current("newer")}>更新</button>
      <button type="button" className="small-secondary" disabled={!entries.length || filterKey !== activeFilter} onClick={() => void copy()}>表示中ログをコピー</button>
      <span className="internal-logs-count" role="status">{loading ? "取得中…" : `${entries.length} 件表示中`}{paused ? " · 自動更新停止中（収集は継続）" : " · 自動更新中"}</span>
    </div>
    {copyStatus && <p role="status">{copyStatus}</p>}
    {error && <p role="alert" className="internal-logs-notice">{error}{entries.length > 0 && " 前回取得したログを表示しています。"}</p>}
    {pageStatus?.storageError && <p role="alert" className="internal-logs-notice">ログの保存・収集で問題が発生しました：{pageStatus.storageError}</p>}
    {!!pageStatus?.droppedEntries && <p role="status" className="internal-logs-notice">この起動中に保存失敗・過負荷により{pageStatus.droppedEntries}件のログを記録できませんでした。</p>}
    {(gap || pageStatus?.historyTruncated) && <p role="status" className="internal-logs-notice">保持期間または容量上限により、古いログの一部は残っていません。</p>}
    {trimmed && <p role="status" className="internal-logs-notice">画面の上限1,000件を超えたため、一部のログを表示から外しました。「最新ログを表示」で読み直せます。</p>}
    <div className="internal-logs-history-controls">
      <button type="button" className="small-secondary" disabled={!hasOlder || loading || invalidDate || filterKey !== activeFilter} onClick={() => { setFollowing(false); void request.current("older"); }}>古いログを読み込む</button>
      <button type="button" className="small-secondary" disabled={loading || invalidDate || filterKey !== activeFilter} onClick={() => { setFollowing(true); followingRef.current = true; setTrimmed(false); void request.current("initial"); }}>最新ログを表示</button>
      {!following && <span>自動スクロール停止中</span>}
    </div>
    <div className="internal-logs-scroll" ref={scroll} tabIndex={0} aria-label="ログ一覧" onScroll={() => {
      const element = scroll.current;
      if (element) { const follow = element.scrollHeight - element.scrollTop - element.clientHeight < 40; followingRef.current = follow; setFollowing(follow); }
    }}>
      <table className="internal-logs-table"><thead><tr><th>日時</th><th>処理</th><th>{kind === "process" ? "状態／重要度" : "出力元"}</th><th>経過時間</th><th>メッセージ</th><th>詳細</th></tr></thead>
        <tbody>{entries.map(entry => <tr key={entry.id} className={selected === entry.id ? "internal-log-selected" : ""}>
          <td><time dateTime={new Date(entry.timestamp).toISOString()}>{dateTime(entry.timestamp)}</time></td>
          <td>{logCategories[entry.category] ?? entry.category}</td>
          <td>{entry.kind === "engine" ? entry.stream : <>{logStates[entry.state] ?? entry.state}<small className={`internal-log-level internal-log-level-${entry.level}`}>{levels[entry.level] ?? entry.level}</small></>}</td>
          <td>{logElapsed(entry.elapsedMs)}</td>
          <td className="internal-log-message"><pre>{entry.message.slice(0, 240)}</pre></td>
          <td><button type="button" className="small-secondary" aria-label={`${dateTime(entry.timestamp)}のログ詳細 ${entry.id}`} aria-expanded={selected === entry.id} onClick={event => { detailTrigger.current = event.currentTarget; setSelected(selected === entry.id ? null : entry.id); }}>詳細</button></td>
        </tr>)}</tbody>
      </table>
      {!entries.length && !loading && !error && !invalidDate && <p className="internal-logs-empty">{hasOlder ? "読み込んだ範囲に条件に一致するログはありません。古いログを読み込むと検索を続けられます。" : "条件に一致するログはありません。収集開始前のログは表示されません。"}</p>}
    </div>
    {detail && <section ref={detailPanel} tabIndex={-1} className="internal-log-detail" aria-label="ログ詳細"><h2>ログ詳細</h2><dl>
      <dt>日時</dt><dd>{dateTime(detail.timestamp)}</dd><dt>会議ID</dt><dd>{detail.meetingId ?? "—"}</dd><dt>ジョブID</dt><dd>{detail.jobId ?? "—"}</dd><dt>操作ID</dt><dd>{detail.operationId ?? "—"}</dd><dt>起動ID</dt><dd>{detail.sessionId}</dd>
      <dt>実行単位</dt><dd>{detail.source ?? "—"}</dd><dt>出力元</dt><dd>{detail.stream ?? "—"}</dd><dt>状態</dt><dd>{detail.state}</dd><dt>経過時間</dt><dd>{logElapsed(detail.elapsedMs)}</dd></dl><pre>{detail.message}</pre><button type="button" className="small-secondary" onClick={() => { setSelected(null); detailTrigger.current?.focus(); }}>詳細を閉じる</button></section>}
  </section>;
}
