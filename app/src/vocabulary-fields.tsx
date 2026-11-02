import { IconButton } from "./action-icon";
import type { VocabularyEntry } from "./vocabulary-state";

export function VocabularyFields({ items, disabled, page, onPage, onUpdate, onRemove }: {
  items: VocabularyEntry[]; disabled: boolean; page: number; onPage: (page: number) => void;
  onUpdate: (id: string, patch: Partial<VocabularyEntry>) => void; onRemove: (entry: VocabularyEntry) => void;
}) {
  const currentPage = Math.min(page, Math.max(0, Math.ceil(items.length / 50) - 1));
  return <>
    <fieldset className="vocabulary-list" disabled={disabled}>{items.slice(currentPage * 50, (currentPage + 1) * 50).map((entry, index) => <article key={entry.id} aria-label={`用語 ${currentPage * 50 + index + 1}`}>
      <label>用語（必須）<input value={entry.term} maxLength={200} onChange={event => onUpdate(entry.id, { term: event.target.value })} /></label>
      <label>読み（任意）<input value={entry.reading ?? ""} maxLength={200} onChange={event => onUpdate(entry.id, { reading: event.target.value || null })} /></label>
      <label>補足（任意・保存表示用）<textarea value={entry.note ?? ""} maxLength={1000} onChange={event => onUpdate(entry.id, { note: event.target.value || null })} /></label>
      <div className="analysis-actions"><label><input type="checkbox" checked={entry.enabled} onChange={event => onUpdate(entry.id, { enabled: event.target.checked })} />有効</label><button type="button" className="small-secondary" onClick={() => onRemove(entry)}>用語を削除</button></div>
    </article>)}</fieldset>
    {items.length > 50 && <nav aria-label="用語のページ" className="transcript-pages"><IconButton label="前へ" icon="previous" type="button" disabled={disabled || currentPage === 0} onClick={() => onPage(currentPage - 1)} /><span>{currentPage + 1} / {Math.ceil(items.length / 50)}</span><IconButton label="次へ" icon="next" type="button" disabled={disabled || (currentPage + 1) * 50 >= items.length} onClick={() => onPage(currentPage + 1)} /></nav>}
  </>;
}
