import { invoke, isTauri } from "@tauri-apps/api/core";
import { InfoPopover } from "./info-popover";
import { useEffect, useState, type MutableRefObject } from "react";
import { DraftSaveQueue, registerPendingEdits, type VersionedDraft } from "./draft-save";
import { effectiveVocabulary, validVocabulary, type VocabularyEntry, type VocabularyStamp } from "./vocabulary-state";

type Book = VersionedDraft<VocabularyEntry[]>;
type View = { global: Book; meeting: Book };

export function VocabularyEditor({ meetingId, locked, stampRef }: {
  meetingId?: string; locked: boolean; stampRef?: MutableRefObject<(() => VocabularyStamp) | null>;
}) {
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
    if (loading || !state.dirty || state.saving || state.error || !state.payload || !validVocabulary(state.payload)) return;
    const timer = window.setTimeout(() => { void queue.flush().catch(() => {}); }, 700);
    return () => window.clearTimeout(timer);
  }, [queue, state, loading]);
  const reload = async () => {
    if (queue.snapshot().dirty && !window.confirm("未保存の用語入力を破棄して再読込しますか？ この入力は復元できません。過去の解析条件は変更しません。")) return;
    setLoading(true); setError("");
    try { const value = await load(); queue.load(value.book); setCommon(value.common); }
    catch (cause) { setError(String(cause)); }
    finally { setLoading(false); }
  };
  const update = (id: string, patch: Partial<VocabularyEntry>) => queue.edit((items) => items.map((item) => item.id === id ? { ...item, ...patch } : item));
  const disabled = locked || loading || !state.payload;
  const items = state.payload ?? [];
  const currentPage = Math.min(page, Math.max(0, Math.ceil(items.length / 50) - 1));
  const count = effectiveVocabulary(common?.payload ?? [], items).length;
  const editor = <>
    {meetingId && <details className="vocabulary-info"><summary>共通用語の適用情報：有効 {common?.payload.filter((e) => e.enabled).length ?? "—"} 件 · この会議の適用候補 {count} 件</summary><p>共通用語は設定画面で編集できます。同じ表記は有効な会議別用語を優先し、無効な会議別用語は共通用語を除外しません。開始時に保存済みの両リストを固定します。</p><ul>{common?.payload.filter((e) => e.enabled).map((e) => <li key={e.id}>{e.term}{e.reading ? `（${e.reading}）` : ""}{e.note ? ` — ${e.note}` : ""}</li>)}</ul></details>}
    <div className="analysis-actions"><button type="button" className="small-secondary" disabled={disabled || items.length >= 500} onClick={() => { queue.edit((value) => [...value, { id: crypto.randomUUID(), term: "", reading: null, note: null, enabled: true }]); setPage(Math.floor(items.length / 50)); }}>用語を追加</button><button type="button" className="small-secondary" disabled={locked || loading || state.saving} onClick={() => void reload()}>用語を再読込</button><button type="button" className="small-secondary" disabled={disabled || state.saving || !state.dirty} onClick={() => void queue.retry().catch(() => {})}>{state.error ? "用語保存を再試行" : "用語を保存"}</button><span role="status">{loading ? "用語を読込中" : state.error ? "用語の保存に失敗" : state.saving ? "用語を保存中" : state.dirty ? "用語は未保存" : state.draft ? "用語は保存済み" : "用語を読込できていません"} · {items.length} / 500 件</span></div>
    {items.length === 0 && <p>登録用語はありません。登録は任意です。</p>}
    {state.dirty && !validVocabulary(items) && <p role="status">用語の空欄・重複・文字数を確認してください。入力が揃うまで自動保存せず、この画面に保持します。</p>}
    <fieldset className="vocabulary-list" disabled={disabled}>{items.slice(currentPage * 50, (currentPage + 1) * 50).map((entry, index) => <article key={entry.id} aria-label={`用語 ${currentPage * 50 + index + 1}`}>
      <label>用語（必須）<input value={entry.term} maxLength={200} onChange={(event) => update(entry.id, { term: event.target.value })} /></label>
      <label>読み（任意）<input value={entry.reading ?? ""} maxLength={200} onChange={(event) => update(entry.id, { reading: event.target.value || null })} /></label>
      <label>補足（任意）<textarea value={entry.note ?? ""} maxLength={1000} onChange={(event) => update(entry.id, { note: event.target.value || null })} /></label>
      <div className="analysis-actions"><label><input type="checkbox" checked={entry.enabled} onChange={(event) => update(entry.id, { enabled: event.target.checked })} />有効</label><button type="button" className="small-secondary" onClick={() => { if (window.confirm(`用語「${entry.term || "空欄"}」を削除しますか？ 登録は復元できません。過去の解析の固定コピーと結果は保持します。`)) queue.edit((value) => value.filter((e) => e.id !== entry.id)); }}>用語を削除</button></div>
    </article>)}</fieldset>
    {items.length > 50 && <nav aria-label="用語のページ" className="transcript-pages"><button type="button" disabled={currentPage === 0 || locked} onClick={() => setPage(currentPage - 1)}>前へ</button><span>{currentPage + 1} / {Math.ceil(items.length / 50)}</span><button type="button" disabled={(currentPage + 1) * 50 >= items.length || locked} onClick={() => setPage(currentPage + 1)}>次へ</button></nav>}
    {(state.error || error) && <p className="action-error" role="alert">{state.error || error} 入力は保持しています。空欄・重複を修正して保存を再試行してください。競合時は入力を控えてから再読込してください。</p>}
  </>;
  const help = <InfoPopover label={meetingId ? "会議の用語" : "共通用語"} details={<p>用語・読み・補足を認識ヒントとして渡します。自動置換や精度向上の保証ではありません。編集は次回の新しい解析に反映し、実行中・同じ条件の再試行・過去の結果は変更しません。共通用語は設定画面で編集できます。同じ表記は有効な会議別用語を優先し、無効な会議別用語は共通用語を除外しません。開始時に保存済みの両リストを固定します。</p>}>用語は自動置換ではなく認識ヒントです。編集は次回の新しい解析に反映し、実行中・同じ条件の再試行・過去の結果は変更しません。</InfoPopover>;
  return meetingId ? <section><div className="heading-with-info"><h3>会議の用語</h3>{help}</div><details className="analysis-settings"><summary>会議別用語（任意）</summary>{editor}</details></section> : <section className="analysis-card" aria-label="共通用語"><div className="heading-with-info"><h2>共通用語</h2>{help}</div>{editor}</section>;
}
