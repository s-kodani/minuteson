import { useId, useLayoutEffect, useRef, type ReactNode } from "react";
import { IconButton } from "./action-icon";
import { topOpenModal } from "./modal-state";

/** A non-modal history popup anchored to the bell, never a workspace dialog. */
export function NotificationPopover({ open, onOpenChange, children }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
}) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const close = (restore = false) => {
    onOpenChange(false);
    if (restore && trigger.current?.isConnected && !topOpenModal()) {
      trigger.current.focus({ preventScroll: true });
    }
  };

  useLayoutEffect(() => {
    const panel = popup.current, button = trigger.current;
    if (!open || !panel || !button) return;
    panel.showPopover();
    const visual = window.visualViewport;
    const place = () => {
      if (topOpenModal() || !button.isConnected || !button.getClientRects().length || button.closest("[hidden],[inert]")) {
        onOpenChange(false);
        return;
      }
      const rect = button.getBoundingClientRect();
      const left = visual?.offsetLeft ?? 0, top = visual?.offsetTop ?? 0;
      const width = Math.min(document.documentElement.clientWidth, visual?.width ?? innerWidth);
      const height = Math.min(document.documentElement.clientHeight, visual?.height ?? innerHeight);
      if (rect.bottom <= top || rect.top >= top + height || rect.right <= left || rect.left >= left + width) {
        onOpenChange(false);
        return;
      }
      const panelWidth = Math.min(440, width - 24);
      panel.style.width = `${panelWidth}px`;
      panel.style.maxHeight = `${Math.min(560, Math.max(1, top + height - rect.bottom - 20))}px`;
      panel.style.left = `${Math.max(left + 12, Math.min(rect.right - panelWidth, left + width - panelWidth - 12))}px`;
      panel.style.top = `${rect.bottom + 8}px`;
    };
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.isComposing) {
        event.preventDefault();
        event.stopImmediatePropagation();
        close(true);
      }
    };
    const outside = (event: PointerEvent) => {
      const path = event.composedPath();
      if (!path.includes(panel) && !path.includes(button)) onOpenChange(false);
    };
    const modal = new MutationObserver(() => { if (topOpenModal()) onOpenChange(false); });
    modal.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["open"] });
    const resize = new ResizeObserver(place);
    resize.observe(button);
    resize.observe(panel);
    place();
    closeButton.current?.focus({ preventScroll: true });
    document.addEventListener("keydown", keydown, true);
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    visual?.addEventListener("resize", place);
    visual?.addEventListener("scroll", place);
    return () => {
      if (panel.matches(":popover-open")) panel.hidePopover();
      modal.disconnect();
      resize.disconnect();
      document.removeEventListener("keydown", keydown, true);
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
      visual?.removeEventListener("resize", place);
      visual?.removeEventListener("scroll", place);
    };
  }, [open, onOpenChange]);

  return <div className="notification-anchor">
    <IconButton ref={trigger} label="通知一覧" icon="bell" className="header-notification-button" aria-haspopup="dialog" aria-expanded={open} aria-controls={id} onClick={() => open ? close(true) : onOpenChange(true)} />
    <div ref={popup} id={id} popover="manual" role="dialog" aria-modal="false" aria-labelledby={`${id}-title`} className="notifications-popover">
      <div className="notifications-popover-head"><h2 id={`${id}-title`}>通知一覧</h2><IconButton ref={closeButton} label="通知一覧を閉じる" icon="close" data-notification-close onClick={() => close(true)} /></div>
      {children}
    </div>
  </div>;
}
