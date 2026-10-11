import { IconButton } from "./action-icon";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";
import type { DataOperation } from "./data-operation";
import { assertPartialRecordingResults, checkedPartialRecordingResult, type PartialRecordingResultItem, type PartialRecordingResults } from "./partial-recording-state";
import { PartialRecordingSummary } from "./partial-recording-summary";

// Read-only history: no recording, analysis, cancellation or recovery command.
export function PartialRecordingResultsPanel({ operation, locked, onProtectionChange }: {
  operation: DataOperation; locked: boolean; onProtectionChange: (active: boolean) => void;
}) {
  const latest = useRef({ operation, locked }); latest.current = { operation, locked };
  const [extra, setExtra] = useState<PartialRecordingResults | null>(null);
  const [pageBusy, setPageBusy] = useState(false), [reading, setReading] = useState(false);
  const [selected, setSelected] = useState<PartialRecordingResultItem | null>(null), [error, setError] = useState("");
  const pageEpoch = useRef(0), readEpoch = useRef(0), running = useRef(false);
  const opener = useRef<HTMLElement | null>(null);
  const identity = JSON.stringify(operation.partialRecordingResults);
  const identityRef = useRef(identity); identityRef.current = identity;
  useEffect(() => { pageEpoch.current++; setExtra(null); setPageBusy(false); }, [identity]);
  useEffect(() => () => { pageEpoch.current++; readEpoch.current++; }, []);
  useEffect(() => { onProtectionChange(reading || !!selected); return () => onProtectionChange(false); }, [reading, selected, onProtectionChange]);
  const read = async (item: PartialRecordingResultItem) => {
    if (running.current || latest.current.locked || !latest.current.operation.connected) return;
    const epoch = ++readEpoch.current, expectedIdentity = identityRef.current;
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    running.current = true; setReading(true); setError("");
    try {
      const value = await invoke("get_partial_recording_result", { sessionId: item.summary.sessionId, reviewHash: item.summary.reviewHash });
      const summary = checkedPartialRecordingResult(value, item.summary);
      if (epoch !== readEpoch.current) return;
      if (!latest.current.operation.connected || identityRef.current !== expectedIdentity) throw Error("保存済み結果の一覧が変更されました。再読込して確認してください。");
      setSelected({ title: item.title, summary });
    } catch (cause) { if (epoch === readEpoch.current) setError(`保存済み結果を照合できません。再救出は実行していません: ${String(cause)}`); }
    finally { if (epoch === readEpoch.current) { running.current = false; setReading(false); } }
  };
  const loadMore = async () => {
    const after = extra ? extra.next : latest.current.operation.partialRecordingResults?.next;
    if (!after || pageBusy || latest.current.locked || !latest.current.operation.connected) return;
    const epoch = ++pageEpoch.current; setPageBusy(true); setError("");
    try {
      const page = await invoke<PartialRecordingResults>("get_partial_recording_results", { after });
      assertPartialRecordingResults(page, after);
      if (epoch === pageEpoch.current) {
        const items = [...(latest.current.operation.partialRecordingResults?.items ?? []), ...(extra?.items ?? []), ...page.items];
        if (new Set(items.map(i => i.summary.meetingId)).size !== items.length) throw Error("保存済み結果の会議IDが重複しています。再読込してください。");
        setExtra({ items: [...(extra?.items ?? []), ...page.items], next: page.next });
      }
    } catch (cause) { if (epoch === pageEpoch.current) setError(String(cause)); }
    finally { if (epoch === pageEpoch.current) setPageBusy(false); }
  };
  const items = [...(operation.partialRecordingResults?.items ?? []), ...(extra?.items ?? [])];
  const next = extra ? extra.next : operation.partialRecordingResults?.next;
  if (!items.length && !selected && !reading && !error) return null;
  return <>
    <section className="analysis-card recovery-panel" aria-label="保存済み部分救出の結果">
      <h2 id="partial-results-heading" tabIndex={-1}>保存済み部分救出の結果</h2>
      <p role="status">部分救出として保存された録音があります。取得終了・末尾損失は不明です。</p>
      <p>保存時の記録を読み取り専用で確認できます。録音・解析を自動再開せず、正常終了とは扱いません。音声削除後・復元後も記録を表示します。</p>
      {!operation.connected && <p role="alert">一覧の最新状態を確認できません。再読込するまで結果の確認を停止しています。</p>}
      <ul>{items.map(item => <li key={item.summary.sessionId}><h3>{item.title}</h3><p>会議ID {item.summary.meetingId}</p>
        <button type="button" className="small-secondary" disabled={locked || reading || !!selected || !operation.connected} onClick={() => void read(item)}>保存時の確定範囲を確認</button></li>)}</ul>
      {next && <IconButton label="次の保存済み結果を表示" icon="next" type="button" className="small-secondary" disabled={locked || pageBusy || !operation.connected} onClick={() => void loadMore()} />}
      {reading && <p role="status">保存済み結果を照合しています。</p>}
      {error && <p role="alert">{error}</p>}
    </section>
    <CommonModal open={!!selected} onDismiss={() => setSelected(null)} dismissLabel="閉じる" dismissText="保存済み結果を閉じる"
      restoreTarget={() => opener.current?.isConnected && !opener.current.matches(":disabled") ? opener.current : document.getElementById("partial-results-heading")}
      className="import-dialog data-dialog" aria-labelledby="partial-result-title">
      <h2 id="partial-result-title" tabIndex={-1}>保存済み部分救出の記録</h2>
      {selected && <><p>{selected.title}<br />会議ID {selected.summary.meetingId}<br />録音ID {selected.summary.sessionId}</p><PartialRecordingSummary summary={selected.summary} saved /></>}
    </CommonModal>
  </>;
}
