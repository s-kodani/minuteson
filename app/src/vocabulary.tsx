import { invoke, isTauri } from "@tauri-apps/api/core";
import { IconButton } from "./action-icon";
import { OptionsModal } from "./options-modal";
import type { ModalReplacement } from "./common-modal";
import { InfoPopover } from "./info-popover";
import { IssueNotice } from "./toast";
import { useActionConfirmation } from "./confirmation-service";
import { useEffect, useId, useRef, useState, type MutableRefObject, type ReactNode } from "react";
import { DraftSaveQueue, registerPendingEdits, type VersionedDraft } from "./draft-save";
import { effectiveVocabulary, validVocabulary, type VocabularyEntry, type VocabularyStamp } from "./vocabulary-state";

type Book = VersionedDraft<VocabularyEntry[]>;
type View = { global: Book; meeting: Book };

export function VocabularyEditor({ meetingId, meetingTitle, locked, stampRef, renderEditor }: {
  meetingId?: string; meetingTitle?: string; locked: boolean; stampRef?: MutableRefObject<(() => VocabularyStamp) | null>;
  renderEditor?: (entry: ReactNode, view: ModalReplacement | null, persistentEditor: ReactNode) => ReactNode;
}) {
  const titleId = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const confirm = useActionConfirmation(locked);
  const [queue] = useState(() => new DraftSaveQueue<VocabularyEntry[], Book>(
    (_id, expectedLockVersion, payload) => invoke("save_vocabulary_book", { meetingId: meetingId ?? null, expectedLockVersion, payload }),
    (payload) => structuredClone(payload),
  ));
  const [state, setState] = useState(() => queue.snapshot());
  const [common, setCommon] = useState<Book | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [page, setPage] = useState(0);
  const load = async () => {
    if (meetingId) {
      const view = await invoke<View>("get_meeting_vocabulary", { meetingId });
      return { book: view.meeting, common: view.global };
    }
    return { book: await invoke<Book>("get_vocabulary_book", { meetingId: null }), common: null };
  };
  useEffect(() => {
    let active = true;
    const unsubscribe = queue.subscribe(() => setState(queue.snapshot()));
    const unregister = registerPendingEdits(async () => {
      await queue.flush();
    });
    if (isTauri()) void load().then((value) => {
      if (active) { queue.load(value.book); setCommon(value.common); setLoading(false); }
    }).catch((cause: unknown) => { if (active) { setError(String(cause)); setLoading(false); } });
    else setLoading(false);
    return () => { active = false; unsubscribe(); unregister(); if (stampRef) stampRef.current = null; };
  }, [queue]);
  useEffect(() => {
    if (!stampRef) return;
    stampRef.current = () => {
      const snapshot = queue.snapshot();
      if (!snapshot.draft || !common || snapshot.dirty || snapshot.error || snapshot.saving) throw new Error("用語リストの読込・保存を完了してください。");
      return { globalVersion: common.lockVersion, meetingVersion: snapshot.draft.lockVersion };
    };
    return () => { stampRef.current = null; };
  }, [queue, common, stampRef]);
  useEffect(() => {
    if (loading || confirm.pending || !state.dirty || state.saving || state.error || !state.payload || !validVocabulary(state.payload)) return;
    const timer = window.setTimeout(() => { void queue.flush().catch(() => {}); }, 700);
    return () => window.clearTimeout(timer);
  }, [queue, state, loading, confirm.pending]);
  const reload = async () => {
    const expected = JSON.stringify(queue.snapshot().payload);
    const read = async () => {
    setLoading(true); setError("");
    try { const value = await load(); queue.load(value.book); setCommon(value.common); }
    catch (cause) { setError(String(cause)); throw cause; }
    finally { setLoading(false); }
    };
    if (queue.snapshot().dirty) await confirm({ title: "用語入力の破棄を確認", target: meetingId ? `会議「${meetingTitle ?? meetingId}」の用語` : "共通用語", confirmLabel: "入力を破棄して再読込する", message: "未保存の用語入力を破棄して再読込します。この入力は復元できません。過去の解析条件は変更しません。", validate: () => JSON.stringify(queue.snapshot().payload) === expected, action: read });
    else try { await read(); } catch { /* error is displayed above */ }
  };
  const remove = (entry: VocabularyEntry) => {
    const expected = JSON.stringify(queue.snapshot().payload);
    void confirm({ title: "用語の削除を確認", target: `${meetingId ? `会議「${meetingTitle ?? meetingId}」の用語` : "共通用語"} · ${entry.term || "空欄の用語"}`, confirmLabel: "用語を削除する", message: `用語「${entry.term || "空欄"}」を削除します。登録は復元できません。過去の解析の固定コピーと結果は保持します。`, validate: () => JSON.stringify(queue.snapshot().payload) === expected, onError: cause => setError(String(cause)), action: async () => { queue.edit(value => value.filter(item => item.id !== entry.id)); if (validVocabulary(queue.snapshot().payload ?? [])) await queue.flush(); } });
  };
  const update = (id: string, patch: Partial<VocabularyEntry>) => queue.edit((items) => items.map((item) => item.id === id ? { ...item, ...patch } : item));
  const disabled = locked || loading || confirm.pending || !state.payload;
  const items = state.payload ?? [];
  const currentPage = Math.min(page, Math.max(0, Math.ceil(items.length / 50) - 1));
  const count = effectiveVocabulary(common?.payload ?? [], items).length;
  const editor = <>
    {meetingId && <details className="vocabulary-info"><summary>共通用語の適用情報：有効 {common?.payload.filter((e) => e.enabled).length ?? "—"} 件 · この会議の適用候補 {count} 件</summary><p>共通用語はプロンプト画面で編集できます。同じ表記は有効な会議別用語を優先し、無効な会議別用語は共通用語を除外しません。開始時に保存済みの両リストを固定します。</p><ul>{common?.payload.filter((e) => e.enabled).map((e) => <li key={e.id}>{e.term}{e.reading ? `（${e.reading}）` : ""}{e.note ? ` — ${e.note}` : ""}</li>)}</ul></details>}
    <div className="analysis-actions"><IconButton label="用語を追加" icon="plus" type="button" className="small-secondary" disabled={disabled || items.length >= 500} onClick={() => { queue.edit((value) => [...value, { id: crypto.randomUUID(), term: "", reading: null, note: null, enabled: true }]); setPage(Math.floor(items.length / 50)); }} /><IconButton label="用語を再読込" icon="refresh" type="button" className="small-secondary" disabled={locked || loading || state.saving || confirm.pending} onClick={() => void reload()} /><button type="button" className="small-secondary" disabled={disabled || state.saving || !state.dirty || !validVocabulary(items)} onClick={() => void queue.retry().catch(() => {})}>{state.error ? "用語保存を再試行" : "用語を保存"}</button><span role="status">{loading ? "用語を読込中" : state.error ? "用語の保存に失敗" : state.saving ? "用語を保存中" : state.dirty ? "用語は未保存" : state.draft ? "用語は保存済み" : "用語を読込できていません"} · {items.length} / 500 件</span></div>
    {items.length === 0 && <p>登録用語はありません。登録は任意です。</p>}
    {state.dirty && !validVocabulary(items) && <p role="status">用語の空欄・重複・文字数を確認してください。入力が揃うまで自動保存せず、この画面に保持します。</p>}
    <fieldset className="vocabulary-list" disabled={disabled}>{items.slice(currentPage * 50, (currentPage + 1) * 50).map((entry, index) => <article key={entry.id} aria-label={`用語 ${currentPage * 50 + index + 1}`}>
      <label>用語（必須）<input value={entry.term} maxLength={200} onChange={(event) => update(entry.id, { term: event.target.value })} /></label>
      <label>読み（任意）<input value={entry.reading ?? ""} maxLength={200} onChange={(event) => update(entry.id, { reading: event.target.value || null })} /></label>
      <label>補足（任意・保存表示用）<textarea value={entry.note ?? ""} maxLength={1000} onChange={(event) => update(entry.id, { note: event.target.value || null })} /></label>
      <div className="analysis-actions"><label><input type="checkbox" checked={entry.enabled} onChange={(event) => update(entry.id, { enabled: event.target.checked })} />有効</label><button type="button" className="small-secondary" onClick={() => remove(entry)}>用語を削除</button></div>
    </article>)}</fieldset>
    {items.length > 50 && <nav aria-label="用語のページ" className="transcript-pages"><IconButton label="前へ" icon="previous" type="button" disabled={currentPage === 0 || locked} onClick={() => setPage(currentPage - 1)} /><span>{currentPage + 1} / {Math.ceil(items.length / 50)}</span><IconButton label="次へ" icon="next" type="button" disabled={(currentPage + 1) * 50 >= items.length || locked} onClick={() => setPage(currentPage + 1)} /></nav>}
  </>;
  const help = <InfoPopover label={meetingId ? "会議の用語" : "共通用語"} details={<p>用語と読みを認識ヒントとして渡します。補足は保存・表示用で、音声認識には渡しません。自動置換や精度向上の保証ではありません。編集は次回の新しい解析に反映し、実行中・同じ条件の再試行・過去の結果は変更しません。共通用語はプロンプト画面で編集できます。同じ表記は有効な会議別用語を優先し、無効な会議別用語は共通用語を除外しません。開始時に保存済みの両リストを固定します。</p>}>用語は自動置換ではなく認識ヒントです。編集は次回の新しい解析に反映し、実行中・同じ条件の再試行・過去の結果は変更しません。</InfoPopover>;
  // Keep the problem visible even when the optional editor is collapsed. A
  // notification can be dismissed without clearing the queue or the input.
  const issue = <IssueNotice scope={meetingId ?? "global"} type="vocabulary-save" label="用語の読込・保存" message={state.error || error ? `${state.error || error} 入力は保持しています。空欄・重複を修正して再試行してください。競合時は入力を控えてから再読込してください。` : ""} action={state.error ? { kind: "retry", label: "用語保存を再試行", disabled: disabled || state.saving || !state.dirty || !validVocabulary(items), run: async () => { if (!locked && !loading && !confirm.pending && validVocabulary(queue.snapshot().payload ?? [])) await queue.retry().catch(() => {}); } } : undefined} />;
  if (meetingId && renderEditor) {
    const entry = <section><div className="heading-with-info"><h3>会議の用語</h3>{help}</div>{issue}<div className="options-modal analysis-settings"><button ref={trigger} type="button" className="small-secondary options-modal-trigger" aria-label="会議別用語（任意）" aria-haspopup="dialog" aria-expanded={open} onClick={event=>{event.currentTarget.focus({preventScroll:true});setOpen(true);}}>会議別用語（任意）</button></div></section>;
    const persistentEditor = <div className="meeting-vocabulary-view" hidden={!open}><h2 id={titleId}>会議別用語（任意）</h2><div className="options-dialog-content">{editor}</div></div>;
    const view: ModalReplacement | null = open ? {
      key:"meeting-vocabulary", labelledBy:titleId, content:null, className:"meeting-vocabulary-dialog",
      blocked:locked || confirm.pending, onDismiss:()=>setOpen(false), restoreTarget:()=>trigger.current, dismissLabel:"閉じる",
    } : null;
    return renderEditor(entry, view, persistentEditor);
  }
  return meetingId ? <section><div className="heading-with-info"><h3>会議の用語</h3>{help}</div>{issue}<OptionsModal className="analysis-settings" label="会議別用語（任意）">{editor}</OptionsModal></section> : <section className="analysis-card" aria-label="共通用語"><div className="heading-with-info"><h2>共通用語</h2>{help}</div>{issue}{editor}</section>;
}
