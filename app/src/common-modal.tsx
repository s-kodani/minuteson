import { useLayoutEffect, useRef, useSyncExternalStore, type ComponentPropsWithoutRef, type RefObject } from "react";
import { outsideDialog } from "./modal-state";
import { closeInfoPopovers } from "./info-popover";
import { confirmations } from "./confirmation-service";
import { ConfirmationBody } from "./confirmation-body";

type Props = Omit<ComponentPropsWithoutRef<"dialog">, "open" | "ref" | "onCancel" | "onClose" | "onClick" | "onPointerDown" | "onKeyDown"> & {
  ref?: RefObject<HTMLDialogElement | null>;
  open: boolean;
  blocked?: boolean;
  onDismiss: () => void | Promise<void>;
  restoreTarget?: () => HTMLElement | null;
  restoreReady?: boolean;
  dismissLabel?: "キャンセル" | "閉じる";
  dismissText?: string;
  externalContent?: boolean;
};

// Native top-layer dialogs supply an inert background. Explicit Tab wrapping
// keeps focus inside even when a browser would cycle through its own chrome.
// Let showModal/close expose actual modality; closed and queued dialogs must
// not advertise an independent, permanent ARIA modal state to WebKit AX.
// Controllers retain all command, identity, consent and unknown-result guards.
export function CommonModal({ ref: suppliedRef, open, blocked = false, onDismiss, restoreTarget, restoreReady = true, dismissLabel = "キャンセル", dismissText, externalContent = false, className = "", children, ...attributes }: Props) {
  const ownRef = useRef<HTMLDialogElement>(null);
  const ref = suppliedRef ?? ownRef;
  const confirmation = useSyncExternalStore(confirmations.subscribe, confirmations.getSnapshot);
  const owned = confirmation?.parent && confirmation.parent === ref.current ? confirmation : null;
  const effectiveBlocked = owned ? owned.phase === "running" : blocked;
  const label = owned?.phase === "failed" ? "閉じる" : dismissLabel;
  const suspendedOpener = useRef<HTMLElement | null>(null);
  const suspended = useRef(false);
  const suspendedScroll = useRef(0);
  const latest = useRef({ open, blocked, onDismiss, restoreTarget, restoreReady });
  latest.current = { open, blocked: effectiveBlocked, onDismiss, restoreTarget, restoreReady };
  const opener = useRef<HTMLElement | null>(null);
  const fallback = useRef<HTMLElement | null>(null);
  const restorePending = useRef(false);
  const downOutside = useRef(false);
  const restoring = useRef(false);
  const initialFocusPending = useRef(false);
  const restore = () => {
    if (restoring.current || !restorePending.current || latest.current.open || !latest.current.restoreReady) return;
    restoring.current = true;
    queueMicrotask(() => {
      restoring.current = false;
      if (!restorePending.current || latest.current.open || !latest.current.restoreReady || document.querySelector("dialog[open]")) return;
      const candidate = latest.current.restoreTarget?.() ?? opener.current;
      const usable = (target: HTMLElement | null): target is HTMLElement => !!target?.isConnected && !target.matches(":disabled") && !!target.getClientRects().length && !target.closest("[hidden], dialog:not([open])");
      const target = usable(candidate) ? candidate : usable(fallback.current) ? fallback.current : null;
      if (target) {
        if (target === fallback.current) target.tabIndex = -1;
        target.focus({ preventScroll: true }); restorePending.current = false;
      }
    });
  };
  const restoreSuspended = () => {
    const element = ref.current;
    if (!suspended.current || !element?.open || confirmations.getSnapshot()?.parent === element || document.querySelector("dialog[open]") !== element) return;
    const target = suspendedOpener.current;
    if (target?.isConnected && target.matches(":disabled")) return;
    suspended.current = false;
    element.scrollTop = suspendedScroll.current;
    (target?.isConnected && target.getClientRects().length ? target : element.querySelector<HTMLElement>("[data-dialog-cancel]"))?.focus({ preventScroll: true });
  };
  useLayoutEffect(() => {
    const element = ref.current;
    if (!open || !element) { restore(); return; }
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const panel = (latest.current.restoreTarget?.() ?? opener.current)?.closest("section");
    fallback.current = panel?.querySelector<HTMLElement>("h2") ?? panel?.querySelector<HTMLElement>("h3") ?? null;
    restorePending.current = false;
    const attempt = () => {
      if (!latest.current.open || !element.isConnected || element.open || document.querySelector("dialog[open]")) return;
      closeInfoPopovers();
      element.showModal();
      const cancel = element.querySelector<HTMLElement>("[data-dialog-cancel]:not(:disabled)");
      const heading = element.querySelector<HTMLElement>("h2, h3");
      initialFocusPending.current = !cancel;
      if (!cancel && heading) heading.tabIndex = -1;
      (cancel ?? heading ?? element).focus({ preventScroll: true });
      element.scrollTop = 0;
    };
    document.addEventListener("close", attempt, true);
    attempt();
    return () => {
      document.removeEventListener("close", attempt, true);
      if (confirmations.getSnapshot()?.parent === element) {
        const target = latest.current.restoreTarget?.() ?? opener.current;
        confirmations.detachParent(element, target?.isConnected ? target : fallback.current);
      }
      if (element.open) { restorePending.current = true; element.close(); }
      // Conditional reviews can still restore a surviving target on unmount.
      queueMicrotask(() => {
        if (!element.isConnected) { latest.current.open = false; restore(); }
      });
    };
  }, [open, ref]);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!open || blocked || !initialFocusPending.current || !element?.open) return;
    const active = document.activeElement;
    if (active === element || active === element.querySelector("h2, h3")) {
      element.querySelector<HTMLElement>("[data-dialog-cancel]:not(:disabled)")?.focus({ preventScroll: true });
      element.scrollTop = 0;
    }
    initialFocusPending.current = false;
  }, [open, blocked, ref]);
  useLayoutEffect(restore, [open, restoreReady]);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element?.open) { if (!open) suspended.current = false; return; }
    if (owned) {
      if (!suspended.current) { suspended.current = true; suspendedOpener.current = owned.opener; suspendedScroll.current = owned.scrollTop; closeInfoPopovers(); element.scrollTop = 0; element.querySelector<HTMLElement>("[data-dialog-cancel]")?.focus({ preventScroll: true }); }
    } else if (suspended.current) {
      const observer = new MutationObserver(restoreSuspended);
      observer.observe(element, { subtree: true, attributes: true, attributeFilter: ["disabled"], childList: true });
      queueMicrotask(restoreSuspended);
      return () => observer.disconnect();
    }
  }, [owned?.id, blocked, ref, open]);
  const dismiss = () => { if (!latest.current.blocked) { if (owned) confirmations.cancel(owned.id); else void latest.current.onDismiss(); } };
  const keyDown = (event: Pick<KeyboardEvent,"key" | "shiftKey" | "target" | "preventDefault" | "stopPropagation" | "defaultPrevented" | "isComposing" | "keyCode" | "repeat">) => {
    const element = ref.current;
    if (!element || event.defaultPrevented || externalContent && (event.isComposing || event.keyCode === 229)) return;
    if (externalContent && event.repeat && event.key === "Escape") { event.preventDefault(); return; }
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); dismiss(); return; }
    if (event.key === "Enter" && (event.target === element || (event.target instanceof HTMLElement && event.target.matches("h2, h3")))) event.preventDefault();
    if (event.key !== "Tab") return;
    const fields = Array.from(element.querySelectorAll<HTMLElement>('button, input, select, textarea, a[href], summary, [tabindex], [contenteditable="true"]'))
      .filter(field => field.tabIndex >= 0 && !field.matches(":disabled") && !field.closest("[hidden], [inert]") && field.getClientRects().length)
      .sort((a, b) => (a.tabIndex > 0 ? a.tabIndex : Number.MAX_SAFE_INTEGER) - (b.tabIndex > 0 ? b.tabIndex : Number.MAX_SAFE_INTEGER));
    const first = fields[0], last = fields.at(-1);
    if (!first) { event.preventDefault(); element.querySelector<HTMLElement>("h2, h3")?.focus({ preventScroll: true }); }
    else if (event.shiftKey && (document.activeElement === first || !fields.includes(document.activeElement as HTMLElement))) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && (document.activeElement === last || !fields.includes(document.activeElement as HTMLElement))) { event.preventDefault(); first.focus(); }
  };
  useLayoutEffect(() => {
    // Portals bubble React events through their owner, not their DOM parent.
    // A reparented workspace must trap keys at its native dialog boundary.
    const element = ref.current;
    if (!externalContent || !element) return;
    element.addEventListener("keydown",keyDown);
    return () => element.removeEventListener("keydown",keyDown);
  }, [externalContent,ref,owned?.id,owned?.phase,effectiveBlocked]);
  return <dialog {...attributes} ref={ref} data-common-modal aria-labelledby={owned ? `${owned.id}-title` : attributes["aria-labelledby"]} className={`common-modal ${className}`}
    onCancel={event => { event.preventDefault(); dismiss(); }}
    onPointerDown={event => { downOutside.current = event.target === event.currentTarget && outsideDialog(event.clientX, event.clientY, event.currentTarget.getBoundingClientRect()); }}
    onClick={event => {
      const outside = event.target === event.currentTarget && outsideDialog(event.clientX, event.clientY, event.currentTarget.getBoundingClientRect());
      if (downOutside.current && outside) dismiss();
      downOutside.current = false;
    }}
    onKeyDown={externalContent ? undefined : event => {
      // Repeated native Escape requests may become non-cancelable. Handle the
      // key here so declining a nested choice cannot close its parent or bypass
      // the in-flight guard. Platform cancel events still use the same handler.
      keyDown({ key:event.key,shiftKey:event.shiftKey,target:event.target,defaultPrevented:event.defaultPrevented,repeat:event.repeat,isComposing:event.nativeEvent.isComposing,keyCode:event.nativeEvent.keyCode,preventDefault:()=>event.preventDefault(),stopPropagation:()=>event.stopPropagation() });
    }}>
    <div className="common-modal-dismiss-row"><button type="button" className="small-secondary" aria-label={dismissText ?? `確認を${label}`} data-dialog-cancel disabled={effectiveBlocked} onClick={dismiss}>{dismissText ?? label}</button></div>
    {owned && <ConfirmationBody value={owned} />}
    <div className="common-modal-body" hidden={!!owned}>{children}</div>
  </dialog>;
}
