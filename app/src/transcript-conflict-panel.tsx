import { useState } from "react";
import { CommonModal } from "./common-modal";
import type { AdvancedDraftReview } from "./transcript-recovery-editor";
import type { ConflictChoice } from "./transcript-conflict";
import type { ConflictInputPlan, InputChoice } from "./transcript-conflict-input";

export interface ConflictPreview { proof: AdvancedDraftReview; plan: ConflictInputPlan; stamp: string }
const labels = { name: "話者名",text: "本文",assignment: "話者割り当て",presence: "発言の保持",manual: "追加話者",merge: "話者統合",split: "発言分割" };
const value = (v: unknown) => v === undefined ? "なし" : v === null ? "明示的な不明" : typeof v === "string" ? v : JSON.stringify(v,null,2);
export function TranscriptConflictPanel({ preview,blocked,onClose,onRefresh,onAccept }: {
  preview: ConflictPreview; blocked: boolean; onClose: () => void;
  onRefresh: () => Promise<void>;
  onAccept: (choices: Record<string,ConflictChoice>,inputs: Record<string,InputChoice>) => Promise<void>;
}) {
  const [choices,setChoices] = useState<Record<string,ConflictChoice>>({});
  const [inputs,setInputs] = useState<Record<string,InputChoice>>({});
  const [page,setPage] = useState(0),[busy,setBusy] = useState(false),[error,setError] = useState("");
  const rows = [...preview.plan.body.rows.map(row => ({ kind: "body" as const,row })),...preview.plan.inputs.map(row => ({ kind: "input" as const,row }))];
  const missing = preview.plan.body.rows.filter(row => !choices[row.key] && !row.suggested).length + preview.plan.inputs.filter(row => !inputs[row.key]).length;
  const run = async (action: () => Promise<void>) => {
    if(busy || blocked) return;
    setBusy(true); setError("");
    try { await action(); } catch(cause) { setError(String(cause)); } finally { setBusy(false); }
  };
  return <CommonModal open blocked={busy} className="transcript-conflict-dialog" onDismiss={onClose} aria-label="文字起こしの保存競合を比較" dismissLabel="閉じる">
    <h3>文字起こしの保存競合を比較</h3>
    <p>基底版 {preview.proof.base.lockVersion} → 最新保存版 {preview.proof.latest.lockVersion}。同じ下書きと解析版だけを比較しています。</p>
    <p>閉じると元の入力・保存要求を保持します。編集再開だけでは本文も回復記録も保存せず、自動保存も停止したままです。再開後に確定や保存再試行を行うと、選んだ変更を保存し、回復入力も選択した内容へ置き換えます。</p>
    <button type="button" disabled={busy || blocked} onClick={() => void run(onRefresh)}>最新の内容を比較し直す</button>
    {preview.proof.pendingPayload && <details><summary>結果未確認の本文保存要求</summary><textarea aria-label="元の本文保存要求" readOnly value={JSON.stringify(preview.proof.pendingPayload,null,2)} rows={4}/><p className="hint">古い要求は再送しません。手元の新しい保存待ち変更は下の比較対象です。</p></details>}
    {preview.proof.recovery.record && <p className="hint">保存済み回復版 {preview.proof.recovery.record.lockVersion}。{preview.proof.recovery.record.matchesDraft ? "最新本文と一致" : "本文更新前の入力を含みます"}。開始時の値をそのまま自動適用しません。</p>}
    <p role="status">比較項目 {rows.length}件 · 未選択 {missing}件 · {page + 1} / {Math.max(1,Math.ceil(rows.length / 10))}ページ</p>
    {!rows.length && <p>差分と未確定入力はありません。最新保存版から編集を再開できます。</p>}
    {rows.slice(page * 10,(page + 1) * 10).map(item => item.kind === "body" ? <fieldset key={`body:${item.row.key}`} className="conflict-item" aria-label={`${labels[item.row.kind]} ${item.row.id}`}>
      <legend>{labels[item.row.kind]} · {item.row.id}{item.row.conflict ? " · 両側の変更あり" : " · 片側の変更"}</legend>
      <div className="conflict-values">{(["base","local","latest"] as const).map((side,index) => <label key={side}>{["比較基底","手元の保存待ち","最新保存版"][index]}<textarea readOnly rows={3} value={value(item.row[side])}/></label>)}</div>
      <label>残す変更<select disabled={busy || blocked} value={choices[item.row.key] ?? item.row.suggested ?? ""} onChange={event => setChoices(current => ({ ...current,[item.row.key]: event.target.value as ConflictChoice }))}>
        <option value="" disabled>選択してください</option><option value="local">手元の保存待ち</option><option value="latest">最新保存版</option>
      </select></label>
    </fieldset> : <fieldset key={`input:${item.row.key}`} className="conflict-item" aria-label={`未確定${labels[item.row.kind]} ${item.row.id}`}>
      <legend>未確定{labels[item.row.kind]} · {item.row.id}</legend>
      <div className="conflict-values">{(["local","stored"] as const).map((side,index) => <label key={side}>{["手元の入力","保存済み回復入力"][index]}<textarea readOnly rows={3} value={item.row[side]?.value ?? "（入力なし）"}/>{item.row[side] && <details><summary>入力開始時の条件</summary><pre>{JSON.stringify(item.row[side],null,2)}</pre></details>}</label>)}</div>
      <label>再開する入力<select disabled={busy || blocked} value={inputs[item.row.key] ?? ""} onChange={event => setInputs(current => ({ ...current,[item.row.key]: event.target.value as InputChoice }))}>
        <option value="" disabled>選択してください</option><option value="local" disabled={!item.row.local}>手元の入力</option><option value="stored" disabled={!item.row.stored}>保存済み回復入力</option><option value="omit">この入力は再開しない</option>
      </select></label>
    </fieldset>)}
    {rows.length > 10 && <div className="transcript-actions"><button type="button" disabled={!page || busy || blocked} onClick={() => setPage(n => n - 1)}>前の比較項目</button><button type="button" disabled={(page + 1) * 10 >= rows.length || busy || blocked} onClick={() => setPage(n => n + 1)}>次の比較項目</button></div>}
    {error && <p role="alert" className="action-error">{error}</p>}
    <div className="transcript-actions"><button type="button" disabled={busy || blocked || missing > 0} onClick={() => void run(() => onAccept(choices,inputs))}>選択した内容で編集を再開</button><button type="button" className="small-secondary" data-dialog-cancel disabled={busy} onClick={onClose}>閉じる</button></div>
  </CommonModal>;
}
