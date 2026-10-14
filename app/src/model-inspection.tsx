import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { ModelIssue } from "./model-issue";
import { assertCompatibilitySummary, assertPlacementReceipt, placementLabel, type ModelPlacementEntry, type ModelPlacementReceipt, type ModelCompatibilitySummary } from "./model-inspection-state";
import { assertOfficialModelOptions, assertDownloadReceipt, assertDownloadReadback, assertDownloadLifetime, assertDownloadCatalog, type OfficialModelOption, type DownloadReceipt } from "./model-download-state";
import { assertProfileCatalog, assertApplicationPreview, assertApplicationResult, canReviewApplication, type ModelApplicationPreview, type ModelProfileView } from "./model-profile-state";

type Pending = { token: string; stage: "download" | "place" | "check" | "preview" | "apply"; option?: OfficialModelOption; entry?: ModelPlacementEntry; preview?: ModelApplicationPreview };
let unresolved: Pending | null = null;
// A confirmed download is reusable after navigation. No automatic retry or
// fresh download replaces an uncertain writer or a retained download receipt.
const downloads = new Map<string, DownloadReceipt>();
const engines = [{ engine: "whisper.cpp", label: "文字起こし" }, { engine: "sherpa-onnx", label: "話者分離" }, { engine: "llama.cpp", label: "議事録生成" }];
// Retain the already installed package on disk, but do not offer the retired
// 0.5B recipe again after it proved unreliable on actual utterance input.
const retiredRecipeHashes = new Set(["36416d1eeca501fdc083e2b31c9f49b4258342174b0f7d52798aa14b827dc44e"]);
const isRetired = (entry: ModelPlacementEntry) => entry.download?.recipeHash != null && retiredRecipeHashes.has(entry.download.recipeHash);
export function ModelInspectionPanel({ locked, recordingActive, refreshEpoch = 0 }: { locked: boolean; recordingActive: boolean; refreshEpoch?: number; compact?: boolean }) {
  const [options, setOptions] = useState<OfficialModelOption[]>([]), [entries, setEntries] = useState<ModelPlacementEntry[]>([]), [profiles, setProfiles] = useState<ModelProfileView[]>([]);
  const [selection, setSelection] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(""), [error, setError] = useState(""), [pending, setPending] = useState<Pending | null>(unresolved);
  const mounted = useRef(true), running = useRef(false), blocked = useRef(locked || recordingActive);
  blocked.current = locked || recordingActive;
  const retain = (value: Pending | null) => { unresolved = value; if (mounted.current) setPending(value); };
  const load = async () => {
    const [placed, catalog] = await Promise.all([invoke<ModelPlacementEntry[]>("get_model_placements"), invoke<ModelProfileView[]>("get_model_profiles")]);
    placed.forEach(placementLabel); assertProfileCatalog(catalog);
    if (mounted.current) { setEntries(placed); setProfiles(catalog); }
    return { placed, catalog };
  };
  useEffect(() => {
    mounted.current = true;
    if (isTauri()) void Promise.all([load(), invoke("get_official_model_options").then(assertOfficialModelOptions)]).then(([, official]) => { if (mounted.current) setOptions(official); }).catch(cause => { if (mounted.current) setError(String(cause)); });
    return () => { mounted.current = false; if (unresolved) void invoke("cancel_model_inspection", { token: unresolved.token }).catch(() => {}); };
  }, []);
  useEffect(() => { if (refreshEpoch && isTauri()) void load().catch(cause => setError(String(cause))); }, [refreshEpoch]);
  useEffect(() => { if (blocked.current && unresolved) void invoke("cancel_model_inspection", { token: unresolved.token }).catch(() => {}); }, [locked, recordingActive]);
  const run = async (label: string, action: () => Promise<void>) => {
    if (running.current || blocked.current || !isTauri()) return;
    running.current = true; setBusy(label); setError("");
    try { await action(); }
    catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { running.current = false; if (mounted.current) { setBusy(""); await load().catch(cause => setError(String(cause))); } }
  };
  const step = async <T,>(stage: Pending["stage"], action: (token: string) => Promise<T>, context: Omit<Pending, "token" | "stage"> = {}): Promise<T> => {
    if (!mounted.current || blocked.current || unresolved) throw Error("モデルの準備を停止しました。現在の状態を確認してください。");
    const token = crypto.randomUUID(); retain({ token, stage, ...context });
    const value = await action(token);
    if (!mounted.current || blocked.current) throw Error("準備の結果を確認してください。");
    retain(null); return value;
  };
  const apply = async (entry: ModelPlacementEntry) => {
    if (!canReviewApplication(entry)) {
      await step("check", async token => {
        const result = await invoke<ModelCompatibilitySummary>("check_model_compatibility", { token, packageId: entry.packageId, expectedReviewHash: entry.reviewHash });
        assertCompatibilitySummary(result, token, entry);
        if (result.state !== "succeeded") throw Error(result.errorMessage || "このモデルは使用できません。");
        return result;
      }, { entry });
      const checked = (await load()).placed.find(item => item.packageId === entry.packageId);
      if (!checked || !canReviewApplication(checked)) throw Error("モデルの動作確認結果を読み込めません。");
      entry = checked;
    }
    const current = (await load()).catalog.find(item => item.name === "standard")!;
    const preview = await step("preview", async token => {
      const value = await invoke<ModelApplicationPreview>("preview_model_profile_application", { token, profile: "standard", packageId: entry.packageId, expectedReviewHash: entry.reviewHash, checkOperationId: entry.compatibility!.operationId });
      assertApplicationPreview(value, entry, "standard", current); return value;
    }, { entry });
    await step("apply", async token => {
      const value = await invoke<ModelProfileView>("apply_model_profile_application", { token, preview });
      assertApplicationResult(value, token, preview); return value;
    }, { entry, preview });
  };
  const useModel = (engine: string, value: string) => {
    return run("モデルを準備しています", async () => {
    if (pending || unresolved) throw Error("前の準備の状態を確認してください。");
    let entry = entries.find(item => item.packageId === value && item.engine === engine && item.placementRecorded && item.bodyRemoved !== true && !isRetired(item));
    const option = options.find(item => "official:" + item.id === value && item.review.manifest.engine === engine);
    if (!entry && option) {
      entry = entries.find(item => item.sourceType === "download" && item.download?.recipeHash === option.review.recipeHash && item.engine === engine && item.placementRecorded && item.bodyRemoved !== true);
      if (!entry) {
        let receipt = downloads.get(option.review.recipeHash);
        if (!receipt) {
          const catalog = assertDownloadCatalog(await invoke("get_model_download_catalog", { token: crypto.randomUUID() }));
          const matching = catalog.entries.filter(item => item.recipeHash === option.review.recipeHash);
          const recorded = matching.find(item => item.state === "completion_recorded") ?? matching[0];
          if (recorded) {
            const read = await assertDownloadReadback(await invoke("read_model_download", { token: crypto.randomUUID(), downloadId: recorded.operationId, expectedRecipeHash: option.review.recipeHash }), recorded.operationId, option.review.recipeHash);
            if (read.state !== "complete" || !read.receipt) throw Error("このモデルのダウンロードは未完了です。途中のファイルは保持しています。再取得は行いません。");
            receipt = read.receipt; downloads.set(option.review.recipeHash, receipt);
          }
        }
        if (!receipt) receipt = await step("download", async token => {
          const result = await assertDownloadReceipt(await invoke("start_model_download", { token, definition: option.definition, expectedRecipeHash: option.review.recipeHash }), token, option.review);
          downloads.set(option.review.recipeHash, result); return result;
        }, { option });
        const downloaded = receipt;
        const placed = await step("place", async token => {
          const result = await invoke<ModelPlacementReceipt>("place_downloaded_model", { token, downloadId: downloaded.operationId, expectedRecipeHash: downloaded.recipeHash, expectedReviewHash: downloaded.report.reviewHash });
          assertPlacementReceipt(result, token, downloaded.report.reviewHash); return result;
        }, { option });
        entry = (await load()).placed.find(item => item.packageId === placed.packageId);
      }
    }
    if (!entry) throw Error("使用するモデルを選択してください。");
    if (entry.sourceType !== "download" && !canReviewApplication(entry)) throw Error("このモデルは使用できません。配布モデルを選択してください。");
    await apply(entry);
    if (mounted.current) setSelection(current => ({ ...current, [engine]: entry!.packageId }));
    });
  };
  const recheck = () => run("モデルの状態を確認しています", async () => {
    const value = unresolved; if (!value) return;
    await invoke("cancel_model_inspection", { token: value.token });
    const lifetime = assertDownloadLifetime(await invoke("get_model_download_lifetime", { token: value.token }));
    if (lifetime.active || !lifetime.cancelled) throw Error("準備の停止完了を待っています。");
    if (value.stage === "download" && value.option) {
      const result = await assertDownloadReadback(await invoke("read_model_download", { token: crypto.randomUUID(), downloadId: value.token, expectedRecipeHash: value.option.review.recipeHash }), value.token, value.option.review.recipeHash);
      if (result.state !== "complete" || !result.receipt) throw Error("ダウンロードが未完了です。途中のファイルは保持しています。");
      downloads.set(value.option.review.recipeHash, result.receipt);
    }
    const current = await load();
    if (value.stage === "place" && !current.placed.some(item => item.packageId === value.token && item.placementRecorded && item.bodyRemoved !== true)) throw Error("取り込みの完了を確認できません。途中のファイルは保持しています。");
    if (value.stage === "apply" && !current.catalog.find(item => item.name === "standard")?.components.some(item => item.packageId === value.entry?.packageId && item.checkOperationId === value.preview?.checkOperationId && item.settingsMatch && item.compatibilityCurrent)) throw Error("モデル選択の保存を確認できません。現在の設定は変更していません。");
    retain(null);
  });
  const disabled = locked || recordingActive || !!busy || !!pending || !isTauri();
  const installed = entries.filter(item => item.placementRecorded && item.bodyRemoved !== true);
  const selectableInstalled = installed.filter(item => !isRetired(item));
  const selectedProfile = profiles.find(item => item.name === "standard");
  return <section className="analysis-card model-inspection" aria-label="AIモデル">
    <h2>ローカルAI</h2>
    <div className="model-selection-list">{engines.map(({ engine, label }) => {
      const used = selectedProfile?.components.find(item => item.engine === engine);
      const value = selection[engine] ?? (selectableInstalled.some(item => item.packageId === used?.packageId) ? used?.packageId : "") ?? "";
      const entry = selectableInstalled.find(item => item.packageId === value);
      const option = options.find(item => "official:" + item.id === value);
      const available = !!entry || !!option && installed.some(item => item.download?.recipeHash === option.review.recipeHash);
      return <div className="model-selection-row" key={engine}><label>{label}<select aria-label={label + "のモデル"} value={value} disabled={disabled} onChange={event => setSelection(current => ({ ...current, [engine]: event.target.value }))}>
        <option value="">モデルを選択</option>
        {options.filter(item => item.review.manifest.engine === engine).map(item => <option key={item.id} value={"official:" + item.id}>{item.label}</option>)}
        {selectableInstalled.filter(item => item.engine === engine).map(item => <option key={item.packageId} value={item.packageId} disabled={item.sourceType !== "download" && !canReviewApplication(item)}>{item.modelName}</option>)}
      </select></label><button type="button" disabled={disabled || !value || !!entry && entry.sourceType !== "download" && !canReviewApplication(entry) || used?.packageId === value && used.settingsMatch && used.compatibilityCurrent} onClick={() => void useModel(engine, value)}>{available ? "使用" : "ダウンロードして使用"}</button></div>;
    })}</div>
    {!!installed.length && <section className="model-inventory" aria-label="導入済みモデル"><h3>導入済みモデル</h3><ul>{installed.map(item => <li key={item.packageId}><strong>{item.modelName}</strong><span>{isRetired(item) ? "選択対象外" : selectedProfile?.components.some(component => component.packageId === item.packageId && component.settingsMatch && component.compatibilityCurrent) ? "使用中" : "導入済み"}</span></li>)}</ul></section>}
    {busy && <p role="status">{busy}</p>}
    <ModelIssue label="モデルの準備" message={error} />
    {pending && <button type="button" disabled={!!busy || locked || recordingActive} onClick={() => void recheck()}>現在の状態を確認</button>}
  </section>;
}
