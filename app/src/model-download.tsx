import { IconButton } from "./action-icon";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";
import { SettingsDisclosure } from "./settings-disclosure";
import { IssueNotice, IssueStatus, ToastNotice } from "./toast";
import { assertPlacementReceipt, type ModelPlacementEntry, type ModelPlacementReceipt } from "./model-inspection-state";
import { acceptDownloadProgress, assertDownloadCatalog, assertDownloadCopyResult, assertDownloadLifetime, assertDownloadReadback, assertDownloadReceipt, assertDownloadReview, assertOfficialModelOptions, downloadDigest, downloadTransferBytes, parseDownloadDefinition, type DownloadCatalog, type DownloadReceipt, type DownloadReview, type OfficialModelOption } from "./model-download-state";

type Pending = { kind: "download" | "copy"; token: string; downloadId: string; review: DownloadReview; reviewHash?: string; stopped: boolean };
// Navigation must not create a new writer after an uncertain response. Restart
// discovers native intents separately; this is not persistent success evidence.
let unresolved: Pending | null = null;
export function ModelDownloadPanel({ locked, onBusyChange, onChanged, compact = false, placements = [], onShowPlacements }: { locked: boolean; onBusyChange: (value: boolean) => void; onChanged: () => Promise<void>; compact?: boolean; placements?: ModelPlacementEntry[]; onShowPlacements?: () => void }) {
  const [definition, setDefinition] = useState("");
  const [review, setReview] = useState<DownloadReview | null>(null);
  const [receipt, setReceipt] = useState<DownloadReceipt | null>(null);
  const [pending, setPending] = useState<Pending | null>(() => unresolved && structuredClone(unresolved));
  const [catalog, setCatalog] = useState<DownloadCatalog | null>(null);
  const [catalogError, setCatalogError] = useState("");
  const [official, setOfficial] = useState<OfficialModelOption[] | null>(null);
  const [officialError, setOfficialError] = useState("");
  const [officialOpen, setOfficialOpen] = useState(false);
  const [recordsOpen, setRecordsOpen] = useState(false);
  const [recordsFeedback, setRecordsFeedback] = useState("");
  const [recordsError, setRecordsError] = useState("");
  const [error, setError] = useState(() => unresolved ? "前の操作の結果は未確定です。同じ操作の停止・結果確認を行ってください。" : "");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState("");
  const [received, setReceived] = useState(0);
  const [modal, setModal] = useState<"download" | "copy" | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);
  const mounted = useRef(true), operation = useRef<string | null>(null), running = useRef(false), sequence = useRef(0), catalogSequence = useRef(0), opener = useRef<HTMLElement | null>(null);
  const officialTrigger = useRef<HTMLButtonElement | null>(null);
  const recordsTrigger = useRef<HTMLButtonElement | null>(null), receiptButton = useRef<HTMLButtonElement | null>(null), recordsCopyFocus = useRef(false);
  const recordsVisible = useRef(recordsOpen); recordsVisible.current = recordsOpen;
  const current = useRef({ pending, review, receipt, locked }); current.current = { pending, review, receipt, locked };
  // This is a matching historical copy record, not current file availability,
  // CPU readiness or an applied profile. Direct the user to those separate
  // records instead of inviting a second copy of an already registered source.
  const knownCopy = !!onShowPlacements && !!receipt && placements.some(entry => entry.placementRecorded && entry.sourceType === "download" && entry.download?.operationId === receipt.operationId && entry.download.recipeHash === receipt.recipeHash && entry.reviewHash === receipt.report.reviewHash);
  const retain = (value: Pending | null) => { unresolved = value && structuredClone(value); if (mounted.current) setPending(value); };
  const begin = (label: string, token?: string) => { if (running.current || current.current.locked) return false; running.current = true; operation.current = token ?? null; setBusy(label); setMessage(""); return true; };
  const end = () => { operation.current = null; running.current = false; if (mounted.current) setBusy(""); };
  const loadCatalog = async () => {
    if (!isTauri()) return;
    const token = crypto.randomUUID(), request = ++catalogSequence.current;
    // Catalog calls are read-only but use Inspector/lease too. Avoid concurrent
    // listing while this panel has its own long-running reader/writer.
    if (running.current) return;
    running.current = true; setBusy("取得記録を読み取っています");
    operation.current = token;
    try {
      const result = assertDownloadCatalog(await invoke("get_model_download_catalog", { token }));
      if (mounted.current && request === catalogSequence.current) { setCatalog(result); setCatalogError(""); }
    } catch (cause) { if (mounted.current && request === catalogSequence.current) setCatalogError(String(cause)); throw cause; }
    finally { end(); }
  };
  useEffect(() => {
    mounted.current = true;
    void loadCatalog().catch(() => {});
    let dispose = false, unlisten: (() => void) | undefined;
    if (isTauri()) void invoke("get_official_model_options").then(assertOfficialModelOptions).then(options => {
      if (!dispose && mounted.current) { setOfficial(options); setOfficialError(""); }
    }).catch(cause => { if (!dispose && mounted.current) setOfficialError(String(cause)); });
    if (isTauri()) void listen("model-download-progress", event => {
      const value = current.current.pending;
      if (!mounted.current || value?.kind !== "download") return;
      setReceived(previous => acceptDownloadProgress(event.payload, value.token, value.review, previous) ?? previous);
    }).then(value => { if (dispose) value(); else unlisten = value; }).catch(() => {});
    return () => {
      mounted.current = false; dispose = true; unlisten?.(); sequence.current++; catalogSequence.current++;
      for (const token of new Set([operation.current, unresolved?.token])) if (token && isTauri()) void invoke("cancel_model_inspection", { token }).catch(() => {});
      onBusyChange(false);
    };
  }, []);
  useEffect(() => { onBusyChange(!!busy || !!pending); }, [busy, pending, onBusyChange]);
  // Discard consent, not half of a verified receipt/review pair. A later copy
  // requires a fresh checkbox and native physical readback before any writes.
  useEffect(() => { if (locked && !busy) { setModal(null); setAcknowledged(false); } }, [locked, busy]);
  const preview = async (element: HTMLElement, source = definition) => {
    if (pending || !isTauri() || !begin("定義を確認しています")) return;
    const request = ++sequence.current;
    opener.current = element;
    try {
      const recipe = parseDownloadDefinition(source), digest = await downloadDigest(recipe);
      const result = await assertDownloadReview(await invoke("review_model_download", { definition: source }), digest);
      if (mounted.current && request === sequence.current && !current.current.locked) { setOfficialOpen(false); setDefinition(source); setReview(result); setReceipt(null); setError(""); setAcknowledged(false); setModal("download"); }
    } catch (cause) { if (mounted.current && request === sequence.current) setError(String(cause)); }
    finally { end(); }
  };
  const read = async (downloadId: string, recipeHash: string) => {
    const token = crypto.randomUUID(); operation.current = token;
    return assertDownloadReadback(await invoke("read_model_download", { token, downloadId, expectedRecipeHash: recipeHash }), downloadId, recipeHash);
  };
  const start = async () => {
    if (!review || pending || !acknowledged || !isTauri()) return;
    const source = definition, checked = structuredClone(review), token = crypto.randomUUID();
    if (!begin("取得・容量とSHAの照合・同期を行っています", token)) return;
    retain({ kind: "download", token, downloadId: token, review: checked, stopped: false }); setReceived(0); setAcknowledged(false); setReceipt(null);
    try {
      if (await downloadDigest(parseDownloadDefinition(source)) !== checked.recipeHash) throw Error("取得定義が変わりました。再確認してください。");
      const result = await assertDownloadReceipt(await invoke("start_model_download", { token, definition: source, expectedRecipeHash: checked.recipeHash }), token, checked);
      if (mounted.current && unresolved?.token === token) { retain(null); setReceipt(result); setReview(checked); setModal(null); setError(""); setMessage("取得素材を照合しました。この操作ではコピー・実行・適用を行っていません。"); }
    } catch (cause) { if (mounted.current) { setError(`取得結果は未確定です: ${String(cause)} 同じ操作の停止・結果確認を行ってください。自動再取得は行いません。`); setModal(null); } }
    finally { end(); if (mounted.current && !unresolved) await loadCatalog().catch(() => {}); }
  };
  const copy = async () => {
    if (!receipt || !review || pending || knownCopy || !acknowledged || !isTauri()) return;
    const checked = structuredClone(receipt), sourceReview = structuredClone(review), token = crypto.randomUUID();
    if (!begin("実体を再照合し管理領域へコピー・同期しています", token)) return;
    retain({ kind: "copy", token, downloadId: checked.operationId, review: sourceReview, reviewHash: checked.report.reviewHash, stopped: false }); setAcknowledged(false);
    try {
      const result = await invoke<ModelPlacementReceipt>("place_downloaded_model", { token, downloadId: checked.operationId, expectedRecipeHash: checked.recipeHash, expectedReviewHash: checked.report.reviewHash });
      assertPlacementReceipt(result, token, checked.report.reviewHash);
      if (result.totalBytes !== checked.report.totalBytes) throw Error("コピー容量が確認内容と一致しません");
      if (mounted.current && unresolved?.token === token) { opener.current = officialTrigger.current; retain(null); setReceipt(null); setModal(null); setError(""); setMessage("コピーの確定記録を確認しました。CPU互換性・現在の本体・適用状態は下の配置記録を確認してください。"); await onChanged(); }
    } catch (cause) { if (mounted.current) { setError(`コピー結果は未確定です: ${String(cause)} 同じ操作の停止・結果確認を行ってください。コピーは再送しません。`); setModal(null); } }
    finally { end(); }
  };
  const stop = async () => {
    const token = operation.current ?? pending?.token;
    if (!token || !isTauri()) return;
    try { await invoke("cancel_model_inspection", { token }); if (mounted.current) setError("停止を要求しました。処理終了・確定結果はまだ確認していません。同じ操作の結果確認を行ってください。"); }
    catch (cause) { if (mounted.current) setError(`停止要求を確認できません: ${String(cause)} 結果は未確定のまま保持します。`); }
  };
  const recover = async () => {
    const value = pending && structuredClone(pending);
    if (!value || !isTauri() || !begin("同じ操作の停止・結果を確認しています")) return;
    try {
      // Tombstone the ORIGINAL writer before observing its lifetime. A false
      // active response alone cannot fence a delayed start arriving afterwards.
      await invoke("cancel_model_inspection", { token: value.token });
      const lifetime = assertDownloadLifetime(await invoke("get_model_download_lifetime", { token: value.token }));
      if (lifetime.active || !lifetime.cancelled) throw Error("元の処理の停止完了をまだ確認できません。時間を置いて同じ操作の結果を確認してください。");
      value.stopped = true; retain(value);
      if (value.kind === "download") {
        const result = await read(value.downloadId, value.review.recipeHash);
        if (mounted.current && unresolved?.token === value.token) { retain(null); setReview(result.review); setReceipt(result.receipt); setError(""); setMessage(result.state === "complete" ? "同じ取得の実体照合を確認しました。まだ未配置・未実行です。" : "取得は未完了です。途中ファイルを保全しました。再開・再取得・コピーは行っていません。"); }
      } else {
        const result = assertDownloadCopyResult(await invoke("get_downloaded_model_placement_result", { packageId: value.token, downloadId: value.downloadId, expectedRecipeHash: value.review.recipeHash, expectedReviewHash: value.reviewHash }), value.token, value.reviewHash!, value.review.totalBytes);
        if (result.active || !result.cancelled) throw Error("コピー処理の停止完了をまだ確認できません");
        if (mounted.current && unresolved?.token === value.token) { retain(null); setReceipt(null); setError(""); setMessage(result.receipt?.placementRecorded ? "同じコピーの確定記録を確認しました。現在の本体・CPU互換性・適用状態は配置記録を確認してください。" : "コピーの確定記録はありません。途中ファイル・意図を保全し、コピーを再送していません。"); await onChanged(); }
      }
    } catch (cause) { if (mounted.current) setError(`同じ操作の結果を確定できません: ${String(cause)} 元の操作IDを保持します。`); }
    finally { end(); if (mounted.current && !unresolved) await loadCatalog().catch(() => {}); }
  };
  const retire = async () => {
    // Explicit acknowledgement after cancellation + actual worker termination.
    // Neither deleting preserved files nor asserting successful completion.
    const value = pending;
    if (!value?.stopped || !isTauri() || !begin("停止状態と取得履歴を再確認しています")) return;
    try {
      await invoke("cancel_model_inspection", { token: value.token });
      const life = assertDownloadLifetime(await invoke("get_model_download_lifetime", { token: value.token }));
      if (life.active || !life.cancelled) throw Error("停止完了を確認できません");
      end(); await loadCatalog();
      if (mounted.current && unresolved?.token === value.token) { retain(null); setReceipt(null); setReview(null); setMessage("結果未確定として操作の確認を終了しました。記録・途中ファイルは保全します。新しい取得は別途明示確認が必要です。"); }
    } catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { end(); }
  };
  const inspectEntry = async (entry: DownloadCatalog["entries"][number]) => {
    if (pending || !entry.recipeHash || !isTauri() || !begin("選択した取得の実体を再照合しています")) return;
    setRecordsFeedback(""); setRecordsError("");
    try {
      const result = await read(entry.operationId, entry.recipeHash);
      if (mounted.current) {
        const feedback = result.state === "complete" ? "取得素材を照合しました。この操作ではコピー・実行・適用を行っていません。" : "取得は未完了です。途中ファイルを保全し、自動再開は行いません。";
        setReview(result.review); setReceipt(result.receipt); setError(""); setMessage(feedback); setRecordsFeedback(feedback);
        // Only completed readback advances presentation. Never start a copy,
        // CPU trial or apply; do not steal focus after the user closed records.
        if (result.state === "complete" && recordsVisible.current) { recordsCopyFocus.current = true; setRecordsOpen(false); }
      }
    } catch (cause) { if (mounted.current) { setReceipt(null); setError(String(cause)); setRecordsError(String(cause)); } }
    finally { end(); }
  };
  const restoreRecordsFocus = () => {
    const target = recordsCopyFocus.current ? receiptButton.current : recordsTrigger.current;
    // The shared modal deliberately restores focus without scrolling. Only an
    // explicit successful readback advances to a new, potentially offscreen
    // action. Reveal it after restoration, without moving ordinary cancel focus
    // or stealing focus if another operation/navigation has already intervened.
    if (recordsCopyFocus.current && target) queueMicrotask(() => {
      if (recordsCopyFocus.current && !recordsVisible.current && document.activeElement === target)
        target.scrollIntoView({ block: "nearest", inline: "nearest" });
    });
    return target;
  };
  const supported = catalog?.supported !== false, disabled = !!busy || locked || !!pending || !isTauri();
  return <section className="model-download" aria-label="モデルの初回取得">
    <SettingsDisclosure label="モデルをダウンロード" className="model-download-settings" open={officialOpen} onOpenChange={setOfficialOpen} triggerRef={officialTrigger}>
      <p>必要な実行ファイル・モデルを準備します。文字起こしの実行ファイルはアプリに同梱されています。品質・速度は正式評価前です。</p>
      <div className="model-preparation-choices">
        {official?.map(option => <article key={option.id}>
          <div><h4>{option.label}</h4><p>取得 {(downloadTransferBytes(option.review) / 1024 ** 3).toFixed(2)} GiB · 空き容量の目安 {((option.review.totalBytes * 2 + (option.review.archiveBytes ?? 0) + 16 * 1024 ** 2) / 1024 ** 3).toFixed(2)} GiB</p></div>
          <button type="button" disabled={disabled} onClick={event => void preview(officialTrigger.current ?? event.currentTarget, option.definition)}>準備内容を確認</button>
        </article>)}
        {official !== null && !official.some(option => option.review.manifest.engine === "whisper.cpp") && <article><h4>文字起こし</h4><p>自動導入は準備中です。手元のモデルは「ローカルから取り込む」で導入できます。</p></article>}
        {official !== null && !official.some(option => option.review.manifest.engine === "sherpa-onnx") && <article><h4>話者分離</h4><p>自動導入は準備中です。手元のモデルは「ローカルから取り込む」で導入できます。</p></article>}
      </div>
      {!isTauri() ? <p>モデルの取得はデスクトップアプリで利用できます。</p> : official === null && !officialError && <p role="status">モデル選択情報を読み込んでいます。</p>}
      <IssueStatus label="モデル選択情報" message={officialError} />
      <p>取得だけでは実行・適用しません。確認後に通信を開始し、その後に管理コピー・CPU動作確認・適用を行います。</p>
    </SettingsDisclosure>
    <SettingsDisclosure label="取得定義からダウンロード" className="model-download-settings" initialOpen={!compact} attention={!!(pending || modal || error || busy && busy !== "取得記録を読み取っています")}>
    <h3>モデルの初回取得（macOS）</h3>
    <p>詳細操作です。独自に用意した定義のHTTPS配布元から取得します。容量・SHA-256・利用条件を確認してください。取得とコピーでは実行ファイルを起動せず、設定・会議・ジョブを変更しません。</p>
    {!supported ? <p>Windowsのモデル取得は保留中です。</p> : <>
      <label>取得定義のJSONファイル（64KiB以内）<input type="file" accept=".json,application/json" disabled={disabled} onChange={event => {
        const file = event.target.files?.[0], request = ++sequence.current; if (!file) return;
        if (file.size > 65_536) { setError("取得定義は64KiB以内で指定してください"); return; }
        void file.text().then(text => { if (mounted.current && request === sequence.current && !current.current.locked && !unresolved) { setDefinition(text); setReview(null); setReceipt(null); } }).catch(cause => { if (mounted.current && request === sequence.current) setError(String(cause)); });
      }} /></label>
      <label className="prompt-label" htmlFor="model-download-definition">取得定義（JSON）<textarea id="model-download-definition" rows={7} value={definition} disabled={disabled} onChange={event => { sequence.current++; setDefinition(event.target.value); setReview(null); setReceipt(null); setAcknowledged(false); }} /></label>
      <details><summary>取得定義の構成</summary><p>通常ファイルは形式版1、TAR内の指定ファイルを取り込む場合は形式版2を使います。全件に容量・SHA-256と同順の配布元が必要です。HTTPSのみで、認証情報・query・fragmentは禁止です。圧縮ファイル全体と指定ファイルの両方を照合します。取得素材・管理コピー・圧縮ファイルを保持する空き容量が必要です。</p></details>
      <div className="analysis-actions"><button type="button" disabled={disabled || !definition.trim()} onClick={event => void preview(event.currentTarget)}>取得定義を確認</button>
        {!!busy && <button type="button" className="secondary" onClick={() => void stop()}>停止を要求する</button>}
      </div>
    </>}
    </SettingsDisclosure>
    {!!busy && <p role="status">{busy}。停止要求だけでは停止完了になりません。</p>}
    {!modal && pending?.kind === "download" && <p aria-label="モデル取得進捗">受信 {received.toLocaleString()} / {downloadTransferBytes(pending.review).toLocaleString()} bytes（全量受信でも照合・同期・完了は未確認）</p>}
    <IssueNotice scope="model-download" type="error" label="モデル取得・コピー" message={!modal ? error : ""} />
    {pending && <div aria-label="未確定のモデル取得操作"><p>操作ID: <code>{pending.token}</code> · 自動再送は行いません。</p><button type="button" disabled={!!busy || locked} onClick={() => void recover()}>同じ操作の停止・結果確認</button>{pending.stopped && <button type="button" className="secondary" disabled={!!busy || locked} onClick={() => void retire()}>結果未確定のまま確認を終了（ファイル保全）</button>}</div>}
    <ToastNotice scope="model-download" type="result" message={!modal ? message : ""} level="info" />
    {message && <p role="status" aria-label="モデル取得結果">{message}</p>}
    {receipt && <div aria-label="照合済み取得素材"><p>取得素材の照合済み · {receipt.report.manifest.modelName} / {receipt.report.manifest.modelVersion}</p><p>取得ID: <code>{receipt.operationId}</code> · {receipt.report.totalBytes.toLocaleString()} bytes</p>{knownCopy && <p>この取得のコピー記録があります。現在の本体・動作確認・適用状態は配置記録で確認してください。</p>}<button ref={receiptButton} type="button" disabled={disabled} onClick={event => { if (knownCopy) { onShowPlacements?.(); return; } opener.current = event.currentTarget; setAcknowledged(false); setModal("copy"); }}>{knownCopy ? "配置・構成記録を開く" : "照合済み素材のコピーを確認"}</button></div>}
    <SettingsDisclosure label="取得記録" className="model-download-settings" open={recordsOpen} onOpenChange={open => { recordsCopyFocus.current = false; setRecordsOpen(open); }} triggerRef={recordsTrigger} restoreTarget={restoreRecordsFocus} attention={!!catalogError || !!catalog?.unknownEntries || !!catalog?.entries.some(entry => entry.state !== "completion_recorded")}>
    <h4>保持している取得記録</h4>
    <IconButton label="取得記録を再読込" icon="refresh" type="button" className="small-secondary" disabled={!!busy || locked || !!pending || !isTauri()} onClick={() => void loadCatalog().catch(() => {})} />
    <IssueNotice scope="model-download" type="catalog-error" label="取得記録の読込" message={catalogError} />
    {!!busy && <p role="status">{busy}</p>}
    <IssueStatus label="取得素材の照合" message={recordsError} />
    {recordsFeedback && <p role="status" aria-label="取得記録の照合結果">{recordsFeedback}</p>}
    {catalog && <><p>一覧の「完了記録あり」は実体照合済みではありません。選択後に全ファイルを再照合します。自動再開・再取得・削除は行いません。</p>{catalog.unknownEntries > 0 && <p>由来不明の保持物: {catalog.unknownEntries} 件（変更しません）</p>}<ul className="model-download-catalog">{catalog.entries.map(entry => <li key={entry.operationId}><strong>{entry.modelName ?? "要確認の取得"}</strong> / {entry.modelVersion ?? "不明"}<p>{entry.state === "completion_recorded" ? "完了記録あり（実体未照合）" : entry.state === "incomplete" ? "未完了・途中ファイル保全" : "要対応・記録または管理領域を確認できません"}<br /><code>{entry.operationId}</code></p><button type="button" className="small-secondary" disabled={disabled || !entry.recipeHash} onClick={() => void inspectEntry(entry)}>この取得の実体を照合</button></li>)}</ul></>}
    </SettingsDisclosure>
    <CommonModal open={!!modal} blocked={!!busy} onDismiss={() => { setModal(null); setAcknowledged(false); }} restoreTarget={() => opener.current} className="model-application-dialog" aria-labelledby="model-download-title">
      <h3 id="model-download-title">{modal === "copy" ? "管理領域へのコピーを確認" : "モデルの取得内容を確認"}</h3>
      {review && <><p>{review.manifest.modelName} · {review.manifest.engine} / {review.manifest.engineVersion}</p>
        <p>取得 {(downloadTransferBytes(review) / 1024 ** 3).toFixed(2)} GiB · モデル容量 {(review.totalBytes / 1024 ** 3).toFixed(2)} GiB</p>
        <p>取得・コピーでは実行や適用を行いません。配布元と利用条件を確認してください。SHA一致だけでは安全性・互換性を証明しません。</p>
        <details className="model-download-definition-details"><summary>配布元・容量・構成の詳細</summary>
          <p>版 {review.manifest.modelVersion} · {review.manifest.target} · 合計 {review.totalBytes.toLocaleString()} bytes</p><p>ライセンス参照: {review.manifest.licenseRef}</p><p>定義SHA: <code>{review.recipeHash}</code></p>
          {review.recipeSchemaVersion && <p>通信容量 {downloadTransferBytes(review).toLocaleString()} bytes · アーカイブ保持 {review.archiveBytes!.toLocaleString()} bytes · 必要な空き容量はモデル容量の約2倍＋アーカイブ分です。</p>}
          {review.bundledBytes !== undefined && <p>アプリ同梱分 {review.bundledBytes.toLocaleString()} bytes（通信容量には含みません）。実行ファイルは固定公式ソースからアプリのビルド時に構築したCPU版です。同梱物の署名はQA用で、正式配布の署名・公証は未検証です。</p>}
          <p>取得素材・管理コピー・圧縮ファイルと中断した途中物を保持します。配布元の真正性・品質は未検証です。CPU実行と解析への適用には別の確認が必要です。</p>
          <ul>{review.manifest.files.map((file, index) => <li key={file.relativePath}><code>{file.relativePath}</code> · {file.role} · {file.expectedSize.toLocaleString()} bytes<br /><code>{review.sources[index].url}</code><br /><code>{file.expectedHash}</code>{review.sources[index].archive && <p>アーカイブ内: <code>{review.sources[index].archive!.memberPath}</code> · アーカイブ容量 {review.sources[index].archive!.expectedSize.toLocaleString()} bytes · SHA <code>{review.sources[index].archive!.expectedHash}</code></p>}</li>)}</ul>
        </details>
        <label><input type="checkbox" checked={acknowledged} disabled={!!busy || locked} onChange={event => setAcknowledged(event.target.checked)} />配布元・容量・SHA・ライセンスと、未実行・未適用であることを確認しました</label>
        <button type="button" disabled={!acknowledged || !!busy || locked || !!pending} onClick={() => void (modal === "copy" ? copy() : start())}>{modal === "copy" ? "確認した素材を管理領域へコピー" : "確認した定義で取得する"}</button>
      </>}
      {!!busy && <><p role="status">{busy}。終了まで確認を閉じられません。</p><button type="button" className="secondary" onClick={() => void stop()}>停止を要求する</button></>}
      {modal && pending?.kind === "download" && <p aria-label="モデル取得進捗">受信 {received.toLocaleString()} / {downloadTransferBytes(pending.review).toLocaleString()} bytes（全量受信でも照合・同期・完了は未確認）</p>}
      <IssueStatus label="モデル取得・コピー" message={modal ? error : ""} />
    </CommonModal>
  </section>;
}
