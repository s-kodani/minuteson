import { fromMarkdown } from "mdast-util-from-markdown";
import type { Nodes } from "mdast";
import { markdownAudioInterval } from "./minutes-markdown-state.ts";

/** Replace parsed audio links in source order, leaving all other Markdown
 * bytes (including code examples) intact. Copying never rewrites the document. */
export function minutesCopyText(markdown: string): string {
  const tree = fromMarkdown(markdown);
  const definitions = new Map<string, Extract<Nodes, { type: "definition" }>>();
  const retainedReferences = new Set<string>();
  const convertedReferences = new Set<string>();
  const collect = (node: Nodes) => {
    if (node.type === "definition" && !definitions.has(node.identifier)) definitions.set(node.identifier, node);
    if (node.type === "imageReference") retainedReferences.add(node.identifier);
    if ("children" in node) node.children.forEach(collect);
  };
  collect(tree);
  const replacements: { start: number; end: number; text: string }[] = [];
  const visit = (node: Nodes) => {
    const url = node.type === "link" ? node.url : node.type === "linkReference" ? definitions.get(node.identifier)?.url : undefined;
    const interval = url ? markdownAudioInterval(url) : null;
    const start = node.position?.start.offset, end = node.position?.end.offset;
    if (interval && start !== undefined && end !== undefined) {
      const seconds = Math.floor(interval.startUs / 1_000_000);
      const time = [Math.floor(seconds / 3600), Math.floor(seconds % 3600 / 60), seconds % 60].map(value => String(value).padStart(2, "0")).join(":");
      replacements.push({ start, end, text: `(${time})` });
      if (node.type === "linkReference") convertedReferences.add(node.identifier);
    } else {
      if (node.type === "linkReference") retainedReferences.add(node.identifier);
      if ("children" in node) node.children.forEach(visit);
    }
  };
  visit(tree);
  for (const identifier of convertedReferences) {
    const definition = definitions.get(identifier)!;
    const start = definition.position?.start.offset, end = definition.position?.end.offset;
    if (!retainedReferences.has(identifier) && start !== undefined && end !== undefined) replacements.push({ start, end, text: "" });
  }
  let result = markdown;
  for (const replacement of replacements.sort((a, b) => b.start - a.start)) result = result.slice(0, replacement.start) + replacement.text + result.slice(replacement.end);
  return result;
}
