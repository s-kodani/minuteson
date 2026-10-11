import { useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type Ref, type RefObject } from "react";
import { createPortal } from "react-dom";
import { timeLabel } from "./transcript";
import type { MinutesContent, Inline, Block, MinutesView } from "./minutes-state";
import { MinutesEvidenceGroup } from "./minutes-evidence";
import { FixedTranscriptWorkspace } from "./fixed-transcript";
import { documentOutline, documentPage, type DocumentFragment, type DocumentOutlineEntry } from "./minutes-pagination";
import { MinutesPages } from "./minutes-pages";
import type { WorkspacePanelHandle } from "./workspace-panel";

// Typed document nodes become React text elements, never model-supplied HTML.
export type MinutesDocumentHandle = { showEvidence: (id: string, opener?: HTMLElement) => void };
export function MinutesDocument({ content, title, view, locked = false, verify, panelHost = null, panelHandle, active = true, onFullChange, navigateHeading, readerHidden = false, handle }: {
  content: MinutesContent; title: string; view: MinutesView; locked?: boolean; verify?: (id: string, verified: boolean) => void;
  panelHost?: HTMLElement | null; panelHandle?: RefObject<WorkspacePanelHandle | null>; active?: boolean; onFullChange?: (full: boolean) => void;
  navigateHeading?: { disabled: boolean; go: (entry: DocumentOutlineEntry) => boolean; target: (blockIndex: number) => HTMLElement | null };
  readerHidden?: boolean; handle?: Ref<MinutesDocumentHandle>;
}) {
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [evidencePage, setEvidencePage] = useState<number | null>(null);
  const [showFull, setShowFull] = useState(false);
  const fullEvidence = useRef<string | null>(null);
  const evidenceOpener = useRef<HTMLElement | null>(null);
  const section = useRef<HTMLElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const outline = useMemo(() => documentOutline(content.blocks), [content.blocks]);
  const [tocTarget,setTocTarget] = useState<{ blockIndex: number; serial: number } | null>(null);
  const handledToc = useRef(0);
  const currentActive = useRef(active); currentActive.current = active;
  const plan = documentPage(content.blocks, page);
  const previousPage = useRef(plan.page);
  useLayoutEffect(() => {
    if (previousPage.current === plan.page) return;
    previousPage.current = plan.page;
    if ((tocTarget?.serial ?? 0) === handledToc.current && currentActive.current) {
      heading.current?.focus({ preventScroll: true });
      heading.current?.scrollIntoView({ block: "start" });
    }
  }, [plan.page]);
  const bookmark = useRef<{ x: number; y: number; opener: HTMLButtonElement | null; panel: boolean; panelOpener: HTMLElement | null; scroll: { element: HTMLElement; top: number; left: number }[] } | null>(null);
  const evidence = content.evidence.find((e) => e.id === selected);
  const fullSource = content.evidence.find((e) => e.id === fullEvidence.current);
  const full = showFull && !!evidence && !!fullSource;
  useLayoutEffect(() => { onFullChange?.(full); }, [full,onFullChange]);
  const sourceOpener = (id: string) => [...((panelHost ?? section.current)?.querySelectorAll<HTMLButtonElement>("[data-fixed-transcript-opener]") ?? [])].find(el => el.dataset.fixedTranscriptOpener === id) ?? null;
  const showTranscript = (id: string) => {
    if (locked || !content.evidence.some((e) => e.id === id)) return;
    const scroll = [], seen = new Set<HTMLElement>();
    for (const root of [section.current,panelHost]) for (let element = root; element; element = element.parentElement) if (!seen.has(element)) {
      seen.add(element); scroll.push({ element, top: element.scrollTop, left: element.scrollLeft });
    }
    bookmark.current = { x: window.scrollX, y: window.scrollY, opener: sourceOpener(id), panel: !!panelHost && !panelHost.closest("[hidden]"), panelOpener: evidenceOpener.current, scroll };
    // Group members can open the full transcript without losing the group page.
    fullEvidence.current = id;
    panelHandle?.current?.dismiss(() => section.current?.querySelector<HTMLElement>(".fixed-transcript-workspace h3") ?? null);
    setShowFull(true);
  };
  useLayoutEffect(() => {
    if (selected && !evidence) { setSelected(null); setShowFull(false); }
    if (showFull && !fullSource) setShowFull(false);
  }, [selected, evidence?.id, showFull, fullSource?.id]);
  useLayoutEffect(() => {
    if (full || !bookmark.current) return;
    const saved = bookmark.current; bookmark.current = null;
    if (saved.panel) panelHandle?.current?.reveal(saved.panelOpener);
    const frame = requestAnimationFrame(() => {
      if (!currentActive.current) return;
      for (const { element, top, left } of saved.scroll) if (element.isConnected) { element.scrollTop = top; element.scrollLeft = left; }
      if (saved.opener?.isConnected) saved.opener.focus({ preventScroll: true });
      window.scrollTo(saved.x, saved.y);
    });
    return () => cancelAnimationFrame(frame);
  }, [full]);
  const goToHeading = (entry: DocumentOutlineEntry) => {
    if (locked || full) return;
    if (navigateHeading && !navigateHeading.go(entry)) return;
    setPage(entry.page);
    setTocTarget(previous => ({ blockIndex:entry.blockIndex,serial:(previous?.serial ?? 0)+1 }));
    panelHandle?.current?.dismiss(() => navigateHeading?.target(entry.blockIndex) ?? section.current?.querySelector<HTMLElement>(`[data-minutes-block="${entry.blockIndex}"]`) ?? null);
  };
  useLayoutEffect(() => {
    if (!tocTarget) return;
    handledToc.current = tocTarget.serial;
    if (navigateHeading) return;
    const frame = requestAnimationFrame(() => {
      if (!currentActive.current) return;
      const target = section.current?.querySelector<HTMLElement>(`[data-minutes-block="${tocTarget.blockIndex}"]`);
      if (target && !target.closest("[hidden]")) { target.focus({ preventScroll:true }); target.scrollIntoView({ block:"start" }); }
    });
    return () => cancelAnimationFrame(frame);
  }, [tocTarget]);
  useLayoutEffect(() => {
    if (!selected || full || !panelHost) return;
    const frame = requestAnimationFrame(() => {
      if (currentActive.current && !panelHost.closest("[hidden]")) panelHost.querySelector<HTMLElement>(".evidence-selected")?.scrollIntoView({ block:"nearest" });
    });
    return () => cancelAnimationFrame(frame);
    // Recenter only a newly selected reference. Returning from a different
    // member of its group must restore the actual opener's bookmark instead.
  }, [selected,panelHost]);
  const showEvidence = (id: string, from?: HTMLElement) => {
    if (locked || full || !active || !content.evidence.some(entry => entry.id === id)) return;
    if (from?.isConnected && from.getClientRects().length && !from.matches(":disabled") && !from.closest("[hidden]")) from.focus({ preventScroll:true });
    evidenceOpener.current = from ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    setSelected(id); setEvidencePage(null); panelHandle?.current?.reveal(evidenceOpener.current);
  };
  useImperativeHandle(handle, () => ({ showEvidence }), [locked,full,active,content.evidence,panelHandle]);
  const source = (id: string | null | undefined) => id && content.items.find((i) => i.id === id)?.sourceType === "manual" ? <small className="manual-label">手動追加・修正</small> : null;
  const inline = (nodes: Inline[]) => nodes.map((node, index) => {
    if (node.type === "evidence") {
      const e = content.evidence.find((e) => e.id === node.evidenceLinkId && e.minutesItemId === node.minutesItemId);
      return e ? <button type="button" className="evidence-chip" disabled={locked} key={index} onClick={event => showEvidence(e.id, event.currentTarget)} title="生成時に固定した発言を確認">{timeLabel(e.startUs)}{e.verificationState === "user_verified" ? " ✓" : " · 未確認"}</button> : <span key={index}>根拠不明</span>;
    }
    let text = <span className="document-text">{node.text}</span>;
    if (node.bold) text = <strong>{text}</strong>;
    if (node.italic) text = <em>{text}</em>;
    return <span key={index}>{text}</span>;
  });
  const block = (value: Block, { blockIndex: index, rowStart, rowEnd }: DocumentFragment) => {
    switch (value.type) {
      case "heading": {
        const Heading = (["h1", "h2", "h3", "h4", "h5", "h6"] as const)[Math.max(0, Math.min(5, value.level - 1))];
        return <Heading key={index} tabIndex={-1} data-minutes-block={index}>{source(value.itemId)}{inline(value.content)}</Heading>;
      }
      case "paragraph": return <p key={index}>{source(value.itemId)}{inline(value.content)}</p>;
      case "list": {
        const List = value.ordered ? "ol" : "ul";
        return <List key={index} start={value.ordered ? rowStart + 1 : undefined} aria-label={`リスト${index + 1}${value.items.length ? `の項目${rowStart + 1}〜${rowEnd}（全${value.items.length}件）` : "（0件）"}`}>{value.items.slice(rowStart, rowEnd).map((item, i) => <li key={rowStart + i} aria-posinset={rowStart + i + 1} aria-setsize={value.items.length}>{source(item.itemId)}{inline(item.content)}</li>)}</List>;
      }
      case "table": return <div className="document-table" key={index}><table aria-rowcount={value.rows.length + 1}><caption>表{index + 1}: {value.rows.length ? `行${rowStart + 1}〜${rowEnd} / 全${value.rows.length}行` : "0行"}</caption><thead><tr aria-rowindex={1}>{value.headers.map((cell, i) => <th key={i}>{inline(cell)}</th>)}</tr></thead><tbody>{value.rows.slice(rowStart, rowEnd).map((row, i) => <tr key={rowStart + i} aria-rowindex={rowStart + i + 2}>{row.cells.map((cell, j) => <td key={j}>{j === 0 && source(row.itemId)}{inline(cell)}</td>)}</tr>)}</tbody></table></div>;
    }
  };
  const references = <div className="minutes-reference-content" hidden={full}>
    <details className="minutes-outline-tools"><summary>目次</summary><nav className="minutes-outline" aria-label="議事録目次">
      {outline.length ? <ol>{outline.map(entry => <li key={entry.blockIndex} style={{ paddingInlineStart:`${Math.min(3,entry.level-1)*12}px` }}>
        <button type="button" className="small-secondary" disabled={locked || navigateHeading?.disabled} aria-current={tocTarget?.blockIndex === entry.blockIndex ? "location" : undefined}
          aria-label={`${entry.label}へ移動（見出し${entry.blockIndex + 1}）`} onClick={() => goToHeading(entry)}>{entry.label}</button>
      </li>)}</ol> : <p>見出しがありません。本文はそのまま表示します。</p>}
    </nav></details>
    {evidence ? <MinutesEvidenceGroup key={`${view.id}:${evidence.id}`} view={view} content={content} selectedId={evidence.id} page={evidencePage} changePage={setEvidencePage} suspended={full || !active} locked={locked} close={() => { setSelected(null); panelHandle?.current?.dismiss(); }} showTranscript={showTranscript} verify={verify} dockPlayback={!!panelHost} /> : <p className="hint">本文の根拠時刻を選ぶと、生成時に固定した発言・話者・音声を確認できます。</p>}
  </div>;
  return <section className="minutes-document" aria-label={title} ref={section}>
    {full && fullSource && <FixedTranscriptWorkspace key={`${view.id}:${fullSource.id}`} view={view} evidence={fullSource} locked={locked} active={active} dockPlayback={!!panelHost} back={() => setShowFull(false)} />}
    {/* Keep the reader/group mounted but inaccessible while looking at the
        fixed transcript. This preserves loaded context and group pagination,
        so asynchronous reloading cannot change the bookmarked layout. */}
    <div hidden={full || readerHidden}>
    <h3 ref={heading} tabIndex={-1}>{title}</h3>
    <div className="minutes-document-layout"><article>{plan.fragments.map((fragment) => block(content.blocks[fragment.blockIndex], fragment))}</article>
      {!panelHost && references}
    </div>
    <MinutesPages plan={plan} mode="閲覧" changePage={setPage} />
    </div>
    {panelHost && createPortal(references,panelHost)}
  </section>;
}
