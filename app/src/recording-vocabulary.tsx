import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "./ipc";
import { IconButton } from "./action-icon";
import { OptionsModal } from "./options-modal";
import { VocabularyFields } from "./vocabulary-fields";
import { VocabularyHelp } from "./vocabulary-help";
import { effectiveVocabulary, validVocabulary, type VocabularyEntry } from "./vocabulary-state";
import { useActionConfirmation } from "./confirmation-service";

// These entries belong to the unsaved recording preparation. Only the native
// start command persists them, together with the new meeting.
export function RecordingVocabulary({ items, locked, onChange }: {
  items: VocabularyEntry[]; locked: boolean; onChange: (items: VocabularyEntry[]) => void;
}) {
  const [common, setCommon] = useState<VocabularyEntry[] | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(0);
  const confirm = useActionConfirmation(locked);
  const current = useRef({ items, locked, onChange });
  current.current = { items, locked, onChange };
  const mounted = useRef(false);
  const read = async () => {
    setLoading(true); setError("");
    try {
      const book = await invoke<{ payload: VocabularyEntry[] }>("get_vocabulary_book", { meetingId: null });
      if (mounted.current) setCommon(book.payload);
    } catch (cause) { if (mounted.current) setError(String(cause)); }
    finally { if (mounted.current) setLoading(false); }
  };
  useEffect(() => {
    mounted.current = true;
    if (isTauri()) void read();
    return () => { mounted.current = false; };
  }, []);
  const disabled = locked || confirm.pending;
  const update = (id: string, patch: Partial<VocabularyEntry>) => onChange(items.map(item => item.id === id ? { ...item, ...patch } : item));
  const remove = (entry: VocabularyEntry) => {
    const expected = JSON.stringify(items);
    void confirm({ title: "録音前の用語の削除を確認", target: entry.term || "空欄の用語", confirmLabel: "用語を削除する",
      message: "録音前の入力からこの用語を削除します。この入力は復元できません。共通用語は変更しません。",
      validate: () => !current.current.locked && JSON.stringify(current.current.items) === expected,
      action: async () => current.current.onChange(current.current.items.filter(item => item.id !== entry.id)),
    });
  };
  return <section className="recording-vocabulary">
    <h3>会議の用語</h3>
    <OptionsModal label="会議の用語（任意）">
      <div className="heading-with-info"><span>用語辞書について</span><VocabularyHelp label="録音の用語" /></div>
      <p>録音開始を確定すると、この会議の用語として保存します。文字起こしの開始時に共通用語と合わせて使用する内容を固定します。録音後も文字起こし画面から編集できます。</p>
      <details className="vocabulary-info"><summary>共通用語：有効 {common?.filter(entry => entry.enabled).length ?? "—"} 件 · この会議の適用候補 {common ? effectiveVocabulary(common, items).length : "—"} 件</summary>
        <p>同じ表記は有効な会議別用語を優先します。共通用語はプロンプト画面で編集できます。</p>
        <ul>{common?.filter(entry => entry.enabled).map(entry => <li key={entry.id}>{entry.term}{entry.reading ? `（${entry.reading}）` : ""}{entry.note ? ` — ${entry.note}` : ""}</li>)}</ul>
        {error && <p role="status">共通用語を確認できません：{error}</p>}
        <IconButton label="共通用語を再読込" icon="refresh" type="button" className="small-secondary" disabled={loading || disabled} onClick={() => void read()} />
      </details>
      <div className="analysis-actions"><IconButton label="用語を追加" icon="plus" type="button" className="small-secondary" disabled={disabled || items.length >= 500} onClick={() => {
        onChange([...items, { id: crypto.randomUUID(), term: "", reading: null, note: null, enabled: true }]); setPage(Math.floor(items.length / 50));
      }} /><span role="status">録音開始時に保存 · {items.length} / 500 件</span></div>
      {!items.length && <p>登録用語はありません。登録は任意です。</p>}
      {!validVocabulary(items) && <p role="status">用語の空欄・重複・文字数を修正してから録音を開始してください。入力は保持しています。</p>}
      <VocabularyFields items={items} disabled={disabled} page={page} onPage={setPage} onUpdate={update} onRemove={remove} />
    </OptionsModal>
  </section>;
}
