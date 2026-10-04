import { invoke } from "@tauri-apps/api/core";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { changeList, changeTable, convertBlock, editInline, formatInline, inlineAt, insertBlock, removeBlock, type InlineAddress, type InlineSelection } from "./minutes-edit";
import { MinutesDocument } from "./minutes-document";
import { MinutesRichField } from "./minutes-rich-field";
import type { Inline, MinutesContent, MinutesDraft, MinutesView } from "./minutes-state";
import { timeLabel } from "./transcript";

export type EditorGuard = (nextId: string | null) => boolean;
export function MinutesEditor({ view, initialDraft, locked, registerGuard }: {
  view: MinutesView; initialDraft: MinutesDraft | null; locked: boolean; registerGuard: (guard: EditorGuard | null) => void;
}) {
  const [queue] = useState(() => new DraftSaveQueue<MinutesContent, MinutesDraft>(
    (draftId, expectedLockVersion, payload) => invoke("save_minutes_draft", { draftId, expectedLockVersion, payload }),
    (value) => structuredClone(value),
  ));
  const [saved, setSaved] = useState(() => queue.snapshot());
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [selection, setSelection] = useState<InlineSelection | null>(null);
  const [page, setPage] = useState(0);
  const [epoch, setEpoch] = useState(0);
  const busyRef = useRef(false);
  const failure = useRef("");
  const compositions = useRef(new Set<string>());
  const mounted = useRef(true);
  const editorRoot = useRef<HTMLElement>(null);
  const focusBlock = useRef<number | null>(null);
  useLayoutEffect(() => {
    if (!editing || focusBlock.current === null) return;
    const field = editorRoot.current?.querySelector<HTMLElement>(`.minutes-rich-field[data-block="${focusBlock.current}"]`) ?? editorRoot.current?.querySelector<HTMLElement>(".minutes-rich-field");
    focusBlock.current = null;
    field?.focus();
  }, [epoch, editing, page]);
  useEffect(() => {
    mounted.current = true;
    const stop = queue.subscribe(() => setSaved(queue.snapshot()));
    const unregister = registerPendingEdits(async () => {
      if (compositions.current.size) throw new Error("議事録の日本語入力を確定してから操作してください");
      if (failure.current) throw new Error(failure.current);
      await queue.flush();
    });
    registerGuard((next) => next === view.id || (!queue.snapshot().dirty && !queue.snapshot().saving && !compositions.current.size && !failure.current));
    return () => { mounted.current = false; stop(); unregister(); registerGuard(null); };
  }, [queue, view.id]);
  useEffect(() => {
    if (!initialDraft) return;
    const local = queue.snapshot();
    if (initialDraft.baseMinutesId !== view.id) { setError("議事録下書きの基底版が一致しません"); return; }
    if (local.draft && local.draft.lockVersion >= initialDraft.lockVersion) return;
    if (local.saving) return;
    if (local.dirty || compositions.current.size || failure.current) { setError("別の保存内容を検出しました。編集中の文書を保持しています。保存または再読込で確認してください。"); return; }
    queue.load(initialDraft);
  }, [initialDraft, queue, view.id]);
  useEffect(() => {
    if (!saved.dirty || saved.saving || saved.error || compositions.current.size || failure.current) return;
    const timer = window.setTimeout(() => { void queue.flush().catch(() => {}); }, 800);
    return () => window.clearTimeout(timer);
  }, [saved, queue]);
  const run = async (action: () => Promise<void>) => {
    if (busyRef.current || locked) return;
    busyRef.current = true; setBusy(true); setError("");
    try { await action(); } catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { busyRef.current = false; if (mounted.current) setBusy(false); }
  };
  const begin = () => run(async () => {
    const draft = await invoke<MinutesDraft>("create_minutes_draft", { meetingId: view.meetingId, minutesId: view.id });
    if (!mounted.current) return;
    if (draft.baseMinutesId !== view.id) throw new Error("議事録編集の基底版が変更されています");
    queue.load(draft); focusBlock.current = 0; setEditing(true);
  });
  const undo = () => run(async () => {
    if (failure.current || compositions.current.size) throw new Error("入力を確定するか再読込してから取り消してください");
    await queue.flush(); const draft = queue.snapshot().draft; if (!draft) return;
    const result = await invoke<MinutesDraft>("undo_minutes_edit", { draftId: draft.id, expectedLockVersion: draft.lockVersion });
    if (mounted.current) { queue.load(result); setSelection(null); setEpoch((v) => v + 1); }
  });
  const reload = () => run(async () => {
    if ((queue.snapshot().dirty || failure.current) && !window.confirm("未保存の議事録編集を破棄して保存済み内容を再読込しますか？")) return;
    if (compositions.current.size) throw new Error("日本語入力を確定してから再読込してください");
    const draft = await invoke<MinutesDraft | null>("get_minutes_draft", { meetingId: view.meetingId, minutesId: view.id });
    if (mounted.current) { queue.load(draft); failure.current = ""; setSelection(null); setEpoch((v) => v + 1); }
  });
  const failed = (cause: unknown) => { failure.current = String(cause); setError(String(cause)); };
  const edit = (action: (doc: MinutesContent) => MinutesContent, fromDOM = false) => {
    if (!editing || locked || busy) return false;
    try { queue.edit(action); if (fromDOM) failure.current = ""; setError(""); return true; } catch (cause) { if (fromDOM) failed(cause); else setError(String(cause)); return false; }
  };
  const doc = saved.payload ?? view.content;
  const disabled = locked || busy || !editing || !saved.draft;
  const chipLabel = (id: string) => { const e = doc.evidence.find((e) => e.id === id); return e ? `${timeLabel(e.startUs)}${e.verificationState === "user_verified" ? " ✓" : " · 未確認"}` : "根拠"; };
  const field = (nodes: Inline[], address: InlineAddress, label: string, evidenceOnly = false) => <MinutesRichField key={`${epoch}:${JSON.stringify(address)}`} value={nodes} label={label} blockIndex={address.block} disabled={disabled} evidenceOnly={evidenceOnly} chipLabel={chipLabel}
    onChange={(nodes) => edit((d) => editInline(d, address, nodes), true)} onSelection={(range) => setSelection({ address, ...range })} onFailure={failed}
    onCompositionChange={(active) => { const key = JSON.stringify(address); if (active) compositions.current.add(key); else { compositions.current.delete(key); setSaved(queue.snapshot()); } }} />;
  const selectedBlock = selection ? doc.blocks[selection.address.block] : null;
  const structural = (action: (doc: MinutesContent) => MinutesContent, nextBlock = selection?.address.block ?? 0) => {
    if (!edit(action)) return;
    focusBlock.current = Math.max(0, Math.min(nextBlock, (queue.snapshot().payload?.blocks.length ?? 1) - 1));
    setPage(Math.floor(focusBlock.current / 50)); setSelection(null); setEpoch((v) => v + 1);
  };
  const toolbarButton = (label: string, action: () => void, unavailable = false) => <button type="button" className="small-secondary" disabled={disabled || unavailable} onMouseDown={(e) => e.preventDefault()} onClick={action}>{label}</button>;
  const manualLabel = (id: string | null | undefined) => id && doc.items.find((i) => i.id === id)?.sourceType === "manual" ? <small className="manual-label">手動追加・修正</small> : null;
  const total = Math.max(1, Math.ceil(doc.blocks.length / 50)); const currentPage = Math.min(page, total - 1);
  return <section ref={editorRoot} className="minutes-editor" aria-label="議事録文書と編集">
    <div className="transcript-actions">
      {!editing && <button type="button" className="small-secondary" disabled={locked || busy} onClick={() => void begin()}>議事録を編集</button>}
      {editing && <button type="button" className="small-secondary" disabled={locked || busy} onClick={() => void run(async () => { if (failure.current || compositions.current.size) throw new Error("入力を確定してから編集を終了してください"); await queue.flush(); setEditing(false); setSelection(null); })}>編集を終了</button>}
      {saved.draft && <span role="status">{saved.error ? "議事録の保存要確認" : saved.saving ? "議事録を保存中" : saved.dirty ? "議事録は未保存" : "議事録は保存済み"}</span>}
      {saved.error && <button type="button" className="small-secondary" disabled={locked || busy || saved.saving} onClick={() => void run(async () => { if (failure.current || compositions.current.size) throw new Error("入力を確定してから保存してください"); await queue.retry(); })}>議事録保存を再試行</button>}
      {saved.draft && <button type="button" className="small-secondary" disabled={locked || busy || saved.saving || compositions.current.size > 0 || (!saved.dirty && saved.draft.lockVersion === 0)} onClick={() => void undo()}>直前の議事録編集を取り消す</button>}
      <button type="button" className="small-secondary" disabled={locked || busy || saved.saving} onClick={() => void reload()}>議事録編集を再読込</button>
    </div>
    {editing && <>
      <p>文字を選択して太字・斜体を適用できます。見出し・箇条書き・表は選択中の要素へ適用します。本文・担当者・期限の変更は手動修正として保存します。根拠の引用は変えません。</p>
      <div className="minutes-toolbar" role="toolbar" aria-label="議事録の書式と構造">
        {(["bold", "italic"] as const).map((mark) => <span key={mark}>{toolbarButton(mark === "bold" ? "太字" : "斜体", () => { if (selection) edit((d) => editInline(d, selection.address, formatInline(inlineAt(d, selection.address), selection.start, selection.end, mark))); }, !selection || selection.end <= selection.start)}</span>)}
        {toolbarButton("段落", () => { if (selection) structural((d) => convertBlock(d, selection.address, "paragraph")); }, !selection || selectedBlock?.type === "table")}
        {([1, 2, 3] as const).map((level) => <span key={level}>{toolbarButton(`見出し${level}`, () => { if (selection) structural((d) => convertBlock(d, selection.address, "heading", level)); }, !selection || selectedBlock?.type === "table")}</span>)}
        {toolbarButton("箇条書き", () => { if (selection) structural((d) => convertBlock(d, selection.address, "list")); }, !selection || selectedBlock?.type === "table")}
        {toolbarButton("番号付きリスト", () => { if (selection) structural((d) => convertBlock(d, selection.address, "list", 2, true)); }, !selection || selectedBlock?.type === "table")}
        {toolbarButton("項目を追加", () => { if (selection) structural((d) => changeList(d, selection.address, false)); }, selectedBlock?.type !== "list")}
        {toolbarButton("項目を削除", () => { if (selection) structural((d) => changeList(d, selection.address, true)); }, selectedBlock?.type !== "list")}
        {(["paragraph", "heading", "list", "table"] as const).map((type) => <span key={type}>{toolbarButton(({ paragraph: "段落を挿入", heading: "見出しを挿入", list: "リストを挿入", table: "表を挿入" })[type], () => structural((d) => insertBlock(d, selection?.address.block ?? d.blocks.length - 1, type), (selection?.address.block ?? doc.blocks.length - 1) + 1))}</span>)}
        {(["addRow", "removeRow", "addColumn", "removeColumn"] as const).map((action) => <span key={action}>{toolbarButton(({ addRow: "表の行を追加", removeRow: "表の行を削除", addColumn: "表の列を追加", removeColumn: "表の列を削除" })[action], () => { if (selection) structural((d) => changeTable(d, selection.address, action)); }, selectedBlock?.type !== "table" || (action === "removeRow" && selection?.address.header))}</span>)}
        {toolbarButton("ブロックを削除", () => { if (selection && window.confirm("選択した文書ブロックと、その項目・根拠を下書きから削除しますか？ 保存済みの生成原文は保持し、編集の取り消しで戻せます。")) structural((d) => removeBlock(d, selection.address.block)); }, !selection)}
      </div>
      <div className="minutes-wysiwyg" aria-label="議事録リッチテキスト">
        {doc.blocks.slice(currentPage * 50, (currentPage + 1) * 50).map((b, offset) => {
          const index = currentPage * 50 + offset;
          if (b.type === "heading" || b.type === "paragraph") return <section key={`${epoch}:${index}`} className={b.type === "heading" ? `rich-heading rich-heading-${b.level}` : "rich-paragraph"}>{manualLabel(b.itemId)}{field(b.content, { block: index }, `議事録ブロック${index + 1}`)}</section>;
          if (b.type === "list") { const List = b.ordered ? "ol" : "ul"; return <List key={`${epoch}:${index}`}>{b.items.map((row, item) => <li key={item}>{manualLabel(row.itemId)}{field(row.content, { block: index, item }, `議事録リスト${index + 1}の項目${item + 1}`)}</li>)}</List>; }
          return <div className="document-table" key={`${epoch}:${index}`}><table><thead><tr>{b.headers.map((cell, column) => <th key={column}>{field(cell, { block: index, header: true, column }, `議事録表${index + 1}の見出し${column + 1}`)}</th>)}</tr></thead><tbody>{b.rows.map((row, r) => <tr key={r}>{row.cells.map((cell, column) => <td key={column}>{column === 0 && manualLabel(row.itemId)}{field(cell, { block: index, row: r, column }, `議事録表${index + 1}の行${r + 1}列${column + 1}`, !!row.itemId && column === 3)}</td>)}</tr>)}</tbody></table></div>;
        })}
      </div>
      {total > 1 && <div className="transcript-pages"><button type="button" disabled={currentPage === 0 || locked || compositions.current.size > 0} onClick={() => { setPage(currentPage - 1); setSelection(null); }}>前の編集ページ</button><span>{currentPage + 1} / {total}</span><button type="button" disabled={currentPage + 1 >= total || locked || compositions.current.size > 0} onClick={() => { setPage(currentPage + 1); setSelection(null); }}>次の編集ページ</button></div>}
    </>}
    {error && <p className="action-error" role="alert">{error}</p>}
    {saved.error && <p className="action-error" role="alert">議事録を保存できませんでした。入力はこの画面に保持しています。修正後に再試行してください: {saved.error}</p>}
    <MinutesDocument key={view.id} view={view} locked={locked || busy} content={doc} title={editing ? "編集中の議事録プレビュー・根拠" : "現在の議事録"} />
  </section>;
}
