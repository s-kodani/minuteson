import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { InspectionAttempt, inspectionLabel, placementLabel, assertPlacementReceipt, assertCompatibilitySummary, type ModelCompatibilitySummary, type ModelInspectionReport, type ModelPlacementReceipt, type ModelPlacementEntry } from "./model-inspection-state";
import { assertApplicationPreview, assertApplicationResult, assertAppliedPreferences, assertProfileCatalog, canReviewApplication, preferenceCommand, profileName, type ModelApplicationPreview, type ModelProfileName, type ModelProfileView } from "./model-profile-state";
import type { ModelBodyDeletionControl } from "./model-body-deletion";
import { ToastNotice } from "./toast";
import { ModelIssue } from "./model-issue";
import { ModelDownloadPanel } from "./model-download";
import { SettingsDisclosure } from "./settings-disclosure";
import { ModelImportGuide } from "./model-import-guide";
import { assertGuidedInspection, type GuidedModelSource } from "./model-import-guide-state";
import { ModelInventory } from "./model-inventory";

type PendingApplication = { preview: ModelApplicationPreview; entry: ModelPlacementEntry; operationId: string };
// Navigation cannot replace an uncertain writer with a fresh operation.
let unresolvedApplication: { application: PendingApplication; message: string } | null = null;

