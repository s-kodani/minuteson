import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";
import { useActionConfirmation } from "./confirmation-service";
import { TranscriptRecoveryDiscard } from "./transcript-recovery-discard";
import type { TranscriptRecovery } from "./transcript-recovery-save";
import type { DraftPayload } from "./transcript-state";

type Record = TranscriptRecovery<DraftPayload>;
interface Summary {
  draftId: string; baseAnalysisId: string; operationId: string; contentHash: string;
  lockVersion: number; updatedAt: number; bufferCount: number; matchesDraft: boolean; currentContext: boolean;
}
interface Page { items: Summary[]; nextCursor: string | null }
export function TranscriptRecoveryPanel({ meetingId, review, currentDraftId, blocked, canChangeCurrent, onResume, onDiscarded, onPending }:
  { meetingId: string; review: Record | null; currentDraftId?: string; blocked: boolean; canChangeCurrent: boolean;
    onResume: () => Promise<void>; onDiscarded: (record: Record) => void;
    onPending: (pending: boolean) => void }) {
  const confirm = useActionConfirmation(blocked);
  const latest = useRef({ blocked, canChangeCurrent, onPending, onDiscarded });
  latest.current = { blocked, canChangeCurrent, onPending, onDiscarded };
  const [discard] = useState(() => new TranscriptRecoveryDiscard<DraftPayload>(
    request => invoke("discard_transcript_edit_recovery",{ request }),
    request => invoke("get_transcript_edit_recovery",{ meetingId: request.meetingId,draftId: request.draftId,baseAnalysisId: request.baseAnalysisId }),
    () => crypto.randomUUID(),
  ));
  const [state,setState] = useState(() => discard.snapshot());
  const [open,setOpen] = useState(false),[preview,setPreview] = useState<Record | null>(null);
  const [inventory,setInventory] = useState<Page | null>(null),[cursors,setCursors] = useState<(string | null)[]>([null]);
  const [page,setPage] = useState(0),[previewPage,setPreviewPage] = useState(0);
  const [busy,setBusy] = useState(false),[error,setError] = useState(""),[message,setMessage] = useState("");
  const live = useRef(true);
  useEffect(() => {
    const off = discard.subscribe(() => {
      setState(discard.snapshot()); latest.current.onPending(!!discard.snapshot().pending || discard.snapshot().busy);
    });
    return () => { live.current = false; off(); discard.dispose(); };
  },[discard]);
  const accepted = (record: Record) => {
    setPreview(record); setMessage("確認した未確定入力を破棄しました。確定本文と編集履歴は変更していません。");
    latest.current.onDiscarded(record);
  };
  const run = async (action: () => Promise<void>) => {
    if(busy || discard.snapshot().busy) return;
    setBusy(true); setError("");
    try { await action(); } catch(cause) { if(live.current) setError(String(cause)); }
    finally { if(live.current) setBusy(false); }
  };
  const list = async (at = 0,next = cursors) => {
    const result = await invoke<Page>("list_transcript_edit_recoveries",{ meetingId,after: next[at] ?? null });
    if(!live.current) return;
    if(!Array.isArray(result.items) || result.items.length > 20 || result.items.some(item => !Number.isSafeInteger(item.bufferCount) || item.bufferCount < 0)) throw Error("回復一覧の形式を確認できません");
    setInventory(result); setPage(at); setCursors(next);
  };
  const detail = async (item: Summary) => {
    const result = await invoke<Record | null>("get_transcript_edit_recovery",{ meetingId,draftId: item.draftId,baseAnalysisId: item.baseAnalysisId });
    if(!live.current) return;
    if(!result || result.meetingId !== meetingId || result.draftId !== item.draftId || result.baseAnalysisId !== item.baseAnalysisId
      || result.operationId !== item.operationId || result.lockVersion !== item.lockVersion || result.contentHash !== item.contentHash
      || result.payload.buffers.length !== item.bufferCount) throw Error("一覧の確認後に回復記録が変わりました。一覧を再読込してください。");
    setPreview(result); setPreviewPage(0); setMessage("");
  };
  const current = preview?.draftId === currentDraftId;
  const resumeAllowed = !!preview && !!review && preview.draftId === review.draftId
    && preview.lockVersion === review.lockVersion && preview.contentHash === review.contentHash
    && preview.matchesDraft && preview.currentContext;
  const destructiveBlocked = blocked || busy || !!state.pending || state.busy || (current && !canChangeCurrent);
  const requestDiscard = () => {
    const expected = preview;
    if(!expected || destructiveBlocked || !expected.payload.buffers.length) return;
    void confirm({ title: "未確定入力の破棄を確認", target: `解析版 ${expected.baseAnalysisId} · 未確定入力 ${expected.payload.buffers.length}件`,
      confirmLabel: "確認した未確定入力を破棄する",
      message: "表示した回復記録の未確定入力だけを破棄します。この入力は復元できません。確定本文・話者名・Undo・旧解析版は変更しません。",
      validate: () => preview === expected && !discard.snapshot().pending && (!current || latest.current.canChangeCurrent),
      action: async () => { const saved = await discard.start(expected); if(live.current) accepted(saved); },
      onError: cause => setError(String(cause)),
    });
  };
  return <>
    {review && <p className="hint" role="status">保存済みの未確定入力 {review.payload.buffers.length}件があります。確認してから編集を再開してください。{(!review.matchesDraft || !review.currentContext) && "現在の本文とは条件が異なるため、自動適用しません。"}
      <button type="button" className="small-secondary" disabled={busy || state.busy || blocked} onClick={() => { setPreview(review); setPreviewPage(0); setOpen(true); setError(""); }}>未確定入力を確認</button></p>}
    <button type="button" className="small-secondary" disabled={busy || state.busy || blocked} onClick={() => { setOpen(true); void run(() => list(0,[null])); }}>旧下書きを含む回復記録</button>
    {state.pending && <div className="action-error" role="alert">
      回復記録の破棄結果が未確認です。入力を保持し、別の破棄と画面移動を止めています。
      <button type="button" disabled={blocked || busy || state.busy} onClick={() => void run(async () => {
        const result = await discard.recheck();
        if(result.status === "discarded") accepted(result.record);
        else setMessage("回復記録は確認した内容のままです。結果確認では破棄していません。同じ要求の再試行は別操作です。");
      })}>破棄結果を確認</button>
      <button type="button" disabled={blocked || busy || state.busy} onClick={() => void run(async () => accepted(await discard.retry()))}>同じ破棄要求を再試行</button>
      <button type="button" disabled={busy || state.busy} onClick={() => { setPreview(state.pending!.preview); setOpen(true); }}>確認した入力を見る</button>
    </div>}
    {!open && message && <p role="status">{message}</p>}
    {!open && (error || state.error) && <p className="action-error" role="alert">{error || state.error}</p>}
    <CommonModal open={open} className="transcript-recovery-dialog" blocked={busy || state.busy || confirm.pending} onDismiss={() => setOpen(false)} aria-labelledby="transcript-recovery-title" dismissLabel="閉じる">
      <h3 id="transcript-recovery-title">未確定入力の回復記録</h3>
      <p>編集確定前の入力です。閲覧や編集再開だけでは本文を確定せず、旧下書きや旧解析版も採用しません。</p>
      <button type="button" disabled={busy || state.busy || blocked} onClick={() => void run(() => list(0,[null]))}>回復一覧を再読込</button>
      {inventory && <div className="recovery-inventory" aria-label="回復記録の一覧">
        <p>一覧 {page + 1}ページ · {inventory.items.length}件（最大20件）</p>
        {!inventory.items.length && <p>回復記録はありません。</p>}
        {inventory.items.map(item => <p key={item.draftId}>解析版 {item.baseAnalysisId} · 入力 {item.bufferCount}件 · {item.currentContext ? "現在の下書き" : "旧下書き"} · {item.matchesDraft ? "本文と一致" : "本文更新あり"}
          <button type="button" disabled={busy || state.busy || blocked} onClick={() => void run(() => detail(item))}>この記録を見る</button></p>)}
        <button type="button" disabled={page === 0 || busy || blocked} onClick={() => void run(() => list(page - 1))}>前の一覧</button>
        <button type="button" disabled={!inventory.nextCursor || busy || blocked} onClick={() => void run(() => list(page + 1,[...cursors.slice(0,page + 1),inventory.nextCursor]))}>次の一覧</button>
      </div>}
      {preview && <div aria-label="回復入力のプレビュー">
        <p>解析版 {preview.baseAnalysisId} · 回復版 {preview.lockVersion} · 入力 {preview.payload.buffers.length}件</p>
        {(!preview.currentContext || !preview.matchesDraft) && <p className="hint">旧版または本文更新後の記録です。編集再開はできません。必要な本文を選択して手動でコピーできます。</p>}
        {preview.payload.buffers.slice(previewPage * 10,(previewPage + 1) * 10).map(buffer => <div key={buffer.utteranceId}>
          <p>発言 {buffer.utteranceId}</p><label>入力開始時の本文<textarea readOnly value={buffer.text} rows={2}/></label><label>未確定の本文<textarea readOnly value={buffer.value} rows={2}/></label>
        </div>)}
        {preview.payload.buffers.length > 10 && <div><button type="button" disabled={previewPage === 0} onClick={() => setPreviewPage(n => n - 1)}>前の入力</button><span>{previewPage + 1} / {Math.ceil(preview.payload.buffers.length / 10)}</span><button type="button" disabled={(previewPage + 1) * 10 >= preview.payload.buffers.length} onClick={() => setPreviewPage(n => n + 1)}>次の入力</button></div>}
        {!!preview.payload.buffers.length && <div className="recovery-actions">
          <button type="button" disabled={!resumeAllowed || destructiveBlocked} onClick={() => void run(async () => { await onResume(); if(live.current) { setOpen(false); setMessage("未確定入力の編集を再開しました。本文はまだ確定していません。"); } })}>確認した入力の編集を再開</button>
          <button type="button" className="stop-button" disabled={destructiveBlocked} onClick={requestDiscard}>この回復入力を破棄</button>
        </div>}
      </div>}
      {message && <p role="status">{message}</p>}
      {(error || state.error) && <p role="alert" className="action-error">{error || state.error}</p>}
      <button type="button" className="small-secondary" data-dialog-cancel disabled={busy || state.busy || confirm.pending} onClick={() => setOpen(false)}>閉じる</button>
    </CommonModal>
  </>;
}
