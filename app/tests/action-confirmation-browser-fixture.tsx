// Real shared confirmation UI with synthetic actions only; no native APIs.
import { StrictMode, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { CommonModal } from "../src/common-modal";
import { ConfirmationHost } from "../src/confirmation-host";
import { useActionConfirmation } from "../src/confirmation-service";
import "../src/style.css";
import "../src/prototype-alignment.css";

declare global { interface Window { actionFixture: { commands: { target: string; version: number }[]; mounts: number; version: number; mode: string; progress: boolean; cancellationCalls: number; completeCancel: () => void; complete: () => void; closeParent: () => void; removeOwner: () => void; blocked: (v: boolean) => void } } }
window.actionFixture = { commands: [], mounts: 0, version: 1, mode: "success", progress: false, cancellationCalls: 0, completeCancel: () => {}, complete: () => {}, closeParent: () => {}, removeOwner: () => {}, blocked: () => {} };
function Owner({ nested = false, blocked }: { nested?: boolean; blocked: boolean }) {
  const confirm = useActionConfirmation(blocked), [busy, setBusy] = useState(false), [text, setText] = useState("保持する編集中の本文");
  useEffect(() => { window.actionFixture.mounts++; }, []);
  const request = async (opener: HTMLElement) => {
    const version = window.actionFixture.version, mode = window.actionFixture.mode, target = nested ? "比較中の候補 #2" : "合成対象 #1";
    setBusy(true);
    try { await confirm({ opener, title: "対象の変更を確認", target, confirmLabel: "対象を変更する", message: "この対象の未保存入力と古い直前版は復元できなくなります。現在の版とその編集は保持します。", validate: () => window.actionFixture.version === version, progressAction: window.actionFixture.progress ? { label: "準備の停止を要求する", action: async () => { window.actionFixture.cancellationCalls++; await new Promise<void>(resolve => { window.actionFixture.completeCancel = resolve; }); } } : undefined, action: async () => {
      window.actionFixture.commands.push({ target, version });
      if (mode === "delay") await new Promise<void>(resolve => { window.actionFixture.complete = resolve; });
      if (mode === "timed") await new Promise<void>(resolve => setTimeout(resolve, 3000));
      if (mode === "lost") throw Error("確定した結果の応答を受け取れませんでした");
    } }); } finally { setBusy(false); }
  };
  return <section aria-label={nested ? "比較内容" : "通常画面"}><h3>{nested ? "比較内容" : "通常画面"}</h3>
    <label>編集内容<input aria-label={nested ? "比較中の入力" : "通常の入力"} value={text} onChange={e => setText(e.target.value)} /></label>
    {nested && Array.from({ length: 35 }, (_, i) => <p key={i}>比較対象 {i + 1} の本文と根拠です。</p>)}
    <button type="button" disabled={busy || blocked} onClick={e => void request(e.currentTarget)}>{nested ? "候補を採用" : "変更の確認を開く"}</button>
  </section>;
}
function Fixture() {
  const [parent, setParent] = useState(false), [removed, setRemoved] = useState(false), [blocked, setBlocked] = useState(false);
  const opener = useRef<HTMLButtonElement>(null);
  window.actionFixture.closeParent = () => setParent(false); window.actionFixture.removeOwner = () => setRemoved(true); window.actionFixture.blocked = setBlocked;
  return <main className="content"><h2>実行前確認の検証</h2>
    <label>合成実行の結果<select aria-label="合成実行の結果" defaultValue="success" onChange={event => { window.actionFixture.mode = event.target.value; }}><option value="success">成功</option><option value="timed">3秒後に成功</option><option value="lost">応答喪失</option></select></label>
    {!removed && <Owner blocked={blocked} />}
    <button ref={opener} onClick={() => setParent(true)}>比較を開く</button>
    <CommonModal className={new URLSearchParams(location.search).get("layout") === "import" ? "import-dialog" : undefined} open={parent} onDismiss={() => setParent(false)} restoreTarget={() => opener.current} aria-labelledby="action-parent-title"><h2 id="action-parent-title">候補の比較</h2><Owner nested blocked={blocked} /></CommonModal>
    <ConfirmationHost />
  </main>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><Fixture /></StrictMode>);
