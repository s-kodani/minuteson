import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { InspectionAttempt, inspectionLabel, placementLabel, assertPlacementReceipt, assertCompatibilitySummary, type ModelCompatibilitySummary, type ModelInspectionReport, type ModelPlacementReceipt, type ModelPlacementEntry } from "./model-inspection-state";
import { applicationChanges, assertApplicationPreview, assertApplicationResult, assertAppliedPreferences, assertProfileCatalog, canReviewApplication, preferenceCommand, profileConfigurationLabel, profileName, type ModelApplicationPreview, type ModelProfileName, type ModelProfileView } from "./model-profile-state";

export function ModelInspectionPanel({ locked, recordingActive }: { locked: boolean; recordingActive: boolean }) {
  const [directory, setDirectory] = useState("");
  const [busy, setBusy] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState("");
  const [report, setReport] = useState<ModelInspectionReport | null>(null);
  const [placing, setPlacing] = useState(false);
  const [checking, setChecking] = useState(false);
  const [placements, setPlacements] = useState<ModelPlacementEntry[]>([]);
  const [catalogError, setCatalogError] = useState("");
  const [placementMessage, setPlacementMessage] = useState("");
  const [profiles, setProfiles] = useState<ModelProfileView[] | null>(null);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [reviewing, setReviewing] = useState(false);
  const [applying, setApplying] = useState(false);
  const [application, setApplication] = useState<{ preview: ModelApplicationPreview; entry: ModelPlacementEntry; operationId: string } | null>(null);
  const [applicationError, setApplicationError] = useState("");
  const [applicationUnknown, setApplicationUnknown] = useState(false);
  const catalogSequence = useRef(0);
  const dialog = useRef<HTMLDialogElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const attempt = useRef(new InspectionAttempt());
  const mounted = useRef(true);
  const loadPlacements = async () => {
    if (!isTauri()) return;
    const sequence = ++catalogSequence.current;
    setCatalogLoading(true);
    try {
      const [entries, views] = await Promise.all([invoke<ModelPlacementEntry[]>("get_model_placements"), invoke<ModelProfileView[]>("get_model_profiles")]);
      entries.forEach(placementLabel); assertProfileCatalog(views);
      if (mounted.current && sequence === catalogSequence.current) { setPlacements(entries); setProfiles(views); setCatalogError(""); }
    }
    catch (cause) { if (mounted.current && sequence === catalogSequence.current) { setProfiles(null); setCatalogError(String(cause)); } }
    finally { if (mounted.current && sequence === catalogSequence.current) setCatalogLoading(false); }
  };
  useEffect(() => {
    mounted.current = true;
    void loadPlacements();
    return () => {
      mounted.current = false;
      catalogSequence.current++;
      const token = attempt.current.cancel();
      if (token && isTauri()) void invoke("cancel_model_inspection", { token }).catch(() => {});
    };
  }, []);
  useEffect(() => {
    if (!application || !dialog.current) return;
    const element = dialog.current;
    if (!element.open) element.showModal();
    element.querySelector<HTMLElement>("h2")?.focus({ preventScroll: true });
    element.scrollTop = 0;
    return () => { element.close(); if (opener.current?.isConnected) opener.current.focus(); };
  }, [application]);
  useEffect(() => {
    if (recordingActive && application && !busy) {
      setApplication(null);
      setError("録音状態が変わったため適用の確認を閉じました。録音停止後に再確認してください。");
    }
  }, [recordingActive, application, busy]);
  const reviewApplication = async (entry: ModelPlacementEntry, name: ModelProfileName, element: HTMLElement) => {
    if (busy || locked || recordingActive || catalogLoading || !profiles || !isTauri() || !canReviewApplication(entry)) return;
    const current = profiles.find(p => p.name === name);
    if (!current) return;
    opener.current = element;
    const token = crypto.randomUUID();
    attempt.current.begin(token); setBusy(true); setReviewing(true); setCancelling(false); setError(""); setReport(null); setPlacementMessage("");
    try {
      const preview = await invoke<ModelApplicationPreview>("preview_model_profile_application", { token, profile: name, packageId: entry.packageId, expectedReviewHash: entry.reviewHash, checkOperationId: entry.compatibility!.operationId });
      if (mounted.current && attempt.current.accepts(token)) {
        assertApplicationPreview(preview, entry, name, current);
        setApplicationError(""); setApplicationUnknown(false);
        setApplication({ preview: structuredClone(preview), entry: structuredClone(entry), operationId: crypto.randomUUID() });
      }
    } catch (cause) { if (mounted.current && attempt.current.accepts(token)) setError(String(cause)); }
    finally {
      if (attempt.current.finish(token) && mounted.current) {
        await loadPlacements();
        if (mounted.current) { setBusy(false); setReviewing(false); setCancelling(false); }
      }
    }
  };
  const applyApplication = async () => {
    if (!application || busy || locked || recordingActive || !isTauri()) return;
    const { preview, operationId } = application;
    attempt.current.begin(operationId); setBusy(true); setApplying(true); setCancelling(false); setApplicationError(""); setPlacementMessage("");
    try {
      const result = await invoke<ModelProfileView>("apply_model_profile_application", { token: operationId, preview });
      if (mounted.current && attempt.current.accepts(operationId)) {
        assertApplicationResult(result, operationId, preview);
        setApplication(null); setApplicationUnknown(false);
        setPlacementMessage(`${profileName(preview.profile)}の${preview.after.engine}モデル選択を保存しました。既存ジョブは変更していません。次の新規解析から設定を使用します。品質・速度・通常条件での互換性は未評価です。`);
        try {
          const preferences = await invoke<Parameters<typeof assertAppliedPreferences>[0]>(preferenceCommand(preview.after.engine));
          assertAppliedPreferences(preferences, preview);
        } catch (cause) { if (mounted.current) setError(`構成の保存は成功しましたが、現在の解析設定を確認できません。記録と会議の解析設定を再読込してください: ${String(cause)}`); }
      }
    } catch (cause) {
      if (mounted.current && attempt.current.accepts(operationId)) { setApplicationUnknown(true); setApplicationError(`適用結果を確定できません: ${String(cause)} 同じ操作で結果を確認するか、閉じて構成記録を確認してください。条件が変わった場合は再確認が必要です。`); }
    } finally {
      const accepted = attempt.current.accepts(operationId);
      if (attempt.current.finish(operationId) && mounted.current) {
        if (!accepted) { setApplication(null); setPlacementMessage("取消要求後の処理が終了しました。保存が先に確定した可能性があるため、下の構成記録を確認してください。"); }
        await loadPlacements();
        if (mounted.current) { setBusy(false); setApplying(false); setCancelling(false); }
      }
    }
  };
  const inspect = async (event: FormEvent) => {
    event.preventDefault();
    if (busy || locked || recordingActive || !isTauri()) return;
    const token = crypto.randomUUID();
    attempt.current.begin(token);
    setBusy(true); setPlacing(false); setChecking(false); setCancelling(false); setError(""); setReport(null); setPlacementMessage("");
    try {
      const result = await invoke<ModelInspectionReport>("inspect_model_package", { token, directory: directory.trim() });
      if (mounted.current && attempt.current.accepts(token)) {
        inspectionLabel(result);
        setReport(result);
      }
    } catch (cause) {
      if (mounted.current && attempt.current.accepts(token)) setError(String(cause));
    } finally {
      if (attempt.current.finish(token) && mounted.current) { setBusy(false); setCancelling(false); }
    }
  };
  const place = async () => {
    if (!report || busy || locked || recordingActive || !isTauri()) return;
    if (!window.confirm(`「${report.manifest.modelName}」の検査済みファイル（${report.totalBytes.toLocaleString()} bytes）をアプリのモデル管理領域へコピーしますか？\n元ファイル・既存会議・ジョブ・設定は変更しません。実行ファイルは起動せず、配置後も互換性検証待ちです。\n中断時の途中配置は削除せず保全します。配布元・ライセンスの確認は利用者が行ってください。`)) return;
    const token = crypto.randomUUID(), reviewHash = report.reviewHash;
    attempt.current.begin(token); setBusy(true); setPlacing(true); setChecking(false); setCancelling(false); setError(""); setPlacementMessage("");
    try {
      const receipt = await invoke<ModelPlacementReceipt>("place_model_package", { token, directory: directory.trim(), expectedReviewHash: reviewHash });
      if (mounted.current && attempt.current.accepts(token)) { assertPlacementReceipt(receipt, token, reviewHash); setPlacementMessage("管理領域への配置を記録しました。エンジン互換性は未検証で、AI準備完了ではありません。"); setReport(null); }
    } catch (cause) {
      if (mounted.current && attempt.current.accepts(token)) setError(`${String(cause)} 未完了の配置は保全されます。再検査後、新しい操作として配置し直せます。`);
    } finally {
      if (attempt.current.finish(token) && mounted.current) { setBusy(false); setPlacing(false); setCancelling(false); await loadPlacements(); }
    }
  };
  const check = async (entry: ModelPlacementEntry) => {
    if (!entry.placementRecorded || busy || locked || recordingActive || !isTauri()) return;
    if (!window.confirm(`「${entry.modelName}」のCPU互換性を確認しますか？\n取り込んだ実行ファイル（${entry.engine} / 申告版 ${entry.engineVersion}）を利用者の権限で起動します。配布元・ライセンス・実行ファイルを信頼できることを確認してください。ネットワークや利用者のファイルへのアクセスを隔離する仕組みはありません。\nCPU・1スレッドで合成入力だけを使用します。会議音声は使いません。アプリは既存ジョブ・設定・プロファイルを変更しません。推論の待ち時間は最大120秒（ファイル照合は別）で、途中ログを保全します。\n成功しても精度・速度・話者数の正式評価や、解析への適用済みにはなりません。`)) return;
    const token = crypto.randomUUID();
    attempt.current.begin(token); setBusy(true); setPlacing(false); setChecking(true); setCancelling(false); setError(""); setReport(null); setPlacementMessage("");
    try {
      const summary = await invoke<ModelCompatibilitySummary>("check_model_compatibility", { token, packageId: entry.packageId, expectedReviewHash: entry.reviewHash });
      if (mounted.current && attempt.current.accepts(token)) {
        assertCompatibilitySummary(summary, token, entry);
        if (summary.state === "failed") setError(`今回のCPU互換性確認は失敗しました: ${summary.errorMessage} モデルの恒久的な不適合を示すものではありません。ログを確認し、新しい操作として再確認できます。`);
        else setPlacementMessage(summary.state === "succeeded" ? "固定した合成入力でCPU互換性を確認しました。プロファイルは変更していません。適用状態は構成記録を確認してください。精度・速度・実データの互換性は未評価です。" : "互換性確認を取り消しました。ログを保全しました。自動再実行はしません。");
      }
    } catch (cause) {
      if (mounted.current && attempt.current.accepts(token)) setError(String(cause));
    } finally {
      if (attempt.current.finish(token) && mounted.current) { setBusy(false); setChecking(false); setCancelling(false); await loadPlacements(); }
    }
  };
  const cancel = async () => {
    const token = attempt.current.cancel();
    if (!token) return;
    setCancelling(true); setReport(null); setError("");
    try { await invoke("cancel_model_inspection", { token }); }
    catch (cause) { if (mounted.current) setError(`取消要求を送信できませんでした。処理終了を待ってください: ${String(cause)}`); }
  };
  return <section className="analysis-card model-inspection" aria-label="AIモデルの事前検査">
    <h2>AIモデル</h2>
    <p>手動取り込みの事前検査・管理領域へのコピー・CPU互換性確認と、標準／高精度へのモデル選択を利用できます。AI処理・録音・取り込みを止めてから開始してください。準備中は新しいAI処理を待機させ、録音開始・アプリ終了時は取消と停止完了を待ちます。互換性確認だけは明示的な確認後に実行ファイルを起動します。検査・コピー・適用では起動しません。ダウンロードは開発中です。</p>
    <details className="analysis-settings"><summary>検査するフォルダーの準備方法</summary>
      <p>実行ファイルと、そのエンジン用のモデルを同じフォルダーへ置き、直下に <code>mon-model.json</code> を作成してください。ファイルのパスはフォルダー内の相対パスです。下記はmacOS Apple Silicon向けwhisper.cppの例です。版・ライセンス参照・ファイル名は実際の配布情報に合わせてください。対象Windowsの検証は保留中です。</p>
      <pre>{JSON.stringify({ schemaVersion: 1, engine: "whisper.cpp", engineVersion: "配布元の版", modelName: "モデル名", modelVersion: "モデルの版", quantization: null, licenseRef: "配布元のライセンス参照", target: "macos_aarch64", files: [{ relativePath: "whisper-cli", role: "executable", expectedSize: null, expectedHash: null }, { relativePath: "model.bin", role: "asr", expectedSize: null, expectedHash: null }] }, null, 2)}</pre>
      <p>sherpa-onnxは <code>segmentation</code> と <code>speaker_embedding</code>、llama.cppは <code>llm</code> が必要です。どのエンジンにも <code>executable</code> が1件必要です。必要な実行依存ライブラリは <code>runtime_library</code>、whisper.cppの任意のVADモデルは <code>vad</code> で指定できます。配布元の容量・SHAが分かる場合はexpectedSize・expectedHashを指定し、不明ならnullにしてください。</p>
      <p>ローカルで計算したSHAや、取り込み元のmanifestとの一致だけでは配布元の真正性・ライセンス許諾・動作互換性を検証したことにはなりません。未記載のファイルは検査対象外です。ライブラリを含む必要な構成をすべて記載してください。</p>
    </details>
    <form onSubmit={event => void inspect(event)}>
      <label className="prompt-label" htmlFor="model-package-directory">パッケージフォルダーの絶対パス<input id="model-package-directory" value={directory} maxLength={4096} disabled={busy || locked || recordingActive || !isTauri()} onChange={event => { setDirectory(event.target.value); setReport(null); setError(""); }} placeholder="/Users/…/model-package" /></label>
      <div className="analysis-actions"><button type="submit" disabled={busy || locked || recordingActive || !isTauri() || !directory.trim()}>ファイルを事前検査</button>{busy && !application && <button type="button" className="secondary" disabled={cancelling} onClick={() => void cancel()}>{checking ? "互換性確認を取り消す" : placing ? "配置を取り消す" : reviewing ? "適用の事前確認を取り消す" : "検査を取り消す"}</button>}</div>
    </form>
    {busy && <p role="status">{cancelling ? "取消要求済み。処理の終了を待っています。確定した結果は下の記録を確認してください。" : applying ? "モデルと試行証拠を再照合し、確認した設定・構成版を保存しています。" : reviewing ? "モデルとCPU試行証拠を再照合し、変更内容を準備しています。" : checking ? "配置ファイルを再照合し、CPU・1スレッドで固定した合成入力の互換性を確認しています。" : placing ? "確認済みの素材を再照合し、管理領域へコピー・同期しています。" : "対象OS・CPU、構成、容量・SHAを検査しています。大きなモデルは時間がかかります。"}</p>}
    {!isTauri() && <p>デスクトップアプリで利用できます。</p>}
    {recordingActive && <p>録音停止後に検査・配置・互換性確認・適用できます。録音開始要求があった場合は処理を取り消し、停止完了を待ちます。</p>}
    {error && <p className="action-error" role="alert">{error}</p>}
    {placementMessage && <p role="status">{placementMessage}</p>}
    {report && <div aria-label="モデル検査結果"><h3>{inspectionLabel(report)}</h3>
      <p>{report.manifest.modelName} / {report.manifest.modelVersion} · {report.manifest.engine} / {report.manifest.engineVersion} · {report.manifest.target}</p>
      <p>ファイル {report.files.length} 件・合計 {report.totalBytes.toLocaleString()} bytes。実行ファイルは起動していません。モデルの読み込み・推論互換性・精度・速度は未検証です。</p>
      <details><summary>ファイルの検査記録</summary><p>manifest SHA: <code>{report.manifestHash}</code></p><ul>{report.files.map(file => <li key={file.relativePath}><strong>{file.relativePath}</strong>（{file.role}）<p>{file.actualSize.toLocaleString()} bytes · 容量指定: {file.expectedSizeChecked ? "一致" : "なし"} · SHA指定: {file.expectedHashChecked ? "一致" : "なし"}<br /><code>{file.actualHash}</code></p></li>)}</ul><p>ライセンス参照（未確認）: {report.manifest.licenseRef}</p></details>
      <p>この検査結果は今回の読み取り時点の記録です。ファイルを変更した場合は再検査が必要です。この結果でAI処理を開始することはできません。コピーはOSのアプリデータ領域へ保存し、会議保存先と分離します。未完了の配置は自動削除しません。</p>
      <button type="button" className="secondary" disabled={busy || locked || recordingActive || !isTauri()} onClick={() => void place()}>管理領域へコピー</button>
    </div>}
    <section aria-label="モデル配置記録"><h3>モデル配置記録</h3><button type="button" className="small-secondary" disabled={busy || locked || catalogLoading || !isTauri()} onClick={() => void loadPlacements()}>配置・構成記録を再読込</button>
      {catalogLoading && <p role="status">配置とプロファイル構成を再読込しています。</p>}
      <p>コピーと最後のCPU互換性確認の記録です。容量は検査した素材の合計で、途中コピー済みの容量ではありません。互換性確認はwhisper.cpp・sherpa-onnxの10秒無音入力と、llama.cppの固定JSON生成のみです。申告版は実行ファイルから検証した版ではありません。成功後のファイル変更・実会議の精度・速度は保証しません。再起動時に未完了の処理を自動再開しません。バックアップではパッケージ名・SHA・構成と数値設定のメタデータを未配置の履歴として保持します。モデル本体・配置パス・端末内の利用可能状態・CPU互換性試行ログは移送しません。復元後は現在のモデル選択を変更せず、モデルの準備は別途行います。</p>
      {catalogError && <p className="action-error" role="alert">{catalogError}</p>}
      {placements.length === 0 ? <p>モデル配置記録はありません。</p> : <ul>{placements.map(entry => <li key={entry.packageId}><strong>{entry.modelName}</strong> / {entry.modelVersion} · {entry.engine} / {entry.engineVersion}<p>{placementLabel(entry)} · {entry.totalBytes.toLocaleString()} bytes<br />配置ID: <code>{entry.packageId}</code></p>{entry.compatibility && <p>確認操作ID: <code>{entry.compatibility.operationId}</code>{entry.compatibility.errorMessage && <><br />{entry.compatibility.errorMessage}</>}</p>}<div className="analysis-actions"><button type="button" className="small-secondary" disabled={!entry.placementRecorded || entry.compatibility?.state === "preparing" || busy || locked || recordingActive || !isTauri()} onClick={() => void check(entry)}>{entry.compatibility ? "互換性を再確認" : "互換性を確認"}</button>{(["standard", "high_accuracy"] as const).map(name => <button key={name} type="button" className="small-secondary" disabled={!canReviewApplication(entry) || !profiles || catalogLoading || busy || locked || recordingActive || !isTauri()} onClick={event => void reviewApplication(entry, name, event.currentTarget)}>{profileName(name)}への適用を確認</button>)}</div></li>)}</ul>}
    </section>
    <section aria-label="プロファイル構成記録"><h3>プロファイル構成記録</h3>
      <p>保存した管理モデルの選択を表示します。設定や最新の互換性記録との一致は、現在のファイル健全性・GPU互換性・AI準備完了を保証しません。会議の解析条件を手動変更した場合は不一致として表示し、再適用の確認時に現在の数値条件を維持します。</p>
      {!profiles ? <p>構成記録を読込できていません。配置・構成記録を再読込してください。</p> : profiles.map(view => <article key={view.name}><h4>{profileName(view.name)} · {view.id ? `構成版 #${view.profileVersion}` : "構成版なし"}</h4><p>{profileConfigurationLabel(view)}</p>{view.components.length > 0 && <ul>{view.components.map(component => <li key={component.engine}><strong>{component.engine}</strong> · {component.modelName} / {component.modelVersion}<p>{component.settingsMatch ? "保存設定と一致" : "現在の設定と不一致"} · {component.compatibilityCurrent ? "最新のCPU成功記録と一致" : "最新の互換性記録と不一致"}<br />配置ID: <code>{component.packageId}</code></p></li>)}</ul>}<p>不足: {(["whisper.cpp", "sherpa-onnx", "llama.cpp"] as const).filter(engine => !view.components.some(c => c.engine === engine)).join("、") || "なし（DB上の選択のみ）"}</p></article>)}
    </section>
    {application && <dialog ref={dialog} className="model-application-dialog" aria-labelledby="model-application-title" onCancel={event => { event.preventDefault(); if (!busy) setApplication(null); }}>
      <h2 id="model-application-title" tabIndex={-1}>{profileName(application.preview.profile)}へのモデル適用を確認</h2>
      <p>{application.entry.modelName} / {application.entry.modelVersion} · {application.preview.after.engine}</p>
      <p>構成版 #{application.preview.expectedProfileVersion} → #{application.preview.expectedProfileVersion + 1} · 解析設定版 #{application.preview.expectedPreferencesVersion}</p>
      <p>下記のファイルパスだけを変更します。他のプロファイル・エンジン、既存ジョブ・会議は変更しません。保存後の新規解析に使用し、解析を自動開始する操作ではありません。</p>
      <table className="model-application-changes"><caption>変更するファイルと維持する解析条件</caption><thead><tr><th scope="col">設定</th><th scope="col">変更前</th><th scope="col">適用後</th></tr></thead><tbody>{applicationChanges(application.preview).map(change => <tr key={change.key}><th scope="row">{settingLabel(change.key)}<small>{change.file ? "ファイル選択" : "維持"}</small></th><td>{settingText(change.before)}</td><td>{settingText(change.after)}</td></tr>)}</tbody></table>
      <p>CPU試行は1スレッド・GPUなし等の固定条件です。ここに表示した通常条件のGPU・メモリ・実会議の互換性や品質・速度は未検証です。保存直前にモデル・試行証拠と設定版を再照合し、変更があれば拒否します。管理パッケージとジョブの構成版参照への統合は開発中です。</p>
      <details><summary>確認内容の識別子</summary><p>配置ID: <code>{application.preview.packageId}</code><br />CPU確認操作ID: <code>{application.preview.checkOperationId}</code><br />確認内容SHA: <code>{application.preview.reviewDigest}</code><br />適用操作ID: <code>{application.operationId}</code></p></details>
      {applicationError && <p className="action-error" role="alert">{applicationError}</p>}
      {error && <p className="action-error" role="alert">{error}</p>}
      {busy && <p role="status">{cancelling ? "取消要求済み。保存が先に確定する場合があります。処理終了後に構成記録を確認してください。" : applying ? "モデルと試行証拠を再照合して保存しています。" : "記録を確認しています。"}</p>}
      <div className="analysis-actions"><button type="button" disabled={busy || locked || recordingActive} onClick={() => void applyApplication()}>{applicationUnknown ? "同じ操作で適用結果を確認" : "確認した内容で適用"}</button><button type="button" className="secondary" disabled={busy} onClick={() => setApplication(null)}>{applicationUnknown ? "閉じて構成記録を確認" : "適用せず閉じる"}</button>{applying && <button type="button" className="secondary" disabled={cancelling} onClick={() => void cancel()}>適用を取り消す</button>}</div>
    </dialog>}
  </section>;
}

function settingText(value: unknown): string {
  if (Array.isArray(value)) return value.length ? value.join("\n") : "なし";
  if (value === null || value === "") return "未選択";
  if (typeof value === "boolean") return value ? "有効" : "無効";
  return String(value);
}

function settingLabel(key: string): string {
  return ({ executablePath: "実行ファイル", modelPath: "モデル", vadModelPath: "VADモデル", segmentationModelPath: "区間分割モデル", embeddingModelPath: "話者埋め込みモデル", runtimeLibraryPaths: "実行依存ライブラリー", threads: "CPUスレッド数", useGpu: "GPU使用", clusterThresholdMilli: "クラスタ閾値（1000分の1単位）", windowShiftMilli: "窓移動比率（1000分の1単位）", minDurationOnMs: "最小発話長（ms）", minDurationOffMs: "結合ギャップ（ms）", contextTokens: "コンテキスト上限（tokens）", predictionTokens: "出力上限（tokens）", gpuLayers: "GPUへ配置する層数", seed: "seed", promptBudget: "抽出入力上限（bytes）" } as Record<string, string>)[key] ?? key;
}
