// Production notification presenter and modal; no files, devices or native IPC.
import { createRoot } from "react-dom/client";
import { useState } from "react";
import { CommonModal } from "../src/common-modal";
import { ErrorNotice, NotificationList, ToastHost } from "../src/toast";
import "../src/style.css";
import "../src/prototype-alignment.css";

function Fixture() {
  const [error, setError] = useState(""), [open, setOpen] = useState(false), [history, setHistory] = useState(false);
  return <div className="app-layout"><div className="main-column">
    <header className="topbar"><button onClick={() => setHistory(value => !value)}>通知履歴</button><ToastHost /></header>
    <main><h1>通知の回帰検証</h1><button onClick={() => setOpen(true)}>編集を開く</button>
      <NotificationList visible={history} />
      <CommonModal open={open} onDismiss={() => setOpen(false)} dismissLabel="閉じる" aria-label="編集の検証">
        <h2>編集</h2><p>入力は保存の再試行まで保持されます。</p><input aria-label="編集内容" defaultValue="保持する入力" />
        <button onClick={() => setError("合成の保存失敗。元データは保持しています。")}>保存失敗を発生</button>
        <button onClick={() => setError("")}>再試行を成功</button>
        <ErrorNotice scope="synthetic-editor" type="save" active={!!error}>{error}</ErrorNotice>
      </CommonModal>
    </main>
  </div></div>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
