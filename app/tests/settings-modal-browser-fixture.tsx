// No IPC, device input or real persistence; exercises actual shared UI ownership.
import { useId, useState } from "react";
import { createRoot } from "react-dom/client";
import { OptionsModal } from "../src/options-modal";
import { SettingsModalScope } from "../src/settings-modal-scope";
import { useActionConfirmation } from "../src/confirmation-service";
import { ConfirmationHost } from "../src/confirmation-host";
import "../src/style.css";
import "../src/prototype-alignment.css";
function Fixture() {
  const [commands, setCommands] = useState(0);
  const [reviewOpen, setReviewOpen] = useState(false), [reviewBusy, setReviewBusy] = useState(false);
  const reviewTitle = useId();
  const confirm = useActionConfirmation(false);
  return <main className="content"><h1>共有設定の合成検証</h1><p role="status">実行回数 {commands}</p>
    <OptionsModal label="親の設定"><label>親の入力<input aria-label="親の入力" defaultValue="親の保持内容" /></label>
      <OptionsModal label="子の設定"><label>子の入力<input aria-label="子の入力" defaultValue="子の保持内容" /></label>
        <button type="button" onClick={event=>void confirm({opener:event.currentTarget,title:"子設定の変更確認",target:"合成入力のみ",message:"実データは変更しません。",confirmLabel:"合成変更を実行",validate:()=>true,action:async()=>{setCommands(value=>value+1);}})}>変更確認を開く</button>
      </OptionsModal>
    </OptionsModal>
    <OptionsModal label="別の設定"><label>別の入力<input aria-label="別の入力" defaultValue="別の保持内容" /></label></OptionsModal>
    <OptionsModal label="読込設定" replacement={reviewOpen ? {key:"async-review",labelledBy:reviewTitle,content:null,blocked:reviewBusy,onDismiss:()=>setReviewOpen(false),dismissLabel:"閉じる"} : null}
      persistentReplacement={<div hidden={!reviewOpen}><h3 id={reviewTitle}>読込中の確認画面</h3>
        <button type="button" disabled={!reviewBusy} onClick={()=>setReviewBusy(false)}>合成の読込を完了</button>
        <button type="button" onClick={()=>setReviewBusy(false)}>合成の読込を完了（焦点保持）</button>
        <label>読込後の入力<input aria-label="読込後の入力" disabled={reviewBusy} defaultValue="未保存の保持内容" /></label>
      </div>}>
      <button type="button" onClick={event=>{event.currentTarget.focus();setReviewBusy(true);setReviewOpen(true);}}>合成読込を開く</button>
    </OptionsModal>
  </main>;
}
createRoot(document.getElementById("root")!).render(<><SettingsModalScope><Fixture /></SettingsModalScope><ConfirmationHost /></>);
