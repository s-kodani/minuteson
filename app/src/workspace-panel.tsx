import { useEffect, useId, useImperativeHandle, useLayoutEffect, useRef, useState, type Ref } from "react";
import { CommonModal } from "./common-modal";
import { IconButton } from "./action-icon";

export type WorkspacePanelHandle = { reveal: (opener?: HTMLElement | null) => void; dismiss: (target?: () => HTMLElement | null) => void };

// Reparent one portal mount rather than render a second editor. Inputs, their
// selection, scroll position and the controller behind them survive resizing.
export function WorkspacePanel({ content, label, active = true, side: placement = "left", handle, collapsible = false, query = "(max-width: 1119px)" }: { content: HTMLElement; label: string; active?: boolean; side?: "left" | "right"; handle?: Ref<WorkspacePanelHandle>; collapsible?: boolean; query?: string }) {
  const [narrow, setNarrow] = useState(() => window.matchMedia(query).matches);
  const [open, setOpen] = useState(false);
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const side = useRef<HTMLDivElement>(null), drawer = useRef<HTMLDivElement>(null);
  const focus = useRef<HTMLElement | null>(null);
  const restore = useRef<(() => HTMLElement | null) | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const previousOpen = useRef(false);
  const reveal = (from?: HTMLElement | null) => {
    if (!active) return;
    if (!open) opener.current = from ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    restore.current = null; if (narrow || collapsible) setOpen(true);
  };
  const dismiss = (target?: () => HTMLElement | null) => { restore.current = target ?? null; setOpen(false); };
  useImperativeHandle(handle, () => ({ reveal, dismiss }), [narrow,active,collapsible,open]);
  useLayoutEffect(() => {
    const closed = previousOpen.current && !open;
    previousOpen.current = open;
    if (!closed || narrow || !collapsible || !active) return;
    const frame = requestAnimationFrame(() => {
      if (!currentActive.current || document.querySelector("dialog[open]")) return;
      const candidate = restore.current?.() ?? opener.current;
      const usable = (target: HTMLElement | null) => target?.isConnected && !!target.getClientRects().length && !target.matches(":disabled") && !target.closest("[hidden]");
      (usable(candidate) ? candidate : trigger.current)?.focus({ preventScroll:true });
    });
    return () => cancelAnimationFrame(frame);
  }, [open,narrow,collapsible,active]);
  const currentActive = useRef(active); currentActive.current = active;
  useEffect(() => {
    const media = window.matchMedia(query);
    const change = () => {
      restore.current = null;
      const target = document.activeElement;
      focus.current = target instanceof HTMLElement && content.contains(target) ? target : null;
      setNarrow(media.matches);
      setOpen(previous => currentActive.current && (collapsible ? previous || !!focus.current : media.matches && !!focus.current));
    };
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, [content,collapsible,query]);
  useLayoutEffect(() => {
    const destination = narrow ? drawer.current : side.current;
    if (destination && content.parentElement !== destination) destination.append(content);
    const target = focus.current;
    if (active && ((!narrow && !collapsible) || open) && target?.isConnected) {
      const frame = requestAnimationFrame(() => {
        if (target.getClientRects().length && !target.matches(":disabled") && !target.closest("[hidden]")) {
          target.focus({ preventScroll: true });
          // A hidden host can reset its child's scroll while switching layout.
          // Reveal the restored control in this pane, without scrolling the
          // workspace, fixed header or an unrelated document bookmark.
          const pane = content.getBoundingClientRect(), control = target.getBoundingClientRect();
          if (control.top < pane.top) content.scrollTop += Math.floor(control.top - pane.top);
          else if (control.bottom > pane.bottom) content.scrollTop += Math.ceil(control.bottom - pane.bottom);
        }
        focus.current = null;
      });
      return () => cancelAnimationFrame(frame);
    }
  }, [content, narrow, open, active,collapsible]);
  useLayoutEffect(() => { if (!active) { setOpen(false); focus.current = null; } }, [active]);
  useLayoutEffect(() => () => content.remove(), [content]);
  return <>
    <aside className="workspace-panel-shell" aria-label={label} data-narrow={narrow} data-side={placement} data-collapsible={collapsible} data-open={open}>
      <button ref={trigger} type="button" className="small-secondary workspace-drawer-trigger" hidden={!narrow && (!collapsible || open)}
        aria-haspopup={narrow ? "dialog" : undefined} aria-expanded={open} aria-controls={narrow ? id : `${id}-side`} onClick={() => reveal()}>{label}を開く</button>
      <div id={`${id}-side`} className="workspace-panel-side-host" hidden={narrow || collapsible && !open}>
        {collapsible && <IconButton className="workspace-panel-close" label={`${label}を閉じる`} icon="close" onClick={() => dismiss()} />}
        <div ref={side} className="workspace-panel-side-content" />
      </div>
    </aside>
    <CommonModal id={id} open={active && narrow && open} externalContent onDismiss={() => dismiss()} restoreTarget={() => restore.current?.() ?? opener.current ?? trigger.current}
      dismissLabel="閉じる" dismissText={`${label}を閉じる`} aria-label={label} className={`workspace-drawer workspace-drawer-${placement}`}>
      <div ref={drawer} className="workspace-panel-drawer-host" />
    </CommonModal>
  </>;
}
