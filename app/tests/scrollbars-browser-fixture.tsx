// Synthetic content; actual shared styles and native modal, no IPC or files.
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { CommonModal } from "../src/common-modal";
import "../src/style.css";
import "../src/prototype-alignment.css";
import "../src/internal-logs.css";
import "../src/window-titlebar.css";

const lines = Array.from({ length:80 }, (_, index) => `確認内容 ${index + 1}: スクロールして最後まで閲覧できます。`);
function Fixture() {
  const [open, setOpen] = useState(false);
  return <main style={{ padding:24, display:"grid", gap:24 }}>
    <h1>共通スクロールバー</h1>
    <button style={{ justifySelf:"start" }} onClick={event => { event.currentTarget.focus({preventScroll:true}); setOpen(true); }}>スクロール確認を開く</button>
    <div style={{ display:"flex", flexWrap:"wrap", gap:24 }}>
      <section><h2>縦横スクロール</h2><div className="meeting-table-frame" data-testid="both" style={{ width:320, height:180 }}>
        <div style={{ width:900, height:640, padding:16, background:"var(--surface)" }}>{lines.map((line, index) => <p key={line} style={{ margin:0, lineHeight:"20px" }}>{index < 30 ? line : "末尾"}</p>)}</div>
      </div></section>
      <section><h2>ログパネル</h2><div className="internal-logs-scroll" data-testid="vertical" style={{ width:320, height:180 }}>{lines.map(line => <p key={line} style={{ margin:8 }}>{line}</p>)}</div></section>
      <section><h2>テキスト入力</h2><textarea aria-label="スクロールする入力" defaultValue={lines.join("\n")} style={{ width:320, height:180, borderRadius:8, color:"var(--text)", background:"var(--surface)", resize:"both" }} /></section>
      <section><h2>スクロール不要</h2><div className="meeting-table-frame" data-testid="short" style={{ width:320, height:180, padding:16 }}>全体が収まる内容</div></section>
    </div>
    <CommonModal open={open} onDismiss={() => setOpen(false)} className="import-dialog" aria-labelledby="scroll-modal-title">
      <h2 id="scroll-modal-title">スクロール確認</h2>
      {lines.map(line => <p key={line}>{line}</p>)}
      <button onClick={() => setOpen(false)}>末尾から閉じる</button>
    </CommonModal>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
