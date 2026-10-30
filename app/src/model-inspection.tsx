import { InfoPopover } from "./info-popover";
import { VoiceMatchingSettings } from "./voice-matching-settings";
import { SpeakerCompletionSettings } from "./speaker-completion-settings";
import type { ModelDeletionControl } from "./model-deletion";
import { OptionsModal } from "./options-modal";
import { bytesLabel } from "./import-state";
import { VadSettings } from "./vad-settings";
import { checkedVadOptions, defaultVadOptions, type VadOptions } from "./vad-state";
import type { AnalysisPreferences, EnginePreferences } from "./analysis-state";
import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { usePublishProgress } from "./operation-progress";
import { acceptTransferProgress, modelProgress, type TransferProgress } from "./operation-progress-state";
import { ModelIssue } from "./model-issue";
import { assertCompatibilitySummary, assertPlacementReceipt, placementLabel, type ModelPlacementEntry, type ModelPlacementReceipt, type ModelCompatibilitySummary } from "./model-inspection-state";
import { assertOfficialModelOptions, assertDownloadReceipt, assertDownloadReadback, assertDownloadLifetime, assertDownloadCatalog, assertDownloadCopyResult, type OfficialModelOption, type DownloadReceipt } from "./model-download-state";
import { assertProfileCatalog, assertApplicationPreview, assertApplicationResult, assertAppliedPreferences, canReviewApplication, type ModelApplicationPreview, type ModelProfileView } from "./model-profile-state";
import type { DiarizationPreferences } from "./diarization-state";
import { modelChoices, modelChoiceAction, modelDisplayName, installedModelGroups, isRetiredModel as isRetired, canReuseOfficialModel, isAsrEngine } from "./model-selection-state";

