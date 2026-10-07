import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { CommonModal } from "./common-modal";
import { ConfirmationBody } from "./confirmation-body";
import { confirmations } from "./confirmation-service";

export function ConfirmationHost() {
  const value = useSyncExternalStore(confirmations.subscribe, confirmations.getSnapshot);
  const opener = useRef<HTMLElement | null>(null);
  const [, updateFocusReadiness] = useState(0);
  if (value && !value.parent) opener.current = value.opener;
  const restoreReady = !opener.current?.isConnected || !opener.current.matches(":disabled");
  useEffect(() => {
    if (value || !opener.current?.matches(":disabled")) return;
    const observer = new MutationObserver(() => updateFocusReadiness(n => n + 1));
    observer.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["disabled"], childList: true });
    return () => observer.disconnect();
  }, [value?.id]);
  useEffect(() => {
    if (!value?.parent) return;
    const parent = value.parent;
    const detach = () => { if (!parent.isConnected || !parent.open) confirmations.detachParent(parent); };
    const observer = new MutationObserver(detach);
    observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["open"] });
    document.addEventListener("close", detach, true); detach();
    return () => { observer.disconnect(); document.removeEventListener("close", detach, true); };
  }, [value?.parent]);
  return <CommonModal open={!!value && !value.parent} blocked={value?.phase === "running"} dismissLabel={value?.phase === "failed" ? "閉じる" : "キャンセル"} onDismiss={() => { if (value) confirmations.cancel(value.id); }} restoreReady={restoreReady} restoreTarget={() => opener.current} aria-labelledby={value ? `${value.id}-title` : undefined}>
    {value && !value.parent && <ConfirmationBody value={value} />}
  </CommonModal>;
}
