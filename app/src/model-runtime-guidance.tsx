import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { CommonModal } from "./common-modal";
import { invoke, isTauri } from "./ipc";
import { backgroundReadFailure } from "./background-read-failure";
import { useStartupRecoveryPending } from "./startup-attention";
import { assertOfficialModelOptions } from "./model-download-state";
import { placementLabel, type ModelPlacementEntry } from "./model-inspection-state";
import { assertProfileCatalog, type ModelProfileView } from "./model-profile-state";
import { runtimeUpdateChoices, type ModelChoice } from "./model-selection-state";

// Dismissal lasts for this webview process, including React remounts.
let dismissed = false;
export function ModelRuntimeGuidance({ locked, onOpenModels }: { locked: boolean; onOpenModels: () => Promise<void> }) {
  const recoveryPending = useStartupRecoveryPending();
  const [targets, setTargets] = useState<ModelChoice[]>([]), [closed, setClosed] = useState(dismissed);
  const [available, setAvailable] = useState(false), [error, setError] = useState("");
  useEffect(() => {
    if (!isTauri() || closed || recoveryPending || locked) return;
    let disposed = false;
    const failure = backgroundReadFailure("model_runtime_guidance");
    const read = async () => {
      try {
        const [entries, options, profiles, introduction] = await Promise.all([
          invoke<ModelPlacementEntry[]>("get_model_placements"), invoke("get_official_model_options").then(assertOfficialModelOptions),
          invoke<ModelProfileView[]>("get_model_profiles"), invoke<{ pending: boolean }>("get_model_download_guidance"),
        ]);
        entries.forEach(placementLabel); assertProfileCatalog(profiles);
        if (!disposed) setTargets(introduction.pending ? [] : runtimeUpdateChoices(entries, options, profiles));
      } catch (cause) { if (!disposed) failure.failed(cause); }
    };
    void read();
    const subscription = listen("model-inventory-changed", () => void read());
    return () => { disposed = true; void subscription.then(unlisten => unlisten()).catch(() => {}); };
  }, [closed, recoveryPending, locked]);
  useEffect(() => {
    if (closed || !targets.length || locked || recoveryPending) { setAvailable(false); return; }
    // Wait for any other native dialog/popover to close. Never stack an unsolicited modal.
    const refresh = () => setAvailable(!document.querySelector('dialog[open], [popover]:popover-open'));
    refresh();
    const observer = new MutationObserver(() => {
      // Our own dialog must not cause its visibility to oscillate.
      if (document.querySelector('dialog[open][data-runtime-guidance]')) return;
      refresh();
    });
    observer.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["open", "popover"], childList: true });
    return () => observer.disconnect();
  }, [closed, targets.length, locked, recoveryPending]);
  const dismiss = () => { dismissed = true; setClosed(true); };
  const openModels = async () => {
    try { dismiss(); await onOpenModels(); }
    catch { dismissed = false; setClosed(false); setError("設定を開けませんでした。もう一度お試しください。"); }
  };
  return <CommonModal data-runtime-guidance open={!closed && targets.length > 0 && available && !locked && !recoveryPending}
    onDismiss={dismiss} restoreTarget={() => document.querySelector<HTMLElement>('.sidebar button[aria-label="設定"]')} aria-labelledby="model-runtime-guidance-title">
    <h2 id="model-runtime-guidance-title">AIモデルの実行環境を更新できます</h2>
    <p>導入済みの次のモデルに、実行環境の更新があります。</p>
    <ul>{targets.map(target => <li key={target.value}>{target.option?.review.manifest.engine === "llama.cpp" ? "議事録生成" : target.option?.review.manifest.engine === "sherpa-onnx" ? "話者分離" : "文字起こし"}：{target.label}</li>)}</ul>
    <p>設定の「ローカルAI」から「実行環境を更新して使用」を選び、利用条件を確認して更新してください。</p>
    <p>現在の設定はそのまま使えます。更新後、不要になった旧モデルデータは自動で整理します。</p>
    {error && <p role="alert">{error}</p>}
    <div className="analysis-actions"><button type="button" onClick={() => void openModels()}>モデルの設定を開く</button><button type="button" className="small-secondary" onClick={dismiss}>あとで</button></div>
  </CommonModal>;
}