export function ModelInspectionPanel({ locked, recordingActive, refreshEpoch = 0 }: { locked: boolean; recordingActive: boolean; deletionControl?: ModelBodyDeletionControl; refreshEpoch?: number; compact?: boolean }) {
  const [manualOpen, setManualOpen] = useState(false), [downloadOpen, setDownloadOpen] = useState(false), [manualBusy, setBusy] = useState(false), [downloadBusy, setDownloadBusy] = useState(false), [pickerBusy, setPickerBusy] = useState(false);
  const [cancelling, setCancelling] = useState(false), [error, setError] = useState(""), [status, setStatus] = useState(""), [message, setMessage] = useState("");
  const [report, setReport] = useState<ModelInspectionReport | null>(null), [placements, setPlacements] = useState<ModelPlacementEntry[]>([]);
  const [profiles, setProfiles] = useState<ModelProfileView[] | null>(null), [catalogLoading, setCatalogLoading] = useState(false), [catalogError, setCatalogError] = useState("");
  const [selectedPackage, setSelectedPackage] = useState(""), [trustedPackage, setTrustedPackage] = useState("");
  const [application, setApplication] = useState<PendingApplication | null>(() => unresolvedApplication && structuredClone(unresolvedApplication.application));
  const [applicationUnknown, setApplicationUnknown] = useState(() => !!unresolvedApplication), [applicationError, setApplicationError] = useState(() => unresolvedApplication?.message ?? "");
  const manualTrigger = useRef<HTMLButtonElement>(null), attempt = useRef(new InspectionAttempt()), mounted = useRef(true), catalogSequence = useRef(0), running = useRef(false);
  const reviewSource = useRef<GuidedModelSource | null>(null);
  const current = useRef({ locked, recordingActive, report, placements, profiles, application, selectedPackage, trustedPackage });
  current.current = { locked, recordingActive, report, placements, profiles, application, selectedPackage, trustedPackage };
  const busy = manualBusy || downloadBusy || pickerBusy;
  const loadPlacements = async () => {
    if (!isTauri()) return;
    const sequence = ++catalogSequence.current; setCatalogLoading(true);
    try {
      const [entries, views] = await Promise.all([invoke<ModelPlacementEntry[]>("get_model_placements"), invoke<ModelProfileView[]>("get_model_profiles")]);
      entries.forEach(placementLabel); assertProfileCatalog(views);
      if (mounted.current && sequence === catalogSequence.current) { setPlacements(entries); setProfiles(views); setCatalogError(""); }
    } catch (cause) { if (mounted.current && sequence === catalogSequence.current) { setProfiles(null); setCatalogError(String(cause)); } }
    finally { if (mounted.current && sequence === catalogSequence.current) setCatalogLoading(false); }
  };
  useEffect(() => {
    mounted.current = true; void loadPlacements();
    return () => { mounted.current = false; catalogSequence.current++; const token = attempt.current.cancel(); if (token && isTauri()) void invoke("cancel_model_inspection", { token }).catch(() => {}); };
  }, []);
  useEffect(() => { if (refreshEpoch > 0) void loadPlacements(); }, [refreshEpoch]);
  useEffect(() => {
    if (!(locked || recordingActive) || !running.current) return;
    const token = attempt.current.cancel();
    if (token) { setCancelling(true); setTrustedPackage(""); void invoke("cancel_model_inspection", { token }).catch(cause => { if (mounted.current) setError(`停止要求を確認できません: ${String(cause)}`); }); }
  }, [locked, recordingActive]);
  useEffect(() => {
    if (locked || recordingActive || !(manualOpen || downloadOpen)) setTrustedPackage("");
    if ((locked || recordingActive) && !manualBusy && application && !applicationUnknown) setApplication(null);
  }, [locked, recordingActive, manualOpen, downloadOpen, manualBusy, application, applicationUnknown]);
  const begin = (token: string, label: string) => {
    if (running.current || downloadBusy || pickerBusy || current.current.locked || current.current.recordingActive || !isTauri()) return false;
    running.current = true; attempt.current.begin(token); setBusy(true); setCancelling(false); setError(""); setMessage(""); setStatus(label); return true;
  };
  const end = async (token: string) => {
    if (attempt.current.finish(token)) {
      await loadPlacements(); running.current = false;
      if (mounted.current) { setBusy(false); setCancelling(false); setStatus(""); }
    }
  };
  const inspectSource = async (source: GuidedModelSource) => {
    const token = crypto.randomUUID(); if (!begin(token, "選択したファイルを検査しています")) return;
    reviewSource.current = null; setReport(null);
    try {
      const result = await invoke<ModelInspectionReport>("inspect_model_package", { token, ...source });
      if (mounted.current && attempt.current.accepts(token)) { inspectionLabel(result); assertGuidedInspection(result, source); reviewSource.current = structuredClone(source); setReport(result); }
    } catch (cause) { if (mounted.current && attempt.current.accepts(token)) setError(String(cause)); }
    finally { await end(token); }
  };
  const place = async () => {
    const review = current.current.report, source = reviewSource.current;
    if (!review || !source || application) return;
    const reviewHash = review.reviewHash, token = crypto.randomUUID();
    if (!begin(token, "モデルを取り込んでいます")) return;
    try {
      if (current.current.report?.reviewHash !== reviewHash || reviewSource.current !== source) throw Error("検査した素材が変わりました。再検査してください。");
      // A lost/invalid response must not expose a replay button for this review.
      // Native records retain the original intent; a new import needs new review.
      setReport(null); reviewSource.current = null;
      const receipt = await invoke<ModelPlacementReceipt>("place_model_package", { token, ...source, expectedReviewHash: reviewHash });
      if (mounted.current && attempt.current.accepts(token)) { assertPlacementReceipt(receipt, token, reviewHash); setSelectedPackage(receipt.packageId); setReport(null); setMessage("モデルを取り込みました。動作確認後に使用できます。"); }
    } catch (cause) { if (mounted.current && attempt.current.accepts(token)) setError(`${String(cause)} 途中のファイルは保持しています。`); }
    finally { await end(token); }
  };
  const selected = placements.find(entry => entry.packageId === selectedPackage && entry.placementRecorded && entry.bodyRemoved !== true);
  const check = async (entry: ModelPlacementEntry) => {
    if (current.current.trustedPackage !== entry.packageId || !current.current.placements.some(item => item.packageId === entry.packageId && item.reviewHash === entry.reviewHash && item.placementRecorded && item.bodyRemoved !== true) || application) return;
    const token = crypto.randomUUID(); if (!begin(token, "モデルのCPU動作を確認しています")) return;
    setTrustedPackage("");
    try {
      const summary = await invoke<ModelCompatibilitySummary>("check_model_compatibility", { token, packageId: entry.packageId, expectedReviewHash: entry.reviewHash });
      if (mounted.current && attempt.current.accepts(token)) {
        assertCompatibilitySummary(summary, token, entry);
        if (summary.state === "failed") throw Error(`動作確認に失敗しました: ${summary.errorMessage}`);
        setMessage(summary.state === "succeeded" ? "CPU動作を確認しました。使用するプロファイルを選んでください。" : "動作確認を中止しました。");
      }
    } catch (cause) { if (mounted.current && attempt.current.accepts(token)) setError(String(cause)); }
    finally { await end(token); }
  };
  const reviewApplication = async (entry: ModelPlacementEntry, name: ModelProfileName) => {
    const profile = current.current.profiles?.find(p => p.name === name);
    if (!profile || application || catalogLoading || catalogError || !canReviewApplication(entry)) return;
    const token = crypto.randomUUID(); if (!begin(token, "使用するモデルを確認しています")) return;
    try {
      const preview = await invoke<ModelApplicationPreview>("preview_model_profile_application", { token, profile: name, packageId: entry.packageId, expectedReviewHash: entry.reviewHash, checkOperationId: entry.compatibility!.operationId });
      if (mounted.current && attempt.current.accepts(token) && !current.current.locked && !current.current.recordingActive) {
        assertApplicationPreview(preview, entry, name, profile); setApplicationError(""); setApplicationUnknown(false); setApplication({ preview: structuredClone(preview), entry: structuredClone(entry), operationId: crypto.randomUUID() });
      }
    } catch (cause) { if (mounted.current && attempt.current.accepts(token)) setError(String(cause)); }
    finally { await end(token); }
  };
  const applyApplication = async () => {
    const value = current.current.application; if (!value) return;
    const { preview, operationId } = value; if (!begin(operationId, "モデルの選択を保存しています")) return;
    unresolvedApplication = { application: structuredClone(value), message: "モデル選択の保存結果が未確認です。導入画面から同じ操作を確認してください。" }; setApplicationError("");
    try {
      const result = await invoke<ModelProfileView>("apply_model_profile_application", { token: operationId, preview });
      if (mounted.current && attempt.current.accepts(operationId)) {
        assertApplicationResult(result, operationId, preview); unresolvedApplication = null; setApplication(null); setApplicationUnknown(false); setApplicationError(""); setMessage(`${profileName(preview.profile)}で使用するモデルを保存しました。`);
        try { const preferences = await invoke<Parameters<typeof assertAppliedPreferences>[0]>(preferenceCommand(preview.after.engine)); assertAppliedPreferences(preferences, preview); }
        catch (cause) { if (mounted.current) setError(`モデル選択は保存しましたが、解析設定を確認できません: ${String(cause)}`); }
      }
    } catch (cause) {
      const message = `保存結果が未確認です: ${String(cause)} 同じ操作の結果を確認してください。`;
      if (unresolvedApplication?.application.operationId === operationId) unresolvedApplication.message = message;
      if (mounted.current) { setApplicationUnknown(true); setApplicationError(message); }
    } finally {
      if (!attempt.current.accepts(operationId) && mounted.current && unresolvedApplication) { setApplicationUnknown(true); setApplicationError(unresolvedApplication.message); }
      await end(operationId);
    }
  };
  const cancel = async () => {
    const token = attempt.current.cancel(); if (!token) return; setCancelling(true); setReport(null);
    try { await invoke("cancel_model_inspection", { token }); }
    catch (cause) { if (mounted.current) setError(`停止要求を確認できません: ${String(cause)}`); }
  };
  const releaseUnknownApplication = async () => {
    const value = current.current.application;
    if (!value || !applicationUnknown || !begin(crypto.randomUUID(), "現在のモデル選択を確認しています")) return;
    // Fence the original writer, then read current configuration. Ending this
    // confirmation is not rollback or proof that the original write failed.
    try {
      await invoke("cancel_model_inspection", { token: value.operationId });
      const [entries, views] = await Promise.all([invoke<ModelPlacementEntry[]>("get_model_placements"), invoke<ModelProfileView[]>("get_model_profiles")]);
      entries.forEach(placementLabel); assertProfileCatalog(views);
      if (!mounted.current || current.current.application?.operationId !== value.operationId || current.current.locked || current.current.recordingActive) throw Error("確認対象または操作条件が変わりました。");
      setPlacements(entries); setProfiles(views); setCatalogError("");
      if (unresolvedApplication?.application.operationId === value.operationId) unresolvedApplication = null;
      setApplication(null); setApplicationUnknown(false); setApplicationError(""); setMessage("現在のモデル選択を読み取り、未確定の確認を終了しました。保存の取消や巻き戻しは行っていません。");
    } catch (cause) { if (mounted.current) setApplicationError(`確認を終了できません: ${String(cause)}`); }
    finally { const token = attempt.current.cancel(); if (token) await end(token); }
  };
  const setupContent = <section className="model-setup-steps" aria-label="モデルを使用する準備">
    {placements.some(entry => entry.placementRecorded && entry.bodyRemoved !== true) && <label>準備するモデル<select aria-label="準備するモデル" value={selectedPackage} disabled={busy || locked || recordingActive || !!application} onChange={event => { setSelectedPackage(event.target.value); setTrustedPackage(""); setMessage(""); }}><option value="">モデルを選択</option>{placements.filter(entry => entry.placementRecorded && entry.bodyRemoved !== true).map(entry => <option key={entry.packageId} value={entry.packageId}>{entry.modelName} · {entry.engine}</option>)}</select></label>}
    {selected && !application && <>
      {!canReviewApplication(selected) && <><p>動作確認では取り込んだ実行ファイルを起動します。配布元と利用条件を確認してください。合成入力だけを使用し、会議音声は使いません。CPU動作確認は正式な品質評価ではありません。</p><details><summary>実行時の注意事項</summary><p>実行ファイルは利用者の権限で起動します。隔離する仕組みはありません。最大120秒のCPU試行であり、実会議での精度・速度やGPUの動作は保証しません。</p></details><label><input type="checkbox" checked={trustedPackage === selected.packageId} disabled={busy || locked || recordingActive} onChange={event => setTrustedPackage(event.target.checked ? selected.packageId : "")} />このモデルの実行ファイルを信頼し、起動して動作確認することを許可する</label><button type="button" disabled={busy || locked || recordingActive || trustedPackage !== selected.packageId} onClick={() => void check(selected)}>動作を確認</button></>}
      {canReviewApplication(selected) && <label>使用するプロファイル<select aria-label="使用するプロファイル" value="" disabled={busy || locked || recordingActive || !profiles || catalogLoading || !!catalogError} onChange={event => { if (event.target.value) void reviewApplication(selected, event.target.value as ModelProfileName); }}><option value="">選択してください</option><option value="standard">標準</option><option value="high_accuracy">高精度</option></select></label>}
    </>}
    {application && <div className="model-application-review" aria-label="モデル選択の確認"><p>{application.entry.modelName}を{profileName(application.preview.profile)}で使用します。新規解析のモデル選択だけを保存し、既存の会議やジョブは変更しません。</p><ModelIssue label="モデル選択の保存" message={applicationError} /><div className="analysis-actions"><button type="button" disabled={busy || locked || recordingActive} onClick={() => void applyApplication()}>{applicationUnknown ? "同じ操作の結果を確認" : "このモデルを使用"}</button>{!applicationUnknown && <button type="button" className="secondary" disabled={busy} onClick={() => setApplication(null)}>キャンセル</button>}</div>{applicationUnknown && <details><summary>結果を確認できない場合</summary><p>現在の構成を読み取り、この確認を終了できます。元の操作記録・設定は保持し、保存の取消や巻き戻しは行いません。</p><button type="button" disabled={busy || locked || recordingActive} onClick={() => void releaseUnknownApplication()}>現在の構成を読み取り確認を終了</button></details>}</div>}
    {status && <p role="status">{cancelling ? "停止要求済み。終了を待っています。" : status}</p>}
    <ModelIssue label="モデル準備" message={error} />
    {message && <p role="status">{message}</p>}
    {manualBusy && <button type="button" className="secondary" disabled={cancelling} onClick={() => void cancel()}>停止を要求</button>}
  </section>;
  return <section className="analysis-card model-inspection" aria-label="AIモデルの事前検査">
    <h2>ローカルAI</h2>
    <div className="model-setup-entrypoints">
      <ModelDownloadPanel locked={locked || recordingActive} setupBlocked={manualBusy || pickerBusy || !!application} onOpenChange={open => { setDownloadOpen(open); setTrustedPackage(""); }} onBusyChange={setDownloadBusy} onChanged={loadPlacements} placements={placements} onPlaced={setSelectedPackage} setupContent={setupContent} />
      <SettingsDisclosure label="ローカルから取り込む" className="model-inspection-settings" open={manualOpen} onOpenChange={open => { if (!manualBusy && !pickerBusy) { setManualOpen(open); setTrustedPackage(""); if (open) void loadPlacements(); } }} triggerRef={manualTrigger} restoreTarget={() => manualTrigger.current}>
        <ModelImportGuide disabled={manualBusy || downloadBusy || locked || recordingActive || !!application} onInspect={inspectSource} onPickingChange={setPickerBusy} onChanged={() => { reviewSource.current = null; setReport(null); setMessage(""); setError(""); }} />
        {report && <section className="model-inspection-report" aria-label="モデル検査結果"><h3>{report.manifest.modelName}</h3><p>{report.manifest.engine} · {(report.totalBytes / 1024 ** 2).toFixed(1)} MiB · 検査済み・未導入</p><p>アプリの管理領域にコピーします。元ファイルは変更せず、実行やモデル選択の保存は行いません。</p><button type="button" disabled={busy || locked || recordingActive || !!application} onClick={() => void place()}>取り込む</button></section>}
        {setupContent}
      </SettingsDisclosure>
    </div>
    <ModelInventory entries={placements} profiles={profiles} loading={catalogLoading} error={catalogError} />
    {!(manualOpen || downloadOpen) && status && <p role="status">{status}</p>}
    <ModelIssue label="モデル準備" message={!(manualOpen || downloadOpen) ? error : ""} />
    <ToastNotice scope="models" type="operation-error" level="error" message={!(manualOpen || downloadOpen) ? error : ""} />
    {applicationUnknown && !(manualOpen || downloadOpen) && <p role="alert">モデル選択の保存結果が未確認です。「ローカルから取り込む」から同じ操作を確認してください。</p>}
    <ToastNotice scope="models" type="preparation-result" message={message} level="info" />
  </section>;
}
