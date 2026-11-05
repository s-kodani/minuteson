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
import { modelPreparationError } from "./model-preparation-error";
import { assertCompatibilitySummary, assertPlacementReceipt, placementLabel, type ModelPlacementEntry, type ModelPlacementReceipt, type ModelCompatibilitySummary } from "./model-inspection-state";
import { assertOfficialModelOptions, assertDownloadReceipt, assertDownloadReadback, assertDownloadLifetime, assertDownloadCatalog, assertDownloadCopyResult, type OfficialModelOption, type DownloadReceipt } from "./model-download-state";
import { assertProfileCatalog, assertApplicationPreview, assertApplicationResult, assertAppliedPreferences, canReviewApplication, type ModelApplicationPreview, type ModelProfileView } from "./model-profile-state";
import type { DiarizationPreferences } from "./diarization-state";
import { modelChoices, modelChoiceAction, modelDisplayName, installedModelGroups, canReuseOfficialModel, isAsrEngine } from "./model-selection-state";
import { modelVocabularySupport } from "./model-vocabulary-support";
import { ToastNotice } from "./toast";
import { ModelLicenseDialog } from "./model-license";
import { assertModelLicenseReview, sameModelLicenseTarget, type ModelLicenseReview, type ModelLicenseSummary, type ModelLicenseTarget } from "./model-license-state";
import type { ModelChoice } from "./model-selection-state";

