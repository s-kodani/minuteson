import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "./ipc";
import { CommonModal } from "./common-modal";
import { ErrorNotice } from "./toast";
import { useStartupRecoveryPending } from "./startup-attention";

function checked(value: unknown): { pending: boolean } {
  if (!value || typeof value !== "object" || typeof (value as { pending?: unknown }).pending !== "boolean") throw Error("初回モデル案内の状態を確認できません。");
  return value as { pending: boolean };
}
export function ModelDownloadGuidance({ locked, onOpenModels }: { locked: boolean; onOpenModels: () => void | Promise<void> }) {
  const [pending, setPending] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const active = useRef(true), saving = useRef(false);
  const recoveryPending = useStartupRecoveryPending();
  useEffect(() => {
    active.current = true;
    if (isTauri()) void invoke("get_model_download_guidance").then(checked).then(value => { if (active.current) setPending(value.pending); }).catch(cause => { if (active.current) setError(String(cause)); });
    return () => { active.current = false; };
  }, []);
  const dismiss = async (openModels = false) => {
    if (saving.current || locked) return;
    saving.current = true; setBusy(true); setError("");
    try {
      const value = checked(await invoke("dismiss_model_download_guidance"));
      if (value.pending) throw Error("初回モデル案内の終了を保存できませんでした。もう一度お試しください。");
      if (active.current) { setPending(false); if (openModels) await onOpenModels(); }
    } catch (cause) { if (active.current) setError(String(cause)); }
    finally { saving.current = false; if (active.current) setBusy(false); }
  };
  return <>
    {!pending && <ErrorNotice scope="model-download-guidance" type="read" active={!!error}>{error}</ErrorNotice>}
    <CommonModal open={pending && !recoveryPending && !locked} blocked={busy} onDismiss={() => dismiss()} dismissLabel="閉じる" restoreTarget={() => document.querySelector<HTMLElement>('.sidebar button[aria-label="設定"]')} aria-labelledby="model-download-guidance-title">
      <h2 id="model-download-guidance-title">ローカルAIモデルをダウンロード</h2>
      <p>文字起こし・話者分離・議事録生成を利用するには、AIモデルのダウンロードが必要です。</p>
      <p>初回の取得にはインターネット接続と空き容量が必要です。導入後のAI処理は、このPC内で実行します。</p>
      <p>設定の「ローカルAI」から、モデルを選んでダウンロードできます。</p>
      {error && <p role="alert">{error}</p>}
      <div className="analysis-actions"><button type="button" disabled={busy} onClick={() => void dismiss(true)}>モデルの設定を開く</button><button type="button" className="small-secondary" disabled={busy} onClick={() => void dismiss()}>あとで</button></div>
    </CommonModal>
  </>;
}
