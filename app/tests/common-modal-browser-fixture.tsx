// Development-only UI fixture: no IPC, recordings or file changes.
import { StrictMode, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { CommonModal } from "../src/common-modal";
import { OptionsModal } from "../src/options-modal";
import { useActionConfirmation } from "../src/confirmation-service";
import { InfoPopover } from "../src/info-popover";
import "../src/style.css";
import "../src/prototype-alignment.css";

declare global { interface Window { modalFixture: { closeCalls: number; commands: number; setBlocked: (value: boolean) => void; finish: () => void; openBlocked: () => void; closeWhileLocked: () => void; removeOpenerAndClose: () => void } } }
function EscapeProbe() {
  const input = useRef<HTMLInputElement>(null);
  const send = (flags: KeyboardEventInit) => {
    input.current?.focus();
    input.current?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true, ...flags }));
  };
  return <div aria-label="合成キー検証" style={{ display: "grid", gap: 12, marginBlock: 16 }}>
    <label style={{ display: "grid", gap: 8 }}>確認欄<input ref={input} aria-label="確認欄"/></label>
    <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
      <button type="button" className="small-secondary" onClick={() => send({ isComposing: true })}>変換中のEscを合成</button>
      <button type="button" className="small-secondary" onClick={() => send({ keyCode: 229 })}>変換キー229のEscを合成</button>
      <button type="button" className="small-secondary" onClick={() => send({ repeat: true })}>長押しのEscを合成</button>
    </div>
  </div>;
}
function Fixture() {
  const [open, setOpen] = useState(false), [second, setSecond] = useState(false), [blocked, setBlocked] = useState(false);
  const [removed, setRemoved] = useState(false);
  const [stableOpen, setStableOpen] = useState(false);
  const stableTrigger = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLButtonElement>(null);
  const [settingsHost, setSettingsHost] = useState<HTMLDivElement | null>(null);
  const confirm = useActionConfirmation(false);
  window.modalFixture ??= { closeCalls: 0, commands: 0, setBlocked, finish: () => {}, openBlocked: () => {}, closeWhileLocked: () => {}, removeOpenerAndClose: () => {} };
  window.modalFixture.setBlocked = setBlocked;
  window.modalFixture.finish = () => setBlocked(false);
  window.modalFixture.openBlocked = () => { setBlocked(true); setOpen(true); };
  window.modalFixture.closeWhileLocked = () => { setBlocked(true); setOpen(false); };
  window.modalFixture.removeOpenerAndClose = () => { setRemoved(true); setOpen(false); };
  const dismiss = () => { window.modalFixture.closeCalls++; setOpen(false); };
  return <main className="content"><section aria-label="共通モーダル検証">
    <h2>共通モーダル検証</h2><label>背景の入力<input aria-label="背景の入力" defaultValue="保持する内容"/></label>
    <OptionsModal label="合成の編集管理" triggerHost={settingsHost}>
      <label>管理中の入力<input aria-label="管理中の入力" defaultValue="保持する管理設定"/></label>
      <OptionsModal label="管理内の詳細設定"><label>詳細設定の入力<input aria-label="詳細設定の入力" defaultValue="保持する詳細設定"/></label>
        <InfoPopover label="詳細設定の注意">合成の説明です。ファイル・権限は変更しません。</InfoPopover>
        <button type="button" onClick={event => void confirm({ opener:event.currentTarget, title:"詳細設定内の変更確認", target:"合成の詳細設定のみ", message:"実ファイルは変更しません。", confirmLabel:"合成の詳細設定を変更する", validate:()=>true, action:async()=>{} })}>詳細設定内で確認を開く</button>
      </OptionsModal>
      <button type="button" onClick={event => void confirm({ opener:event.currentTarget, title:"管理内の変更確認", target:"合成設定のみ", message:"実ファイルは変更しません。", confirmLabel:"合成設定を変更する", validate:()=>true, action:async()=>{} })}>管理内で確認を開く</button>
    </OptionsModal>
    <OptionsModal label="合成の全体設定"><div ref={setSettingsHost}/></OptionsModal>
    {!removed && <button ref={opener} disabled={blocked} onClick={() => setOpen(true)}>確認を開く</button>}
    <CommonModal open={open} blocked={blocked} onDismiss={dismiss} restoreReady={!blocked} restoreTarget={() => opener.current} aria-labelledby="fixture-title"
      persistentReplacement={<><h2 id="stable-vocabulary-title">同じ確認内の用語設定</h2><label>切り替えで保持する用語<input aria-label="切り替えで保持する用語" defaultValue="初期用語"/></label><button type="button" onClick={event => void confirm({opener:event.currentTarget,title:"切り替え設定内の変更確認",target:"合成の用語のみ",message:"実ファイルは変更しません。",confirmLabel:"合成用語を変更する",validate:()=>true,action:async()=>{}})}>切り替え設定内で確認を開く</button></>}
      replacement={stableOpen ? {key:"stable-vocabulary",labelledBy:"stable-vocabulary-title",content:null,blocked,dismissLabel:"閉じる",onDismiss:()=>setStableOpen(false),restoreTarget:()=>stableTrigger.current} : null}>
      <h2 id="fixture-title">対象の変更確認</h2><p>合成した対象です。実ファイルは変更しません。変更する内容と残す内容を確認してください。</p>
      <EscapeProbe/>
      <button ref={stableTrigger} type="button" onClick={event=>{event.currentTarget.focus({preventScroll:true});setStableOpen(true);}}>同じ確認内の用語設定を開く</button>
      <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
        <button type="button" disabled={blocked} onClick={() => { window.modalFixture.commands++; setBlocked(true); }}>対象を変更する</button>
        <button type="button" className="small-secondary" onClick={() => setSecond(true)}>次の確認も要求</button>
        <button type="button" className="small-secondary" hidden={!blocked} onClick={() => setBlocked(false)}>合成処理の完了を通知</button>
      </div>
      {Array.from({ length: 40 }, (_, i) => <p key={i}>確認対象 {i + 1}: 長い対象一覧もモーダル本文の中でスクロールします。</p>)}
      <button type="button" data-dialog-cancel disabled={blocked} onClick={dismiss}>変更せず閉じる</button>
    </CommonModal>
    <CommonModal open={second} onDismiss={() => setSecond(false)} restoreTarget={() => opener.current} aria-labelledby="second-title"><h2 id="second-title">次の対象の確認</h2><p>先の確認が閉じるまで重ねません。</p></CommonModal>
  </section></main>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><Fixture/></StrictMode>);