type PlacementInput = { packageId: string; downloadId: string; recipeHash: string; reviewHash: string; totalBytes: number };
type Pending = { token: string; startedAtMs: number; stage: "download" | "place" | "check" | "preview" | "apply"; option?: OfficialModelOption; download?: DownloadReceipt; placement?: PlacementInput; verifying?: boolean; entry?: ModelPlacementEntry; preview?: ModelApplicationPreview };
type LicenseRequest = { id: number; engine: string; value: string; target: ModelLicenseTarget; summary?: ModelLicenseSummary; actionLabel: string; confirmation: boolean; review: ModelLicenseReview | null; loading: boolean; error: string; acknowledged: boolean; confirming: boolean; opener: HTMLElement | null };
function licenseTarget(choice: ModelChoice): ModelLicenseTarget | null {
  return choice.option ? { recipeHash: choice.option.review.recipeHash } : choice.entry ? { packageId: choice.entry.packageId, expectedReviewHash: choice.entry.reviewHash } : null;
}
class ModelPreparationWaiting extends Error {}
let unresolved: Pending | null = null;
// A confirmed download is reusable after navigation. No automatic retry or
// fresh download replaces an uncertain writer or a retained download receipt.
const downloads = new Map<string, DownloadReceipt>();
const automaticMergeChoices = [{value:500,label:"従来（分けやすい）"},{value:900,label:"ややまとめる"},{value:1100,label:"まとめる"},{value:1300,label:"強くまとめる"},{value:1500,label:"さらに強くまとめる"}];
const engines = [{ engine: "asr", label: "文字起こし" }, { engine: "sherpa-onnx", label: "話者分離" }, { engine: "llama.cpp", label: "議事録生成" }];
export function ModelInspectionPanel({ locked, recordingActive, refreshEpoch = 0, visible = true, stopRequested, deletion }: { locked: boolean; recordingActive: boolean; refreshEpoch?: number; visible?: boolean; stopRequested?: boolean; deletion?: ModelDeletionControl }) {
  const [options, setOptions] = useState<OfficialModelOption[]>([]), [entries, setEntries] = useState<ModelPlacementEntry[]>([]), [profiles, setProfiles] = useState<ModelProfileView[]>([]);
  const [backends, setBackends] = useState<{ id: string; vocabularyDescription: string; supportsVadOverlap: boolean; defaultVad: import("./vad-state").VadOptions }[]>([]);
  useEffect(() => { if (visible && isTauri()) void invoke<{ id: string; vocabularyDescription: string; supportsVadOverlap: boolean; defaultVad: import("./vad-state").VadOptions }[]>("get_asr_backends").then(setBackends).catch(cause => { setBackends([]); setError(modelPreparationError(cause)); }); }, [visible]);
  const [whisperPreferences, setWhisperPreferences] = useState<EnginePreferences | null>(null);
  const [selection, setSelection] = useState<Record<string, string>>(() => {
    const option = unresolved?.option;
    return option ? { [isAsrEngine(option.review.manifest.engine) ? "asr" : option.review.manifest.engine]: `official:${option.id}` } : {};
  });
  const [cpuThreads, setCpuThreads] = useState<number | null>(null), [threadChoice, setThreadChoice] = useState("");
  const [clusterThreshold, setClusterThreshold] = useState<number | null>(null), [thresholdChoice, setThresholdChoice] = useState("");
  const [busy, setBusy] = useState(""), [error, setError] = useState(""), [pending, setPending] = useState<Pending | null>(unresolved);
  const [recovering, setRecovering] = useState(false);
  const [license, setLicense] = useState<LicenseRequest | null>(null);
  const [runtimeNotice,setRuntimeNotice] = useState<{id:string;message:string}|null>(null);
  const licenseRequest = useRef<LicenseRequest | null>(null), licenseSequence = useRef(0);
  const latest = useRef({ visible, locked, recordingActive, options, entries, profiles, selection });
  latest.current = { visible, locked, recordingActive, options, entries, profiles, selection };
  const updateLicense = (id: number, patch: Partial<LicenseRequest>) => {
    if (!mounted.current || licenseRequest.current?.id !== id) return;
    licenseRequest.current = { ...licenseRequest.current, ...patch }; setLicense(licenseRequest.current);
  };
  const closeLicense = () => { licenseRequest.current = null; setLicense(null); };
  const failure = useRef<unknown>(null);
  const [target, setTarget] = useState(unresolved?.option?.label ?? unresolved?.entry?.modelName ?? "ローカルAI"), [transfer, setTransfer] = useState<TransferProgress | null>(null);
  const [waitingToken, setWaitingToken] = useState<string | null>(null);
  const mustStop = recordingActive || (stopRequested ?? locked);
  const mounted = useRef(true), running = useRef(false), blocked = useRef(mustStop);
  blocked.current = mustStop;
  const startedAt = useRef(unresolved?.startedAtMs);
  const progress = busy || pending ? modelProgress(target, pending?.stage === "place" && pending.verifying ? "place_verify" : pending?.stage, transfer, !busy || recovering ? "recovering" : waitingToken === pending?.token ? "queued" : "running", startedAt.current) : null;
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
    return () => { mounted.current = false; licenseRequest.current = null; if (unresolved) void invoke("cancel_model_inspection", { token: unresolved.token }).catch(() => {}); };
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
    }).catch(cause => { if (mounted.current) setError(modelPreparationError(cause)); });
  }, [visible]);
  useEffect(() => { if (refreshEpoch && isTauri()) { downloads.clear(); setSelection({}); void load().catch(cause => setError(modelPreparationError(cause))); } }, [refreshEpoch]);
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
    }).catch(cause => { if (active) setError(modelPreparationError(cause)); });
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
    }).catch(cause => { if (active) setError(modelPreparationError(cause)); });
    return () => { active = false; };
  }, [visible, profileId, selectedWhisper?.packageId]);
  useEffect(() => { if (mustStop && unresolved) void invoke("cancel_model_inspection", { token: unresolved.token }).catch(() => {}); }, [mustStop]);
  useEffect(() => { if (!visible || locked || recordingActive || mustStop) closeLicense(); }, [visible, locked, recordingActive, mustStop]);
  const run = async (label: string, action: () => Promise<void>) => {
    if (running.current || locked || blocked.current || !isTauri()) return;
    startedAt.current = unresolved?.startedAtMs ?? Date.now();
    running.current = true; failure.current = null; setBusy(label); setWaitingToken(null); setError("");
    try { await action(); return true; }
    catch (cause) { if (unresolved) failure.current = cause; else if (mounted.current) setError(modelPreparationError(cause)); return false; }
    finally { if (mounted.current) { await load().catch(cause => { if (!unresolved) setError(modelPreparationError(cause)); }); setBusy(""); } running.current = false; }
  };
  const step = async <T,>(stage: Pending["stage"], action: (token: string) => Promise<T>, context: Omit<Pending, "token" | "stage" | "startedAtMs"> = {}): Promise<T> => {
    if (!mounted.current || blocked.current || unresolved) throw Error("モデルの準備を中断しました。");
    const token = crypto.randomUUID(); setTransfer(null); setWaitingToken(null); retain({ token, stage, ...context, startedAtMs: startedAt.current! });
    const value = await action(token);
    if (!mounted.current || blocked.current) throw Error("モデルの準備を中断しました。");
    retain(null); return value;
  };
  const stop = async (token: string) => {
    await invoke("cancel_model_inspection", { token });
    const lifetime = assertDownloadLifetime(await invoke("get_model_download_lifetime", { token }));
    if (lifetime.active) throw new ModelPreparationWaiting("モデルの準備を完了できませんでした。もう一度お試しください。");
    if (!lifetime.cancelled) throw Error("モデルの準備を停止できませんでした。");
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
  const placeModel = async (placement: PlacementInput, option: OfficialModelOption) => step("place", async token => {
    let active = true;
    const unlisten = await listen<{operationId: string; recipeHash: string; phase: string}>("model-placement-progress", event => {
      if (active && mounted.current && event.payload.operationId === token && event.payload.recipeHash === placement.recipeHash && event.payload.phase === "verifying" && unresolved?.token === token) retain({ ...unresolved, verifying: true });
    });
    try {
      const result = await invoke<ModelPlacementReceipt>("place_downloaded_model", { token, packageId: placement.packageId, downloadId: placement.downloadId, expectedRecipeHash: placement.recipeHash, expectedReviewHash: placement.reviewHash });
      assertPlacementReceipt(result, placement.packageId, placement.reviewHash); return result;
    } finally { active = false; unlisten(); }
  }, { option, placement });
  const executeModelUse = (engine: string, choice: ModelChoice, candidateEntries: ModelPlacementEntry[]) => {
    return run("モデルを準備しています", async () => {
    if (pending || unresolved) throw Error("モデルの準備が終わるまでお待ちください。");
    let entry = choice.entry;
    const option = choice.option;
    setTarget(choice.label);
    if (!entry && option) {
      entry = candidateEntries.find(item => item.sourceType === "download" && canReuseOfficialModel(item, option) && (engine === "asr" ? isAsrEngine(item.engine) : item.engine === engine) && item.placementRecorded && item.bodyRemoved !== true);
      if (!entry) {
        let receipt = downloads.get(option.review.recipeHash);
        {
          // A cached receipt can now refer to a payload already moved by an
          // interrupted placement. Resolve its persistent placement first.
          const catalog = assertDownloadCatalog(await invoke("get_model_download_catalog", { token: crypto.randomUUID() }));
          if (!catalog.supported) throw Error("この端末ではモデルをダウンロードできません。");
          const matching = catalog.entries.filter(item => item.recipeHash === option.review.recipeHash)
            .sort((a, b) => Number(b.placement?.state === "pending") - Number(a.placement?.state === "pending") || Number(b.state === "completion_recorded") - Number(a.state === "completion_recorded"));
          for (const recorded of matching) {
            // Fence delayed starts as well as active workers before deciding a
            // previous attempt is terminal. Partial bytes are never adopted.
            try { await stop(recorded.operationId); }
            catch (cause) {
              if (cause instanceof ModelPreparationWaiting) retain({ token: recorded.operationId, startedAtMs: startedAt.current!, stage: "download", option });
              throw cause;
            }
            if (recorded.placement) {
              if (recorded.placement.state === "committed") {
                if (receipt?.operationId === recorded.operationId) { receipt = undefined; downloads.delete(option.review.recipeHash); }
                continue;
              }
              const placed = await placeModel({ packageId: recorded.placement.packageId, downloadId: recorded.operationId, recipeHash: option.review.recipeHash, reviewHash: option.review.packageReviewHash, totalBytes: option.review.totalBytes }, option);
              entry = (await load()).placed.find(item => item.packageId === placed.packageId);
              if (!entry) throw Error("再開したモデルの配置結果がありません。");
              break;
            }
            if (receipt?.operationId === recorded.operationId) break;
            const read = await assertDownloadReadback(await invoke("read_model_download", { token: crypto.randomUUID(), downloadId: recorded.operationId, expectedRecipeHash: option.review.recipeHash }), recorded.operationId, option.review.recipeHash);
            if (read.state === "complete" && read.receipt) {
              receipt = read.receipt; downloads.set(option.review.recipeHash, receipt); break;
            }
          }
        }
        if (!entry) {
          if (!receipt) receipt = await step("download", async token => {
            // Subscribe before starting: a fast initial event must not be lost.
            let active = true;
            const unlisten = await listen<unknown>("model-download-progress", event => {
              if (active && mounted.current) setTransfer(current => acceptTransferProgress(current, event.payload, token, option.review.recipeHash, option.review.downloadBytes));
            });
            try {
              if (!mounted.current || blocked.current) throw Error("モデルの準備を中断しました。");
              setTransfer(current => current ?? { receivedBytes: 0, totalBytes: option.review.downloadBytes });
              const result = await assertDownloadReceipt(await invoke("start_model_download", { token, definition: option.definition, expectedRecipeHash: option.review.recipeHash }), token, option.review);
              downloads.set(option.review.recipeHash, result); return result;
            } finally { active = false; unlisten(); }
          }, { option });
          const downloaded = receipt;
          const placed = await placeModel({ packageId: crypto.randomUUID(), downloadId: downloaded.operationId, recipeHash: downloaded.recipeHash, reviewHash: downloaded.report.reviewHash, totalBytes: downloaded.report.totalBytes }, option);
          entry = (await load()).placed.find(item => item.packageId === placed.packageId);
        }
      }
    }
    if (!entry) throw Error("使用するモデルを選択してください。");
    if (entry.sourceType !== "download" && !canReviewApplication(entry)) throw Error("このモデルは使用できません。配布モデルを選択してください。");
    await apply(entry);
    if (mounted.current) setSelection(current => ({ ...current, [engine]: entry!.packageId }));
    });
  };
  const currentLicenseChoice = (request: Pick<LicenseRequest, "engine" | "value" | "target">, current = latest.current) => {
    const used = current.profiles.find(profile => profile.name === "standard")?.components.find(item => request.engine === "asr" ? isAsrEngine(item.engine) : item.engine === request.engine);
    const selected = current.selection[request.engine] ?? used?.packageId ?? "";
    const choice = modelChoices(request.engine, current.entries, current.options, current.profiles).find(item => item.value === selected || item.packageIds.includes(selected));
    const target = choice && licenseTarget(choice);
    if (!choice || choice.value !== request.value || !target || !sameModelLicenseTarget(target, request.target)) throw Error("モデルの選択内容が変わりました。利用条件を開き直してください。");
    return { choice, used };
  };
  const canConfirmLicense = () => mounted.current && latest.current.visible && !latest.current.locked && !latest.current.recordingActive && !blocked.current && !running.current && !unresolved && isTauri();
  const useModel = async (engine: string, value: string, confirmation = true, actionLabel = "使用", opener?: HTMLElement) => {
    if (!canConfirmLicense() || licenseRequest.current) return;
    const choice = modelChoices(engine, entries, options, profiles).find(item => item.value === value || item.packageIds.includes(value));
    const target = choice && licenseTarget(choice);
    if (!choice || !target) return;
    const request: LicenseRequest = { id: ++licenseSequence.current, engine, value: choice.value, target, summary: choice.option?.licenseSummary, actionLabel, confirmation,
      review: null, loading: true, error: "", acknowledged: false, confirming: false, opener: opener ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null) };
    licenseRequest.current = request; setLicense(request);
    try {
      const review = await assertModelLicenseReview(await invoke("get_model_license_review", { target }), target, request.summary);
      if (!canConfirmLicense() || licenseRequest.current?.id !== request.id) return;
      currentLicenseChoice(request);
      updateLicense(request.id, { review, loading: false });
    } catch (cause) { updateLicense(request.id, { loading: false, error: modelPreparationError(cause) }); }
  };
  const confirmLicense = async () => {
    const request = licenseRequest.current;
    if (!request?.confirmation || !request.acknowledged || !request.review || request.loading || request.error || request.confirming || !canConfirmLicense()) return;
    updateLicense(request.id, { confirming: true });
    try {
      currentLicenseChoice(request);
      // Review is read-only. Re-read selection, package and policy before any
      // operation token, compatibility process, download or profile mutation.
      const [current, official] = await Promise.all([load(false), invoke("get_official_model_options").then(assertOfficialModelOptions)]);
      if (!canConfirmLicense() || licenseRequest.current?.id !== request.id) return;
      const { choice, used } = currentLicenseChoice(request, { ...latest.current, options: official, entries: current.placed, profiles: current.catalog });
      if (modelChoiceAction(choice, current.placed, used) === "in_use") throw Error("このモデルはすでに使用中です。現在の設定を確認してください。");
      if (choice.entry && choice.entry.sourceType !== "download" && !canReviewApplication(choice.entry)) throw Error("このモデルは使用できません。配布モデルを選択してください。");
      const review = await assertModelLicenseReview(await invoke("get_model_license_review", { target: request.target }), request.target, choice.option?.licenseSummary);
      if (!canConfirmLicense() || licenseRequest.current?.id !== request.id) return;
      currentLicenseChoice(request);
      if (review.disclosureHash !== request.review.disclosureHash) throw Error("モデルの利用条件が変わりました。利用条件を開き直してください。");
      const runtimeUpdate = request.engine === "llama.cpp" && modelChoiceAction(choice,current.placed,used) === "update_runtime";
      setOptions(official); setEntries(current.placed); setProfiles(current.catalog);
      closeLicense();
      const succeeded = await executeModelUse(request.engine, choice, current.placed);
      if (succeeded && runtimeUpdate) setRuntimeNotice({id:crypto.randomUUID(),message:"議事録生成エンジンを更新しました。"});
    } catch (cause) { updateLicense(request.id, { confirming: false, acknowledged: false, error: modelPreparationError(cause) }); }
  };
  const recheck = async (): Promise<"waiting" | "resolved" | "applied"> => {
    const value = unresolved; if (!value) return "resolved";
    try { await stop(value.token); }
    catch (cause) { if (cause instanceof ModelPreparationWaiting) return "waiting"; throw cause; }
    let applied = false;
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
      const placement = value.placement ?? (value.download && { packageId: value.token, downloadId: value.download.operationId, recipeHash: value.download.recipeHash, reviewHash: value.download.report.reviewHash, totalBytes: value.download.report.totalBytes });
      if (!placement) throw Error("元の取得結果を確認できません。");
      const result = assertDownloadCopyResult(await invoke("get_downloaded_model_placement_result", { token: value.token, packageId: placement.packageId, downloadId: placement.downloadId, expectedRecipeHash: placement.recipeHash, expectedReviewHash: placement.reviewHash }), placement.packageId, placement.reviewHash, placement.totalBytes);
      if (result.active || !result.cancelled) throw Error("取り込みの停止完了を待っています。");
      if (result.receipt?.placementRecorded && !current.placed.some(item => item.packageId === placement.packageId && item.reviewHash === placement.reviewHash && item.placementRecorded && item.bodyRemoved !== true)) throw Error("取り込み結果と現在のモデルが一致しません。");
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
      applied = true;
    }
    // Stopped, unsuccessful operations are not successful model selections.
    // Release the UI for a new explicit action; never replay apply/copy here.
    // The next action loads current settings and builds a fresh bound preview.
    retain(null);
    return applied ? "applied" : "resolved";
  };
  useEffect(() => {
    if (!pending || mustStop || !isTauri()) return;
    let active = true, timer: number | undefined, failures = 0;
    const schedule = (delay = 1000) => { if (active) timer = window.setTimeout(attempt, delay); };
    const attempt = async () => {
      if (!active || !mounted.current) return;
      if (running.current || blocked.current) { schedule(); return; }
      running.current = true; setRecovering(true); setBusy("再試行の準備をしています");
      try {
        const result = await recheck();
        failures = 0;
        if (mounted.current) setError(result === "resolved"
          ? modelPreparationError(failure.current ?? "モデルの準備を完了できませんでした。もう一度お試しください。")
          : "");
        if (result !== "waiting") failure.current = null;
      } catch {
        failures++;
        if (failures >= 3 && mounted.current) setError("モデルの準備を完了できません。アプリを再起動してください。");
      } finally {
        running.current = false;
        if (mounted.current) { setRecovering(false); setBusy(""); }
        if (unresolved) schedule(failures >= 3 ? 5000 : 1000);
      }
    };
    void attempt();
    return () => { active = false; if (timer !== undefined) window.clearTimeout(timer); };
  }, [pending, mustStop]);
  const disabled = locked || recordingActive || !!busy || !!pending || !isTauri();
  const installed = installedModelGroups(entries, profiles);

  const selectedProfile = profiles.find(item => item.name === "standard");
  const copyStatus = (item: ModelPlacementEntry) => selectedProfile?.components.some(component => component.packageId === item.packageId) ? "使用中" : "導入済み";
  const purposeOf = (item: ModelPlacementEntry) => engines.find(e => e.engine === "asr" ? isAsrEngine(item.engine) : e.engine === item.engine)?.label ?? item.engine;
  const renderCopy = (item: ModelPlacementEntry, grouped = false) => <InstalledModelCopy key={item.packageId} item={item} name={modelDisplayName(item, options)} status={copyStatus(item)} purpose={purposeOf(item)} vocabularySupport={modelVocabularySupport(item, backends)} grouped={grouped} disabled={disabled} deletion={deletion} />;
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
      return <div className="model-selection-item" key={engine}><div className="model-selection-row"><label>{label}<select aria-label={label + "のモデル"} value={value} disabled={disabled} onChange={event => setSelection(current => ({ ...current, [engine]: event.target.value }))}>
        <option value="">モデルを選択</option>
        {choices.map(item => <option key={item.value} value={item.value} disabled={!item.option && !!item.entry && item.entry.sourceType !== "download" && !canReviewApplication(item.entry)}>{item.label}</option>)}
      </select></label><button type="button" disabled={disabled || !value || !!entry && entry.sourceType !== "download" && !canReviewApplication(entry) || action === "in_use"} onClick={event => void useModel(engine, value, true, actionLabel, event.currentTarget)}>{actionLabel}</button></div>
        {action === "update_runtime" && <p className="model-runtime-update-help">{used && choice?.packageIds.includes(used.packageId) ? "現在は更新前の実行環境を使用中です。" : ""}モデル本体は変更しません。再利用できるモデルデータはコピーします。旧コピーを保持するため追加の空き容量が必要です。</p>}
        {choice && <div className="model-license-summary"><div><span>{choice.option?.licenseSummary.licenseNames.join(" / ") ?? "条件未確認"}</span><button type="button" disabled={disabled} onClick={event => void useModel(engine, value, false, "使用", event.currentTarget)}>利用条件を見る</button></div>
          <p>{choice.option?.licenseSummary.summary ?? "このモデルと実行エンジンの利用条件は確認できていません。配布元の利用条件を確認してください。"}</p>
          {choice.option?.licenseSummary.restrictions.map((restriction, index) => <p className="model-license-restriction" key={index}>{restriction}</p>)}
        </div>}
        {used && !choice?.packageIds.includes(used.packageId) && <p className="model-current-selection">現在使用中：{entries.find(item => item.packageId === used.packageId) ? modelDisplayName(entries.find(item => item.packageId === used.packageId)!, options) : "導入済みモデル"}</p>}
      </div>;
    })}</div>
    <ModelIssue label="モデルの準備" message={error} />
    {pending && (!busy || recovering) && <p role="status">再試行の準備をしています…</p>}
  </section>
  <section className="analysis-card local-ai-settings" aria-label="ローカルAI詳細設定" hidden={!visible}>
    <h2>ローカルAI詳細設定</h2>
    <section className="ai-settings-group" aria-label="発話検知設定"><h3>発話検知設定</h3>
    <p>音声から発話区間を検出する設定です。無音の扱いや発話の区切りを調整できます。</p>
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
      const status = group.entries.some(item => copyStatus(item) === "使用中") ? "使用中" : "導入済み";
      return <li key={group.key} className="model-inventory-group"><div className="model-inventory-summary"><strong>{modelDisplayName(group.entry, options)}</strong>
        <span>{purposeOf(group.entry)} · 合計 {bytesLabel(group.totalBytes)} · {group.entries.length}コピー · {status}</span></div>
        <details className="model-copies"><summary>コピーを管理（{group.entries.length}件）</summary><div role="group" aria-label={`${modelDisplayName(group.entry, options)}のコピー`}>
          {group.entries.map(item => renderCopy(item, true))}
        </div></details>
      </li>;
    })}</ul>
    {installed.length > 0 && (!deletion?.canReviewModel || disabled) && <div id="model-deletion-unavailable" role="status"><p>{recordingActive ? "録音・音声保存が完了すると削除できます。" : busy || pending ? "モデル準備が完了すると削除できます。" : deletion?.reviewUnavailableReason ?? "ほかの操作が完了すると削除できます。"}</p></div>}
    {deletion?.listing?.items.map(item => <div key={item.operationId} className="model-deletion-pending"><p>{item.modelName ?? "モデル"}：削除の確認が必要です。{item.errorMessage}</p>
      {item.reviewAllowed && <button type="button" className="small-secondary" disabled={!deletion.canReviewPending} onClick={() => void deletion.reviewPending(item.operationId)}>残りの削除を確認</button>}</div>)}
    {deletion?.protectedNow && <p role="status">モデル削除の受付結果を確認中です。</p>}
    {deletion?.unknown && <button type="button" disabled={deletion.rpcBusy} onClick={() => void deletion.retryUnknown()}>同じ削除操作の結果を確認</button>}
  </section>
  <ToastNotice scope="model" type="runtime-update" eventId={runtimeNotice?.id} message={runtimeNotice?.message} onClose={()=>setRuntimeNotice(null)} />
  {license && <ModelLicenseDialog key={license.id} open={visible} review={license.review} loading={license.loading} error={license.error} confirmation={license.confirmation} actionLabel={license.actionLabel}
    acknowledged={license.acknowledged} confirming={license.confirming} onAcknowledge={acknowledged => updateLicense(license.id, { acknowledged })} onConfirm={() => void confirmLicense()} onDismiss={closeLicense} restoreTarget={() => license.opener} />}
  </>;
}

