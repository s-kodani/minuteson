import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { InspectionAttempt, inspectionLabel, type ModelInspectionReport } from "./model-inspection-state";

export function ModelInspectionPanel({ locked }: { locked: boolean }) {
  const [directory, setDirectory] = useState("");
  const [busy, setBusy] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState("");
  const [report, setReport] = useState<ModelInspectionReport | null>(null);
  const attempt = useRef(new InspectionAttempt());
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const token = attempt.current.cancel();
      if (token && isTauri()) void invoke("cancel_model_inspection", { token }).catch(() => {});
    };
  }, []);
  const inspect = async (event: FormEvent) => {
    event.preventDefault();
    if (busy || locked || !isTauri()) return;
    const token = crypto.randomUUID();
    attempt.current.begin(token);
    setBusy(true); setCancelling(false); setError(""); setReport(null);
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
  const cancel = async () => {
    const token = attempt.current.cancel();
    if (!token) return;
    setCancelling(true); setReport(null); setError("");
    try { await invoke("cancel_model_inspection", { token }); }
    catch (cause) { if (mounted.current) setError(`取消要求を送信できませんでした。検査終了を待ってください: ${String(cause)}`); }
  };
  return <section className="analysis-card model-inspection" aria-label="AIモデルの事前検査">
    <h2>AIモデル</h2>
    <p>手動取り込み前のファイル検査を利用できます。モデルの導入・ダウンロード・プロファイルへの適用は開発中です。この操作はファイルの読み取りだけを行い、録音・既存ジョブの条件や設定を変更しません。</p>
    <details className="analysis-settings"><summary>検査するフォルダーの準備方法</summary>
      <p>実行ファイルと、そのエンジン用のモデルを同じフォルダーへ置き、直下に <code>mon-model.json</code> を作成してください。ファイルのパスはフォルダー内の相対パスです。下記はmacOS Apple Silicon向けwhisper.cppの例です。版・ライセンス参照・ファイル名は実際の配布情報に合わせてください。対象Windowsの検証は保留中です。</p>
      <pre>{JSON.stringify({ schemaVersion: 1, engine: "whisper.cpp", engineVersion: "配布元の版", modelName: "モデル名", modelVersion: "モデルの版", quantization: null, licenseRef: "配布元のライセンス参照", target: "macos_aarch64", files: [{ relativePath: "whisper-cli", role: "executable", expectedSize: null, expectedHash: null }, { relativePath: "model.bin", role: "asr", expectedSize: null, expectedHash: null }] }, null, 2)}</pre>
      <p>sherpa-onnxは <code>segmentation</code> と <code>speaker_embedding</code>、llama.cppは <code>llm</code> が必要です。どのエンジンにも <code>executable</code> が1件必要です。必要な実行依存ライブラリは <code>runtime_library</code>、whisper.cppの任意のVADモデルは <code>vad</code> で指定できます。配布元の容量・SHAが分かる場合はexpectedSize・expectedHashを指定し、不明ならnullにしてください。</p>
      <p>ローカルで計算したSHAや、取り込み元のmanifestとの一致だけでは配布元の真正性・ライセンス許諾・動作互換性を検証したことにはなりません。未記載のファイルは検査対象外です。ライブラリを含む必要な構成をすべて記載してください。</p>
    </details>
    <form onSubmit={event => void inspect(event)}>
      <label className="prompt-label" htmlFor="model-package-directory">パッケージフォルダーの絶対パス<input id="model-package-directory" value={directory} maxLength={4096} disabled={busy || locked || !isTauri()} onChange={event => { setDirectory(event.target.value); setReport(null); setError(""); }} placeholder="/Users/…/model-package" /></label>
      <div className="analysis-actions"><button type="submit" disabled={busy || locked || !isTauri() || !directory.trim()}>ファイルを事前検査</button>{busy && <button type="button" className="secondary" disabled={cancelling} onClick={() => void cancel()}>検査を取り消す</button>}</div>
    </form>
    {busy && <p role="status">{cancelling ? "取消要求済み。検査の終了を待っています。" : "対象OS・CPU、構成、容量・SHAを検査しています。大きなモデルは時間がかかります。"}</p>}
    {!isTauri() && <p>デスクトップアプリで利用できます。</p>}
    {error && <p className="action-error" role="alert">{error}</p>}
    {report && <div aria-label="モデル検査結果"><h3>{inspectionLabel(report)}</h3>
      <p>{report.manifest.modelName} / {report.manifest.modelVersion} · {report.manifest.engine} / {report.manifest.engineVersion} · {report.manifest.target}</p>
      <p>ファイル {report.files.length} 件・合計 {report.totalBytes.toLocaleString()} bytes。実行ファイルは起動していません。モデルの読み込み・推論互換性・精度・速度は未検証です。</p>
      <details><summary>ファイルの検査記録</summary><p>manifest SHA: <code>{report.manifestHash}</code></p><ul>{report.files.map(file => <li key={file.relativePath}><strong>{file.relativePath}</strong>（{file.role}）<p>{file.actualSize.toLocaleString()} bytes · 容量指定: {file.expectedSizeChecked ? "一致" : "なし"} · SHA指定: {file.expectedHashChecked ? "一致" : "なし"}<br /><code>{file.actualHash}</code></p></li>)}</ul><p>ライセンス参照（未確認）: {report.manifest.licenseRef}</p></details>
      <p>この検査結果は今回の読み取り時点の記録です。ファイルを変更した場合は再検査が必要です。モデルはまだ導入されておらず、この結果でAI処理を開始することはできません。</p>
    </div>}
  </section>;
}
