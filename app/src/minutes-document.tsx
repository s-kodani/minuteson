import { useLayoutEffect, useRef, useState } from "react";
import { timeLabel } from "./transcript";
import type { MinutesContent, Inline, Block, MinutesView } from "./minutes-state";
import { MinutesEvidenceGroup } from "./minutes-evidence";
import { FixedTranscriptWorkspace } from "./fixed-transcript";

// Typed document nodes become React text elements, never model-supplied HTML.
export function MinutesDocument({ content, title, view, locked = false, verify }: { content: MinutesContent; title: string; view: MinutesView; locked?: boolean; verify?: (id: string, verified: boolean) => void }) {
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [evidencePage, setEvidencePage] = useState<number | null>(null);
  const [showFull, setShowFull] = useState(false);
  const fullEvidence = useRef<string | null>(null);
  const section = useRef<HTMLElement>(null);
  const bookmark = useRef<{ x: number; y: number; evidenceId: string; scroll: { element: HTMLElement; top: number; left: number }[] } | null>(null);
  const evidence = content.evidence.find((e) => e.id === selected);
  const fullSource = content.evidence.find((e) => e.id === fullEvidence.current);
  const full = showFull && !!evidence && !!fullSource;
  const showTranscript = (id: string) => {
    if (locked || !content.evidence.some((e) => e.id === id)) return;
    const scroll = [];
    for (let element = section.current; element; element = element.parentElement) scroll.push({ element, top: element.scrollTop, left: element.scrollLeft });
    bookmark.current = { x: window.scrollX, y: window.scrollY, evidenceId: id, scroll };
    // Group members can open the full transcript without losing the group page.
    fullEvidence.current = id;
    setShowFull(true);
  };
  useLayoutEffect(() => {
    if (selected && !evidence) { setSelected(null); setShowFull(false); }
    if (showFull && !fullSource) setShowFull(false);
  }, [selected, evidence?.id, showFull, fullSource?.id]);
  useLayoutEffect(() => {
    if (full || !bookmark.current) return;
    const saved = bookmark.current; bookmark.current = null;
    for (const { element, top, left } of saved.scroll) if (element.isConnected) { element.scrollTop = top; element.scrollLeft = left; }
    const opener = [...(section.current?.querySelectorAll<HTMLButtonElement>("[data-fixed-transcript-opener]") ?? [])].find((el) => el.dataset.fixedTranscriptOpener === saved.evidenceId);
    opener?.focus({ preventScroll: true });
    window.scrollTo(saved.x, saved.y);
  }, [full]);
  const source = (id: string | null | undefined) => id && content.items.find((i) => i.id === id)?.sourceType === "manual" ? <small className="manual-label">手動追加・修正</small> : null;
  const inline = (nodes: Inline[]) => nodes.map((node, index) => {
    if (node.type === "evidence") {
      const e = content.evidence.find((e) => e.id === node.evidenceLinkId && e.minutesItemId === node.minutesItemId);
      return e ? <button type="button" className="evidence-chip" disabled={locked} key={index} onClick={() => { setSelected(e.id); setEvidencePage(null); }} title="生成時に固定した発言を確認">{timeLabel(e.startUs)}{e.verificationState === "user_verified" ? " ✓" : " · 未確認"}</button> : <span key={index}>根拠不明</span>;
    }
    let text = <span className="document-text">{node.text}</span>;
    if (node.bold) text = <strong>{text}</strong>;
    if (node.italic) text = <em>{text}</em>;
    return <span key={index}>{text}</span>;
  });
  const block = (value: Block, index: number) => {
    switch (value.type) {
      case "heading": {
        const Heading = (["h1", "h2", "h3", "h4", "h5", "h6"] as const)[Math.max(0, Math.min(5, value.level - 1))];
        return <Heading key={index}>{source(value.itemId)}{inline(value.content)}</Heading>;
      }
      case "paragraph": return <p key={index}>{source(value.itemId)}{inline(value.content)}</p>;
      case "list": {
        const List = value.ordered ? "ol" : "ul";
        return <List key={index}>{value.items.map((item, i) => <li key={i}>{source(item.itemId)}{inline(item.content)}</li>)}</List>;
      }
      case "table": return <div className="document-table" key={index}><table><thead><tr>{value.headers.map((cell, i) => <th key={i}>{inline(cell)}</th>)}</tr></thead><tbody>{value.rows.map((row, i) => <tr key={i}>{row.cells.map((cell, j) => <td key={j}>{j === 0 && source(row.itemId)}{inline(cell)}</td>)}</tr>)}</tbody></table></div>;
    }
  };
  const total = Math.max(1, Math.ceil(content.blocks.length / 50));
  const currentPage = Math.min(page, total - 1);
  return <section className="minutes-document" aria-label={title} ref={section}>
    {full && fullSource && <FixedTranscriptWorkspace key={`${view.id}:${fullSource.id}`} view={view} evidence={fullSource} locked={locked} back={() => setShowFull(false)} />}
    {/* Keep the reader/group mounted but inaccessible while looking at the
        fixed transcript. This preserves loaded context and group pagination,
        so asynchronous reloading cannot change the bookmarked layout. */}
    <div hidden={full}>
    <h3>{title}</h3>
    <div className="minutes-document-layout"><article>{content.blocks.slice(currentPage * 50, (currentPage + 1) * 50).map(block)}</article>
      {evidence && <MinutesEvidenceGroup key={`${view.id}:${evidence.id}`} view={view} content={content} selectedId={evidence.id} page={evidencePage} changePage={setEvidencePage} suspended={full} locked={locked} close={() => setSelected(null)} showTranscript={showTranscript} verify={verify} />}
    </div>
    {total > 1 && <div className="transcript-pages"><button type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>前へ</button><span>{currentPage + 1} / {total}</span><button type="button" disabled={currentPage + 1 >= total} onClick={() => setPage(currentPage + 1)}>次へ</button></div>}
    </div>
  </section>;
}
