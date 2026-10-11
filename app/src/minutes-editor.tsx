import { invoke } from "@tauri-apps/api/core";
import { useActionConfirmation } from "./confirmation-service";
import { IssueNotice } from "./toast";
import { OptionsModal } from "./options-modal";
import { IconButton, type ActionIconName } from "./action-icon";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { DraftSaveQueue, registerPendingEdits } from "./draft-save";
import { changeList, changeTable, convertBlock, editInline, formatInline, inlineAt, insertBlock, removeBlock, setEvidenceVerification, type InlineAddress, type InlineSelection } from "./minutes-edit";
import { MinutesDocument, type MinutesDocumentHandle } from "./minutes-document";
import { MinutesRichField } from "./minutes-rich-field";
import { documentAddress, documentPage, documentPageForAddress, firstDocumentAddress, inlineAddressKey } from "./minutes-pagination";
import { MinutesPages } from "./minutes-pages";
import { WorkspacePanel, type WorkspacePanelHandle } from "./workspace-panel";
import type { Inline, MinutesContent, MinutesDraft, MinutesView } from "./minutes-state";
import { timeLabel } from "./transcript";

export type EditorGuard = (nextId: string | null) => boolean;
const formatIcons: Partial<Record<string, ActionIconName>> = {
  "太字": "bold", "斜体": "italic", "見出し1": "heading1", "見出し2": "heading2", "見出し3": "heading3",
  "箇条書き": "list", "番号付きリスト": "numberedList", "表を挿入": "table",
};
export function MinutesEditor({ view, meetingTitle, initialDraft, locked, registerGuard, visible = true, actionsHost = null, optionsHost = null }: {
  view: MinutesView; meetingTitle?: string; initialDraft: MinutesDraft | null; locked: boolean; registerGuard: (guard: EditorGuard | null) => void; visible?: boolean; actionsHost?: HTMLElement | null; optionsHost?:HTMLElement | null;
}) {
  const confirm = useActionConfirmation(locked);
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
  const [full,setFull] = useState(false);
  const [formatOpen,setFormatOpen] = useState(false);
  const [formatActionsHost,setFormatActionsHost] = useState<HTMLDivElement | null>(null);
  const formatTrigger = useRef<HTMLButtonElement>(null);
  const focusAfterOptions = useRef(false);
  const [referenceMount] = useState(() => { const node = document.createElement("div"); node.className = "workspace-panel-body minutes-reference-body"; return node; });
  const referenceHandle = useRef<WorkspacePanelHandle | null>(null);
  const documentHandle = useRef<MinutesDocumentHandle | null>(null);
  const busyRef = useRef(false);
  const failure = useRef("");
  const compositions = useRef(new Set<string>());
  const mounted = useRef(true);
  const editorRoot = useRef<HTMLElement>(null);
  const editToggle = useRef<HTMLButtonElement>(null);
  const retryToggle = useRef<HTMLButtonElement>(null);
  const focusAddress = useRef<InlineAddress | null>(null);
  const focusAfterConfirmation = useRef(false);
  const doc = saved.payload ?? view.content;
  // A removed chip's immutable reference survives only this editing epoch so
  // native text Undo can restore it. Reload/structural edits discard the pool.
  const inputEvidence = useRef({ epoch, viewId: view.id, links: new Map(doc.evidence.map(e => [e.id, e])) });
  if (inputEvidence.current.epoch !== epoch || inputEvidence.current.viewId !== view.id) {
    inputEvidence.current = { epoch, viewId: view.id, links: new Map() };
  }
  for (const evidence of doc.evidence) inputEvidence.current.links.set(evidence.id, evidence);
  const plan = documentPage(doc.blocks, page);
  useLayoutEffect(() => {
    if (confirm.pending) { focusAfterConfirmation.current = true; return; }
    const afterConfirmation = focusAfterConfirmation.current;
    focusAfterConfirmation.current = false;
    const afterOptions = focusAfterOptions.current;
    focusAfterOptions.current = false;
    if (!editing || busy || locked || focusAddress.current === null) return;
    // The confirmation's layout effects close its native dialog and restore its
    // opener first. Focus the surviving rich field after that restoration; a
    // removed field must not leave focus on an unrelated preview heading.
    const focus = () => {
      if (document.querySelector("dialog[open]") || focusAddress.current === null) return;
      if (afterOptions && document.activeElement !== formatTrigger.current && document.activeElement !== document.body) return;
      const key = inlineAddressKey(focusAddress.current);
      const field = [...(editorRoot.current?.querySelectorAll<HTMLElement>(".minutes-rich-field") ?? [])].find((field) => field.dataset.inlineAddress === key) ?? editorRoot.current?.querySelector<HTMLElement>(".minutes-rich-field");
      if (!field?.isConnected || !field.matches('[contenteditable="true"]')) return;
      focusAddress.current = null;
      field.focus({ preventScroll: true });
    };
    if (!afterConfirmation && !afterOptions) { focus(); return; }
    const frame = requestAnimationFrame(focus);
    return () => cancelAnimationFrame(frame);
  }, [epoch, editing, plan.page, confirm.pending, busy, locked,formatOpen]);
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
    if (confirm.pending || !saved.dirty || saved.saving || saved.error || compositions.current.size || failure.current) return;
    const timer = window.setTimeout(() => { void queue.flush().catch(() => {}); }, 800);
    return () => window.clearTimeout(timer);
  }, [saved, queue, confirm.pending]);
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
    queue.load(draft); focusAddress.current = firstDocumentAddress(draft.payload.blocks, 0); setPage(0); setEditing(true);
  });
  const undo = () => run(async () => {
    if (failure.current || compositions.current.size) throw new Error("入力を確定するか再読込してから取り消してください");
    await queue.flush(); const draft = queue.snapshot().draft; if (!draft) return;
    const result = await invoke<MinutesDraft>("undo_minutes_edit", { draftId: draft.id, expectedLockVersion: draft.lockVersion });
    if (mounted.current) { queue.load(result); setSelection(null); setEpoch((v) => v + 1); }
  });
  const reload = () => run(async () => {
    if (compositions.current.size) throw new Error("日本語入力を確定してから再読込してください");
    const read = async () => {
    const draft = await invoke<MinutesDraft | null>("get_minutes_draft", { meetingId: view.meetingId, minutesId: view.id });
    if (mounted.current) { queue.load(draft); failure.current = ""; setSelection(null); setEpoch((v) => v + 1); }
    };
    if (queue.snapshot().dirty || failure.current) {
      const expected = JSON.stringify(queue.snapshot().payload), expectedFailure = failure.current;
      await confirm({ title: "議事録編集の破棄を確認", target: `会議「${meetingTitle ?? view.meetingId}」の議事録編集`, confirmLabel: "入力を破棄して再読込する", message: "未保存の議事録編集を破棄して保存済み内容を再読込します。この入力は復元できません。生成原文と保存済み編集は保持します。", validate: () => !compositions.current.size && failure.current === expectedFailure && JSON.stringify(queue.snapshot().payload) === expected, onError: cause => { if (mounted.current) setError(String(cause)); }, action: read });
    } else await read();
  });
  const failed = (cause: unknown) => { failure.current = String(cause); setError(String(cause)); };
  const edit = (action: (doc: MinutesContent) => MinutesContent, fromDOM = false) => {
    if (!editing || locked || busy) return false;
    try { queue.edit(action); if (fromDOM) failure.current = ""; setError(""); return true; } catch (cause) { if (fromDOM) failed(cause); else setError(String(cause)); return false; }
  };
  const disabled = locked || busy || confirm.pending || !editing || !saved.draft;
  const changingPageDisabled = disabled || !!failure.current || compositions.current.size > 0;
  const chipLabel = (id: string) => { const e = doc.evidence.find((e) => e.id === id); return e ? `${timeLabel(e.startUs)}${e.verificationState === "user_verified" ? " ✓" : " · 未確認"}` : "根拠"; };
  const field = (nodes: Inline[], address: InlineAddress, label: string, evidenceOnly = false) => <MinutesRichField key={`${epoch}:${JSON.stringify(address)}`} value={nodes} label={label} blockIndex={address.block} addressKey={inlineAddressKey(address)} disabled={disabled} evidenceOnly={evidenceOnly} chipLabel={chipLabel}
    onChange={(nodes, nativeHistory) => edit((d) => editInline(d, address, nodes, nativeHistory ? [...inputEvidence.current.links.values()] : []), true)} onSelection={(range) => setSelection({ address, ...range })} onFailure={failed} onEvidence={(id,opener) => documentHandle.current?.showEvidence(id,opener)}
    onCompositionChange={(active) => { const key = JSON.stringify(address); if (active) compositions.current.add(key); else compositions.current.delete(key); setSaved(queue.snapshot()); }} />;
  const selectedBlock = selection ? doc.blocks[selection.address.block] : null;
  const structural = (action: (doc: MinutesContent) => MinutesContent, next: InlineAddress | number = selection?.address ?? { block: 0 }) => {
    if (compositions.current.size || failure.current) return false;
    if (!edit(action)) return false;
    const blocks = queue.snapshot().payload!.blocks;
    const requested = typeof next === "number" ? { block: next } : next;
    focusAddress.current = documentAddress(blocks, requested);
    const nextPage = documentPageForAddress(blocks, focusAddress.current ?? requested);
    if (!focusAddress.current) focusAddress.current = firstDocumentAddress(blocks, nextPage);
    setPage(nextPage); setSelection(null); setEpoch((v) => v + 1);
    if (formatOpen) {
      focusAfterOptions.current = true;
      setFormatOpen(false);
    }
    return true;
  };
  const changePage = (next: number) => {
    if (changingPageDisabled || busyRef.current || compositions.current.size || failure.current) return;
    focusAddress.current = firstDocumentAddress(doc.blocks, next);
    setPage(next); setSelection(null); setEpoch((value) => value + 1);
  };
  const headingField = (index: number) => [...(editorRoot.current?.querySelectorAll<HTMLElement>(".minutes-rich-field") ?? [])].find(field => field.dataset.inlineAddress === inlineAddressKey({ block:index })) ?? null;
  const headingNavigation = {
    disabled: changingPageDisabled,
    target: headingField,
    go: (entry: { blockIndex: number; page: number }) => {
      if (changingPageDisabled || busyRef.current || compositions.current.size || failure.current) return false;
      setPage(entry.page); setSelection(null);
      requestAnimationFrame(() => {
        const target = headingField(entry.blockIndex);
        if (target?.isConnected && !target.closest("[hidden]") && !document.querySelector("dialog[open]")) { target.focus({ preventScroll:true }); target.scrollIntoView({ block:"start" }); }
      });
      return true;
    },
  };
  const conversionAddress = selection ? { block: selection.address.block + (selectedBlock?.type === "list" && (selection.address.item ?? 0) > 0 ? 1 : 0) } : { block: 0 };
  const toolbarButton = (label: string, action: (opener: HTMLButtonElement) => void, unavailable = false) => formatIcons[label]
    ? <IconButton label={label} icon={formatIcons[label]!} disabled={changingPageDisabled || unavailable} onMouseDown={(e) => e.preventDefault()} onClick={event => action(event.currentTarget)} />
    : <button type="button" className="small-secondary" disabled={changingPageDisabled || unavailable} onMouseDown={(e) => e.preventDefault()} onClick={event => action(event.currentTarget)}>{label}</button>;
  const manualLabel = (id: string | null | undefined) => id && doc.items.find((i) => i.id === id)?.sourceType === "manual" ? <small className="manual-label">手動追加・修正</small> : null;
  const retrySaving = () => run(async () => {
    if (failure.current || compositions.current.size) throw new Error("入力を確定してから保存してください");
    await queue.retry();
  });
  const retryFromEditor = async (actionOpener: HTMLButtonElement) => {
    // The retry control disappears as the error clears. Preserve only the
    // focus owned by this entry, never a choice made while saving.
    actionOpener.focus({ preventScroll: true });
    await retrySaving();
    requestAnimationFrame(() => {
      const target = queue.snapshot().error ? retryToggle.current : editToggle.current;
      const active = document.activeElement;
      if (mounted.current && target?.getClientRects().length && !target.disabled
        && (active === actionOpener || active === document.body) && !document.querySelector("dialog[open]")) target.focus({ preventScroll: true });
    });
  };
  const finishEditing = (actionOpener: HTMLButtonElement) => {
    // macOS pointer activation need not focus a button. Capture the explicit
    // exit entry before run disables controls and the rich field loses focus.
    actionOpener.focus({ preventScroll: true });
    const focused = document.activeElement;
    return run(async () => {
      if (failure.current || compositions.current.size) throw new Error("入力を確定してから編集を終了してください");
      await queue.flush();
      const restore = (focused === document.activeElement || document.activeElement === document.body)
        && (editorRoot.current?.contains(focused) || actionOpener === focused);
      setEditing(false); setSelection(null);
      if (restore) requestAnimationFrame(() => {
        const target = editToggle.current;
        const active = document.activeElement;
        if (mounted.current && target?.getClientRects().length && !target.disabled
          && (active === focused || active === document.body) && !document.querySelector("dialog[open]")) target.focus({ preventScroll: true });
      });
    });
  };
  const primaryActions = <div className="transcript-actions minutes-edit-actions" hidden={full}>
      {!editing && <IconButton ref={editToggle} label="議事録を編集" icon="edit" disabled={locked || busy} onClick={() => void begin()} />}
      {editing && <IconButton ref={editToggle} label="編集を終了" icon="close" disabled={locked || busy} onClick={event => void finishEditing(event.currentTarget)} />}
      {saved.draft && <span role="status" aria-label="議事録の保存状態" className="minutes-save-status">{saved.error ? "保存要確認" : saved.saving ? "保存中" : saved.dirty ? "未保存" : "保存済み"}</span>}
      {saved.error && <button ref={retryToggle} type="button" className="small-secondary" disabled={locked || busy || saved.saving || confirm.pending} onClick={event => void retryFromEditor(event.currentTarget)}>議事録保存を再試行</button>}
      <OptionsModal className="minutes-edit-options" label="その他の編集操作" triggerLabel="編集の管理" icon="edit" hidden={!visible} triggerHost={optionsHost}>
      {saved.draft && <IconButton label="直前の議事録編集を取り消す" icon="undo" disabled={locked || busy || saved.saving || compositions.current.size > 0 || (!saved.dirty && saved.draft.lockVersion === 0)} onClick={() => void undo()} />}
      <IconButton label="議事録編集を再読込" icon="refresh" disabled={locked || busy || saved.saving} onClick={() => void reload()} />
      </OptionsModal>
    </div>;
  return <section ref={editorRoot} className="minutes-editor" aria-label="議事録文書と編集">
    {actionsHost ? createPortal(primaryActions, actionsHost) : primaryActions}
    {editing && <>
      <OptionsModal className="workspace-tools minutes-format-tools" hidden={full || !visible} label="議事録の書式・構造操作" triggerLabel="書式の詳細" icon="more" triggerHost={formatActionsHost} triggerRef={formatTrigger} open={formatOpen} onOpenChange={next=>{focusAfterOptions.current=!next;setFormatOpen(next);}}><div className="workspace-tools-body">
      <p>文字を選択して太字・斜体を適用できます。見出し・箇条書き・表は選択中の要素へ適用します。本文・担当者・期限の変更は手動修正として保存します。根拠の引用は変えません。</p>
      <div className="minutes-toolbar" role="toolbar" aria-label="議事録の書式と構造">
        {toolbarButton("斜体", () => { if (selection) edit((d) => editInline(d, selection.address, formatInline(inlineAt(d, selection.address), selection.start, selection.end, "italic"))); }, !selection || selection.end <= selection.start)}
        {toolbarButton("段落", () => { if (selection) structural((d) => convertBlock(d, selection.address, "paragraph"), conversionAddress); }, !selection || selectedBlock?.type === "table")}
        {([1, 3] as const).map((level) => <span key={level}>{toolbarButton(`見出し${level}`, () => { if (selection) structural((d) => convertBlock(d, selection.address, "heading", level), conversionAddress); }, !selection || selectedBlock?.type === "table")}</span>)}
        {toolbarButton("番号付きリスト", () => { if (selection) structural((d) => convertBlock(d, selection.address, "list", 2, true)); }, !selection || selectedBlock?.type === "table")}
        {toolbarButton("項目を追加", () => { if (selection) structural((d) => changeList(d, selection.address, false), { ...selection.address, item: (selection.address.item ?? 0) + 1 }); }, selectedBlock?.type !== "list")}
        {toolbarButton("項目を削除", () => { if (selection) structural((d) => changeList(d, selection.address, true)); }, selectedBlock?.type !== "list")}
        {(["paragraph", "heading", "list"] as const).map((type) => <span key={type}>{toolbarButton(({ paragraph: "段落を挿入", heading: "見出しを挿入", list: "リストを挿入" })[type], () => structural((d) => insertBlock(d, selection?.address.block ?? d.blocks.length - 1, type), (selection?.address.block ?? doc.blocks.length - 1) + 1))}</span>)}
        {(["addRow", "removeRow", "addColumn", "removeColumn"] as const).map((action) => <span key={action}>{toolbarButton(({ addRow: "表の行を追加", removeRow: "表の行を削除", addColumn: "表の列を追加", removeColumn: "表の列を削除" })[action], () => { if (selection) structural((d) => changeTable(d, selection.address, action), action === "addRow" ? { block: selection.address.block, row: (selection.address.row ?? (selectedBlock?.type === "table" ? selectedBlock.rows.length - 1 : 0)) + 1, column: 0 } : selection.address); }, selectedBlock?.type !== "table" || (action === "removeRow" && selection?.address.header))}</span>)}
        {toolbarButton("ブロックを削除", opener => {
          if (!selection) return;
          const block = selection.address.block, expected = JSON.stringify(queue.snapshot().payload);
          void confirm({ opener, title: "議事録ブロックの削除を確認", target: `議事録ブロック${block + 1}`, confirmLabel: "ブロックを削除する", message: "選択した文書ブロックと、その項目・根拠を下書きから削除します。保存済みの生成原文は保持し、直前の編集の取り消しで戻せます。", validate: () => !compositions.current.size && !failure.current && JSON.stringify(queue.snapshot().payload) === expected, onError: cause => { if (mounted.current) setError(String(cause)); }, action: async () => { await queue.flush(); if (!structural(d => removeBlock(d, block))) throw Error("ブロックを変更できませんでした。入力と選択を再確認してください。"); await queue.flush(); } });
        }, !selection)}
      </div>
      </div></OptionsModal>
    </>}
    <div className="minutes-split" data-full={full}>
    <div className="minutes-body-scroll" role="region" aria-label="議事録本文" tabIndex={0}>
    {editing && <>
      <div className="minutes-primary-toolbar" role="toolbar" aria-label="議事録の基本書式" hidden={full}>
        {toolbarButton("太字", () => { if (selection) edit(d => editInline(d, selection.address, formatInline(inlineAt(d, selection.address), selection.start, selection.end, "bold"))); }, !selection || selection.end <= selection.start)}
        {toolbarButton("見出し2", () => { if (selection) structural(d => convertBlock(d, selection.address, "heading", 2), conversionAddress); }, !selection || selectedBlock?.type === "table")}
        {toolbarButton("箇条書き", () => { if (selection) structural(d => convertBlock(d, selection.address, "list")); }, !selection || selectedBlock?.type === "table")}
        {toolbarButton("表を挿入", () => structural(d => insertBlock(d, selection?.address.block ?? d.blocks.length - 1, "table"), (selection?.address.block ?? doc.blocks.length - 1) + 1))}
        <div ref={setFormatActionsHost} className="minutes-format-actions-host" />
      </div>
      <div className="minutes-wysiwyg" aria-label="議事録リッチテキスト" hidden={full}>
        {plan.fragments.map(({ blockIndex: index, rowStart, rowEnd }) => {
          const b = doc.blocks[index];
          if (b.type === "heading" || b.type === "paragraph") return <section key={`${epoch}:${index}`} className={b.type === "heading" ? `rich-heading rich-heading-${b.level}` : "rich-paragraph"}>{manualLabel(b.itemId)}{field(b.content, { block: index }, `議事録ブロック${index + 1}`)}</section>;
          if (b.type === "list") { const List = b.ordered ? "ol" : "ul"; return <List key={`${epoch}:${index}`} start={b.ordered ? rowStart + 1 : undefined}>{b.items.slice(rowStart, rowEnd).map((row, offset) => { const item = rowStart + offset; return <li key={item} aria-posinset={item + 1} aria-setsize={b.items.length}>{manualLabel(row.itemId)}{field(row.content, { block: index, item }, `議事録リスト${index + 1}の項目${item + 1}`)}</li>; })}</List>; }
          return <div className="document-table" key={`${epoch}:${index}`}><table aria-rowcount={b.rows.length + 1}><caption>表{index + 1}: {b.rows.length ? `行${rowStart + 1}〜${rowEnd} / 全${b.rows.length}行` : "0行"}</caption><thead><tr aria-rowindex={1}>{b.headers.map((cell, column) => <th key={column}>{field(cell, { block: index, header: true, column }, `議事録表${index + 1}の見出し${column + 1}`)}</th>)}</tr></thead><tbody>{b.rows.slice(rowStart, rowEnd).map((row, offset) => { const r = rowStart + offset; return <tr key={r} aria-rowindex={r + 2}>{row.cells.map((cell, column) => <td key={column}>{column === 0 && manualLabel(row.itemId)}{field(cell, { block: index, row: r, column }, `議事録表${index + 1}の行${r + 1}列${column + 1}`, !!row.itemId && column === 3)}</td>)}</tr>; })}</tbody></table></div>;
        })}
      </div>
      <div hidden={full}><MinutesPages plan={plan} mode="編集" disabled={changingPageDisabled} changePage={changePage} /></div>
    </>}
    {error && <p className="action-error" role="alert">{error}</p>}
    <IssueNotice scope={view.meetingId} type="minutes-save" label="議事録の保存に失敗" message={saved.error ? `入力はこの画面に保持しています。修正後に再試行してください: ${saved.error}` : ""} action={{ kind: "retry", label: "議事録保存を再試行", disabled: locked || busy || saved.saving || confirm.pending, run: retrySaving }} />
    <MinutesDocument key={view.id} view={view} locked={locked || busy} content={doc} title={editing ? "編集中の議事録プレビュー・根拠" : "現在の議事録"}
      handle={documentHandle} readerHidden={editing} panelHost={referenceMount} panelHandle={referenceHandle} active={visible} onFullChange={setFull} navigateHeading={editing ? headingNavigation : undefined}
      verify={disabled || failure.current || compositions.current.size ? undefined : (id, verified) => { edit((d) => setEvidenceVerification(d, id, verified)); }} />
    </div>
    <WorkspacePanel collapsible content={referenceMount} label="議事録の目次・根拠" side="right" active={visible} handle={referenceHandle} />
    </div>
  </section>;
}
