import { useLayoutEffect, useRef, useSyncExternalStore, type ComponentPropsWithoutRef, type ReactNode, type RefObject } from "react";
import { modalKeyGuard, modalOrder, outsideDialog, topOpenModal, visibleModalTarget } from "./modal-state";
import { closeInfoPopovers } from "./info-popover";
import { confirmations } from "./confirmation-service";
import { ConfirmationBody } from "./confirmation-body";
import { ActionIcon } from "./action-icon";
import { createPortal } from "react-dom";

// A review can replace its settings view inside the same native boundary.
// The original inputs remain mounted, and confirmation ownership is unchanged.
export type ModalReplacement = {
  key: string;
  // A child view returns to its persistent parent and actual entry point.
  parentKey?: string;
  labelledBy: string;
  content: ReactNode;
  className?: string;
  blocked?: boolean;
  onDismiss: () => void | Promise<void>;
  restoreTarget?: () => HTMLElement | null;
  dismissLabel?: "キャンセル" | "閉じる";
  dismissText?: string;
};

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
  nested?: boolean;
  // A row action must not inherit the hidden maintenance settings view.
  portal?: boolean;
  replacement?: ModalReplacement | null;
  // A secondary view can keep its fields mounted in this same native boundary
  // while inactive. Replacement still owns dismissal/focus, never persistence.
  persistentReplacement?: ReactNode;
};

