// Development-only presentation fixture. No Tauri, files, devices or IPC.
import { StrictMode, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { CommonModal } from "../src/common-modal";
import { IssueNotice, ToastHost, ToastNotice, toasts } from "../src/toast";
import { toastKey, type ToastInput } from "../src/toast-state";
import "../src/style.css";

declare global { interface Window { toastFixture: { keys: Set<string>; show: (input: ToastInput) => void; clear: () => void; issue: (message: string) => void; changeScope: () => void; detach: () => void; retryCalls: number; unsafeCommands: number; actionCalls: number; showAction: () => void } } }
function Fixture() {
  const [error, setError] = useState(""), [result, setResult] = useState(""), [mounted, setMounted] = useState(true), [open, setOpen] = useState(false);
  const [scope, setScope] = useState("fixture");
  const trigger = useRef<HTMLButtonElement>(null);
  window.toastFixture ??= { keys: new Set(), show: () => {}, clear: () => {}, issue: setError, changeScope: () => {}, detach: () => {}, retryCalls: 0, unsafeCommands: 0, actionCalls: 0, showAction: () => {} };
  const fixture = window.toastFixture;
  fixture.keys.add(toastKey(scope, "save"));
  fixture.changeScope = () => { setError(""); setScope("other-meeting"); };
  fixture.show = input => { fixture.keys.add(input.key); toasts.show(input); };
  fixture.clear = () => { for (const key of [...fixture.keys, toastKey("fixture", "save"), toastKey("fixture", "result")]) toasts.resolve(key); fixture.keys.clear(); setError(""); setResult(""); };
  fixture.issue = setError;
  fixture.detach = () => setMounted(false);
  fixture.showAction = () => fixture.show({ key: "action", message: "話者変更を保存しました", level: "success", action: { kind: "undo", label: "元に戻す", run: () => { fixture.actionCalls++; } } });
  return <div className="app-layout"><aside className="sidebar">検証用</aside><div className="main-column">
    <header className="topbar">検証用ヘッダー</header><ToastHost />
    <main className="content" aria-label="通知検証ワークスペース"><h1>通知の合成検証</h1>
      <label>保持する入力<input aria-label="保持する入力" defaultValue="編集中の点検日" /></label>
      <button type="button" onClick={() => setResult("書き出しました")}>成功を通知</button>
      <ToastNotice scope="fixture" type="result" message={result} onClose={() => setResult("")} />
      <button type="button" onClick={() => setError("保存先の容量が不足しています。入力は保持しています。")}>保存失敗を通知</button>
      {mounted && <IssueNotice scope={scope} type="save" label="保存に失敗" message={error} action={{ kind: "retry", label: "保存を再試行", run: () => { fixture.retryCalls++; setError(""); } }}><button type="button" onClick={() => { fixture.retryCalls++; setError(""); }}>画面から保存を再試行</button></IssueNotice>}
      <button type="button" ref={trigger} onClick={() => setOpen(true)}>変更の確認を開く</button>
      <CommonModal open={open} onDismiss={() => setOpen(false)} restoreTarget={() => trigger.current} aria-labelledby="toast-modal-title"><h2 id="toast-modal-title">変更の確認</h2><p>実ファイルは変更しません。</p></CommonModal>
      {Array.from({ length: 40 }, (_, index) => <p key={index}>本文 {index}: 本文だけをスクロールしても通知は固定位置に表示します。</p>)}
    </main><div className="toast-fixture-player" style={{ flex: "none", height: 100, padding: 12, background: "var(--surface)", borderTop: "1px solid var(--border)" }} aria-label="固定プレーヤー">固定プレーヤー（合成表示）</div>
  </div></div>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><Fixture /></StrictMode>);
