import { useState } from "react";
import { timeLabel } from "./transcript";
import type { MinutesContent, Inline, Block } from "./minutes-state";

// Typed document nodes become React text elements, never model-supplied HTML.
export function MinutesDocument({ content, title }: { content: MinutesContent; title: string }) {
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const evidence = content.evidence.find((e) => e.id === selected);
  const source = (id: string | null | undefined) => id && content.items.find((i) => i.id === id)?.sourceType === "manual" ? <small className="manual-label">手動追加・修正</small> : null;
  const inline = (nodes: Inline[]) => nodes.map((node, index) => {
    if (node.type === "evidence") {
      const e = content.evidence.find((e) => e.id === node.evidenceLinkId && e.minutesItemId === node.minutesItemId);
      return e ? <button type="button" className="evidence-chip" key={index} onClick={() => setSelected(e.id)} title="生成時に固定した発言を確認">{timeLabel(e.startUs)}{e.verificationState === "user_verified" ? " ✓" : " · 未確認"}</button> : <span key={index}>根拠不明</span>;
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
  return <section className="minutes-document" aria-label={title}>
    <h3>{title}</h3>
    <div className="minutes-document-layout"><article>{content.blocks.slice(currentPage * 50, (currentPage + 1) * 50).map(block)}</article>
      {evidence && <aside className="fixed-evidence" aria-label="固定された根拠発言"><div className="card-head"><h4>根拠の発言</h4><button type="button" className="small-secondary" onClick={() => setSelected(null)}>閉じる</button></div>
        <p>{timeLabel(evidence.startUs)}–{timeLabel(evidence.endUs)} · {evidence.verificationState === "user_verified" ? "利用者確認済み" : "未確認"}</p><blockquote>{evidence.quoteText}</blockquote>
        <small>固定解析版: {evidence.analysisRevisionId}<br />発言ID: {evidence.utteranceId}</small><p>生成時の引用です。音声再生と元発言への移動は接続作業中です。</p>
      </aside>}
    </div>
    {total > 1 && <div className="transcript-pages"><button type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>前へ</button><span>{currentPage + 1} / {total}</span><button type="button" disabled={currentPage + 1 >= total} onClick={() => setPage(currentPage + 1)}>次へ</button></div>}
  </section>;
}
