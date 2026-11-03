// Synthetic IPC and tiny fake metadata only. No native model IO or process.
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "./startup-ready-mocks";
import { emit } from "@tauri-apps/api/event";
import { ModelInspectionPanel } from "../src/model-inspection";
import { ToastHost, NotificationList } from "../src/toast";
import { parseDownloadDefinition, downloadDigest, packageReviewHash, downloadByteTotals, downloadTransferBytes, type DownloadReview } from "../src/model-download-state";
import "../src/style.css";
import "../src/prototype-alignment.css";
import officialChoices from "../model-catalog/macos.json";
import whisperNetwork from "../model-catalog/whisper-small-network.json";
import vadNetwork from "../model-catalog/whisper-vad-network.json";
import { defaultVadOptions } from "../src/vad-state";
import whisperVariants from "../model-catalog/whisper-variants.json";
import { modelLicenseFixture } from "./model-license-fixture";
// Build-specific bundle hashes cannot be a static catalog. Browser-only fake
// runtime metadata exercises the same review shape; never actual IO/CPU proof.
const fakeRuntimeFiles = [
  { relativePath: "bin/whisper-cli", role: "executable", expectedSize: 100, expectedHash: "a".repeat(64) },
  { relativePath: "notices/whisper-MIT.txt", role: "notice", expectedSize: 16, expectedHash: "a".repeat(64) },
  { relativePath: "build-info.json", role: "notice", expectedSize: 32, expectedHash: "a".repeat(64) },
];
const hybridChoice = { id: "whisper-synthetic-bundle", label: "文字起こし", description: "同梱処理の画面検証のみ・実ファイルなし", recipe: {
  schemaVersion: 4, manifest: { schemaVersion: 1, engine: "whisper.cpp", engineVersion: "v1.9.4", modelName: "Whisper small（UI合成）", modelVersion: "5359861c739e955e79d9a303bcbc70fb988958b1", quantization: null, licenseRef: "synthetic runtime; fixed model metadata; not actual acquisition", target: "macos_aarch64", files: [...fakeRuntimeFiles, ...whisperNetwork.files, ...vadNetwork.files] },
  sources: [...fakeRuntimeFiles.map(file => ({ relativePath: file.relativePath, url: "mon-bundled://whisper-v1.9.4/whisper-runtime.tar", archive: { format: "tar", expectedSize: 2_478_080, expectedHash: "a".repeat(64), memberPath: file.relativePath } })), ...whisperNetwork.sources, ...vadNetwork.sources],
} };
const hybridChoices = whisperVariants.map(variant => {
  const choice = structuredClone(hybridChoice), path = `models/ggml-${variant.name}.bin`;
  choice.id = `whisper-synthetic-${variant.name}`;
  choice.label = `Whisper ${variant.name}（多言語）`;
  choice.recipe.manifest.modelName = choice.label;
  choice.recipe.manifest.files[3] = { ...choice.recipe.manifest.files[3], relativePath: path,
    expectedSize: variant.bytes, expectedHash: variant.sha256 };
  choice.recipe.sources[3] = { ...choice.recipe.sources[3], relativePath: path,
    url: `https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-${variant.name}.bin` };
  return choice;
});
const f = (window as any).modelDownloadFixture = { calls: [] as any[], mode: "success", failCancel: false, failCatalog: false, failLifetime: false, forgedReadback: false, readMissing: false, changed: 0, active: new Set<string>(), cancelled: new Set<string>(), operations: new Map<string, any>(), placements: new Map<string, any>(), finish: () => {}, setLocked: (_: boolean) => {}, setVisible: (_: boolean) => {}, setParent: (_: boolean) => {}, busy: false };
f.profiles = ["standard", "high_accuracy"].map(name => ({ name, id: null, profileVersion: 0, components: [], configurationComplete: false }));
f.entries = [{ packageId: "44444444-4444-4444-8444-444444444444", modelName: "既存の合成モデル", modelVersion: "1", engine: "whisper.cpp", engineVersion: "test", totalBytes: 33, placementRecorded: true, ready: false, reviewHash: "a".repeat(64), compatibility: null }];
f.settingsByEngine = {
  "whisper.cpp": { executablePath: "", modelPath: "", vadModelPath: null, threads: 2, useGpu: false },
  "sherpa-onnx": { executablePath: "", segmentationModelPath: "", embeddingModelPath: "", runtimeLibraryPaths: [], threads: 2, clusterThresholdMilli: 500, windowShiftMilli: 10000, minDurationOnMs: 300, minDurationOffMs: 500 },
  "llama.cpp": { executablePath: "", modelPath: "", threads: 2, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 },
};
f.preferenceVersions = { "whisper.cpp": 0, "sherpa-onnx": 0, "llama.cpp": 0 };
const componentRoles = { "whisper.cpp": ["asr"], "sherpa-onnx": ["segmentation", "speaker_embedding"], "llama.cpp": ["llm"] };
(window as any).isTauri = true; mockWindows("main");
f.definition = JSON.stringify({ schemaVersion: 4, manifest: { schemaVersion: 1, engine: "whisper.cpp", engineVersion: "test", modelName: "合成取得素材", modelVersion: "1", quantization: null, licenseRef: "synthetic fixture only", target: "macos_aarch64", files: [{ relativePath: "cli", role: "executable", expectedSize: 16, expectedHash: "a".repeat(64) }, { relativePath: "model", role: "asr", expectedSize: 17, expectedHash: "b".repeat(64) }] }, sources: [{ relativePath: "cli", url: "https://example.com/cli" }, { relativePath: "model", url: "https://example.com/model.tar", archive: { format: "tar", expectedSize: 100, expectedHash: "c".repeat(64), memberPath: "model" } }] });
async function reviewed(definition: string): Promise<DownloadReview> {
  const recipe = parseDownloadDefinition(definition);
  return { manifest: recipe.manifest, sources: recipe.sources, recipeHash: await downloadDigest(recipe), packageReviewHash: await packageReviewHash(recipe.manifest), recipeSchemaVersion: recipe.schemaVersion, ...downloadByteTotals(recipe), totalBytes: recipe.manifest.files.reduce((n, file) => n + file.expectedSize, 0), installed: false, ready: false, publisherAuthenticity: "not_verified" };
}
async function receipt(operationId: string, review: DownloadReview) {
  const manifestHash = await downloadDigest(review.manifest), files = review.manifest.files.map(file => ({ relativePath: file.relativePath, role: file.role, actualSize: file.expectedSize, actualHash: file.expectedHash, expectedSizeChecked: true, expectedHashChecked: true }));
  return { operationId, recipeHash: review.recipeHash, directory: "/synthetic/payload", installed: false, ready: false, report: { manifest: review.manifest, manifestHash, reviewHash: await downloadDigest([1, review.manifest, manifestHash, files]), files, totalBytes: review.totalBytes, engineCompatibility: "not_verified", publisherAuthenticity: "not_verified", installed: false, ready: false } };
}
f.verify = async () => { const call = f.calls.findLast(call => call.command === "place_downloaded_model"); await emit("model-placement-progress", { operationId: call.args.token, recipeHash: call.args.expectedRecipeHash, phase: "verifying" }); };
f.progress = async (operationId: string, receivedBytes: number, wrong = false) => { const review = f.operations.get(operationId).review; await emit("model-download-progress", { operationId, recipeHash: wrong ? "c".repeat(64) : review.recipeHash, receivedBytes, totalBytes: downloadTransferBytes(review) }); };
f.seed = async () => { const id = "33333333-3333-4333-8333-333333333333", review = await reviewed(f.definition); f.operations.set(id, { review, state: "complete", receipt: await receipt(id, review) }); return id; };
async function officialOptions() {
    if ((window as any).windowsNativeModelOptions) return Promise.all((window as any).windowsNativeModelOptions.map(async (option: any) => {
      const { licenseNames, summary, restrictions, disclosureHash } = await modelLicenseFixture(option.label, { recipeHash: option.review.recipeHash }, false, f.policyRevision);
      return { ...option, licenseSummary: { licenseNames, summary, restrictions, disclosureHash } };
    }));
    if (new URLSearchParams(location.search).get("official") === "1") return Promise.all([...hybridChoices, ...officialChoices].map(async choice => {
      const definition = JSON.stringify(parseDownloadDefinition(JSON.stringify(choice.recipe)));
      const review = await reviewed(definition), { licenseNames, summary, restrictions, disclosureHash } = await modelLicenseFixture(choice.label, { recipeHash: review.recipeHash }, choice.id.includes("lfm3"), f.policyRevision);
      return { id: choice.id, label: choice.label, description: choice.description, definition, review, tokenLimits: "tokenLimits" in choice ? choice.tokenLimits : null, licenseSummary: { licenseNames, summary, restrictions, disclosureHash } };
    }));
    const review = await reviewed(f.definition), { licenseNames, summary, restrictions, disclosureHash } = await modelLicenseFixture("文字起こし（合成）", { recipeHash: review.recipeHash }, f.licenseRestricted, f.policyRevision);
    return [{ id: "synthetic-catalog", label: "文字起こし（合成）", description: "画面検証のみ・実ファイルなし", definition: f.definition, review, licenseSummary: { licenseNames, summary, restrictions, disclosureHash } }];
}
export const modelDownloadIPC = async (command: string, args: any) => {
  f.calls.push({ command, args: structuredClone(args) });
  if (command === "get_official_model_options") return officialOptions();
  if (command === "get_model_license_review") {
    if (f.licenseFailure) throw Error("模擬の利用条件読込失敗");
    const options = await officialOptions();
    const option = "recipeHash" in args.target ? options.find((o: any) => o.review.recipeHash === args.target.recipeHash) : null;
    const entry = "packageId" in args.target ? f.entries.find((e: any) => e.packageId === args.target.packageId && e.reviewHash === args.target.expectedReviewHash) : null;
    if (!option && !entry) throw Error("模擬の利用条件対象不一致");
    const result = await modelLicenseFixture(option?.label ?? entry.modelName, args.target, option?.id.includes("lfm3") || f.licenseRestricted, f.policyRevision);
    if (f.changedPolicy) result.summary = "変更された合成条件";
    if (f.licenseDelay) return new Promise(resolve => { f.finishLicense = () => resolve(result); });
    return result;
  }
  if (["get_voice_matching_preferences", "get_speaker_completion_preferences"].includes(command)) {
    const id = command === "get_voice_matching_preferences" ? "voice_matching" : "speaker_completion";
    f[id] ??= { id, lockVersion: 0, payload: { similarityThresholdMilli: 800, marginThresholdMilli: 100 } };
    return structuredClone(f[id]);
  }
  if (["save_voice_matching_preferences", "save_speaker_completion_preferences"].includes(command)) {
    const id = command === "save_voice_matching_preferences" ? "voice_matching" : "speaker_completion";
    if (args.expectedLockVersion !== f[id].lockVersion) throw Error("stale settings");
    f[id] = { id, lockVersion: f[id].lockVersion + 1, payload: structuredClone(args.payload) };
    return structuredClone(f[id]);
  }
  if (command === "get_model_placements") return structuredClone(f.entries);
  if (command === "get_model_profiles") return structuredClone(f.profiles);
  if (["get_analysis_preferences", "get_asr_preferences"].includes(command)) return { lockVersion: f.preferenceVersions["whisper.cpp"], payload: { standard: structuredClone(f.settingsByEngine["whisper.cpp"]), highAccuracy: structuredClone(f.settingsByEngine["whisper.cpp"]) } };
  if (command === "get_diarization_preferences") {
    if (f.mode === "threadReadFailure") throw Error("模擬CPU設定読込失敗");
    return { lockVersion: f.preferenceVersions["sherpa-onnx"], payload: { standard: structuredClone(f.settingsByEngine["sherpa-onnx"]), highAccuracy: structuredClone(f.settingsByEngine["sherpa-onnx"]) } };
  }
  if (command === "check_model_compatibility") {
    const entry = f.entries.find((item: any) => item.packageId === args.packageId);
    if (!entry || entry.reviewHash !== args.expectedReviewHash) throw Error("模擬モデル不一致");
    entry.compatibility = { operationId: args.token, packageId: entry.packageId, reviewHash: entry.reviewHash, adapterRevision: 1, state: f.mode === "checkFailure" ? "failed" : "succeeded", errorMessage: f.mode === "checkFailure" ? "模擬CPU確認失敗" : null };
    return structuredClone(entry.compatibility);
  }
  if (command === "preview_model_profile_application") {
    if (f.mode === "previewFailure") throw Error("模擬使用設定の確認失敗");
    const entry = f.entries.find((item: any) => item.packageId === args.packageId), profile = f.profiles.find((item: any) => item.name === args.profile);
    const before = structuredClone(f.settingsByEngine[entry.engine]), after = structuredClone(before);
    after.executablePath = `/synthetic/${entry.packageId}/cli`;
    if (entry.engine === "sherpa-onnx") {
      after.segmentationModelPath = `/synthetic/${entry.packageId}/segmentation.onnx`;
      after.embeddingModelPath = `/synthetic/${entry.packageId}/embedding.onnx`;
      after.runtimeLibraryPaths = [`/synthetic/${entry.packageId}/runtime.dylib`];
    } else after.modelPath = `/synthetic/${entry.packageId}/model`;
    if (entry.engine === "whisper.cpp" && entry.modelFiles?.some((f: any) => f.role === "vad")) {
      after.vadModelPath = `/synthetic/${entry.packageId}/vad`;
      after.vadOptions = args.whisperVadOptions ?? before.vadOptions ?? structuredClone(defaultVadOptions);
    }
    if (args.diarizationThreads !== undefined) after.threads = args.diarizationThreads;
    if (args.diarizationClusterThresholdMilli !== undefined) after.clusterThresholdMilli = args.diarizationClusterThresholdMilli;
    return { schemaVersion: 1, profile: args.profile, packageId: entry.packageId, reviewHash: entry.reviewHash, checkOperationId: entry.compatibility.operationId, expectedProfileId: profile.id, expectedProfileHash: profile.id ? "c".repeat(64) : null, expectedProfileVersion: profile.profileVersion, expectedPreferencesVersion: f.preferenceVersions[entry.engine], before: { engine: entry.engine, settings: before }, ...(args.whisperVadOptions === undefined ? {} : { whisperVadOptions: args.whisperVadOptions }), after: { engine: entry.engine, settings: after }, ...(args.diarizationThreads === undefined ? {} : { diarizationThreads: args.diarizationThreads }), ...(args.diarizationClusterThresholdMilli === undefined ? {} : { diarizationClusterThresholdMilli: args.diarizationClusterThresholdMilli }), reviewDigest: "b".repeat(64) };
  }
  if (command === "apply_model_profile_application") {
    const preview = args.preview, entry = f.entries.find((item: any) => item.packageId === preview.packageId), profile = f.profiles.find((item: any) => item.name === preview.profile);
    if (profile.profileVersion !== preview.expectedProfileVersion || profile.id !== preview.expectedProfileId) throw Error("模擬設定競合");
    if (f.mode === "applyFailure") throw Error("模擬選択保存失敗");
    profile.id = args.token; profile.profileVersion++;
    profile.components = [...profile.components.filter((item: any) => item.engine !== entry.engine), { roles: componentRoles[entry.engine as keyof typeof componentRoles], packageId: entry.packageId, modelName: entry.modelName, modelVersion: entry.modelVersion, engine: entry.engine, checkOperationId: entry.compatibility.operationId, settingsMatch: true, compatibilityCurrent: true }];
    profile.configurationComplete = profile.components.length === 3;
    if (JSON.stringify(f.settingsByEngine[entry.engine]) !== JSON.stringify(preview.after.settings)) f.preferenceVersions[entry.engine]++;
    f.settingsByEngine[entry.engine] = structuredClone(preview.after.settings);
    if (f.mode === "applyLost") throw Error("模擬選択保存応答喪失");
    return structuredClone(profile);
  }
  if (command === "get_model_download_catalog") {
    if (f.failCatalog) throw Error("模擬取得一覧の読込失敗");
    if (f.active.size) throw Error("模擬モデル準備が実行中");
    return { supported: true, unknownEntries: 0, entries: Array.from(f.operations, ([operationId, item]: any) => ({ operationId, recipeHash: item.review.recipeHash, modelName: item.review.manifest.modelName, modelVersion: item.review.manifest.modelVersion, totalBytes: item.review.totalBytes, state: item.state === "complete" ? "completion_recorded" : "incomplete", ...(item.placement ? { placement: item.placement } : {}) })) };
  }
  if (command === "cancel_model_inspection") { if (f.failCancel) throw Error("模擬停止要求の応答喪失"); f.cancelled.add(args.token); return null; }
  if (command === "get_model_download_lifetime") { if (f.failLifetime) throw Error("模擬停止状態の読込失敗"); return { active: f.active.has(args.token), cancelled: f.cancelled.has(args.token) }; }
  if (command === "read_model_download") {
    if (f.active.has(args.downloadId)) throw Error("元の処理がまだ実行中");
    const item = f.operations.get(args.downloadId);
    if (item?.placement) throw Error("移動済みの取得元は読み戻せません");
    if (!item || f.readMissing) throw Error("模擬取得記録を読めません");
    if (item.review.recipeHash !== args.expectedRecipeHash) throw Error("模擬定義SHAの不一致");
    const result = { state: item.state, review: item.review, receipt: item.state === "complete" ? item.receipt : null };
    return f.forgedReadback ? { ...result, receipt: { ...item.receipt, ready: true } } : structuredClone(result);
  }
  if (command === "start_model_download") {
    if (f.cancelled.has(args.token) || f.active.size) throw Error("模擬の取消済み・競合操作");
    if (f.mode === "beforeIntentFailure") throw Error("模擬取得準備失敗");
    const review = await reviewed(args.definition); if (review.recipeHash !== args.expectedRecipeHash) throw Error("模擬定義SHAの不一致");
    f.active.add(args.token); const result = await receipt(args.token, review);
    const item = { review, state: "incomplete", receipt: result }; f.operations.set(args.token, item);
    if (f.mode === "modelFileFailure") { f.active.delete(args.token); throw { code: "operation_result_unknown", message: "モデルの親フォルダーにリンクまたは変更があります", operationId: args.token, retryDisposition: "reconcile" }; }
    if (f.mode === "downloadFailure") { f.active.delete(args.token); throw Error("模擬通信失敗"); }
    const finish = () => { f.active.delete(args.token); if (f.cancelled.has(args.token)) throw Error("模擬取得の中断・途中保全"); item.state = "complete"; return structuredClone(result); };
    if (f.mode === "earlyLost") { f.finish = finish; throw Error("処理は継続中だが開始応答が喪失"); }
    if (f.mode === "delay") return new Promise((resolve, reject) => { f.finish = () => { try { resolve(finish()); } catch (cause) { reject(cause); } }; });
    const complete = finish(); if (f.mode === "lost") throw Error("模擬取得の完了応答喪失");
    return f.mode === "forgedReceipt" ? { ...complete, ready: true } : complete;
  }
  if (command === "place_downloaded_model") {
    const item = f.operations.get(args.downloadId);
    if (!item || item.state !== "complete" || item.review.recipeHash !== args.expectedRecipeHash || item.receipt.report.reviewHash !== args.expectedReviewHash) throw Error("模擬コピー対象の不一致");
    if (f.mode === "placeFailure") throw Error("模擬コピー失敗");
    if (item.placement && item.placement.packageId !== args.packageId) throw Error("配置IDが変更されました");
    item.placement = { packageId: args.packageId ?? args.token, state: "pending" };
    if (f.mode === "movedFailure") throw Error("模擬：移動後の検証中断");
    f.active.add(args.token);
    const result = { packageId: args.packageId ?? args.token, reviewHash: args.expectedReviewHash, totalBytes: item.review.totalBytes, placementRecorded: true, engineCompatibility: "not_verified", ready: false };
    const finish = () => { f.active.delete(args.token); if (f.cancelled.has(args.token)) { f.placements.set(result.packageId, { ...result, placementRecorded: false }); throw Error("模擬コピー中断"); } f.placements.set(result.packageId, result); item.placement.state = "committed"; f.entries.push({ ...result, modelName: item.review.manifest.modelName, modelVersion: item.review.manifest.modelVersion, quantization: item.review.manifest.quantization, modelFiles: item.review.manifest.files.filter((file: any) => !["executable", "runtime_library", "notice"].includes(file.role)).map((file: any) => ({ role: file.role, size: file.expectedSize, sha256: file.expectedHash })), engine: item.review.manifest.engine, engineVersion: item.review.manifest.engineVersion, sourceType: "download", download: { operationId: args.downloadId, recipeHash: args.expectedRecipeHash }, compatibility: null }); return result; };
    if (f.mode === "delay") return new Promise((resolve, reject) => { f.finish = () => { try { resolve(finish()); } catch (cause) { reject(cause); } }; });
    const complete = finish(); if (f.mode === "lost") throw Error("模擬コピー完了応答喪失"); return complete;
  }
  if (command === "get_downloaded_model_placement_result") return { active: f.active.has(args.token ?? args.packageId), cancelled: f.cancelled.has(args.token ?? args.packageId), receipt: f.placements.get(args.packageId) ?? null };
  throw Error(`Unexpected download fixture command: ${command}`);
};
mockIPC(modelDownloadIPC, { shouldMockEvents: true });
function Fixture() {
  const [locked, setLocked] = useState(false), [visible, setVisible] = useState(true), [busy, setBusy] = useState(false), [parent, setParent] = useState(false);
  f.setLocked = setLocked; f.setVisible = setVisible; f.setParent = setParent; f.busy = busy;
  const compact = new URLSearchParams(location.search).get("compact") === "1";
  return <main><ToastHost/><h1>モデル取得の合成検証</h1><button onClick={() => setVisible(value => !value)}>設定の表示切替</button><p>共有準備: {busy ? "操作中" : "待機"}</p>
    <output aria-label="合成取得操作数">合成のみ：取得 {f.calls.filter(call => call.command === "start_model_download").length} · コピー {f.calls.filter(call => call.command === "place_downloaded_model").length} · CPU {f.calls.filter(call => call.command === "check_model_compatibility").length} · 適用 {f.calls.filter(call => call.command === "apply_model_profile_application").length}</output>
    {compact && <><button onClick={() => { f.failCatalog = true; }}>合成の取得記録読込失敗</button><button onClick={() => { f.failCatalog = false; }}>合成の取得記録読込復帰</button>
      <button onClick={() => { f.readMissing = true; }}>合成の実体照合失敗</button><button onClick={() => { f.readMissing = false; }}>合成の実体照合復帰</button>
      <button onClick={() => { for (const item of f.operations.values()) item.state = "incomplete"; }}>合成の記録を未完了にする</button></>}
    {visible && <ModelInspectionPanel compact={compact} locked={locked} recordingActive={false} />}<NotificationList visible={false}/></main>;
}
if (!(window as any).skipModelDownloadFixtureMain) createRoot(document.getElementById("root")!).render(<Fixture />);
