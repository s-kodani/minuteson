import { useLayoutEffect, useRef } from "react";
import { compactInline } from "./minutes-edit";
import type { Inline } from "./minutes-state";

function units(node: Node): number {
  if (node instanceof Element && node.hasAttribute("data-caret-sentinel")) return 0;
  if (node instanceof Element && node.hasAttribute("data-evidence-id")) return 1;
  if (node.nodeType === Node.TEXT_NODE) return node.textContent?.length ?? 0;
  return Array.from(node.childNodes).reduce((n, child) => n + units(child), 0);
}
function position(root: HTMLElement): { start: number; end: number } | null {
  const selection = window.getSelection();
  if (!selection?.rangeCount) return null;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.startContainer) || !root.contains(range.endContainer)) return null;
  const before = document.createRange(); before.selectNodeContents(root); before.setEnd(range.startContainer, range.startOffset);
  const through = document.createRange(); through.selectNodeContents(root); through.setEnd(range.endContainer, range.endOffset);
  return { start: units(before.cloneContents()), end: units(through.cloneContents()) };
}
function restore(root: HTMLElement, start: number, end: number) {
  const point = (offset: number): [Node, number] => {
    const visit = (node: Node): [Node, number] | null => {
      if (node instanceof Element && node.hasAttribute("data-evidence-id")) {
        const index = Array.from(node.parentNode!.childNodes).indexOf(node as ChildNode);
        if (offset <= 1) return [node.parentNode!, index + offset];
        offset--; return null;
      }
      if (node.nodeType === Node.TEXT_NODE) { const length = node.textContent?.length ?? 0; if (offset <= length) return [node, offset]; offset -= length; return null; }
      for (const child of Array.from(node.childNodes)) { const result = visit(child); if (result) return result; }
      return null;
    };
    return visit(root) ?? [root, root.childNodes.length];
  };
  const range = document.createRange(); const a = point(start); const b = point(end);
  range.setStart(...a); range.setEnd(...b); const selection = window.getSelection(); selection?.removeAllRanges(); selection?.addRange(range);
}
// Accept only textual formatting and known atomic references. No HTML strings,
// URLs, attributes or scripts become document JSON. Clipboard input is plain text.
function read(root: HTMLElement, known: Map<string, Inline>): Inline[] {
  const nodes: Inline[] = [];
  const append = (text: string, bold: boolean, italic: boolean) => { if (text) nodes.push({ type: "text", text, bold, italic }); };
  const walk = (node: Node, bold: boolean, italic: boolean) => {
    if (node.nodeType === Node.TEXT_NODE) { append(node.textContent ?? "", bold, italic); return; }
    if (!(node instanceof Element)) return;
    if (node.hasAttribute("data-caret-sentinel")) return;
    const id = node.getAttribute("data-evidence-id");
    if (id) { const original = known.get(id); if (!original) throw new Error("不明な根拠の貼り付けを拒否しました"); nodes.push({ ...original }); return; }
    if (["SCRIPT", "STYLE", "IFRAME", "OBJECT"].includes(node.tagName)) return;
    if (node.tagName === "BR") { append("\n", bold, italic); return; }
    const last = nodes.at(-1);
    if (["DIV", "P"].includes(node.tagName) && last && !(last.type === "text" && last.text.endsWith("\n"))) append("\n", bold, italic);
    for (const child of Array.from(node.childNodes)) walk(child, bold || node.tagName === "STRONG" || node.tagName === "B", italic || node.tagName === "EM" || node.tagName === "I");
  };
  for (const child of Array.from(root.childNodes)) walk(child, false, false);
  return compactInline(nodes);
}
function terminalCaret(root: HTMLElement) {
  const sentinels = Array.from(root.querySelectorAll("[data-caret-sentinel]"));
  // A terminal newline needs a visible empty line in contentEditable. This
  // presentation-only BR must never become an extra newline in document JSON.
  const last = Array.from(root.childNodes).reverse().find((node) => !!node.textContent);
  if (last && !(last instanceof Element && last.hasAttribute("data-evidence-id")) && last.textContent?.endsWith("\n")) {
    if (sentinels.length === 1 && root.lastChild === sentinels[0]) return;
    for (const node of sentinels) node.remove();
    const sentinel = document.createElement("br"); sentinel.dataset.caretSentinel = "true"; root.append(sentinel);
  } else for (const node of sentinels) node.remove();
}
export function MinutesRichField({ value, label, blockIndex, addressKey, disabled, evidenceOnly = false, chipLabel, onChange, onSelection, onFailure, onCompositionChange }: {
  value: Inline[]; label: string; blockIndex: number; addressKey: string; disabled: boolean; evidenceOnly?: boolean;
  chipLabel: (id: string) => string;
  onChange: (value: Inline[]) => void; onSelection: (range: { start: number; end: number }) => void; onFailure: (cause: unknown) => void; onCompositionChange: (active: boolean) => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const composing = useRef(false);
  const known = useRef(new Map<string, Inline>());
  for (const node of value) if (node.type === "evidence") known.current.set(node.evidenceLinkId, node);
  const capture = () => { if (root.current) { const range = position(root.current); if (range) onSelection(range); } };
  const changed = () => {
    if (!root.current || composing.current) return;
    try { const nodes = read(root.current, known.current); onChange(nodes); capture(); } catch (cause) { onFailure(cause); }
  };
  const insertText = (value: string) => {
    const field = root.current; const selection = window.getSelection(); if (!field || !selection?.rangeCount || !position(field)) return;
    const range = selection.getRangeAt(0); range.deleteContents(); const text = document.createTextNode(value); range.insertNode(text); terminalCaret(field); range.setStart(text, value.length); range.collapse(true); selection.removeAllRanges(); selection.addRange(range); changed();
  };
  useLayoutEffect(() => {
    const field = root.current; if (!field) return;
    // React's synthetic beforeInput may originate from legacy textInput events.
    // Use the native InputEvent so inputType is available on both WebKit/Chrome.
    const before = (event: InputEvent) => {
      if (disabled) { event.preventDefault(); return; }
      if (evidenceOnly && event.inputType.startsWith("insert")) event.preventDefault();
      else if (["insertParagraph", "insertLineBreak"].includes(event.inputType)) { event.preventDefault(); insertText("\n"); }
    };
    field.addEventListener("beforeinput", before);
    return () => field.removeEventListener("beforeinput", before);
  });
  useLayoutEffect(() => {
    const field = root.current; if (!field || composing.current) return;
    const selection = position(field);
    try {
      if (JSON.stringify(read(field, known.current)) === JSON.stringify(compactInline(value))) {
        // Verification changes labels but not the typed inline nodes. Refresh
        // only the atomic chips; keep text, input composition and caret intact.
        let updated = false;
        for (const chip of field.querySelectorAll<HTMLElement>("[data-evidence-id]")) {
          const id = chip.dataset.evidenceId!;
          const label = chipLabel(id);
          if (chip.textContent !== label) { chip.textContent = label; updated = true; }
        }
        terminalCaret(field);
        if (updated && selection) restore(field, selection.start, selection.end);
        return;
      }
    } catch { /* Rebuild from trusted typed nodes. */ }
    const fragment = document.createDocumentFragment();
    for (const node of value) {
      if (node.type === "evidence") {
        const chip = document.createElement("span"); chip.contentEditable = "false"; chip.className = "evidence-chip";
        chip.dataset.evidenceId = node.evidenceLinkId; chip.textContent = chipLabel(node.evidenceLinkId); chip.title = "固定された根拠（文書下のプレビューから確認できます）";
        fragment.append(chip);
      } else {
        let element: Node = document.createTextNode(node.text);
        if (node.bold) { const wrapper = document.createElement("strong"); wrapper.append(element); element = wrapper; }
        if (node.italic) { const wrapper = document.createElement("em"); wrapper.append(element); element = wrapper; }
        fragment.append(element);
      }
    }
    field.replaceChildren(fragment);
    terminalCaret(field);
    if (selection) restore(field, selection.start, selection.end);
  }, [value, chipLabel]);
  return <div ref={root} className="minutes-rich-field" data-block={blockIndex} data-inline-address={addressKey} data-placeholder={evidenceOnly ? "根拠なし（手動追加）" : "入力してください"} contentEditable={!disabled} suppressContentEditableWarning role="textbox" aria-label={label} aria-multiline="true" aria-readonly={disabled} tabIndex={0}
    onFocus={capture} onKeyUp={capture} onMouseUp={capture} onSelect={capture}
    onCompositionStart={() => { composing.current = true; onCompositionChange(true); }} onCompositionEnd={() => { composing.current = false; changed(); onCompositionChange(false); }} onInput={changed}
    onPaste={(event) => {
      event.preventDefault(); if (disabled || evidenceOnly) return;
      const value = event.clipboardData.getData("text/plain").replace(/\r\n?/g, "\n");
      insertText(value);
    }} onDrop={(e) => e.preventDefault()} />;
}
