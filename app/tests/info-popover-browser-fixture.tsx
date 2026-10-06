// Isolated real components; no IPC, recordings, model access or file mutations.
import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { InfoPopover } from "../src/info-popover";
import { CommonModal } from "../src/common-modal";
import "../src/style.css";

declare global { interface Window { infoFixture: { updates: number; detailActions: number; setAnchor: (x: number, y: number) => void; setHidden: (value: boolean) => void; remove: () => void; update: () => void; confirm: () => void } } }
function Fixture() {
  const [anchor, setAnchor] = useState({ x: 48, y: 80 }), [hidden, setHidden] = useState(false), [removed, setRemoved] = useState(false), [modal, setModal] = useState(false), [tick, setTick] = useState(0);
  window.infoFixture ??= { updates: 0, detailActions: 0, setAnchor: () => {}, setHidden, remove: () => {}, update: () => {}, confirm: () => {} };
  Object.assign(window.infoFixture, { setAnchor: (x: number, y: number) => setAnchor({ x, y }), setHidden, remove: () => setRemoved(true), update: () => { window.infoFixture.updates++; setTick(t => t + 1); }, confirm: () => setModal(true) });
  return <main style={{ padding: 24, minHeight: 1800 }}>
    <h2>説明の検証</h2><label>背景の入力<input aria-label="背景の入力" defaultValue="保持する内容" /></label><p role="status">状態更新 {tick}</p>
    {!removed && <div hidden={hidden} style={{ position: "fixed", left: anchor.x, top: anchor.y }} className="heading-with-info"><h3>PCの音声</h3><InfoPopover label="PCの音声" details={<><p>{"詳細な対象範囲と設定の説明です。".repeat(60)}</p><a href="#detail-action" onClick={e => { e.preventDefault(); window.infoFixture.detailActions++; }}>説明内のリンク</a></>}>PC音声には会議以外の音が含まれる場合があります。同意を確認し、不要な音を止めてください。</InfoPopover></div>}
    <div style={{ position: "fixed", left: 24, bottom: 24 }} className="heading-with-info"><h3>話者の人数</h3><InfoPopover label="話者の人数">人数はトラックごとに指定します。会議全体の人数とは異なります。</InfoPopover></div>
    <button type="button" onClick={() => setModal(true)}>確認を開く</button>
    <CommonModal open={modal} onDismiss={() => setModal(false)} aria-labelledby="info-confirm-title"><h2 id="info-confirm-title">対象の変更確認</h2><div className="heading-with-info"><h3>対象の範囲</h3><InfoPopover label="確認内の補足">実行前の消失範囲は確認本文にも表示します。</InfoPopover></div><p>確認本文：削除したデータは復元できません。</p><input aria-label="確認の入力" defaultValue="確認中の内容" /></CommonModal>
  </main>;
}
createRoot(document.getElementById("root")!).render(<StrictMode><Fixture /></StrictMode>);
