import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { usePublishProgress } from "./operation-progress";
import { acceptTransferProgress, modelProgress, type TransferProgress } from "./operation-progress-state";
import { ModelIssue } from "./model-issue";
import { assertCompatibilitySummary, assertPlacementReceipt, placementLabel, type ModelPlacementEntry, type ModelPlacementReceipt, type ModelCompatibilitySummary } from "./model-inspection-state";
import { assertOfficialModelOptions, assertDownloadReceipt, assertDownloadReadback, assertDownloadLifetime, assertDownloadCatalog, assertDownloadCopyResult, type OfficialModelOption, type DownloadReceipt } from "./model-download-state";
import { assertProfileCatalog, assertApplicationPreview, assertApplicationResult, assertAppliedPreferences, canReviewApplication, type ModelApplicationPreview, type ModelProfileView } from "./model-profile-state";
import type { DiarizationPreferences } from "./diarization-state";
import { modelChoices, installedModelGroups, modelDisplayName, isRetiredModel as isRetired, canReuseOfficialModel } from "./model-selection-state";

type Pending = { token: string; startedAtMs: number; stage: "download" | "place" | "check" | "preview" | "apply"; option?: OfficialModelOption; download?: DownloadReceipt; entry?: ModelPlacementEntry; preview?: ModelApplicationPreview };
let unresolved: Pending | null = null;
// A confirmed download is reusable after navigation. No automatic retry or
// fresh download replaces an uncertain writer or a retained download receipt.
const downloads = new Map<string, DownloadReceipt>();
const engines = [{ engine: "whisper.cpp", label: "文字起こし" }, { engine: "sherpa-onnx", label: "話者分離" }, { engine: "llama.cpp", label: "議事録生成" }];
export function ModelInspectionPanel({ locked, recordingActive, refreshEpoch = 0, visible = true, stopRequested }: { locked: boolean; recordingActive: boolean; refreshEpoch?: number; compact?: boolean; visible?: boolean; stopRequested?: boolean }) {
  const [options, setOptions] = useState<OfficialModelOption[]>([]), [entries, setEntries] = useState<ModelPlacementEntry[]>([]), [profiles, setProfiles] = useState<ModelProfileView[]>([]);
  const [selection, setSelection] = useState<Record<string, string>>({});
  const [cpuThreads, setCpuThreads] = useState<number | null>(null), [threadChoice, setThreadChoice] = useState("");
  const [busy, setBusy] = useState(""), [error, setError] = useState(""), [pending, setPending] = useState<Pending | null>(unresolved);
  const [target, setTarget] = useState("ローカルAI"), [transfer, setTransfer] = useState<TransferProgress | null>(null);
  const [waitingToken, setWaitingToken] = useState<string | null>(null);
  const mustStop = recordingActive || (stopRequested ?? locked);
  const mounted = useRef(true), running = useRef(false), blocked = useRef(mustStop);
  blocked.current = mustStop;
  const startedAt = useRef(unresolved?.startedAtMs);
  const progress = busy || pending ? modelProgress(target, pending?.stage, transfer, !busy ? "interrupted" : waitingToken === pending?.token ? "queued" : "running", startedAt.current) : null;
  usePublishProgress("model", [progress]);
  const retain = (value: Pending | null) => { unresolved = value; if (mounted.current) setPending(value); };
  const load = async (publish = true) => {
    const [placed, catalog] = await Promise.all([invoke<ModelPlacementEntry[]>("get_model_placements"), invoke<ModelProfileView[]>("get_model_profiles")]);
    placed.forEach(placementLabel); assertProfileCatalog(catalog);
    if (publish && mounted.current) { setEntries(placed); setProfiles(catalog); }
    return { placed, catalog };
  };
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; if (unresolved) void invoke("cancel_model_inspection", { token: unresolved.token }).catch(() => {}); };
  }, []);
  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    const subscription = listen<{ operationId: string; waiting: boolean }>("model-preparation-wait", event => {
      if (active && unresolved?.token === event.payload.operationId && typeof event.payload.waiting === "boolean") {
        setWaitingToken(event.payload.waiting ? event.payload.operationId : null);
      }
    });
    return () => { active = false; void subscription.then(unlisten => unlisten()).catch(() => {}); };
  }, []);
  useEffect(() => {
    if (visible && isTauri()) void Promise.all([load(false), invoke("get_official_model_options").then(assertOfficialModelOptions)]).then(([current, official]) => {
      if (mounted.current) { setOptions(official); setEntries(current.placed); setProfiles(current.catalog); }
    }).catch(cause => { if (mounted.current) setError(String(cause)); });
  }, [visible]);
  useEffect(() => { if (refreshEpoch && isTauri()) void load().catch(cause => setError(String(cause))); }, [refreshEpoch]);
  const selectedDiarization = profiles.find(item => item.name === "standard")?.components.find(item => item.engine === "sherpa-onnx");
  const profileId = profiles.find(item => item.name === "standard")?.id;
  useEffect(() => {
    let active = true;
    setCpuThreads(null);
    if (visible && isTauri() && selectedDiarization) void invoke<{ lockVersion: number; payload: DiarizationPreferences }>("get_diarization_preferences").then(value => {
      const threads = value?.payload?.standard?.threads;
      if (!Number.isInteger(threads) || threads < 1 || threads > 64) throw Error("話者分離のCPU使用量を確認できません。");
      if (active) { setCpuThreads(threads); setThreadChoice(String(threads)); }
    }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, [visible, profileId, selectedDiarization?.packageId]);
  useEffect(() => { if (mustStop && unresolved) void invoke("cancel_model_inspection", { token: unresolved.token }).catch(() => {}); }, [mustStop]);
  const run = async (label: string, action: () => Promise<void>) => {
    if (running.current || locked || blocked.current || !isTauri()) return;
    startedAt.current = unresolved?.startedAtMs ?? Date.now();
    running.current = true; setBusy(label); setWaitingToken(null); setError("");
    try { await action(); }
    catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { running.current = false; if (mounted.current) { setBusy(""); await load().catch(cause => setError(String(cause))); } }
  };
  const step = async <T,>(stage: Pending["stage"], action: (token: string) => Promise<T>, context: Omit<Pending, "token" | "stage" | "startedAtMs"> = {}): Promise<T> => {
    if (!mounted.current || blocked.current || unresolved) throw Error("モデルの準備を停止しました。現在の状態を確認してください。");
    const token = crypto.randomUUID(); setTransfer(null); setWaitingToken(null); retain({ token, stage, ...context, startedAtMs: startedAt.current! });
    const value = await action(token);
    if (!mounted.current || blocked.current) throw Error("準備の結果を確認してください。");
    retain(null); return value;
  };
  const stop = async (token: string) => {
    await invoke("cancel_model_inspection", { token });
    const lifetime = assertDownloadLifetime(await invoke("get_model_download_lifetime", { token }));
    if (lifetime.active || !lifetime.cancelled) throw Error("準備の停止完了を待っています。");
  };
  const apply = async (entry: ModelPlacementEntry, diarizationThreads?: number) => {
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
      const value = await invoke<ModelApplicationPreview>("preview_model_profile_application", { token, profile: "standard", packageId: entry.packageId, expectedReviewHash: entry.reviewHash, checkOperationId: entry.compatibility!.operationId, ...(diarizationThreads === undefined ? {} : { diarizationThreads }) });
      assertApplicationPreview(value, entry, "standard", current, diarizationThreads); return value;
    }, { entry });
    await step("apply", async token => {
      const value = await invoke<ModelProfileView>("apply_model_profile_application", { token, preview });
      assertApplicationResult(value, token, preview);
      if (diarizationThreads !== undefined) assertAppliedPreferences(await invoke("get_diarization_preferences"), preview);
      return value;
    }, { entry, preview });
  };
  const useModel = (engine: string, value: string) => {
    return run("モデルを準備しています", async () => {
    if (pending || unresolved) throw Error("前の準備の状態を確認してください。");
    const choice = modelChoices(engine, entries, options, profiles).find(item => item.value === value || item.packageIds.includes(value));
    let entry = choice?.entry;
    const option = choice?.option;
    setTarget(choice?.label ?? "ローカルAI");
    if (!entry && option) {
      entry = entries.find(item => item.sourceType === "download" && canReuseOfficialModel(item, option) && item.engine === engine && item.placementRecorded && item.bodyRemoved !== true && !isRetired(item));
      if (!entry) {
        let receipt = downloads.get(option.review.recipeHash);
        if (!receipt) {
          const catalog = assertDownloadCatalog(await invoke("get_model_download_catalog", { token: crypto.randomUUID() }));
          if (!catalog.supported) throw Error("この端末ではモデルをダウンロードできません。");
          const matching = catalog.entries.filter(item => item.recipeHash === option.review.recipeHash)
            .sort((a, b) => Number(b.state === "completion_recorded") - Number(a.state === "completion_recorded"));
          for (const recorded of matching) {
            // Fence delayed starts as well as active workers before deciding a
            // previous attempt is terminal. Partial bytes are never adopted.
            await stop(recorded.operationId);
            const read = await assertDownloadReadback(await invoke("read_model_download", { token: crypto.randomUUID(), downloadId: recorded.operationId, expectedRecipeHash: option.review.recipeHash }), recorded.operationId, option.review.recipeHash);
            if (read.state === "complete" && read.receipt) {
              receipt = read.receipt; downloads.set(option.review.recipeHash, receipt); break;
            }
          }
        }
        if (!receipt) receipt = await step("download", async token => {
          // Subscribe before starting: a fast initial event must not be lost.
          let active = true;
          const unlisten = await listen<unknown>("model-download-progress", event => {
            if (active && mounted.current) setTransfer(current => acceptTransferProgress(current, event.payload, token, option.review.recipeHash, option.review.downloadBytes));
          });
          try {
            if (!mounted.current || blocked.current) throw Error("モデルの準備を停止しました。現在の状態を確認してください。");
            setTransfer(current => current ?? { receivedBytes: 0, totalBytes: option.review.downloadBytes });
            const result = await assertDownloadReceipt(await invoke("start_model_download", { token, definition: option.definition, expectedRecipeHash: option.review.recipeHash }), token, option.review);
            downloads.set(option.review.recipeHash, result); return result;
          } finally { active = false; unlisten(); }
        }, { option });
        const downloaded = receipt;
        const placed = await step("place", async token => {
          const result = await invoke<ModelPlacementReceipt>("place_downloaded_model", { token, downloadId: downloaded.operationId, expectedRecipeHash: downloaded.recipeHash, expectedReviewHash: downloaded.report.reviewHash });
          assertPlacementReceipt(result, token, downloaded.report.reviewHash); return result;
        }, { option, download: downloaded });
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
    await stop(value.token);
    if (value.stage === "download" && value.option) {
      const catalog = assertDownloadCatalog(await invoke("get_model_download_catalog", { token: crypto.randomUUID() }));
      if (!catalog.supported) throw Error("この端末ではモデルの取得状態を確認できません。");
      // Failure before the private staging directory was created has no result
      // to read. A present but malformed directory remains an error, not absent.
      if (catalog.entries.some(item => item.operationId === value.token)) {
        const result = await assertDownloadReadback(await invoke("read_model_download", { token: crypto.randomUUID(), downloadId: value.token, expectedRecipeHash: value.option.review.recipeHash }), value.token, value.option.review.recipeHash);
        if (result.state === "complete" && result.receipt) downloads.set(value.option.review.recipeHash, result.receipt);
      }
    }
    const current = await load();
    if (value.stage === "place") {
      const download = value.download;
      if (!download) throw Error("元の取得結果を確認できません。");
      const result = assertDownloadCopyResult(await invoke("get_downloaded_model_placement_result", { packageId: value.token, downloadId: download.operationId, expectedRecipeHash: download.recipeHash, expectedReviewHash: download.report.reviewHash }), value.token, download.report.reviewHash, download.report.totalBytes);
      if (result.active || !result.cancelled) throw Error("取り込みの停止完了を待っています。");
      if (result.receipt?.placementRecorded && !current.placed.some(item => item.packageId === value.token && item.reviewHash === download.report.reviewHash && item.placementRecorded && item.bodyRemoved !== true)) throw Error("取り込み結果と現在のモデルが一致しません。");
    }
    if (value.stage === "apply" && value.entry && current.catalog.find(item => item.name === "standard")?.components.some(item => item.packageId === value.entry?.packageId && item.checkOperationId === value.preview?.checkOperationId && item.settingsMatch && item.compatibilityCurrent)) {
      if (value.preview?.diarizationThreads !== undefined) {
        const profile = current.catalog.find(item => item.name === "standard");
        if (profile?.id === value.token) assertAppliedPreferences(await invoke("get_diarization_preferences"), value.preview);
        else if (profile?.id !== value.preview.expectedProfileId) throw Error("話者分離の設定が別の操作で変更されています。現在の設定を確認してください。");
      }
      const entry = value.entry;
      if (mounted.current) setSelection(selection => ({ ...selection, [entry.engine]: entry.packageId }));
    }
    // Stopped, unsuccessful operations are not successful model selections.
    // Release the UI for a new explicit action; never replay apply/copy here.
    // The next action loads current settings and builds a fresh bound preview.
    retain(null);
  });
  const disabled = locked || recordingActive || !!busy || !!pending || !isTauri();
  const installed = entries.filter(item => item.placementRecorded && item.bodyRemoved !== true);
  const inventory = installedModelGroups(installed, profiles);
  const selectedProfile = profiles.find(item => item.name === "standard");
  return <section className="analysis-card model-inspection" aria-label="AIモデル" hidden={!visible}>
    <h2>ローカルAI</h2>
    <div className="model-selection-list">{engines.map(({ engine, label }) => {
      const used = selectedProfile?.components.find(item => item.engine === engine);
      const choices = modelChoices(engine, entries, options, profiles);
      const selected = selection[engine] ?? used?.packageId ?? "";
      const choice = choices.find(item => item.value === selected || item.packageIds.includes(selected));
      const value = choice?.value ?? "";
      const entry = choice?.entry;
      const available = !!entry;
      return <div className="model-selection-row" key={engine}><label>{label}<select aria-label={label + "のモデル"} value={value} disabled={disabled} onChange={event => setSelection(current => ({ ...current, [engine]: event.target.value }))}>
        <option value="">モデルを選択</option>
        {choices.map(item => <option key={item.value} value={item.value} disabled={!item.option && !!item.entry && item.entry.sourceType !== "download" && !canReviewApplication(item.entry)}>{item.label}</option>)}
      </select></label><button type="button" disabled={disabled || !value || !!entry && entry.sourceType !== "download" && !canReviewApplication(entry) || !!entry && used?.packageId === entry.packageId && used.settingsMatch && used.compatibilityCurrent} onClick={() => void useModel(engine, value)}>{available ? "使用" : "ダウンロードして使用"}</button></div>;
    })}</div>
    {selectedDiarization && <div className="diarization-performance">
      <div className="model-selection-row"><label>話者分離のCPU使用量<select aria-label="話者分離のCPU使用量" value={threadChoice} disabled={disabled || cpuThreads === null} onChange={event => setThreadChoice(event.target.value)}>
        {cpuThreads !== null && ![4, 8].includes(cpuThreads) && <option value={cpuThreads}>現在の設定（{cpuThreads}スレッド）</option>}
        <option value="4">標準（4スレッド）</option><option value="8">多め（8スレッド）</option>
      </select></label><button type="button" disabled={disabled || cpuThreads === null || !threadChoice || Number(threadChoice) === cpuThreads || !selectedDiarization.settingsMatch || !selectedDiarization.compatibilityCurrent} onClick={() => void run("話者分離の処理設定を保存しています", async () => {
        const entry = entries.find(item => item.packageId === selectedDiarization.packageId);
        if (!entry || !canReviewApplication(entry)) throw Error("使用中の話者モデルを確認できません。");
        setTarget("話者分離"); await apply(entry, Number(threadChoice));
      })}>CPU使用量を適用</button></div>
      <p>同じモデルでCPUの使用量を調整します。変更は次に開始する処理から適用します。</p>
    </div>}
    {!!inventory.length && <section className="model-inventory" aria-label="導入済みモデル"><h3>導入済みモデル</h3><ul>{inventory.map(({ entry: item, entries: group }) => <li key={item.packageId}><strong>{modelDisplayName(item, options)}</strong><span>{isRetired(item) ? "選択対象外" : selectedProfile?.components.some(component => group.some(entry => entry.packageId === component.packageId) && component.settingsMatch && component.compatibilityCurrent) ? "使用中" : "導入済み"}</span></li>)}</ul></section>}
    <ModelIssue label="モデルの準備" message={error} />
    {pending && <button type="button" disabled={!!busy || locked || recordingActive} onClick={() => void recheck()}>現在の状態を確認</button>}
  </section>;
}