function InstalledModelCopy({ item, name, status, purpose, vocabularySupport, grouped, disabled, deletion }: {
  item: ModelPlacementEntry; name: string; status: string; purpose: string; vocabularySupport: string | null; grouped: boolean; disabled: boolean; deletion?: ModelDeletionControl;
}) {
  return <div className="model-copy" data-model-copy={item.packageId}><div className="model-inventory-summary">
    {!grouped && <strong>{name}</strong>}<span>{purpose} · {bytesLabel(item.totalBytes)} · {status}</span>
    <span className="model-copy-id">コピー {item.packageId.slice(0, 8)} · {item.engineVersion}</span></div>
    <div className="model-inventory-actions"><OptionsModal label={`${name}（${item.packageId.slice(0, 8)}）の詳細`} triggerLabel="詳細" title="モデルの詳細" className="model-details">
      <h3>{name}</h3><dl className="import-info"><dt>用途</dt><dd>{purpose}</dd>
        <dt>モデル名</dt><dd>{item.modelName || "不明"}</dd><dt>バージョン</dt><dd>{item.modelVersion || "不明"}</dd>
        <dt>量子化</dt><dd>{item.quantization || "不明"}</dd><dt>エンジン</dt><dd>{item.engine || "不明"} · {item.engineVersion || "不明"}</dd>
        <dt>発話検知モデル</dt><dd>{item.modelFiles?.some(file => file.role === "vad") ? "あり" : "なし"}</dd>
        {vocabularySupport !== null && <><dt>用語辞書の対応</dt><dd>{vocabularySupport}</dd></>}
        <dt>容量</dt><dd>{bytesLabel(item.totalBytes)}</dd><dt>取得元</dt><dd>{item.sourceType === "download" ? "ダウンロード" : item.sourceType === "manual_import" ? "ローカルからの取り込み" : "不明"}</dd>
        <dt>選択状態</dt><dd>{status}</dd><dt>互換性確認</dt><dd>{placementLabel(item)}</dd>
        <dt>パッケージID</dt><dd className="data-path">{item.packageId}</dd></dl>
    </OptionsModal><button type="button" className="small-secondary deletion-trigger" aria-describedby={(!deletion?.canReviewModel || disabled) ? "model-deletion-unavailable" : undefined} aria-label={`${name}（${item.packageId.slice(0, 8)}）をローカルから削除`} disabled={!deletion?.canReviewModel || disabled} onClick={() => void deletion?.reviewModel(item.packageId, item.reviewHash)}>ローカルから削除</button></div>
  </div>;
}
