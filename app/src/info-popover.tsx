import { useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { infoPosition } from "./info-position";
import { appShortcut } from "./keyboard";
import { topOpenModal } from "./modal-state";
import { ActionIcon } from "./action-icon";

let activeClose: ((restore: boolean) => void) | null = null;
// A confirmation takes precedence; do not move focus away from its controls.
export function closeInfoPopovers() { activeClose?.(false); }

export function InfoPopover({ label, children, details }: { label: string; children: ReactNode; details?: ReactNode }) {
  const id = useId(), titleId = `${id}-title`, detailsId = `${id}-details`;
  const trigger = useRef<HTMLButtonElement>(null), popup = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false), [expanded, setExpanded] = useState(false);
  const liveClose = useRef<(restore: boolean) => void>(() => {});
  const position = useRef<() => void>(() => {});
  liveClose.current = restore => {
    setOpen(false);
    const button = trigger.current;
    if (restore && button?.isConnected && !button.matches(":disabled") && button.getClientRects().length && !button.closest("[hidden], [inert], dialog:not([open])")) button.focus({ preventScroll: true });
  };
  useLayoutEffect(() => {
    const element = popup.current, button = trigger.current;
    if (!open || !element || !button) return;
    const close = (restore: boolean) => liveClose.current(restore);
    activeClose?.(false); activeClose = close;
    setExpanded(false);
    element.showPopover();
    const viewport = () => {
      const visual = window.visualViewport;
      return { left: visual?.offsetLeft ?? 0, top: visual?.offsetTop ?? 0,
        width: Math.min(document.documentElement.clientWidth, visual?.width ?? window.innerWidth),
        height: Math.min(document.documentElement.clientHeight, visual?.height ?? window.innerHeight) };
    };
    const place = () => {
      if (!button.isConnected || !button.getClientRects().length || button.matches(":disabled") || button.closest("[hidden], [inert], dialog:not([open])")) { close(false); return; }
      const rect = button.getBoundingClientRect(), view = viewport();
      if (rect.bottom <= view.top || rect.top >= view.top + view.height || rect.right <= view.left || rect.left >= view.left + view.width) { close(false); return; }
      element.style.width = `${Math.min(320, Math.max(1, view.width - 16))}px`;
      const height = element.scrollHeight + element.offsetHeight - element.clientHeight;
      const placement = infoPosition(rect, view, height);
      Object.assign(element.style, { left: `${placement.left}px`, top: `${placement.top}px`, maxHeight: `${placement.maxHeight}px` });
      element.dataset.side = placement.side;
    };
    position.current = place;
    place(); closeButton.current?.focus({ preventScroll: true });
    const escape = (event: KeyboardEvent) => {
      if (appShortcut(event) === "cancel") { event.preventDefault(); event.stopImmediatePropagation(); close(true); }
    };
    const outside = (event: MouseEvent) => {
      const path = event.composedPath();
      if (!path.includes(element) && !path.includes(button)) {
        // Switching to another info icon must not restore the old trigger.
        close(!(event.target instanceof Element && event.target.closest(".info-trigger")));
      }
    };
    const focusChanged = () => {
      const modal = topOpenModal();
      if (modal && !modal.contains(button)) close(false);
    };
    const resize = new ResizeObserver(place); resize.observe(element); resize.observe(button);
    const hidden = new MutationObserver(() => { focusChanged(); place(); });
    hidden.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["hidden", "open", "disabled"] });
    document.addEventListener("keydown", escape, true);
    document.addEventListener("click", outside, true);
    document.addEventListener("focusin", focusChanged);
    document.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    window.visualViewport?.addEventListener("resize", place);
    window.visualViewport?.addEventListener("scroll", place);
    return () => {
      if (activeClose === close) activeClose = null;
      if (element.matches(":popover-open")) element.hidePopover();
      resize.disconnect(); hidden.disconnect();
      document.removeEventListener("keydown", escape, true);
      document.removeEventListener("click", outside, true);
      document.removeEventListener("focusin", focusChanged);
      document.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
      window.visualViewport?.removeEventListener("resize", place);
      window.visualViewport?.removeEventListener("scroll", place);
    };
  }, [open]);
  useLayoutEffect(() => { if (open) position.current(); });
  return <div className="info-shell">
    <button ref={trigger} type="button" className="info-trigger" aria-label={`${label}の説明を開く`} aria-expanded={open} aria-controls={id} aria-haspopup="dialog" onClick={() => open ? liveClose.current(true) : setOpen(true)}><span aria-hidden="true">ⓘ</span></button>
    <div ref={popup} id={id} popover="manual" role="dialog" aria-modal="false" aria-labelledby={titleId} className="info-popover">
      <div className="info-popover-head"><h3 id={titleId}>{label}</h3><button ref={closeButton} type="button" className="info-close icon-button" aria-label={`${label}の説明を閉じる`} title="説明を閉じる" onClick={() => liveClose.current(true)}><ActionIcon name="close" /></button></div>
      <p>{children}</p>
      {details && <><a href={`#${detailsId}`} aria-expanded={expanded} aria-controls={detailsId} onClick={event => { event.preventDefault(); setExpanded(value => !value); }}>{expanded ? "詳しい説明を閉じる" : "詳しい説明"}</a><div id={detailsId} className="info-details" hidden={!expanded}>{details}</div></>}
    </div>
  </div>;
}