// Native top-layer dialogs supply an inert background. Explicit Tab wrapping
// keeps focus inside even when a browser would cycle through its own chrome.
// Let showModal/close expose actual modality; closed and queued dialogs must
// not advertise an independent, permanent ARIA modal state to WebKit AX.
// Controllers retain all command, identity, consent and unknown-result guards.
export function CommonModal({ ref: suppliedRef, open, blocked = false, onDismiss, restoreTarget, restoreReady = true, dismissLabel = "キャンセル", dismissText, externalContent = false, nested = false, portal = false, replacement = null, persistentReplacement, className = "", children, ...attributes }: Props) {
  const ownRef = useRef<HTMLDialogElement>(null);
  const ref = suppliedRef ?? ownRef;
  const confirmation = useSyncExternalStore(confirmations.subscribe, confirmations.getSnapshot);
  const owned = confirmation?.parent && confirmation.parent === ref.current ? confirmation : null;
  const displayedConfirmation = owned ?? (confirmation && !confirmation.parent && attributes["aria-labelledby"] === `${confirmation.id}-title` ? confirmation : null);
  const effectiveBlocked = owned ? owned.phase === "running" : replacement ? !!replacement.blocked : blocked;
  // A confirmation temporarily replaces settings, so its cancellation label
  // must not inherit that settings dialog's custom "Close" action.
  const label = owned ? owned.phase === "failed" ? "閉じる" : "キャンセル" : replacement?.dismissLabel ?? dismissLabel;
  const customDismissText = owned ? undefined : replacement ? replacement.dismissText : dismissText;
  const suspendedOpener = useRef<HTMLElement | null>(null);
  const suspended = useRef(false);
  const suspendedScroll = useRef(0);
  const latest = useRef({ open, blocked, onDismiss, restoreTarget, restoreReady, replacement });
  latest.current = { open, blocked: effectiveBlocked, onDismiss: replacement?.onDismiss ?? onDismiss, restoreTarget, restoreReady, replacement };
  const baseBody = useRef<HTMLDivElement>(null);
  const replacedView = useRef<{ key: string; opener: HTMLElement | null; scrollTop: number } | null>(null);
  const replacementParents = useRef(new Map<string, { view: NonNullable<typeof replacedView.current>; opener: HTMLElement | null; scrollTop: number }>());
  const replacementInitialFocus = useRef(false);
  const opener = useRef<HTMLElement | null>(null);
  const parentModal = useRef<HTMLDialogElement | null>(null);
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
      const remaining = topOpenModal();
      if (!restorePending.current || latest.current.open || !latest.current.restoreReady || remaining && remaining !== parentModal.current) return;
      const candidate = latest.current.restoreTarget?.() ?? opener.current;
      const usable = (target: HTMLElement | null): target is HTMLElement => !!target?.isConnected && !target.matches(":disabled") && !!target.getClientRects().length && !target.closest("[hidden], [inert], dialog:not([open])") && (!remaining || remaining.contains(target));
      const target = usable(candidate) ? candidate : usable(fallback.current) ? fallback.current : null;
      if (target) {
        if (target === fallback.current && target.tabIndex < 0) target.tabIndex = -1;
        target.focus({ preventScroll: true });
        if (document.activeElement === target) restorePending.current = false;
      }
    });
  };
  const restoreSuspended = () => {
    const element = ref.current;
    if (!suspended.current || !element?.open || confirmations.getSnapshot()?.parent === element || topOpenModal() !== element) return;
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
    const parent = opener.current?.closest<HTMLDialogElement>("dialog[open]") ?? null;
    parentModal.current = parent && (nested || parent.hasAttribute("data-settings-modal")) ? parent : null;
    const panel = (latest.current.restoreTarget?.() ?? opener.current)?.closest("section");
    // Publishing a candidate can remove its opener. A surviving parent dialog
    // remains modal, so its heading—not the inert workspace—is the fallback.
    const boundary = parentModal.current ?? panel;
    fallback.current = boundary?.querySelector<HTMLElement>("h2, h3") ?? boundary?.querySelector<HTMLElement>("[data-dialog-cancel]:not(:disabled)") ?? null;
    restorePending.current = false;
    const attempt = () => {
      const top = topOpenModal();
      if (!latest.current.open || !element.isConnected || element.open || top && top !== parentModal.current) return;
      closeInfoPopovers();
      // Native top-layer order is independent of portal DOM order. Moving a
      // mounted host here makes macOS WebKit lose its accessibility subtree.
      element.showModal();
      modalOrder.opened(element);
      const cancel = visibleModalTarget(element, "[data-dialog-cancel]");
      const heading = visibleModalTarget(element, "h2, h3");
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
      modalOrder.closed(element);
      if (element.open) { restorePending.current = true; element.close(); }
      // Conditional reviews can still restore a surviving target on unmount.
      queueMicrotask(() => {
        if (!element.isConnected) { latest.current.open = false; restore(); }
      });
    };
  }, [open, ref, nested]);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!open || blocked || owned || replacement || !initialFocusPending.current || !element?.open || topOpenModal() !== element) return;
    const active = document.activeElement;
    if (active === element || active === document.body || active === visibleModalTarget(element, "h2, h3")) {
      const cancel = visibleModalTarget(element, "[data-dialog-cancel]");
      if (!cancel) return;
      cancel.focus({ preventScroll: true });
      element.scrollTop = 0;
    }
    initialFocusPending.current = false;
  }, [open, blocked, owned?.id, replacement?.key, ref]);
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
  useLayoutEffect(() => {
    const element = ref.current;
    if (!open || !element?.open) { replacedView.current = null; replacementParents.current.clear(); replacementInitialFocus.current = false; return; }
    if (replacement) {
      if (replacedView.current?.key !== replacement.key) {
        const previous = replacedView.current;
        const returning = replacementParents.current.get(replacement.key);
        if (previous && replacement.parentKey === previous.key) {
          replacementParents.current.set(previous.key, { view: previous, opener: replacement.restoreTarget?.() ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null), scrollTop: element.scrollTop });
        }
        if (returning) replacementParents.current.delete(replacement.key);
        replacedView.current = returning?.view ?? {
          key: replacement.key,
          opener: previous?.opener ?? replacement.restoreTarget?.() ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null),
          scrollTop: previous?.scrollTop ?? element.scrollTop,
        };
        closeInfoPopovers();
        element.scrollTop = returning?.scrollTop ?? 0;
        if (!owned && topOpenModal() === element) {
          const heading = element.querySelector<HTMLElement>(".common-modal-replacement h2, .common-modal-replacement h3");
          if (heading) heading.tabIndex = -1;
          const cancel = element.querySelector<HTMLElement>("[data-dialog-cancel]:not(:disabled)");
          const returnTarget = returning?.opener;
          const usable = returnTarget?.isConnected && element.contains(returnTarget) && !!returnTarget.getClientRects().length && !returnTarget.matches(":disabled") && !returnTarget.closest("[hidden], [inert]");
          replacementInitialFocus.current = !usable && !cancel;
          (usable ? returnTarget : cancel ?? heading)?.focus({ preventScroll: true });
        }
      }
      if (replacementInitialFocus.current && !effectiveBlocked && !owned && topOpenModal() === element) {
        const heading = element.querySelector<HTMLElement>(".common-modal-replacement h2, .common-modal-replacement h3");
        if (document.activeElement === heading || document.activeElement === element || document.activeElement === document.body) {
          element.querySelector<HTMLElement>("[data-dialog-cancel]:not(:disabled)")?.focus({ preventScroll: true });
        }
        replacementInitialFocus.current = false;
      }
      return;
    }
    replacementInitialFocus.current = false;
    const resume = () => {
      const saved = replacedView.current;
      if (!saved || !latest.current.open || latest.current.replacement || latest.current.blocked || !latest.current.restoreReady || confirmations.getSnapshot()?.parent === element || topOpenModal() !== element) return;
      const target = saved.opener;
      if (target?.isConnected && target.matches(":disabled")) return;
      const usable = target?.isConnected && element.contains(target) && !!target.getClientRects().length && !target.closest("[hidden], [inert]");
      const destination = usable ? target : baseBody.current?.querySelector<HTMLElement>("h2, h3");
      if (!destination) return;
      if (!usable) destination.tabIndex = -1;
      suspended.current = false;
      element.scrollTop = saved.scrollTop;
      destination.focus({ preventScroll: true });
      if (document.activeElement === destination) replacedView.current = null;
    };
    const observer = new MutationObserver(resume);
    observer.observe(element, { subtree: true, attributes: true, attributeFilter: ["disabled", "hidden"], childList: true });
    queueMicrotask(resume);
    return () => observer.disconnect();
  }, [replacement?.key, owned?.id, effectiveBlocked, restoreReady, open, ref]);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!displayedConfirmation || !element?.open || topOpenModal() !== element) return;
    const heading = element.querySelector<HTMLElement>(".action-confirmation h2");
    const active = document.activeElement;
    // Disabling the focused execution button can leave focus on body. Keep
    // the running confirmation inside its native boundary, then expose its
    // safe close action on failure without stealing another valid focus.
    const lost = active === document.body || active === element || active === heading || active instanceof HTMLElement && element.contains(active) && active.matches(":disabled");
    if (!lost) return;
    const cancel = element.querySelector<HTMLElement>("[data-dialog-cancel]:not(:disabled)");
    (cancel ?? heading)?.focus({ preventScroll: true });
  }, [displayedConfirmation?.id, displayedConfirmation?.phase, open, ref]);
  const dismiss = () => { if (!latest.current.blocked) { if (owned) confirmations.cancel(owned.id); else void latest.current.onDismiss(); } };
  const keyDown = (event: Pick<KeyboardEvent,"key" | "shiftKey" | "target" | "preventDefault" | "stopPropagation" | "defaultPrevented" | "isComposing" | "keyCode" | "repeat">) => {
    const element = ref.current;
    if (!element) return;
    // Portals and nested settings bubble React keys through their owner. Only
    // the native dialog containing the target may trap or dismiss that key.
    if (event.target instanceof Element && event.target.closest("dialog[open]") !== element) return;
    const guard = modalKeyGuard(event);
    if (guard === "ignore") return;
    if (guard === "consume") { event.preventDefault(); event.stopPropagation(); return; }
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
  const dialog = <dialog {...attributes} ref={ref} data-common-modal aria-label={owned || replacement ? undefined : attributes["aria-label"]} aria-labelledby={owned ? `${owned.id}-title` : replacement?.labelledBy ?? attributes["aria-labelledby"]} className={`common-modal ${className} ${replacement?.className ?? ""}`}
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
    <div className="common-modal-dismiss-row"><button type="button" className="small-secondary icon-button" aria-label={customDismissText ?? `確認を${label}`} title={customDismissText ?? label} data-dialog-cancel disabled={effectiveBlocked} onClick={dismiss}><ActionIcon name="close" /></button></div>
    {owned && <ConfirmationBody value={owned} />}
    <div ref={baseBody} className="common-modal-body" hidden={!!owned || !!replacement}>{children}</div>
    {(replacement || persistentReplacement != null) && <div className="common-modal-replacement" hidden={!replacement || !!owned}>{persistentReplacement ?? replacement?.content}</div>}
  </dialog>;
  return portal ? createPortal(dialog, document.body) : dialog;
}
