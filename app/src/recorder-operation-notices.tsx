import { IconButton } from "./action-icon";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { IssueNotice, ToastNotice } from "./toast";
import { emptyOperations, mergeOperations, operationNames, relayOperation, type OperationSnapshot, type OperationSource } from "./recorder-operation-state";

const bridge = { read: () => invoke("get_recorder_operation_notices"), publish: (source: OperationSource, expectedRevision: number, message: string | null) => invoke("publish_recorder_operation_notice", { source, expectedRevision, message }) };
const Context = createContext<{ snapshot: OperationSnapshot; error: string; refresh: () => Promise<void>; accept: (value: unknown) => void } | null>(null);
export function RecorderOperationProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState(emptyOperations), [error, setError] = useState("");
  const current = useRef(snapshot), read = useRef<() => Promise<void>>(async () => {});
  const accept = (value: unknown) => { const next = mergeOperations(current.current, value); current.current = next; setSnapshot(next); };
  const acceptRef = useRef(accept); acceptRef.current = accept;
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false, running: Promise<void> | null = null, again = false, stop: (() => void) | undefined;
    const refresh = (): Promise<void> => {
      if (disposed) return Promise.resolve();
      if (running) { again = true; return running; }
      running = (async () => {
        do { again = false; try { const value = await bridge.read(); if (!disposed) { acceptRef.current(value); setError(""); } } catch (cause) { if (!disposed) setError(String(cause)); } } while (again && !disposed);
      })().finally(() => { running = null; }); return running;
    };
    read.current = refresh; void refresh();
    const timer = window.setInterval(() => void refresh(), 1000);
    void listen("recorder-operation-notices-changed", () => void refresh()).then(value => { if (disposed) value(); else { stop = value; void refresh(); } }).catch(cause => { if (!disposed) setError(String(cause)); });
    return () => { disposed = true; read.current = async () => {}; clearInterval(timer); stop?.(); };
  }, []);
  return <Context.Provider value={{ snapshot, error, accept, refresh: () => read.current() }}>{children}</Context.Provider>;
}
export function useRecorderOperations() {
  const state = useContext(Context); if (!state) throw Error("録音操作の通知接続がありません"); return state;
}
export function useRecorderOperationPublisher(source: OperationSource, message: string, settled: boolean, resolved: boolean) {
  const state = useRecorderOperations(), accept = useRef(state.accept); accept.current = state.accept;
  const queue = useRef(Promise.resolve()), [error, setError] = useState("");
  useEffect(() => {
    // A remounted blank UI is not evidence that an earlier failure resolved.
    if (!isTauri() || !settled || (!message && !resolved)) return;
    let active = true;
    queue.current = queue.current.catch(() => {}).then(async () => {
      if (!active) return;
      try { const next = await relayOperation(bridge, source, message || null, () => active); if (active && next) { accept.current(next); setError(""); } }
      catch (cause) { if (active) setError(String(cause)); }
    });
    return () => { active = false; };
  }, [source, message, settled, resolved]);
  return error;
}
export function RecorderOperationDetails() {
  const state = useRecorderOperations(), causes = state.snapshot.entries.filter(e => e.message);
  return <section className="recorder-operation-details" aria-label="録音パネルの操作の要対応">
    {state.error && <p role="alert">通知の読込は要確認：{state.error}</p>}
    {causes.length > 0 && <><h3>操作の要対応：{causes.length}件</h3><p>通知を閉じても原因は消えません。メインを開く・停止確認は録音パネルの元の操作から再確認してください。ここから録音の停止やデータ変更は行いません。</p>
      <ul>{causes.map(e => <li key={e.source}><h4>{operationNames[e.source]}</h4><p>{e.message}</p>{e.dismissed && <small>通知は閉じました。原因は確認待ちです。</small>}</li>)}</ul></>}
    <IconButton label="通知の状態だけを再読込" icon="refresh" type="button" className="small-secondary" disabled={!isTauri()} onClick={() => void state.refresh()} />
  </section>;
}
export function RecorderOperationNotifications() {
  const state = useRecorderOperations(), [error, setError] = useState("");
  const dismiss = async (source: OperationSource, expectedRevision: number, eventId: string) => {
    try { state.accept(await invoke("dismiss_recorder_operation_notice", { source, expectedRevision, eventId })); setError(""); }
    catch (cause) { setError(String(cause)); await state.refresh(); }
  };
  return <>
    {state.snapshot.entries.map(e => <ToastNotice key={e.source} scope="recorder-operation" type={e.source} message={!e.dismissed && e.message ? `${operationNames[e.source]}：${e.message}` : ""} level="error" important eventId={e.eventId ?? undefined} onClose={() => { if (e.eventId) void dismiss(e.source, e.revision, e.eventId); }} />)}
    <IssueNotice scope="recorder-operation" type="read" label="録音パネルの操作通知" message={state.error || error} />
  </>;
}
