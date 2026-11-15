// Metadata/IPC only; no native file access, network download or model execution.
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { ModelRuntimeGuidance } from "../src/model-runtime-guidance";
import { ModelInspectionPanel } from "../src/model-inspection";
import { CommonModal } from "../src/common-modal";
import { ToastHost } from "../src/toast";
import { SettingsModalScope } from "../src/settings-modal-scope";
import { parseDownloadDefinition } from "../src/model-download-state";
(window as any).skipModelDownloadFixtureMain = true;
await import("./model-download-browser-fixture");
const f = (window as any).modelDownloadFixture;
const manifest = parseDownloadDefinition(f.definition).manifest;
Object.assign(f.entries[0], { sourceType: "download", download: { operationId: "33333333-3333-4333-8333-333333333333", recipeHash: "f".repeat(64) },
  modelFiles: manifest.files.filter(file => file.role === "asr").map(file => ({ role: file.role, size: file.expectedSize, sha256: file.expectedHash })), reviewHash: "e".repeat(64) });
// No selected profile: the installed but unused package must still be offered.
const query = new URLSearchParams(location.search);
if (query.has("empty")) f.entries = [];
function Fixture() {
  const [locked, setLocked] = useState(query.has("locked")), [other, setOther] = useState(query.has("other"));
  const [visible, setVisible] = useState(false), [epoch, setEpoch] = useState(0), [mount, setMount] = useState(true);
  f.setRuntimeLocked = setLocked; f.setRuntimeMount = setMount;
  return <SettingsModalScope><main><ToastHost /><div className="sidebar"><button aria-label="設定" onClick={() => setVisible(true)}>設定</button></div>
    <button onClick={() => setMount(value => !value)}>案内を再描画</button>
    <ModelInspectionPanel locked={locked} recordingActive={false} visible={visible} runtimeUpdateEpoch={epoch} />
    <CommonModal open={other} onDismiss={() => setOther(false)} aria-label="先行する確認"><h2>先行する確認</h2><button onClick={() => setOther(false)}>確認を閉じる</button></CommonModal>
    {mount && <ModelRuntimeGuidance locked={locked} onOpenModels={async () => { setVisible(true); setEpoch(value => value + 1); }} />}
  </main></SettingsModalScope>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
