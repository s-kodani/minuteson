import { useEffect, useRef, useSyncExternalStore } from "react";
import { ConfirmationController, type ConfirmationOptions } from "./confirmation-state";

export const confirmations = new ConfirmationController<HTMLDialogElement, HTMLElement>();
export function useActionConfirmation(blocked: boolean) {
  const owner = useRef({}), live = useRef(true), blockedRef = useRef(blocked);
  blockedRef.current = blocked;
  const active = useSyncExternalStore(confirmations.subscribe, confirmations.getSnapshot);
  useEffect(() => { live.current = true; return () => { live.current = false; confirmations.disposeOwner(owner.current); }; }, []);
  return Object.assign((options: ConfirmationOptions & { opener?: HTMLElement | null }) => {
    if (!live.current || blockedRef.current) return Promise.resolve("cancelled" as const);
    const parent = document.querySelector<HTMLDialogElement>("dialog[data-common-modal][open]");
    const opener = options.opener !== undefined ? options.opener : document.activeElement instanceof HTMLElement ? document.activeElement : null;
    return confirmations.request(owner.current, parent, opener, { ...options, validate: () => live.current && !blockedRef.current && (options.validate?.() ?? true), onError: cause => { if (live.current) options.onError?.(cause); } }, parent?.scrollTop ?? 0);
  }, { pending: active?.owner === owner.current });
}
