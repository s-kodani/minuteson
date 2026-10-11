import { createContext, useContext, useId, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { CommonModal } from "./common-modal";
import { SettingsModalState, settingsExitTarget, type SettingsView } from "./settings-modal-state";
import { topOpenModal } from "./modal-state";

type Scope = { state: SettingsModalState; id: string; body: HTMLDivElement | null; replacement: HTMLDivElement | null; dialog: RefObject<HTMLDialogElement | null> };
const Context = createContext<Scope | null>(null);
export const useSettingsModalScope = () => useContext(Context);

// Keep a stable native modal for sibling settings. macOS WebKit can lose AX
// when successive sibling showModal() boundaries compete. Fields are portaled
// once into this owner, not reparented at opening or unmounted on dismissal.
export function SettingsModalScope({ children }: { children: ReactNode }) {
  const [state] = useState(() => new SettingsModalState());
  const id = useId();
  const [body, setBody] = useState<HTMLDivElement | null>(null);
  const [replacement, setReplacement] = useState<HTMLDivElement | null>(null);
  const snapshot = useSyncExternalStore(state.subscribe, state.getSnapshot);
  const previous = useRef<SettingsView | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const exitEntry = useRef<HTMLElement | null>(null);
  exitEntry.current = settingsExitTarget(snapshot.active, exitEntry.current, target => !!dialog.current?.contains(target));
  const retained = useRef<SettingsView | null>(null);
  if (snapshot.active) retained.current = snapshot.active;
  const owner = snapshot.active ?? (retained.current ? state.getView(retained.current.id) ?? retained.current : null);
  const context = useMemo(() => ({ state, id, body, replacement, dialog }), [state, id, body, replacement]);
  useLayoutEffect(() => {
    const current = snapshot.active;
    const enteringReplacement = !!current?.replacement && previous.current?.replacement?.key !== current.replacement.key;
    // An async review may open with every action disabled. Its first focus
    // attempt can precede the portaled heading; retry when that review becomes
    // ready, rather than only when its identity changes.
    const replacementReady = !!current?.replacement && previous.current?.id === current.id
      && previous.current.replacement?.key === current.replacement.key
      && !!previous.current.replacement.blocked && !current.replacement.blocked;
    if (current && previous.current && (previous.current.id !== current.id || enteringReplacement || replacementReady) && dialog.current?.open) {
      const replacementReturn = previous.current?.replacement?.parentKey === current.replacement?.key
        ? previous.current?.replacement?.restoreTarget : undefined;
      const target = snapshot.returning?.() ?? replacementReturn?.();
      const usable = target?.isConnected && dialog.current.contains(target) && !!target.getClientRects().length && !target.matches(":disabled") && !target.closest("[hidden], [inert]");
      dialog.current.scrollTop = 0;
      const heading = dialog.current.querySelector<HTMLElement>(".common-modal-replacement h2, .common-modal-replacement h3");
      const destination = usable ? target : dialog.current.querySelector<HTMLElement>("[data-dialog-cancel]:not(:disabled)") ?? heading;
      if (destination === heading && heading) heading.tabIndex = -1;
      const active = document.activeElement;
      const validFocus = active instanceof HTMLElement && dialog.current.contains(active) && active.getClientRects().length
        && !active.matches(":disabled") && !active.closest("[hidden], [inert]");
      if (!replacementReady || !validFocus) destination?.focus({ preventScroll: true });
      // A portaled view can hide its former focus after this layout effect.
      // Repair only a lost focus after that commit; do not steal a chosen field,
      // an owned confirmation, a newer view or a separate native child dialog.
      const element = dialog.current;
      requestAnimationFrame(() => {
        const live = state.getSnapshot().active;
        if (!element.open || topOpenModal() !== element || live?.id !== current.id || live.replacement?.key !== current.replacement?.key) return;
        if (current.replacement && element.getAttribute("aria-labelledby") !== current.replacement.labelledBy) return;
        const active = document.activeElement;
        if (active !== document.body && active !== element && active instanceof HTMLElement && active.isConnected && active.getClientRects().length && !active.matches(":disabled") && !active.closest("[hidden], [inert]")) return;
        const cancel = element.querySelector<HTMLElement>("[data-dialog-cancel]:not(:disabled)");
        (cancel ?? destination)?.focus({ preventScroll: true });
      });
    }
    previous.current = current;
  }, [snapshot]);
  return <Context.Provider value={context}>{children}{createPortal(
    <div className={`options-modal ${owner?.className ?? ""}`} data-options-modal-host style={{ display: "contents" }}>
      <CommonModal ref={dialog} id={id} open={!!snapshot.active} nested externalContent data-settings-modal aria-label={owner?.title} aria-labelledby={owner?.labelledBy} className={owner?.dialogClassName ?? "options-dialog"} dismissLabel={owner?.dismissLabel ?? "閉じる"} blocked={owner?.blocked} restoreReady={owner?.restoreReady}
        onDismiss={() => owner?.dismiss()} restoreTarget={() => exitEntry.current ?? owner?.entry() ?? null} replacement={owner?.replacement}
        persistentReplacement={<div ref={setReplacement} className="settings-modal-slot" />}>
        <div ref={setBody} className="settings-modal-slot" />
      </CommonModal>
    </div>, document.body)}</Context.Provider>;
}
