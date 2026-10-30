export type InfoRect = { left: number; top: number; right: number; bottom: number };
export type InfoViewport = { left: number; top: number; width: number; height: number };

// Use the visible viewport, not vw (which includes classic scrollbars).
export function infoPosition(anchor: InfoRect, viewport: InfoViewport, contentHeight: number, preferredWidth = 320) {
  const margin = 8, gap = 8;
  const width = Math.min(preferredWidth, Math.max(1, viewport.width - margin * 2));
  const leftEdge = viewport.left + margin, rightEdge = viewport.left + viewport.width - margin;
  const topEdge = viewport.top + margin, bottomEdge = viewport.top + viewport.height - margin;
  const below = Math.max(0, bottomEdge - anchor.bottom - gap);
  const above = Math.max(0, anchor.top - gap - topEdge);
  const side = contentHeight <= below || below >= above ? "below" : "above";
  const maxHeight = Math.max(1, Math.min(viewport.height - margin * 2, side === "below" ? below : above));
  const height = Math.min(contentHeight, maxHeight);
  const preferredLeft = anchor.left + width <= rightEdge ? anchor.left : anchor.right - width;
  const left = Math.max(leftEdge, Math.min(preferredLeft, rightEdge - width));
  const top = Math.max(topEdge, Math.min(side === "below" ? anchor.bottom + gap : anchor.top - gap - height, bottomEdge - height));
  return { left, top, width, maxHeight, side };
}