type Pending = { token: string; startedAtMs: number; stage: "download" | "place" | "check" | "preview" | "apply"; option?: OfficialModelOption; download?: DownloadReceipt; entry?: ModelPlacementEntry; preview?: ModelApplicationPreview };
let unresolved: Pending | null = null;
// A confirmed download is reusable after navigation. No automatic retry or
// fresh download replaces an uncertain writer or a retained download receipt.
const downloads = new Map<string, DownloadReceipt>();
const automaticMergeChoices = [{value:500,label:"従来（分けやすい）"},{value:900,label:"ややまとめる"},{value:1100,label:"まとめる"},{value:1300,label:"強くまとめる"},{value:1500,label:"さらに強くまとめる"}];
const engines = [{ engine: "asr", label: "文字起こし" }, { engine: "sherpa-onnx", label: "話者分離" }, { engine: "llama.cpp", label: "議事録生成" }];
export function ModelInspectionPanel({ locked, recordingActive, refreshEpoch = 0, visible = true, stopRequested, deletion }: { locked: boolean; recordingActive: boolean; refreshEpoch?: number; visible?: boolean; stopRequested?: boolean; deletion?: ModelDeletionControl }) {
  const [options, setOptions] = useState<OfficialModelOption[]>([]), [entries, setEntries] = useState<ModelPlacementEntry[]>([]), [profiles, setProfiles] = useState<ModelProfileView[]>([]);
  const [backends, setBackends] = useState<{ id: string; vocabularyDescription: string; supportsVadOverlap: boolean; defaultVad: import("./vad-state").VadOptions }[]>([]);
  useEffect(() => { if (visible && isTauri()) void invoke<{ id: string; vocabularyDescription: string; supportsVadOverlap: boolean; defaultVad: import("./vad-state").VadOptions }[]>("get_asr_backends").then(setBackends).catch(cause => setError(String(cause))); }, [visible]);
  const [whisperPreferences, setWhisperPreferences] = useState<EnginePreferences | null>(null);
  const [selection, setSelection] = useState<Record<string, string>>({});
  const [cpuThreads, setCpuThreads] = useState<number | null>(null), [threadChoice, setThreadChoice] = useState("");
  const [clusterThreshold, setClusterThreshold] = useState<number | null>(null), [thresholdChoice, setThresholdChoice] = useState("");
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
  useEffect(() => { if (refreshEpoch && isTauri()) { downloads.clear(); setSelection({}); void load().catch(cause => setError(String(cause))); } }, [refreshEpoch]);
  const selectedDiarization = profiles.find(item => item.name === "standard")?.components.find(item => item.engine === "sherpa-onnx" && entries.some(entry => entry.packageId === item.packageId && entry.bodyRemoved !== true));
  const profileId = profiles.find(item => item.name === "standard")?.id;
  useEffect(() => {
    let active = true;
    setCpuThreads(null); setClusterThreshold(null);
    if (visible && isTauri() && selectedDiarization) void invoke<{ lockVersion: number; payload: DiarizationPreferences }>("get_diarization_preferences").then(value => {
      const threads = value?.payload?.standard?.threads;
      if (!Number.isInteger(threads) || threads < 1 || threads > 64) throw Error("話者分離のCPU使用量を確認できません。");
      const threshold = value?.payload?.standard?.clusterThresholdMilli;
      if (!Number.isInteger(threshold) || threshold < 0 || threshold > 2000) throw Error("自動の話者統合の強さを確認できません。");
      if (active) { setCpuThreads(threads); setThreadChoice(String(threads)); setClusterThreshold(threshold); setThresholdChoice(String(threshold)); }
    }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, [visible, profileId, selectedDiarization?.packageId]);
  const selectedWhisper = profiles.find(item => item.name === "standard")?.components.find(item => isAsrEngine(item.engine) && entries.some(entry => entry.packageId === item.packageId && entry.bodyRemoved !== true));
  useEffect(() => {
    let active = true;
    setWhisperPreferences(null);
    if (visible && isTauri() && selectedWhisper) void invoke<{ payload: AnalysisPreferences }>("get_asr_preferences").then(value => {
      const preferences = value?.payload?.standard;
      if (!preferences || !(preferences.vadModelPath === null || typeof preferences.vadModelPath === "string")) throw Error("VAD設定を確認できません。");
      if (preferences.vadOptions !== undefined) checkedVadOptions(preferences.vadOptions);
      if (active) setWhisperPreferences(preferences);
    }).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; };
  }, [visible, profileId, selectedWhisper?.packageId]);
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
  const apply = async (entry: ModelPlacementEntry, diarizationThreads?: number, diarizationClusterThresholdMilli?: number, whisperVadOptions?: VadOptions) => {
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
      const value = await invoke<ModelApplicationPreview>("preview_model_profile_application", { token, profile: "standard", packageId: entry.packageId, expectedReviewHash: entry.reviewHash, checkOperationId: entry.compatibility!.operationId, ...(whisperVadOptions === undefined ? {} : { whisperVadOptions }), ...(diarizationThreads === undefined ? {} : { diarizationThreads }), ...(diarizationClusterThresholdMilli === undefined ? {} : { diarizationClusterThresholdMilli }) });
      assertApplicationPreview(value, entry, "standard", current, diarizationThreads, diarizationClusterThresholdMilli, whisperVadOptions); return value;
    }, { entry });
    await step("apply", async token => {
      const value = await invoke<ModelProfileView>("apply_model_profile_application", { token, preview });
      assertApplicationResult(value, token, preview);
      if (diarizationThreads !== undefined || diarizationClusterThresholdMilli !== undefined) assertAppliedPreferences(await invoke("get_diarization_preferences"), preview);
      if (whisperVadOptions !== undefined) assertAppliedPreferences(await invoke("get_asr_preferences"), preview);
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
      entry = entries.find(item => item.sourceType === "download" && canReuseOfficialModel(item, option) && (engine === "asr" ? isAsrEngine(item.engine) : item.engine === engine) && item.placementRecorded && item.bodyRemoved !== true && !isRetired(item));
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
      if (value.preview && (value.preview.diarizationThreads !== undefined || value.preview.diarizationClusterThresholdMilli !== undefined)) {
        const profile = current.catalog.find(item => item.name === "standard");
        if (profile?.id === value.token) assertAppliedPreferences(await invoke("get_diarization_preferences"), value.preview);
        else if (profile?.id !== value.preview.expectedProfileId) throw Error("話者分離の設定が別の操作で変更されています。現在の設定を確認してください。");
      }
      if (value.preview?.whisperVadOptions !== undefined && current.catalog.find(item => item.name === "standard")?.id === value.token) assertAppliedPreferences(await invoke("get_asr_preferences"), value.preview);
      const entry = value.entry;
      if (mounted.current) setSelection(selection => ({ ...selection, [entry.engine]: entry.packageId }));
    }
    // Stopped, unsuccessful operations are not successful model selections.
    // Release the UI for a new explicit action; never replay apply/copy here.
    // The next action loads current settings and builds a fresh bound preview.
    retain(null);
  });
  const disabled = locked || recordingActive || !!busy || !!pending || !isTauri();
  const installed = installedModelGroups(entries, profiles);

  const selectedProfile = profiles.find(item => item.name === "standard");
  const copyStatus = (item: ModelPlacementEntry) => isRetired(item) ? "選択対象外" : selectedProfile?.components.some(component => component.packageId === item.packageId) ? "使用中" : "導入済み";
  const purposeOf = (item: ModelPlacementEntry) => engines.find(e => e.engine === "asr" ? isAsrEngine(item.engine) : e.engine === item.engine)?.label ?? item.engine;
  const renderCopy = (item: ModelPlacementEntry, grouped = false) => <InstalledModelCopy key={item.packageId} item={item} name={modelDisplayName(item, options)} status={copyStatus(item)} purpose={purposeOf(item)} grouped={grouped} disabled={disabled} deletion={deletion} />;
  return <><section className="analysis-card model-inspection" aria-label="AIモデル" hidden={!visible}>
    <h2>ローカルAI</h2>
    <div className="model-selection-list">{engines.map(({ engine, label }) => {
      const used = selectedProfile?.components.find(item => engine === "asr" ? isAsrEngine(item.engine) : item.engine === engine);
      const choices = modelChoices(engine, entries, options, profiles);
      const selected = selection[engine] ?? used?.packageId ?? "";
      const choice = choices.find(item => item.value === selected || item.packageIds.includes(selected));
      const value = choice?.value ?? "";
      const entry = choice?.entry;
      const action = modelChoiceAction(choice, entries, used);
      const actionLabel = { in_use: "使用中", add_vad: "VADを追加して使用", use: "使用", update_runtime: "実行環境を更新して使用", download: "ダウンロードして使用" }[action];
      return <div className="model-selection-row" key={engine}><label>{label}<select aria-label={label + "のモデル"} value={value} disabled={disabled} onChange={event => setSelection(current => ({ ...current, [engine]: event.target.value }))}>
        <option value="">モデルを選択</option>
        {choices.map(item => <option key={item.value} value={item.value} disabled={!item.option && !!item.entry && item.entry.sourceType !== "download" && !canReviewApplication(item.entry)}>{item.label}</option>)}
      </select></label><button type="button" disabled={disabled || !value || !!entry && entry.sourceType !== "download" && !canReviewApplication(entry) || action === "in_use"} onClick={() => void useModel(engine, value)}>{actionLabel}</button></div>;
    })}</div>
    <ModelIssue label="モデルの準備" message={error} />
    {pending && <button type="button" disabled={!!busy || locked || recordingActive} onClick={() => void recheck()}>現在の状態を確認</button>}
  </section>
  <section className="analysis-card local-ai-settings" aria-label="ローカルAI詳細設定" hidden={!visible}>
    <h2>ローカルAI詳細設定</h2>
    <section className="ai-settings-group" aria-label="発話検知設定"><h3>発話検知設定</h3>
    {whisperPreferences && <p>{backends.find(b => b.id === (whisperPreferences.backend?.id ?? "whisper.cpp"))?.vocabularyDescription}</p>}
    {selectedWhisper && whisperPreferences && <VadSettings
      value={whisperPreferences.vadOptions ?? { ...defaultVadOptions, enabled: !!whisperPreferences.vadModelPath }}
      overlapAvailable={backends.find(b => b.id === (whisperPreferences.backend?.id ?? "whisper.cpp"))?.supportsVadOverlap ?? true}
      defaults={backends.find(b => b.id === (whisperPreferences.backend?.id ?? "whisper.cpp"))?.defaultVad ?? defaultVadOptions}
      available={!!whisperPreferences.vadModelPath}
      disabled={disabled || !selectedWhisper.settingsMatch || !selectedWhisper.compatibilityCurrent}
      onApply={options => void run("VAD設定を保存しています", async () => {
        const entry = entries.find(item => item.packageId === selectedWhisper.packageId);
        if (!entry || !canReviewApplication(entry)) throw Error("使用中の文字起こしモデルを確認できません。");
        setTarget("発話区間検知（VAD）"); await apply(entry, undefined, undefined, options);
      })} />}
    {(!selectedWhisper || !whisperPreferences) && <p>文字起こしモデルを選択すると設定できます。読込エラーは「ローカルAI」を確認してください。</p>}
    </section>
    <section className="ai-settings-group" aria-label="話者分離のCPU使用量"><div className="heading-with-info"><h3>話者分離のCPU使用量</h3><InfoPopover label="話者分離のCPU使用量">同じモデルでCPUの使用量を調整します。変更は次に開始する処理から適用します。</InfoPopover></div>
    {selectedDiarization ? <div className="diarization-performance">
      <div className="model-selection-row"><label>話者分離のCPU使用量<select aria-label="話者分離のCPU使用量" value={threadChoice} disabled={disabled || cpuThreads === null} onChange={event => setThreadChoice(event.target.value)}>
        {cpuThreads !== null && ![4, 8].includes(cpuThreads) && <option value={cpuThreads}>現在の設定（{cpuThreads}スレッド）</option>}
        <option value="4">標準（4スレッド）</option><option value="8">多め（8スレッド）</option>
      </select></label><button type="button" disabled={disabled || cpuThreads === null || !threadChoice || Number(threadChoice) === cpuThreads || !selectedDiarization.settingsMatch || !selectedDiarization.compatibilityCurrent} onClick={() => void run("話者分離の処理設定を保存しています", async () => {
        const entry = entries.find(item => item.packageId === selectedDiarization.packageId);
        if (!entry || !canReviewApplication(entry)) throw Error("使用中の話者モデルを確認できません。");
        setTarget("話者分離"); await apply(entry, Number(threadChoice));
      })}>CPU使用量を適用</button></div>
    </div> : <p>話者分離モデルを選択すると設定できます。</p>}
    </section>
    <section className="ai-settings-group" aria-label="自動の話者統合"><div className="heading-with-info"><h3>自動の話者統合</h3><InfoPopover label="自動の話者統合">人数が「自動」のときのまとめやすさを調整します。強くすると話者の増えすぎを抑えられますが、別人を同じ話者にまとめる場合があります。次に開始する処理から適用します。</InfoPopover></div>
    {selectedDiarization ? <div className="diarization-performance">
      <div className="model-selection-row"><label>自動の話者統合<select aria-label="自動の話者統合" value={thresholdChoice} disabled={disabled || clusterThreshold === null} onChange={event => setThresholdChoice(event.target.value)}>
        {clusterThreshold !== null && !automaticMergeChoices.some(choice => choice.value === clusterThreshold) && <option value={clusterThreshold}>現在の設定（{clusterThreshold / 1000}）</option>}
        {automaticMergeChoices.map(choice => <option key={choice.value} value={choice.value}>{choice.label}</option>)}
      </select></label><button type="button" disabled={disabled || clusterThreshold === null || !thresholdChoice || Number(thresholdChoice) === clusterThreshold || !selectedDiarization.settingsMatch || !selectedDiarization.compatibilityCurrent} onClick={() => void run("自動の話者統合の強さを保存しています", async () => {
        const entry = entries.find(item => item.packageId === selectedDiarization.packageId);
        if (!entry || !canReviewApplication(entry)) throw Error("使用中の話者モデルを確認できません。");
        setTarget("自動の話者統合"); await apply(entry, undefined, Number(thresholdChoice));
      })}>統合の強さを適用</button></div>
    </div> : <p>話者分離モデルを選択すると設定できます。</p>}
    </section>
    <VoiceMatchingSettings embedded locked={disabled} />
    <SpeakerCompletionSettings embedded locked={disabled} />
  </section>
  <section className="analysis-card model-inventory" aria-label="導入済みモデル管理" hidden={!visible}>
    <h2 tabIndex={-1}>導入済みモデル管理</h2>
    {!installed.length && <p>導入済みモデルはありません。「ローカルAI」からモデルを導入できます。</p>}
    <ul>{installed.map(group => {
      if (group.entries.length === 1) return <li key={group.key}>{renderCopy(group.entry)}</li>;
      const status = group.entries.some(item => copyStatus(item) === "使用中") ? "使用中" : group.entries.every(isRetired) ? "選択対象外" : "導入済み";
      return <li key={group.key} className="model-inventory-group"><div className="model-inventory-summary"><strong>{modelDisplayName(group.entry, options)}</strong>
        <span>{purposeOf(group.entry)} · 合計 {bytesLabel(group.totalBytes)} · {group.entries.length}コピー · {status}</span></div>
        <details className="model-copies"><summary>コピーを管理（{group.entries.length}件）</summary><div role="group" aria-label={`${modelDisplayName(group.entry, options)}のコピー`}>
          {group.entries.map(item => renderCopy(item, true))}
        </div></details>
      </li>;
    })}</ul>
    {installed.length > 0 && (!deletion?.canReviewModel || disabled) && <div id="model-deletion-unavailable" role="status"><p>{recordingActive ? "録音・音声保存が完了すると削除できます。" : busy ? "モデル準備が完了すると削除できます。" : pending ? "モデル準備の受付結果を「現在の状態を確認」で確認後に削除できます。" : deletion?.reviewUnavailableReason ?? "ほかの操作が完了すると削除できます。"}</p></div>}
    {deletion?.listing?.items.map(item => <div key={item.operationId} className="model-deletion-pending"><p>{item.modelName ?? "モデル"}：削除の確認が必要です。{item.errorMessage}</p>
      {item.reviewAllowed && <button type="button" className="small-secondary" disabled={!deletion.canReviewPending} onClick={() => void deletion.reviewPending(item.operationId)}>残りの削除を確認</button>}</div>)}
    {deletion?.protectedNow && <p role="status">モデル削除の受付結果を確認中です。</p>}
    {deletion?.unknown && <button type="button" disabled={deletion.rpcBusy} onClick={() => void deletion.retryUnknown()}>同じ削除操作の結果を確認</button>}
  </section></>;
}

