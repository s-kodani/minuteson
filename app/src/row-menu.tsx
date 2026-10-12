import { useId, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { IconButton } from "./action-icon";
import { topOpenModal } from "./modal-state";
import "./row-menu.css";

let activeClose: (() => void) | null = null;
export function RowMenu({ label, disabled, children }: { label: string; disabled?: boolean; children: ReactNode }) {
  const id = useId(), trigger = useRef<HTMLButtonElement>(null), popup = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const close = (restore = false) => {
    setOpen(false);
    if (restore && trigger.current?.isConnected && !trigger.current.disabled && !topOpenModal()) trigger.current.focus({ preventScroll: true });
  };
  useLayoutEffect(() => {
    const menu = popup.current, button = trigger.current;
    if (!open || disabled || !menu || !button) { if (open && disabled) setOpen(false); return; }
    const dismiss = () => setOpen(false);
    activeClose?.(); activeClose = dismiss;
    menu.showPopover();
    const place = () => {
      if (topOpenModal() || !button.isConnected || !button.getClientRects().length || button.disabled || button.closest("[hidden],[inert]")) { dismiss(); return; }
      const rect = button.getBoundingClientRect(), view = window.visualViewport;
      const left = view?.offsetLeft ?? 0, top = view?.offsetTop ?? 0;
      const width = view?.width ?? innerWidth, height = view?.height ?? innerHeight;
      if (rect.bottom <= top || rect.top >= top + height || rect.right <= left || rect.left >= left + width) { dismiss(); return; }
      menu.style.width = `${Math.min(200, width - 16)}px`;
      menu.style.maxHeight = `${Math.max(1, height - 16)}px`;
      const bounds = menu.getBoundingClientRect();
      menu.style.left = `${Math.max(left + 8, Math.min(rect.right - bounds.width, left + width - bounds.width - 8))}px`;
      menu.style.top = `${Math.max(top + 8, Math.min(rect.bottom + 6 + bounds.height <= top + height - 8 ? rect.bottom + 6 : rect.top - bounds.height - 6, top + height - bounds.height - 8))}px`;
    };
    const items = () => [...menu.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')];
    const key = (event: KeyboardEvent) => {
      if (event.isComposing || event.altKey || event.metaKey || event.ctrlKey) return;
      if (event.key === "Escape") { event.preventDefault(); event.stopImmediatePropagation(); close(true); }
      else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
        event.preventDefault(); const buttons = items(), at = buttons.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : (at + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
        buttons[next]?.focus({ preventScroll: true });
      }
    };
    const outside = (event: Event) => { if (!event.composedPath().includes(menu) && !event.composedPath().includes(button)) dismiss(); };
    const resize = new ResizeObserver(place); resize.observe(menu); resize.observe(button);
    const modal = new MutationObserver(place); modal.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["open", "hidden", "disabled"] });
    place(); items()[0]?.focus({ preventScroll: true });
    document.addEventListener("keydown", key, true); document.addEventListener("pointerdown", outside, true); document.addEventListener("focusin", outside);
    document.addEventListener("scroll", place, true); window.addEventListener("resize", place);
    viewListen("addEventListener", place);
    return () => {
      if (activeClose === dismiss) activeClose = null;
      if (menu.matches(":popover-open")) menu.hidePopover();
      resize.disconnect(); modal.disconnect();
      document.removeEventListener("keydown", key, true); document.removeEventListener("pointerdown", outside, true); document.removeEventListener("focusin", outside);
      document.removeEventListener("scroll", place, true); window.removeEventListener("resize", place);
      viewListen("removeEventListener", place);
    };
  }, [open, disabled]);
  return <div className="meeting-row-actions">
    <IconButton ref={trigger} icon="more" label={label} className="small-secondary" disabled={disabled} aria-haspopup="menu" aria-expanded={open} aria-controls={id} onClick={() => open ? close(true) : setOpen(true)} />
    <div ref={popup} id={id} popover="manual" role="menu" aria-label={label} className="meeting-row-menu" onClick={event => {
      const button = event.target instanceof Element ? event.target.closest("button") : null;
      if (button && !button.disabled) close();
    }} data-row-menu-trigger={id}>{children}</div>
  </div>;
}
/** A confirmation returns to the stable ellipsis, not its now-hidden item. */
export function rowMenuReturnTarget(active: HTMLElement | null) {
  const id = active?.closest<HTMLElement>("[data-row-menu-trigger]")?.dataset.rowMenuTrigger;
  return id ? [...document.querySelectorAll<HTMLButtonElement>('.meeting-row-actions > button')].find(button => button.getAttribute("aria-controls") === id) ?? active : active;
}
function viewListen(method: "addEventListener" | "removeEventListener", listener: () => void) {
  window.visualViewport?.[method]("resize", listener);
  window.visualViewport?.[method]("scroll", listener);
}
