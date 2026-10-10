// Development-only UI fixture: no IPC, recordings or file changes.
import { StrictMode, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { CommonModal } from "../src/common-modal";
import "../src/style.css";
import "../src/prototype-alignment.css";

declare global { interface Window { modalFixture: { closeCalls: number; commands: number; setBlocked: (value: boolean) => void; finish: () => void; openBlocked: () => void; closeWhileLocked: () => void; removeOpenerAndClose: () => void } } }
function Fixture() {
  const [open, setOpen] = useState(false), [second, setSecond] = useState(false), [blocked, setBlocked] = useState(false);
  const [removed, setRemoved] = useState(false);
  const opener = useRef<HTMLButtonElement>(null);
  window.modalFixture ??= { closeCalls: 0, commands: 0, setBlocked, finish: () => {}, openBlocked: () => {}, closeWhileLocked: () => {}, removeOpenerAndClose: () => {} };
  window.modalFixture.setBlocked = setBlocked;
  window.modalFixture.finish = () => setBlocked(false);
  window.modalFixture.openBlocked = () => { setBlocked(true); setOpen(true); };
  window.modalFixture.closeWhileLocked = () => { setBlocked(true); setOpen(false); };
  window.modalFixture.removeOpenerAndClose = () => { setRemoved(true); setOpen(false); };
  const dismiss = () => { window.modalFixture.closeCalls++; setOpen(false); };
  return <main className="content"><section aria-label="共通モーダル検証">
    <h2>共通モーダル検証</h2><label>背景の入力<input aria-label="背景の入力" defaultValue="保持する内容"/></label>
    {!removed && <button ref={opener} disabled={blocked} onClick={() => setOpen(true)}>確認を開く</button>}
    <CommonModal open={open} blocked={blocked} onDismiss={dismiss} restoreReady={!blocked} restoreTarget={() => opener.current} aria-labelledby="fixture-title">
      <h2 id="fixture-title">対象の変更確認</h2><p>合成した対象です。実ファイルは変更しません。変更する内容と残す内容を確認してください。</p>
      <label>確認欄<input aria-label="確認欄"/></label>
      <button type="button" disabled={blocked} onClick={() => { window.modalFixture.commands++; setBlocked(true); }}>対象を変更する</button>
      <button type="button" onClick={() => setSecond(true)}>次の確認も要求</button>
      <button type="button" hidden={!blocked} onClick={() => setBlocked(false)}>合成処理の完了を通知</button>
      {Array.from({ length: 40 }, (_, i) => <p key={i}>確認対象 {i + 1}: 長い対象一覧もモーダル本文の中でスクロールします。</p>)}
      <button type="button" data-dialog-cancel disabled={blocked} onClick={dismiss}>変更せず閉じる</button>
    </CommonModal>
    <CommonModal open={second} onDismiss={() => setSecond(false)} restoreTarget={() => opener.current} aria-labelledby="second-title"><h2 id="second-title">次の対象の確認</h2><p>先の確認が閉じるまで重ねません。</p></CommonModal>
  </section></main>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><Fixture/></StrictMode>);
