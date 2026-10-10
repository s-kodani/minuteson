// Presentation-only fixture: no native IPC, files, audio, preferences or models.
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { SettingsDisclosure } from "../src/settings-disclosure";
import "../src/style.css";
import "../src/prototype-alignment.css";
function Fixture() {
  const [attention, setAttention] = useState(false);
  return <main className="content"><h1>設定の開閉・合成検証</h1><p>実データ・モデル・録音の変更なし</p>
    <button onClick={() => setAttention(true)}>合成エラーを表示</button><button onClick={() => setAttention(false)}>合成エラーを解除</button>
    <SettingsDisclosure label="詳細設定" attention={attention}>
      {attention && <p role="alert">合成：対応が必要です</p>}
      <label>保存前の入力<textarea defaultValue="保持する入力" /></label>
    </SettingsDisclosure>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
