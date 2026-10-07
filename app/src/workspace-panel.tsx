import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { CommonModal } from "./common-modal";

// Reparent one portal mount rather than render a second editor. Inputs, their
// selection, scroll position and the controller behind them survive resizing.
export function WorkspacePanel({ content, label, active = true }: { content: HTMLElement; label: string; active?: boolean }) {
  const query = "(max-width: 1119px)";
  const [narrow, setNarrow] = useState(() => window.matchMedia(query).matches);
  const [open, setOpen] = useState(false);
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const side = useRef<HTMLDivElement>(null), drawer = useRef<HTMLDivElement>(null);
  const focus = useRef<HTMLElement | null>(null);
  const currentActive = useRef(active); currentActive.current = active;
  useEffect(() => {
    const media = window.matchMedia(query);
    const change = () => {
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
        if (target.getClientRects().length && !target.matches(":disabled") && !target.closest("[hidden]")) target.focus({ preventScroll: true });
        focus.current = null;
      });
      return () => cancelAnimationFrame(frame);
    }
  }, [content, narrow, open, active]);
  useLayoutEffect(() => { if (!active) { setOpen(false); focus.current = null; } }, [active]);
  useLayoutEffect(() => () => content.remove(), [content]);
  return <>
    <aside className="workspace-panel-shell" aria-label={label} data-narrow={narrow}>
      <button ref={trigger} type="button" className="small-secondary workspace-drawer-trigger" hidden={!narrow}
        aria-haspopup="dialog" aria-expanded={narrow && open} aria-controls={id} onClick={() => setOpen(true)}>{label}を開く</button>
      <div ref={side} className="workspace-panel-side-host" hidden={narrow} />
    </aside>
    <CommonModal id={id} open={active && narrow && open} externalContent onDismiss={() => setOpen(false)} restoreTarget={() => trigger.current}
      dismissLabel="閉じる" dismissText={`${label}を閉じる`} aria-label={label} className="workspace-drawer">
      <div ref={drawer} className="workspace-panel-drawer-host" />
    </CommonModal>
  </>;
}
