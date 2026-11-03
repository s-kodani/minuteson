import { ErrorNotice, ToastNotice } from "./toast";
import { InfoPopover } from "./info-popover";
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { invoke, isTauri } from "./ipc.ts";
import { copyLogText, logCategories, logElapsed, logStates, mergeLogEntries, type InternalLogEntry, type InternalLogFilter, type InternalLogPage } from "./internal-logs-state";
import { IconButton } from "./action-icon";
import "./internal-logs.css";

type Mode = "initial" | "newer" | "older";
const levels: Record<string, string> = { info: "情報", warn: "警告", error: "エラー", raw: "生ログ" };
const dateTime = (value: number) => new Date(value).toLocaleString("ja-JP", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

export function InternalLogs() {
  const [filtersOpen, setFiltersOpen] = useState(false);
  const filtersId = useId();
  const tabsId = useId();
  const [kind, setKind] = useState<"process" | "engine">("process");
  const [category, setCategory] = useState(""), [level, setLevel] = useState(""), [relatedId, setRelatedId] = useState(""), [text, setText] = useState("");
  const [from, setFrom] = useState(""), [to, setTo] = useState("");
  const [entries, setEntries] = useState<InternalLogEntry[]>([]);
  const [paused, setPaused] = useState(false), [following, setFollowing] = useState(true);
  const [loading, setLoading] = useState(true), [error, setError] = useState(""), [copyStatus, setCopyStatus] = useState<{message:string;level:"success"|"error";eventId:string}|null>(null);
  const [pageStatus, setPageStatus] = useState<InternalLogPage["status"] | null>(null);
  const [hasOlder, setHasOlder] = useState(false), [gap, setGap] = useState(false), [trimmed, setTrimmed] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const pausedRef = useRef(paused), followingRef = useRef(following), scroll = useRef<HTMLDivElement>(null);
  const failedMode = useRef<Mode | null>(null);
  const topArmed = useRef(true);
  const scrollUpdate = useRef<{ bottom: true } | { id: string | undefined; offset: number; top: number } | null>(null);
  pausedRef.current = paused; followingRef.current = following;
  const request = useRef<(mode: Mode, automatic?: boolean) => Promise<void>>(async () => {});
  const filter: InternalLogFilter = { kind, ...(category && { category }), ...(kind === "process" && level && { level }), ...(relatedId && { relatedId }), ...(text && { text }), ...(from && { from: new Date(from).getTime() }), ...(to && { to: new Date(to).getTime() }) };
  const filterKey = JSON.stringify(filter);
  const currentFilter = useRef(filterKey);
  currentFilter.current = filterKey;
  const [activeFilter, setActiveFilter] = useState(filterKey);
  useEffect(() => { const timer = window.setTimeout(() => setActiveFilter(filterKey), 250); return () => window.clearTimeout(timer); }, [filterKey]);
  const invalidDate = !!from && !!to && new Date(from).getTime() > new Date(to).getTime();

  useEffect(() => {
    let alive = true, busy = false, initialized = false, older = 0, newer = 0, more = false;
    let queued: Mode | null = null, fetching: Mode | null = null;
    let buffer: InternalLogEntry[] = [];
    const queryFilter = JSON.parse(activeFilter) as InternalLogFilter;
    scrollUpdate.current = null; failedMode.current = null; topArmed.current = true;
    setEntries([]); setPageStatus(null); setGap(false); setTrimmed(false); setHasOlder(false); setError(""); setCopyStatus(null); setLoadingOlder(false);
    followingRef.current = true; setFollowing(true);
    const invalid = queryFilter.from !== undefined && queryFilter.to !== undefined && queryFilter.from > queryFilter.to;
    const current = () => alive && currentFilter.current === activeFilter;
    const pauseHistory = () => { pausedRef.current = true; setPaused(true); followingRef.current = false; setFollowing(false); };
    const preservePosition = () => {
      const element = scroll.current;
      if (!element) return;
      const top = element.getBoundingClientRect().top;
      const header = element.querySelector("thead")?.getBoundingClientRect().height ?? 0;
      const row = Array.from(element.querySelectorAll<HTMLElement>("tr[data-log-id]")).find(row => row.getBoundingClientRect().bottom > top + header);
      scrollUpdate.current = { id: row?.dataset.logId, offset: row ? row.getBoundingClientRect().top - top : 0, top: element.scrollTop };
    };
    const fetch = async (mode: Mode, automatic = false) => {
      if (!current() || invalid || automatic && (pausedRef.current || document.hidden)) return;
      if (mode === "older") {
        if (!initialized || !more) return;
        pauseHistory();
      }
      if (busy) { if (!automatic && fetching !== mode) queued = mode; return; }
      if (!isTauri()) { setError("デスクトップ接続なし。内部ログはデスクトップアプリで確認できます。"); setLoading(false); return; }
      busy = true; fetching = mode; if (!automatic) setLoading(true);
      if (mode === "older") setLoadingOlder(true);
      try {
        do {
          const before = older;
          const page = await invoke<InternalLogPage>("get_internal_logs", { query: { filter: queryFilter, limit: 200, ...(mode === "older" ? { before } : mode === "newer" && initialized ? { after: newer } : {}) } });
          if (!current() || automatic && pausedRef.current) return;
          if (mode === "older" && page.hasMore && page.olderCursor >= before) throw Error("古いログの読み込み位置が進みませんでした。更新ボタンで再試行してください。");
          setError(""); failedMode.current = null; setPageStatus(page.status); setGap(value => value || page.expired);
          if (mode === "older" || !followingRef.current) preservePosition();
          else scrollUpdate.current = { bottom: true };
          if (mode === "initial" || !initialized) {
            buffer = page.entries; older = page.olderCursor; newer = page.newerCursor; more = page.hasMore; initialized = true;
          } else {
            const result = mergeLogEntries(buffer, page.entries, mode === "older" ? "older" : "newer");
            buffer = result.entries;
            if (result.trimmed) setTrimmed(true);
            if (mode === "older") { older = page.olderCursor; more = page.hasMore; }
            else {
              newer = page.newerCursor;
              if (result.trimmed) { older = buffer[0]?.id ?? older; more = true; }
            }
          }
          setEntries([...buffer]); setHasOlder(more);
          // A bounded native scan can return an empty page with more history.
          // Continue by cursor, never by the number of matching rows.
          if (mode === "newer" || page.entries.length || !more) break;
          mode = "older"; automatic = false; fetching = mode; pauseHistory(); setLoading(true); setLoadingOlder(true);
        } while (current());
      } catch (cause) { if (current()) { failedMode.current = mode; setError(`ログを取得できませんでした。${String(cause)}`); queued = null; } }
      finally {
        busy = false; fetching = null;
        if (current()) {
          setLoading(false); setLoadingOlder(false);
          const next = queued; queued = null;
          if (next) void fetch(next);
        }
      }
    };
    request.current = fetch;
    if (invalid) { setLoading(false); }
    else void fetch("initial");
    const timer = window.setInterval(() => { void fetch(initialized ? "newer" : "initial", true); }, 1000);
    return () => { alive = false; window.clearInterval(timer); };
  }, [activeFilter]);

  useLayoutEffect(() => {
    const update = scrollUpdate.current, element = scroll.current;
    scrollUpdate.current = null;
    if (!update || !element || filterKey !== activeFilter) return;
    if ("bottom" in update) element.scrollTop = element.scrollHeight;
    else {
      const row = update.id ? element.querySelector<HTMLElement>(`tr[data-log-id="${update.id}"]`) : null;
      if (row) element.scrollTop += row.getBoundingClientRect().top - element.getBoundingClientRect().top - update.offset;
      else element.scrollTop = update.top;
    }
  }, [entries, activeFilter, filterKey]);
  useEffect(() => {
    const element = scroll.current;
    if (element && !loading && hasOlder && !error && !invalidDate && filterKey === activeFilter && element.scrollHeight <= element.clientHeight + 1) void request.current("older");
  }, [entries, loading, hasOlder, error, invalidDate, filterKey, activeFilter]);
  const latest = () => {
    pausedRef.current = false; setPaused(false); followingRef.current = true; setFollowing(true); setTrimmed(false);
    void request.current("initial");
  };
  const unavailable = loading || invalidDate || filterKey !== activeFilter;
  const copy = async () => {
    try { await navigator.clipboard.writeText(copyLogText(entries)); setCopyStatus({message:`${entries.length}件のログをコピーしました。`,level:"success",eventId:crypto.randomUUID()}); }
    catch { setCopyStatus({message:"コピーできませんでした。もう一度お試しください。",level:"error",eventId:crypto.randomUUID()}); }
  };
  return <section className="internal-logs" aria-label="内部ログ">
    <div className="page-heading heading-with-info"><h1>内部ログ</h1><InfoPopover label="内部ログ"><p>処理の経過とエンジンの出力を確認できます。</p><p>最大30日・合計200 MiBまで保存し、上限を超えると古いログから削除します。会議の削除後もログは保持期限まで残ります。</p></InfoPopover></div>
    <div className="internal-logs-controls">
    <div className="internal-logs-tabs" role="tablist" aria-label="ログの種類">
      {(["process", "engine"] as const).map(tab => <button key={tab} type="button" role="tab" id={`${tabsId}-${tab}`} aria-controls={`${tabsId}-panel`} aria-selected={kind === tab} tabIndex={kind === tab ? 0 : -1} onClick={() => setKind(tab)} onKeyDown={event => {
        if (event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return;
        const next = event.key === "Home" ? "process" : event.key === "End" ? "engine" : ["ArrowLeft", "ArrowRight"].includes(event.key) ? tab === "process" ? "engine" : "process" : null;
        if (next) { event.preventDefault(); document.getElementById(`${tabsId}-${next}`)?.focus(); }
      }}>{tab === "process" ? "処理経過" : "エンジンログ"}</button>)}
    </div>
    <IconButton label="ログを絞り込み" icon="filter" aria-expanded={filtersOpen} aria-controls={filtersId} onClick={() => setFiltersOpen(open => !open)} />
    </div>
    <div role="tabpanel" id={`${tabsId}-panel`} aria-labelledby={`${tabsId}-${kind}`}>
    {kind === "engine" && <div className="heading-with-info"><span>エンジンログ</span><InfoPopover label="エンジンログ">エンジンの診断出力をそのまま表示します。会議本文やローカルパスが含まれる場合があります。stderrは必ずしもエラーではありません。</InfoPopover></div>}
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
      <IconButton label={paused ? "自動更新を再開" : "自動更新を一時停止"} icon={paused ? "play" : "pause"} aria-pressed={paused} disabled={paused && unavailable} onClick={() => { if (paused) latest(); else { pausedRef.current = true; setPaused(true); } }} />
      <IconButton label="更新" icon="refresh" disabled={unavailable} onClick={() => void request.current(failedMode.current ?? "newer")} />
      <IconButton label="表示中ログをコピー" icon="copy" disabled={!entries.length || filterKey !== activeFilter} onClick={() => void copy()} />
      <IconButton label="最新ログへ移動" icon="bottom" disabled={unavailable} onClick={latest} />
      <span className="internal-logs-count" role="status">{loading ? "取得中…" : `${entries.length} 件表示中`}{paused ? " · 自動更新停止中（収集は継続）" : " · 自動更新中"}{!following && " · 自動スクロール停止中"}</span>
    </div>
    <ToastNotice scope="internal-logs" type="clipboard" message={copyStatus?.message} level={copyStatus?.level} eventId={copyStatus?.eventId} />
    <ErrorNotice scope="internal-logs" type="InternalLogs-operation" active={!!(error)}>{error}{entries.length > 0 && " 前回取得したログを表示しています。"}</ErrorNotice>
    {pageStatus?.storageError && <p role="status" className="internal-logs-notice">ログの保存・収集で問題が発生しました：{pageStatus.storageError}</p>}
    {!!pageStatus?.droppedEntries && <p role="status" className="internal-logs-notice">この起動中に保存失敗・過負荷により{pageStatus.droppedEntries}件のログを記録できませんでした。</p>}
    {(gap || pageStatus?.historyTruncated) && <p role="status" className="internal-logs-notice">保持期間または容量上限により、古いログの一部は残っていません。</p>}
    {trimmed && <p role="status" className="internal-logs-notice">画面の上限1,000件を超えたため、一部のログを表示から外しました。「最新ログへ移動」で読み直せます。</p>}
    <div className="internal-logs-scroll" ref={scroll} tabIndex={0} aria-label="ログ一覧" onScroll={() => {
      const element = scroll.current;
      if (element) {
        const pendingPosition = scrollUpdate.current;
        const follow = element.scrollHeight - element.scrollTop - element.clientHeight < 40;
        followingRef.current = follow; setFollowing(follow);
        if (element.scrollTop > 8) topArmed.current = true;
        else if (topArmed.current && (!pendingPosition || "bottom" in pendingPosition) && !unavailable && hasOlder && failedMode.current !== "older") {
          // A live reply can be waiting to follow the newest row. The user's
          // history scroll takes priority; older-page anchor restoration is
          // still protected by the pendingPosition check above.
          scrollUpdate.current = null;
          topArmed.current = false;
          void request.current("older");
        }
      }
    }}>
      <div className="internal-logs-history-status" role="status">{loadingOlder ? "古いログを読み込んでいます…" : failedMode.current === "older" ? "古いログを読み込めませんでした。更新ボタンで再試行できます。" : loading ? "ログを取得しています…" : hasOlder ? "上端までスクロールすると古いログを読み込みます。" : "これより古いログはありません。"}</div>
      <table className="internal-logs-table"><thead><tr><th>日時</th><th>処理</th><th>{kind === "process" ? "状態／重要度" : "出力元"}</th><th>経過時間</th><th>メッセージ</th><th>詳細</th></tr></thead>
        <tbody>{entries.map(entry => <tr key={entry.id} data-log-id={entry.id}>
          <td><time dateTime={new Date(entry.timestamp).toISOString()}>{dateTime(entry.timestamp)}</time></td>
          <td>{logCategories[entry.category] ?? entry.category}</td>
          <td>{entry.kind === "engine" ? entry.stream : <>{logStates[entry.state] ?? entry.state}<small className={`internal-log-level internal-log-level-${entry.level}`}>{levels[entry.level] ?? entry.level}</small></>}</td>
          <td>{logElapsed(entry.elapsedMs)}</td>
          <td className="internal-log-message"><pre>{entry.message.slice(0, 240)}</pre></td>
          <td><InfoPopover label="ログ詳細" triggerText="詳細" triggerLabel={`${dateTime(entry.timestamp)}のログ詳細 ${entry.id}`} popupClassName="internal-log-detail" maxWidth={680} openOnFocus lazy>
            <dl><dt>日時</dt><dd>{dateTime(entry.timestamp)}</dd><dt>会議ID</dt><dd>{entry.meetingId ?? "—"}</dd><dt>ジョブID</dt><dd>{entry.jobId ?? "—"}</dd><dt>操作ID</dt><dd>{entry.operationId ?? "—"}</dd><dt>起動ID</dt><dd>{entry.sessionId}</dd>
            <dt>実行単位</dt><dd>{entry.source ?? "—"}</dd><dt>出力元</dt><dd>{entry.stream ?? "—"}</dd><dt>状態</dt><dd>{entry.state}</dd><dt>経過時間</dt><dd>{logElapsed(entry.elapsedMs)}</dd></dl><pre>{entry.message}</pre>
          </InfoPopover></td>
        </tr>)}</tbody>
      </table>
      {!entries.length && !loading && !error && !invalidDate && <p className="internal-logs-empty">条件に一致するログはありません。収集開始前のログは表示されません。</p>}
    </div>
    </div>
  </section>;
}
