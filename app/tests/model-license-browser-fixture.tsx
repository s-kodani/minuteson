// Real settings modal and model UI; every model operation is synthetic.
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { ModelInspectionPanel } from "../src/model-inspection";
import { OptionsModal } from "../src/options-modal";
import { SettingsModalScope } from "../src/settings-modal-scope";
import { ToastHost } from "../src/toast";
(window as any).skipModelDownloadFixtureMain = true;
await import("./model-download-browser-fixture");
const f = (window as any).modelDownloadFixture;
f.licenseRestricted = true;
const entry = f.entries[0], checkOperationId = "22222222-2222-4222-8222-222222222222";
entry.compatibility = { operationId: checkOperationId, packageId: entry.packageId, reviewHash: entry.reviewHash, adapterRevision: 1, state: "succeeded", errorMessage: null };
Object.assign(f.profiles[0], { id: "33333333-3333-4333-8333-333333333333", profileVersion: 1, components: [{ roles: ["asr"], packageId: entry.packageId, modelName: entry.modelName, modelVersion: entry.modelVersion, engine: entry.engine, checkOperationId, settingsMatch: true, compatibilityCurrent: true }] });
function Fixture() {
  const [visible, setVisible] = useState(true), [locked, setLocked] = useState(false), [recordingActive, setRecordingActive] = useState(false);
  f.setVisible = setVisible; f.setLocked = setLocked; f.setRecordingActive = setRecordingActive;
  return <main><ToastHost /><SettingsModalScope><OptionsModal label="ローカルAI設定" title="ローカルAI設定"><ModelInspectionPanel visible={visible} locked={locked} recordingActive={recordingActive} /></OptionsModal></SettingsModalScope></main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
