import { useEffect, useId, useImperativeHandle, useLayoutEffect, useRef, useState, type Ref } from "react";
import { CommonModal } from "./common-modal";

export type WorkspacePanelHandle = { reveal: () => void; dismiss: (target?: () => HTMLElement | null) => void };

// Reparent one portal mount rather than render a second editor. Inputs, their
// selection, scroll position and the controller behind them survive resizing.
export function WorkspacePanel({ content, label, active = true, side: placement = "left", handle }: { content: HTMLElement; label: string; active?: boolean; side?: "left" | "right"; handle?: Ref<WorkspacePanelHandle> }) {
  const query = "(max-width: 1119px)";
  const [narrow, setNarrow] = useState(() => window.matchMedia(query).matches);
  const [open, setOpen] = useState(false);
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const side = useRef<HTMLDivElement>(null), drawer = useRef<HTMLDivElement>(null);
  const focus = useRef<HTMLElement | null>(null);
  const restore = useRef<(() => HTMLElement | null) | null>(null);
  const reveal = () => { restore.current = null; if (narrow && active) setOpen(true); };
  const dismiss = (target?: () => HTMLElement | null) => { restore.current = target ?? null; setOpen(false); };
  useImperativeHandle(handle, () => ({ reveal, dismiss }), [narrow,active]);
  const currentActive = useRef(active); currentActive.current = active;
  useEffect(() => {
    const media = window.matchMedia(query);
    const change = () => {
      restore.current = null;
      const target = document.activeElement;
      focus.current = target instanceof HTMLElement && content.contains(target) ? target : null;
      setNarrow(media.matches);
      setOpen(media.matches && currentActive.current && !!focus.current);
    };
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, [content]);
  useLayoutEffect(() => {
    const destination = narrow ? drawer.current : side.current;
    if (destination && content.parentElement !== destination) destination.append(content);
    const target = focus.current;
    if (active && (!narrow || open) && target?.isConnected) {
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
  }, [content, narrow, open, active]);
  useLayoutEffect(() => { if (!active) { setOpen(false); focus.current = null; } }, [active]);
  useLayoutEffect(() => () => content.remove(), [content]);
  return <>
    <aside className="workspace-panel-shell" aria-label={label} data-narrow={narrow} data-side={placement}>
      <button ref={trigger} type="button" className="small-secondary workspace-drawer-trigger" hidden={!narrow}
        aria-haspopup="dialog" aria-expanded={narrow && open} aria-controls={id} onClick={reveal}>{label}を開く</button>
      <div ref={side} className="workspace-panel-side-host" hidden={narrow} />
    </aside>
    <CommonModal id={id} open={active && narrow && open} externalContent onDismiss={() => dismiss()} restoreTarget={() => restore.current?.() ?? trigger.current}
      dismissLabel="閉じる" dismissText={`${label}を閉じる`} aria-label={label} className={`workspace-drawer workspace-drawer-${placement}`}>
      <div ref={drawer} className="workspace-panel-drawer-host" />
    </CommonModal>
  </>;
}
