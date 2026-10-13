// Development-only presentation fixture. No Tauri, files, devices or IPC.
import { StrictMode, useLayoutEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { CommonModal } from "../src/common-modal";
import { IssueNotice, ToastHost, ToastNotice, toasts } from "../src/toast";
import { toastKey, type ToastInput } from "../src/toast-state";
import "../src/style.css";
import "../src/prototype-alignment.css";

declare global { interface Window { toastFixture: { keys: Set<string>; show: (input: ToastInput) => void; clear: () => void; issue: (message: string) => void; issueMessage?: () => string; changeScope: () => void; detach: () => void; retryCalls: number; unsafeCommands: number; actionCalls: number; showAction: () => void; debugState?: () => unknown } } }
function Fixture() {
  const [counts, setCounts] = useState({ receipts: 0, retries: 0, actions: 0 });
  const [dark, setDark] = useState(false);
  useLayoutEffect(() => { document.documentElement.dataset.theme = dark ? "dark" : "light"; }, [dark]);
  const [error, setError] = useState(""), [result, setResult] = useState(""), [mounted, setMounted] = useState(true), [open, setOpen] = useState(false);
  const [scope, setScope] = useState("fixture");
  const [eventMounted, setEventMounted] = useState(true), [eventMessage, setEventMessage] = useState("");
  const [docked, setDocked] = useState(new URLSearchParams(location.search).get("docked") === "1");
  const trigger = useRef<HTMLButtonElement>(null);
  window.toastFixture ??= { keys: new Set(), show: () => {}, clear: () => {}, issue: setError, changeScope: () => {}, detach: () => {}, retryCalls: 0, unsafeCommands: 0, actionCalls: 0, showAction: () => {} };
  const fixture = window.toastFixture;
  fixture.issueMessage = () => error;
  const retry = () => { fixture.retryCalls++; setCounts(c => ({ ...c, retries: c.retries + 1 })); setError(""); };
  fixture.debugState = () => ({ pauses: [...Reflect.get(toasts, "pauses")], entries: [...Reflect.get(toasts, "entries").values()].map(e => ({ key: e.toast.key, pauses: [...e.pauses], remaining: e.remaining, started: e.started })) });
  fixture.keys.add(toastKey(scope, "save"));
  fixture.changeScope = () => { setError(""); setScope("other-meeting"); };
  fixture.show = input => { fixture.keys.add(input.key); toasts.show(input); };
  fixture.clear = () => { for (const key of [...fixture.keys, toastKey("fixture", "save"), toastKey("fixture", "result")]) toasts.resolve(key); fixture.keys.clear(); setError(""); setResult(""); };
  fixture.issue = setError;
  fixture.detach = () => setMounted(false);
  fixture.showAction = () => fixture.show({ key: "action", message: "話者変更を保存しました", level: "success", action: { kind: "undo", label: "元に戻す", run: () => { fixture.actionCalls++; setCounts(c => ({ ...c, actions: c.actions + 1 })); } } });
  return <div className="app-layout" data-theme={dark ? "dark" : "light"}><aside className="sidebar">検証用</aside><div className="main-column">
    <header className="topbar"><span>検証用ヘッダー</span><div className="header-recording">待機中</div><ToastHost docked={docked}/></header>
    <main className="content" aria-label="通知検証ワークスペース"><h1>通知の合成検証</h1>
      <label>保持する入力<input aria-label="保持する入力" defaultValue="編集中の点検日" /></label>
      <button type="button" onClick={() => setEventMessage("同じ処理の保存が完了しました")}>同じイベントを通知</button>
      <button type="button" onClick={() => setEventMessage("")}>イベント状態を一時クリア</button>
      <button type="button" onClick={() => setEventMounted(value => !value)}>イベント表示を切り替え</button>
      {eventMounted && <ToastNotice scope="event-fixture" type="result" eventId="save-1" message={eventMessage} level="error" />}
      <button type="button" onClick={() => setResult("書き出しました")}>成功を通知</button>
      <button type="button" onClick={() => setResult("表示設定を保存しました。")}>表示設定の成功を通知</button>
      <button type="button" onClick={() => fixture.show({ key: "multiline", message: "表示設定を保存しました。長い通知でも本文と説明アイコン、閉じるボタンの上下中央を揃えます。", level: "info" })}>複数行を通知</button>
      <button type="button" onClick={() => fixture.showAction()}>操作付きの成功を通知</button>
      <ToastNotice scope="fixture" type="result" message={result} onClose={() => setResult("")} />
      <button type="button" onClick={() => setError("保存先の容量が不足しています。入力は保持しています。")}>保存失敗を通知</button>
      <button type="button" onClick={() => setDocked(value => !value)}>通知の専用配置を切り替え</button>
      <button type="button" onClick={() => { for (const key of ["clipped-1", "clipped-2", "clipped-3"]) fixture.show({ key, level:"info", message:`${key}：重なった通知はホバーかキーボードで展開します。未表示の間は期限を止めます。` }); }}>3件の情報を通知</button>
      <button type="button" onClick={() => { for (let i = 1; i <= 4; i++) fixture.show({ key: `important-${i}`, level:"error", message:`要確認 ${i}：入力内容は保持しています。`, eventId: "batch", onRendered: () => setCounts(c => ({ ...c, receipts: c.receipts + 1 })) }); }}>4件の重要通知</button>
      <button type="button" onClick={() => fixture.clear()}>通知をクリア</button>
      <button type="button" onClick={() => setDark(value => !value)}>検証テーマを切り替え</button>
      <output aria-label="通知の検証カウンター">確認 {counts.receipts} · 再試行 {counts.retries} · 取り消し {counts.actions} · 危険な操作 0</output>
      {mounted && <><IssueNotice scope={scope} type="save" label="保存に失敗" message={error} action={{ kind: "retry", label: "保存を再試行", run: retry }} />{error&&<button type="button" onClick={retry}>画面から保存を再試行</button>}</>}
      <button type="button" ref={trigger} onClick={() => setOpen(true)}>変更の確認を開く</button>
      <CommonModal open={open} onDismiss={() => setOpen(false)} restoreTarget={() => trigger.current} aria-labelledby="toast-modal-title"><h2 id="toast-modal-title">変更の確認</h2><p>実ファイルは変更しません。</p></CommonModal>
      {Array.from({ length: 40 }, (_, index) => <p key={index}>本文 {index}: 本文だけをスクロールしても通知は固定位置に表示します。</p>)}
    </main><div className="toast-fixture-player" style={{ flex: "none", height: 100, padding: 12, background: "var(--surface)", borderTop: "1px solid var(--border)" }} aria-label="固定プレーヤー">固定プレーヤー（合成表示）</div>
  </div></div>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><Fixture /></StrictMode>);