function InstalledModelCopy({ item, name, status, purpose, grouped, disabled, deletion }: {
  item: ModelPlacementEntry; name: string; status: string; purpose: string; grouped: boolean; disabled: boolean; deletion?: ModelDeletionControl;
}) {
  return <div className="model-copy" data-model-copy={item.packageId}><div className="model-inventory-summary">
    {!grouped && <strong>{name}</strong>}<span>{purpose} · {bytesLabel(item.totalBytes)} · {status}</span>
    <span className="model-copy-id">コピー {item.packageId.slice(0, 8)} · {item.engineVersion}</span></div>
    <div className="model-inventory-actions"><OptionsModal label={`${name}（${item.packageId.slice(0, 8)}）の詳細`} triggerLabel="詳細" title="モデルの詳細" className="model-details">
      <h3>{name}</h3><dl className="import-info"><dt>用途</dt><dd>{purpose}</dd>
        <dt>モデル名</dt><dd>{item.modelName || "不明"}</dd><dt>バージョン</dt><dd>{item.modelVersion || "不明"}</dd>
        <dt>量子化</dt><dd>{item.quantization || "不明"}</dd><dt>エンジン</dt><dd>{item.engine || "不明"} · {item.engineVersion || "不明"}</dd>
        <dt>発話検知モデル</dt><dd>{item.modelFiles?.some(file => file.role === "vad") ? "あり" : "なし"}</dd>
        <dt>容量</dt><dd>{bytesLabel(item.totalBytes)}</dd><dt>取得元</dt><dd>{item.sourceType === "download" ? "ダウンロード" : item.sourceType === "manual_import" ? "ローカルからの取り込み" : "不明"}</dd>
        <dt>選択状態</dt><dd>{status}</dd><dt>互換性確認</dt><dd>{placementLabel(item)}</dd>
        <dt>パッケージID</dt><dd className="data-path">{item.packageId}</dd></dl>
    </OptionsModal><button type="button" className="small-secondary deletion-trigger" aria-describedby={(!deletion?.canReviewModel || disabled) ? "model-deletion-unavailable" : undefined} aria-label={`${name}（${item.packageId.slice(0, 8)}）をローカルから削除`} disabled={!deletion?.canReviewModel || disabled} onClick={() => void deletion?.reviewModel(item.packageId, item.reviewHash)}>ローカルから削除</button></div>
  </div>;
}
