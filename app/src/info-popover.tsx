import { useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { infoPosition } from "./info-position";
import { appShortcut } from "./keyboard";
import { topOpenModal } from "./modal-state";
import { ActionIcon } from "./action-icon";

let activeClose: ((restore: boolean) => void) | null = null;
const dismissals = new Set<() => void>();
// A confirmation takes precedence; do not move focus away from its controls.
export function closeInfoPopovers() { dismissals.forEach(close => close()); }

export function InfoPopover({ label, children, details, triggerText, triggerLabel, popupClassName = "", maxWidth = 320, openOnFocus = false, lazy = false }: {
  label: string; children: ReactNode; details?: ReactNode; triggerText?: string; triggerLabel?: string;
  popupClassName?: string; maxWidth?: number; openOnFocus?: boolean; lazy?: boolean;
}) {
  const id = useId(), titleId = `${id}-title`, detailsId = `${id}-details`;
  const trigger = useRef<HTMLButtonElement>(null), popup = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const [mode, setMode] = useState<"hover" | "focus" | "pinned" | null>(null), [expanded, setExpanded] = useState(false);
  const open = mode !== null, currentMode = useRef(mode); currentMode.current = mode;
  const hoverTimer = useRef<number | undefined>(undefined), leaveTimer = useRef<number | undefined>(undefined);
  const clearTimers = () => { window.clearTimeout(hoverTimer.current); window.clearTimeout(leaveTimer.current); hoverTimer.current = leaveTimer.current = undefined; };
  const liveClose = useRef<(restore: boolean) => void>(() => {});
  const position = useRef<() => void>(() => {});
  const restoringFocus = useRef(false);
  liveClose.current = restore => {
    clearTimers(); currentMode.current = null; setMode(null);
    const button = trigger.current;
    if (restore && button?.isConnected && !button.matches(":disabled") && button.getClientRects().length && !button.closest("[hidden], [inert], dialog:not([open])")) {
      restoringFocus.current = true; button.focus({ preventScroll: true }); restoringFocus.current = false;
    }
  };
  useLayoutEffect(() => {
    const dismiss = () => liveClose.current(false);
    dismissals.add(dismiss);
    return () => { dismissals.delete(dismiss); clearTimers(); };
  }, []);
  const enter = (event: React.PointerEvent) => {
    if (event.pointerType !== "mouse") return;
    window.clearTimeout(leaveTimer.current);
    if (currentMode.current || hoverTimer.current !== undefined) return;
    hoverTimer.current = window.setTimeout(() => {
      hoverTimer.current = undefined;
      const button = trigger.current, modal = topOpenModal();
      if (button?.isConnected && button.getClientRects().length && !button.closest("[hidden], [inert], dialog:not([open])") && (!modal || modal.contains(button))) setMode("hover");
    }, 200);
  };
  const leave = () => {
    window.clearTimeout(hoverTimer.current); hoverTimer.current = undefined;
    window.clearTimeout(leaveTimer.current);
    if (currentMode.current === "hover") leaveTimer.current = window.setTimeout(() => {
      if (currentMode.current === "hover" && !popup.current?.contains(document.activeElement)) liveClose.current(false);
    }, 150);
  };
  const toggle = () => {
    clearTimers();
    if (currentMode.current === "pinned") liveClose.current(true);
    else { currentMode.current = "pinned"; setMode("pinned"); if (open) closeButton.current?.focus({ preventScroll: true }); }
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
      element.style.width = `${Math.min(maxWidth, Math.max(1, view.width - 16))}px`;
      const height = element.scrollHeight + element.offsetHeight - element.clientHeight;
      const placement = infoPosition(rect, view, height, maxWidth);
      Object.assign(element.style, { left: `${placement.left}px`, top: `${placement.top}px`, maxHeight: `${placement.maxHeight}px` });
      element.dataset.side = placement.side;
    };
    position.current = place;
    place(); if (currentMode.current === "pinned") closeButton.current?.focus({ preventScroll: true });
    const escape = (event: KeyboardEvent) => {
      if (appShortcut(event) === "cancel") { event.preventDefault(); event.stopImmediatePropagation(); close(true); }
      else if (event.key === "Tab" && !event.isComposing && event.keyCode !== 229 && !event.altKey && !event.ctrlKey && !event.metaKey && element.contains(document.activeElement)) {
        // macOS WebKit can skip links during native Tab navigation. Preserve
        // document order inside this non-modal popup without trapping its ends.
        const targets = [...element.querySelectorAll<HTMLElement>("button:not(:disabled),a[href],input:not(:disabled),select:not(:disabled),textarea:not(:disabled),[tabindex]")]
          .filter(node => node.tabIndex >= 0 && node.getClientRects().length && !node.closest("[hidden],[inert]") && getComputedStyle(node).visibility === "visible");
        const at = targets.indexOf(document.activeElement as HTMLElement), next = at + (event.shiftKey ? -1 : 1);
        if (at >= 0 && next >= 0 && next < targets.length) { event.preventDefault(); event.stopImmediatePropagation(); targets[next].focus({ preventScroll:true }); }
      }
    };
    const outside = (event: MouseEvent) => {
      const path = event.composedPath();
      if (!path.includes(element) && !path.includes(button)) {
        // Switching to another info icon must not restore the old trigger.
        close(currentMode.current === "pinned" && !(event.target instanceof Element && event.target.closest(".info-trigger")));
      }
    };
    const focusChanged = () => {
      const modal = topOpenModal();
      if (modal && !modal.contains(button)) close(false);
      else if (currentMode.current === "focus" && document.activeElement !== button && !element.contains(document.activeElement)) close(false);
    };
    const resize = new ResizeObserver(place); resize.observe(element); resize.observe(button);
    const hidden = new MutationObserver(() => { focusChanged(); place(); });
    hidden.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["hidden", "open", "disabled", "inert", "class"] });
    focusChanged();
    document.addEventListener("keydown", escape, true);
    document.addEventListener("click", outside, true);
    document.addEventListener("focusin", focusChanged);
    document.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    window.visualViewport?.addEventListener("resize", place);
    window.visualViewport?.addEventListener("scroll", place);
    return () => {
      clearTimers();
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
  }, [open, maxWidth]);
  useLayoutEffect(() => { if (open) position.current(); });
  return <div className="info-shell">
    <button ref={trigger} type="button" className={`info-trigger${triggerText ? " small-secondary info-text-trigger" : ""}`} aria-label={triggerLabel ?? `${label}の説明を開く`} aria-expanded={open} aria-controls={id} aria-haspopup="dialog" onPointerEnter={enter} onPointerLeave={leave} onFocus={() => {
      if (openOnFocus && !restoringFocus.current && !currentMode.current) { clearTimers(); currentMode.current = "focus"; setMode("focus"); }
    }} onClick={toggle}>{triggerText ?? <span aria-hidden="true">ⓘ</span>}</button>
    {(!lazy || open) && <div ref={popup} id={id} popover="manual" role="dialog" aria-modal="false" aria-labelledby={titleId} className={`info-popover ${popupClassName}`} onPointerEnter={enter} onPointerLeave={leave} onFocusCapture={() => { if (currentMode.current === "hover") { clearTimers(); currentMode.current = "pinned"; setMode("pinned"); } }}>
      <div className="info-popover-head"><h3 id={titleId}>{label}</h3><button ref={closeButton} type="button" className="info-close icon-button" aria-label={`${label}の説明を閉じる`} title="説明を閉じる" onClick={() => liveClose.current(true)}><ActionIcon name="close" /></button></div>
      <div className="info-popover-content">{!lazy || open ? children : null}</div>
      {details && <><a href={`#${detailsId}`} aria-expanded={expanded} aria-controls={detailsId} onClick={event => { event.preventDefault(); setExpanded(value => !value); }}>{expanded ? "詳しい説明を閉じる" : "詳しい説明"}</a><div id={detailsId} className="info-details" hidden={!expanded}>{details}</div></>}
    </div>}
  </div>;
}
