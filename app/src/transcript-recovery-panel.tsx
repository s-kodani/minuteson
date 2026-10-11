import { invoke } from "@tauri-apps/api/core";
import { IconButton } from "./action-icon";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { CommonModal, type ModalReplacement } from "./common-modal";
import { useActionConfirmation } from "./confirmation-service";
import { TranscriptRecoveryDiscard } from "./transcript-recovery-discard";
import type { TranscriptRecovery } from "./transcript-recovery-save";
import type { DraftPayload } from "./transcript-state";

type Record = TranscriptRecovery<DraftPayload>;
const inputCount = (record: Record) => record.payload.buffers.length + (record.payload.nameBuffers?.length ?? 0);
interface Summary {
  draftId: string; baseAnalysisId: string; operationId: string; contentHash: string;
  lockVersion: number; updatedAt: number; bufferCount: number; matchesDraft: boolean; currentContext: boolean;
}
interface Page { items: Summary[]; nextCursor: string | null }
export function TranscriptRecoveryPanel({ meetingId, review, currentDraftId, blocked, canChangeCurrent, onResume, onDiscarded, onPending, renderPanel }:
  { meetingId: string; review: Record | null; currentDraftId?: string; blocked: boolean; canChangeCurrent: boolean;
    onResume: () => Promise<void>; onDiscarded: (record: Record) => void;
    onPending: (pending: boolean) => void;
    renderPanel?: (entry: ReactNode, view: ModalReplacement | null, persistentBody: ReactNode) => ReactNode }) {
  const titleId = useId();
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
      || inputCount(result) !== item.bufferCount) throw Error("一覧の確認後に回復記録が変わりました。一覧を再読込してください。");
    setPreview(result); setPreviewPage(0); setMessage("");
  };
  const current = preview?.draftId === currentDraftId;
  const resumeAllowed = !!preview && !!review && preview.draftId === review.draftId
    && preview.lockVersion === review.lockVersion && preview.contentHash === review.contentHash
    && preview.matchesDraft && preview.currentContext;
  const destructiveBlocked = blocked || busy || !!state.pending || state.busy || (current && !canChangeCurrent);
  const previewInputs = preview ? [
    ...preview.payload.buffers.map(buffer => ({ kind: "text" as const,buffer })),
    ...(preview.payload.nameBuffers ?? []).map(buffer => ({ kind: "name" as const,buffer })),
  ] : [];
  const requestDiscard = () => {
    const expected = preview;
    if(!expected || destructiveBlocked || !inputCount(expected)) return;
    void confirm({ title: "未確定入力の破棄を確認", target: `解析版 ${expected.baseAnalysisId} · 未確定入力 ${inputCount(expected)}件`,
      confirmLabel: "確認した未確定入力を破棄する",
      message: "表示した回復記録の未確定入力だけを破棄します。この入力は復元できません。確定本文・話者名・Undo・旧解析版は変更しません。",
      validate: () => preview === expected && !discard.snapshot().pending && (!current || latest.current.canChangeCurrent),
      action: async () => { const saved = await discard.start(expected); if(live.current) accepted(saved); },
      onError: cause => setError(String(cause)),
    });
  };
  const entry = <>
    {review && <p className="hint" role="status">保存済みの未確定入力 {inputCount(review)}件があります。確認してから編集を再開してください。{(!review.matchesDraft || !review.currentContext) && "現在の本文とは条件が異なるため、自動適用しません。"}
      <button type="button" className="small-secondary" disabled={busy || state.busy || blocked} onClick={event => { event.currentTarget.focus({preventScroll:true}); setPreview(review); setPreviewPage(0); setOpen(true); setError(""); }}>未確定入力を確認</button></p>}
    <button type="button" className="small-secondary" disabled={busy || state.busy || blocked} onClick={event => { event.currentTarget.focus({preventScroll:true}); setOpen(true); void run(() => list(0,[null])); }}>旧下書きを含む回復記録</button>
    {state.pending && <div className="action-error" role="alert">
      回復記録の破棄結果が未確認です。入力を保持し、別の破棄と画面移動を止めています。
      <button type="button" disabled={blocked || busy || state.busy} onClick={() => void run(async () => {
        const result = await discard.recheck();
        if(result.status === "discarded") accepted(result.record);
        else setMessage("回復記録は確認した内容のままです。結果確認では破棄していません。同じ要求の再試行は別操作です。");
      })}>破棄結果を確認</button>
      <button type="button" disabled={blocked || busy || state.busy} onClick={() => void run(async () => accepted(await discard.retry()))}>同じ破棄要求を再試行</button>
      <button type="button" disabled={busy || state.busy} onClick={event => { event.currentTarget.focus({preventScroll:true}); setPreview(state.pending!.preview); setOpen(true); }}>確認した入力を見る</button>
    </div>}
    {!open && message && <p role="status">{message}</p>}
    {!open && (error || state.error) && <p className="action-error" role="alert">{error || state.error}</p>}
  </>;
  const body = <>
      <h3 id={titleId}>未確定入力の回復記録</h3>
      <p>編集確定前の本文と話者名です。閲覧や編集再開だけでは確定せず、旧下書きや旧解析版も採用しません。</p>
      <IconButton label="回復一覧を再読込" icon="refresh" type="button" disabled={busy || state.busy || blocked} onClick={() => void run(() => list(0,[null]))} />
      {inventory && <div className="recovery-inventory" aria-label="回復記録の一覧">
        <p>一覧 {page + 1}ページ · {inventory.items.length}件（最大20件）</p>
        {!inventory.items.length && <p>回復記録はありません。</p>}
        {inventory.items.map(item => <div className="recovery-inventory-item" key={item.draftId}><p>解析版 {item.baseAnalysisId} · 入力 {item.bufferCount}件 · {item.currentContext ? "現在の下書き" : "旧下書き"} · {item.matchesDraft ? "本文と一致" : "本文更新あり"}</p>
          <button type="button" disabled={busy || state.busy || blocked} onClick={() => void run(() => detail(item))}>この記録を見る</button></div>)}
        <div className="recovery-pagination"><IconButton label="前の一覧" icon="previous" type="button" className="small-secondary" disabled={page === 0 || busy || blocked} onClick={() => void run(() => list(page - 1))} />
        <IconButton label="次の一覧" icon="next" type="button" className="small-secondary" disabled={!inventory.nextCursor || busy || blocked} onClick={() => void run(() => list(page + 1,[...cursors.slice(0,page + 1),inventory.nextCursor]))} /></div>
      </div>}
      {preview && <div aria-label="回復入力のプレビュー">
        <p>解析版 {preview.baseAnalysisId} · 回復版 {preview.lockVersion} · 入力 {previewInputs.length}件（本文 {preview.payload.buffers.length}件・話者名 {preview.payload.nameBuffers?.length ?? 0}件）</p>
        {(!preview.currentContext || !preview.matchesDraft) && <p className="hint">旧版または本文更新後の記録です。編集再開はできません。必要な本文や話者名を選択して手動でコピーできます。</p>}
        {previewInputs.slice(previewPage * 10,(previewPage + 1) * 10).map(item => item.kind === "text" ? <div key={`text:${item.buffer.utteranceId}`}>
          <p>発言 {item.buffer.utteranceId}</p><label>入力開始時の本文<textarea readOnly value={item.buffer.text} rows={2}/></label><label>未確定の本文<textarea readOnly value={item.buffer.value} rows={2}/></label>
        </div> : <div key={`name:${item.buffer.speakerId}`}>
          <p>話者 {item.buffer.speakerId} · {item.buffer.hadOverride ? "開始時に手動名あり" : "開始時に手動名なし"}</p><label>入力開始時の話者名<textarea readOnly value={item.buffer.name ?? ""} rows={2}/></label><label>未確定の話者名<textarea readOnly value={item.buffer.value} rows={2}/></label>
        </div>)}
        {previewInputs.length > 10 && <div className="recovery-pagination"><IconButton label="前の入力" icon="previous" type="button" className="small-secondary" disabled={previewPage === 0} onClick={() => setPreviewPage(n => n - 1)} /><span>{previewPage + 1} / {Math.ceil(previewInputs.length / 10)}</span><IconButton label="次の入力" icon="next" type="button" className="small-secondary" disabled={(previewPage + 1) * 10 >= previewInputs.length} onClick={() => setPreviewPage(n => n + 1)} /></div>}
        {!!previewInputs.length && <div className="recovery-actions">
          <button type="button" disabled={!resumeAllowed || destructiveBlocked} onClick={() => void run(async () => { await onResume(); if(live.current) { setOpen(false); setMessage("未確定入力の編集を再開しました。本文と話者名はまだ確定していません。"); } })}>確認した入力の編集を再開</button>
          <button type="button" className="stop-button" disabled={destructiveBlocked} onClick={requestDiscard}>この回復入力を破棄</button>
        </div>}
      </div>}
      {message && <p role="status">{message}</p>}
      {(error || state.error) && <p role="alert" className="action-error">{error || state.error}</p>}
  </>;
  // Keep the controller and its records mounted while switching presentation.
  // The containing edit-options dialog owns native modality and confirmations.
  if (renderPanel) return renderPanel(entry, open ? {
    key: "transcript-recovery", labelledBy: titleId, content: null,
    className: "transcript-recovery-view", blocked: busy || state.busy || confirm.pending,
    onDismiss: () => setOpen(false), dismissLabel: "閉じる",
  } : null, <div className="transcript-recovery-view" hidden={!open}>{body}</div>);
  return <>{entry}<CommonModal open={open} className="transcript-recovery-dialog" blocked={busy || state.busy || confirm.pending} onDismiss={() => setOpen(false)} aria-labelledby={titleId} dismissLabel="閉じる">{body}</CommonModal></>;
}
