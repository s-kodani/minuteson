import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { InspectionAttempt, inspectionLabel, placementLabel, assertPlacementReceipt, type ModelInspectionReport, type ModelPlacementReceipt, type ModelPlacementEntry } from "./model-inspection-state";

export function ModelInspectionPanel({ locked, recordingActive }: { locked: boolean; recordingActive: boolean }) {
  const [directory, setDirectory] = useState("");
  const [busy, setBusy] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState("");
  const [report, setReport] = useState<ModelInspectionReport | null>(null);
  const [placing, setPlacing] = useState(false);
  const [placements, setPlacements] = useState<ModelPlacementEntry[]>([]);
  const [catalogError, setCatalogError] = useState("");
  const [placementMessage, setPlacementMessage] = useState("");
  const attempt = useRef(new InspectionAttempt());
  const mounted = useRef(true);
  const loadPlacements = async () => {
    if (!isTauri()) return;
    try { const entries = await invoke<ModelPlacementEntry[]>("get_model_placements"); entries.forEach(placementLabel); if (mounted.current) { setPlacements(entries); setCatalogError(""); } }
    catch (cause) { if (mounted.current) setCatalogError(String(cause)); }
  };
  useEffect(() => {
    mounted.current = true;
    void loadPlacements();
    return () => {
      mounted.current = false;
      const token = attempt.current.cancel();
      if (token && isTauri()) void invoke("cancel_model_inspection", { token }).catch(() => {});
    };
  }, []);
  const inspect = async (event: FormEvent) => {
    event.preventDefault();
    if (busy || locked || recordingActive || !isTauri()) return;
    const token = crypto.randomUUID();
    attempt.current.begin(token);
    setBusy(true); setPlacing(false); setCancelling(false); setError(""); setReport(null); setPlacementMessage("");
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
    attempt.current.begin(token); setBusy(true); setPlacing(true); setCancelling(false); setError(""); setPlacementMessage("");
    try {
      const receipt = await invoke<ModelPlacementReceipt>("place_model_package", { token, directory: directory.trim(), expectedReviewHash: reviewHash });
      if (mounted.current && attempt.current.accepts(token)) { assertPlacementReceipt(receipt, token, reviewHash); setPlacementMessage("管理領域への配置を記録しました。エンジン互換性は未検証で、AI準備完了ではありません。"); setReport(null); }
    } catch (cause) {
      if (mounted.current && attempt.current.accepts(token)) setError(`${String(cause)} 未完了の配置は保全されます。再検査後、新しい操作として配置し直せます。`);
    } finally {
      if (attempt.current.finish(token) && mounted.current) { setBusy(false); setPlacing(false); setCancelling(false); await loadPlacements(); }
    }
  };
  const cancel = async () => {
    const token = attempt.current.cancel();
    if (!token) return;
    setCancelling(true); setReport(null); setError("");
    try { await invoke("cancel_model_inspection", { token }); }
    catch (cause) { if (mounted.current) setError(`取消要求を送信できませんでした。検査終了を待ってください: ${String(cause)}`); }
  };
  return <section className="analysis-card model-inspection" aria-label="AIモデルの事前検査">
    <h2>AIモデル</h2>
    <p>手動取り込みの事前検査と、管理領域へのコピーを利用できます。AI処理・録音・取り込みを止めてから開始してください。検査・配置中は新しいAI処理を待機させ、録音開始・アプリ終了時は取消と停止完了を待ちます。事前検査は読み取りのみです。コピーも既存ジョブの条件・解析設定を変更せず、実行ファイルを起動しません。動作互換性の検証・ダウンロード・プロファイルへの適用は開発中です。</p>
    <details className="analysis-settings"><summary>検査するフォルダーの準備方法</summary>
      <p>実行ファイルと、そのエンジン用のモデルを同じフォルダーへ置き、直下に <code>mon-model.json</code> を作成してください。ファイルのパスはフォルダー内の相対パスです。下記はmacOS Apple Silicon向けwhisper.cppの例です。版・ライセンス参照・ファイル名は実際の配布情報に合わせてください。対象Windowsの検証は保留中です。</p>
      <pre>{JSON.stringify({ schemaVersion: 1, engine: "whisper.cpp", engineVersion: "配布元の版", modelName: "モデル名", modelVersion: "モデルの版", quantization: null, licenseRef: "配布元のライセンス参照", target: "macos_aarch64", files: [{ relativePath: "whisper-cli", role: "executable", expectedSize: null, expectedHash: null }, { relativePath: "model.bin", role: "asr", expectedSize: null, expectedHash: null }] }, null, 2)}</pre>
      <p>sherpa-onnxは <code>segmentation</code> と <code>speaker_embedding</code>、llama.cppは <code>llm</code> が必要です。どのエンジンにも <code>executable</code> が1件必要です。必要な実行依存ライブラリは <code>runtime_library</code>、whisper.cppの任意のVADモデルは <code>vad</code> で指定できます。配布元の容量・SHAが分かる場合はexpectedSize・expectedHashを指定し、不明ならnullにしてください。</p>
      <p>ローカルで計算したSHAや、取り込み元のmanifestとの一致だけでは配布元の真正性・ライセンス許諾・動作互換性を検証したことにはなりません。未記載のファイルは検査対象外です。ライブラリを含む必要な構成をすべて記載してください。</p>
    </details>
    <form onSubmit={event => void inspect(event)}>
      <label className="prompt-label" htmlFor="model-package-directory">パッケージフォルダーの絶対パス<input id="model-package-directory" value={directory} maxLength={4096} disabled={busy || locked || recordingActive || !isTauri()} onChange={event => { setDirectory(event.target.value); setReport(null); setError(""); }} placeholder="/Users/…/model-package" /></label>
      <div className="analysis-actions"><button type="submit" disabled={busy || locked || recordingActive || !isTauri() || !directory.trim()}>ファイルを事前検査</button>{busy && <button type="button" className="secondary" disabled={cancelling} onClick={() => void cancel()}>{placing ? "配置を取り消す" : "検査を取り消す"}</button>}</div>
    </form>
    {busy && <p role="status">{cancelling ? "取消要求済み。処理の終了を待っています。配置の結果は下の記録を確認してください。" : placing ? "確認済みの素材を再照合し、管理領域へコピー・同期しています。" : "対象OS・CPU、構成、容量・SHAを検査しています。大きなモデルは時間がかかります。"}</p>}
    {!isTauri() && <p>デスクトップアプリで利用できます。</p>}
    {recordingActive && <p>録音停止後に検査・配置できます。録音開始要求があった場合は処理を取り消し、停止完了を待ちます。</p>}
    {error && <p className="action-error" role="alert">{error}</p>}
    {placementMessage && <p role="status">{placementMessage}</p>}
    {report && <div aria-label="モデル検査結果"><h3>{inspectionLabel(report)}</h3>
      <p>{report.manifest.modelName} / {report.manifest.modelVersion} · {report.manifest.engine} / {report.manifest.engineVersion} · {report.manifest.target}</p>
      <p>ファイル {report.files.length} 件・合計 {report.totalBytes.toLocaleString()} bytes。実行ファイルは起動していません。モデルの読み込み・推論互換性・精度・速度は未検証です。</p>
      <details><summary>ファイルの検査記録</summary><p>manifest SHA: <code>{report.manifestHash}</code></p><ul>{report.files.map(file => <li key={file.relativePath}><strong>{file.relativePath}</strong>（{file.role}）<p>{file.actualSize.toLocaleString()} bytes · 容量指定: {file.expectedSizeChecked ? "一致" : "なし"} · SHA指定: {file.expectedHashChecked ? "一致" : "なし"}<br /><code>{file.actualHash}</code></p></li>)}</ul><p>ライセンス参照（未確認）: {report.manifest.licenseRef}</p></details>
      <p>この検査結果は今回の読み取り時点の記録です。ファイルを変更した場合は再検査が必要です。この結果でAI処理を開始することはできません。コピーはOSのアプリデータ領域へ保存し、会議保存先と分離します。未完了の配置は自動削除しません。</p>
      <button type="button" className="secondary" disabled={busy || locked || recordingActive || !isTauri()} onClick={() => void place()}>管理領域へコピー</button>
    </div>}
    <section aria-label="モデル配置記録"><h3>モデル配置記録</h3><button type="button" className="small-secondary" disabled={busy || locked || !isTauri()} onClick={() => void loadPlacements()}>配置記録を再読込</button>
      <p>記録上のコピー結果です。容量は検査した素材の合計で、途中コピー済みの容量ではありません。配置後の実ファイルや動作互換性は別途再照合が必要です。再起動時に未完了の配置・実行ファイルを自動再開しません。互換性確認と解析への適用、配置パッケージ全体のメタデータ移送は未接続です。モデル本体はバックアップ対象外です。</p>
      {catalogError && <p className="action-error" role="alert">{catalogError}</p>}
      {placements.length === 0 ? <p>モデル配置記録はありません。</p> : <ul>{placements.map(entry => <li key={entry.packageId}><strong>{entry.modelName}</strong> / {entry.modelVersion} · {entry.engine} / {entry.engineVersion}<p>{placementLabel(entry)} · {entry.totalBytes.toLocaleString()} bytes<br />操作ID: <code>{entry.packageId}</code></p></li>)}</ul>}
    </section>
  </section>;
}
